use std::{
    collections::{HashMap, HashSet},
    time::Duration,
};

use super::{
    capability::{
        Capability, CapabilityId, CapabilityKind, Delegations, Denied, Operation, Restriction,
        SecurityContext, Source,
    },
    dirty_io::{Claim, Completion, DirtyIo},
    ets::EtsTables,
    host::{
        self,
        input::{self, Readiness},
        output::IovecWrite,
        signal,
    },
    interpreter::{CodeTable, LoadedModule},
    local_node,
    mailbox::Mailbox,
    persistent_term::PersistentTerms,
    port::Ports,
    process::{
        DirtyWrite, Flush, Frame, LinkId, Monitor, MonitorLinkTree, Origin, OriginMonitor,
        ProcessState, ReceiveTimer, StackSlot, Start, StartFlags, TargetMonitor, TimerReference,
        WalkPosition,
    },
    regex_code::RegexCodes,
    resources::Resources,
    signal::Signal,
    timer::{Receiver, Timer, Timers},
    tty::Ttys,
};
use crate::{
    error::{BeamError, error},
    term::{
        Atom, AtomTable, Group, LiveBytes, MapDrain, PortId, ProcessId, Reference,
        SMALL_INTEGER_MIN, Term, external::block_hash,
    },
};

/// The reduction budget of a process for one turn, as ERTS `CONTEXT_REDS`
/// (`erl_vm.h:53`). Each schedule-in gives the process this budget
/// (`erl_process.c:10183`, `10435`).
pub(super) const CONTEXT_REDS: i32 = 4000;

/// The pid of the root process, `<0.0.0>`. The launcher makes the root
/// process with it, also after it made the policy process.
const ROOT: ProcessId = ProcessId(0);

/// The reductions that `erts_do_exit_process` bumps when a process starts
/// its exit (`erl_process.c:14229`).
const EXIT_REDUCTIONS: i32 = 100;

/// The reductions that the `ERTS_CONTINUE_EXIT_FREE` phase of an exit uses
/// after it removes the process from the process table
/// (`erl_process.c:14475`).
const FREE_REDUCTIONS: i32 = 50;

/// The reductions that an exit uses to cancel each BIF timer to the
/// process, as `ERTS_BTM_CANCEL_REDS` (`erl_hl_timer.c:2404`).
const TIMER_CANCEL_REDUCTIONS: i32 = 80;

/// The reductions that an active process uses at most to handle its
/// signals at schedule-in and in `loop_rec`, as
/// `ERTS_SIG_HANDLE_REDS_MAX_PREFERED` (`erl_proc_sig_queue.h:284`).
const SIGNAL_HANDLING_REDUCTIONS: i32 = 100;

/// The reductions of a fetch that `loop_rec` does at the end of the inner
/// part: `erts_proc_sig_receive_helper` uses 4 for each fetch
/// (`erl_proc_sig_queue.c:7771-7799`).
const RECEIVE_FETCH_REDUCTIONS: i32 = 4;

/// The number of terms whose nested values one scheduler step releases, as
/// the release batch of row 11 of `docs/term-performance-design-codex-v2.md`
/// ("process it in batches of at most 256 nodes"). A process that owned a
/// large graph leaves that graph on the release work list of the scheduler
/// when it exits ([`Scheduler::release_process`]), and the scheduler walks
/// the list in this bound at each step ([`Scheduler::drain_release`]). The
/// bound is host cleanup work: it changes no guest reduction and no guest
/// value.
pub(super) const RELEASE_BATCH: usize = 256;

/// One item of the release work list of the scheduler. One scheduler step
/// moves at most [`RELEASE_BATCH`] units of the list, where a unit is one
/// term, one element of an element array, one pair of a map, one register or
/// Y slot, one dictionary pair, one mailbox entry, one monitor, one frame, or
/// one step of a map walk. A unit pushes at most two terms, so one step moves
/// at most twice the batch. The step returns every item that still holds a
/// term to the list.
#[derive(Debug)]
enum Release {
    /// A term whose nested values the scheduler still has to move. One step
    /// moves the direct children of this term.
    Term(Term),
    /// The element array of a tuple or the free variables of a fun. The array
    /// is the one that the term owned, so the group copies nothing.
    Terms(Vec<Term>),
    /// The pairs of a map.
    Map(MapDrain),
    /// The X registers of a process that left the process table.
    Registers(Vec<Option<Term>>),
    /// The frames of a process that left the process table.
    Frames(Vec<Frame>),
    /// The Y registers of one frame that a process left.
    Slots(Vec<StackSlot>),
    /// The process dictionary of a process that left the process table.
    Dictionary(Vec<(Term, Term)>),
    /// The mailbox of a process that left the process table.
    Mailbox(Mailbox),
    /// The monitors of a process that left the process table.
    Monitors(MonitorLinkTree<Reference, Origin>),
}

/// The number of slots of one chunk of the release work list. The stack
/// allocates one chunk at a time, so a push copies no item that is already on
/// the list.
const RELEASE_CHUNK: usize = 256;

/// The release work list of the scheduler: a stack of items in chunks of
/// [`RELEASE_CHUNK`] slots.
///
/// A plain `Vec` copies each item that it holds when it grows. One step of
/// [`Scheduler::drain_release`] can push many items: a chain whose every node
/// holds one pending sibling leaves one item on the list for each level. The
/// growth of a `Vec` would copy the whole pending stack inside that one step
/// and leave the [`RELEASE_BATCH`] bound. This stack allocates one new chunk
/// when its last chunk is full, and it copies no item.
#[derive(Debug)]
struct ReleaseStack {
    /// The chunks of the stack, oldest first. Each chunk holds at most
    /// [`RELEASE_CHUNK`] items.
    chunks: Vec<Vec<Release>>,
}

impl ReleaseStack {
    /// Return an empty stack.
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// assert!(ReleaseStack::new().is_empty());
    /// ```
    const fn new() -> Self {
        Self { chunks: Vec::new() }
    }

    /// Push one item on the top of the stack.
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// let mut stack = ReleaseStack::new();
    /// stack.push(Release::Term(Term::Integer(1)));
    /// assert_eq!(stack.len(), 1);
    /// ```
    fn push(&mut self, item: Release) {
        match self.chunks.last_mut() {
            Some(chunk) if chunk.len() < RELEASE_CHUNK => chunk.push(item),
            _ => {
                let mut chunk = Vec::with_capacity(RELEASE_CHUNK);
                chunk.push(item);
                self.chunks.push(chunk);
            }
        }
    }

    /// Push each item of `items` on the top of the stack, in order.
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// let mut stack = ReleaseStack::new();
    /// stack.extend([Release::Term(Term::Integer(1))]);
    /// assert_eq!(stack.len(), 1);
    /// ```
    fn extend(&mut self, items: impl IntoIterator<Item = Release>) {
        for item in items {
            self.push(item);
        }
    }

    /// Remove and return the item at the top of the stack.
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// let mut stack = ReleaseStack::new();
    /// stack.push(Release::Term(Term::Integer(1)));
    /// assert!(stack.pop().is_some());
    /// assert!(stack.is_empty());
    /// ```
    fn pop(&mut self) -> Option<Release> {
        let item = self.chunks.last_mut()?.pop()?;
        if self.chunks.last().is_some_and(|chunk| chunk.is_empty()) {
            self.chunks.pop();
        }
        Some(item)
    }

    /// Return true when the stack holds no item.
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// assert!(ReleaseStack::new().is_empty());
    /// ```
    fn is_empty(&self) -> bool {
        self.chunks.is_empty()
    }

    /// Return the number of items of the stack.
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// assert_eq!(ReleaseStack::new().len(), 0);
    /// ```
    #[cfg(test)]
    fn len(&self) -> usize {
        self.chunks.iter().map(|chunk| chunk.len()).sum()
    }

    /// Iterate over the items of the stack, oldest first. The live-byte walk
    /// of a run reads the terms of each item, so this is not a test helper.
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// assert_eq!(ReleaseStack::new().iter().count(), 0);
    /// ```
    fn iter(&self) -> impl Iterator<Item = &Release> {
        self.chunks.iter().flat_map(|chunk| chunk.iter())
    }

    /// Return the number of bytes of the chunks of the stack and of the slots
    /// of each chunk.
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// assert_eq!(ReleaseStack::new().allocated_bytes(), 0);
    /// ```
    #[cfg(test)]
    fn allocated_bytes(&self) -> usize {
        self.chunks.capacity() * std::mem::size_of::<Vec<Release>>()
            + self
                .chunks
                .iter()
                .map(|chunk| chunk.capacity() * std::mem::size_of::<Release>())
                .sum::<usize>()
    }
}

/// The target of `erlang:monitor/2`, after the BIF checked the type and the
/// target term as ERTS `monitor` does (`bif.c:758-981`).
#[derive(Debug)]
pub(super) enum MonitorTarget {
    /// The `process` type with a pid.
    Process(ProcessId),
    /// The `process` type with a registered name, or with
    /// `{Name, nonode@nohost}`.
    ProcessName(Atom),
    /// The `port` type with a registered name, or with
    /// `{Name, nonode@nohost}`.
    PortName(Atom),
    /// The `port` type with a port.
    Port(PortId),
}

/// The options of `erlang:monitor/3`, after the BIF checked them as ERTS
/// `erts_monitor_opts` does (`bif.c:701-756`).
#[derive(Debug)]
pub(super) struct MonitorOptions {
    /// The alias mode of the `alias` option, or `None` without it.
    pub(super) alias: Option<AliasMode>,
    /// The first element of the `DOWN` message: `'DOWN'`, or the `tag`
    /// option.
    pub(super) tag: Term,
    /// True with the `priority` option.
    pub(super) priority: bool,
}

/// The mode of an alias that `erlang:monitor/3` makes, as the alias state
/// of an ERTS monitor (`ERTS_ML_STATE_ALIAS_MASK`, `erl_monitor_link.h`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum AliasMode {
    /// `{alias, explicit_unalias}`: the alias stays after `demonitor` and
    /// after the `DOWN` message (`bif.c:470-482`,
    /// `erl_proc_sig_queue.c:6465-6480`).
    ExplicitUnalias,
    /// `{alias, demonitor}`: the alias ends with its monitor.
    Demonitor,
    /// `{alias, reply_demonitor}`: the alias ends with its monitor, and at
    /// the first message through it, which also removes the monitor
    /// (`erl_proc_sig_queue.c:6047-6070`).
    ReplyDemonitor,
}

/// What one scheduling step did, as one pass of `erts_schedule`
/// (`erl_process.c:9607-10505`).
#[derive(Debug, PartialEq, Eq)]
pub(super) enum Turn {
    /// The current process runs Erlang code now.
    Run,
    /// The step handled the signals of a process or ran its exit, and no
    /// process runs Erlang code.
    Done,
    /// No process is in the run queue.
    Idle,
}

/// A phase of an exit after the process left the process table, as the
/// phases `ERTS_CONTINUE_EXIT_LINKS` to `ERTS_CONTINUE_EXIT_HANDLE_PROC_SIG`
/// of `erts_continue_exit_process` (`erl_process.c:14234-14251`). A tree
/// phase keeps the position of its next entry, or `None` before its first
/// entry, and the list phase keeps the number of entries that it handled,
/// as the ERTS yield state keeps the next entry.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ExitPhase {
    /// The link tree of the process, in post-order.
    Links(Option<WalkPosition>),
    /// The monitor tree of the process, in post-order.
    Monitors(Option<WalkPosition>),
    /// The monitors of other processes and resources on the process
    /// (`ERTS_P_LT_MONITORS`): the entries of `monitored_by` from the index.
    TargetMonitors(usize),
    /// The signals in the queue of the process.
    Signals,
}

/// A scheduler on one OS thread. A process runs until it waits, exits, or
/// switches out at an ERTS switch point. It follows the ERTS model. The
/// process table holds every live process, also an exiting one, and the
/// slot of the running process. The run queue holds only processes that
/// wait for a turn. A process can be in the run queue for Erlang work or
/// only to handle its signals.
#[derive(Debug)]
pub(super) struct Scheduler {
    /// All live processes, including the current process, and the slot of
    /// the current process, like `current_process` in the ERTS scheduler
    /// data.
    processes: ProcessTable,
    /// The processes that left the process table in their exit and switched
    /// out before the end of their exit, as ERTS keeps a process in the
    /// `ERTS_PSFLG_FREE` state (`erl_process.c:14469`, `14814-14816`), with
    /// the phase where each exit continues. No pid lookup finds them. The
    /// run queue holds their pids, and each continues its exit in its next
    /// turn.
    freed: HashMap<ProcessId, (ProcessState, ExitPhase)>,
    /// Processes that wait for a turn, in the queue of their priority. The
    /// current process is not in this queue, as in ERTS.
    runnable: RunQueue,
    /// Number for the next process identifier.
    next_process_id: u64,
    /// Number for the next reference.
    next_reference: u64,
    /// The unique integer counter of the scheduler, as `esdp->unique`
    /// (`erl_bif_unique.c:574`, `812`): the number of
    /// `erlang:unique_integer/0` calls so far.
    unique: u64,
    /// The seed of the hash of `term_to_binary/2` with `local`
    /// (`local_node_hash`, external.c:134), which the first such call
    /// computes (see [`Scheduler::local_node_hash`]).
    local_node_hash: std::cell::OnceCell<u32>,
    /// Return value or exit reason of the root process. The root process
    /// records it when its exit starts, and the run ends only after the
    /// exit completes ([`Self::root_exited`]).
    root_result: Option<Result<Term, Term>>,
    /// Registered names and their processes, as the ERTS `process_reg`
    /// table. An atom name is unique in one atom table, so its text
    /// identifies it. The process also keeps its name in its
    /// `registered_name` field.
    names: HashMap<String, ProcessId>,
    /// Persistent terms of the node. All processes of the run share them.
    pub(super) persistent_terms: PersistentTerms,
    /// ETS tables of the node. All processes of the run share them.
    pub(super) ets: EtsTables,
    /// Typed values of the node, one value of each type, such as the
    /// compiled regular expressions in `RegexCodes` and the TTY resources
    /// in `Ttys`. All processes of the run share them.
    pub(super) resources: Resources,
    /// The atom table of the run, as `erts_atom_table` for one emulator. A
    /// run starts with a copy of the program table, and a BIF such as
    /// `erlang:list_to_atom/1` adds its new atoms here.
    pub(super) atoms: AtomTable,
    /// The code table of the run, with its prepared code. The launcher puts
    /// the start modules of the program into it before the root process
    /// starts, and `erlang:finish_loading/1` adds a new module.
    pub(super) code: CodeTable,
    /// The modules whose NIF library `erlang:load_nif/2` loaded in the run,
    /// as the `nif` field of the ERTS module instance
    /// (`beam/erl_nif.c:4774-4777`, `4990-4997`). PBeam has one instance of
    /// each module, so the module name identifies it.
    pub(super) nif_libraries: HashSet<String>,
    /// The timers of the run and the run clock. All processes of the run
    /// share them.
    pub(super) timers: Timers,
    /// The not connected dist entries of the run, which `erlang:nodes/1,2`
    /// gives with the node type `known`.
    known_nodes: KnownNodes,
    /// The owner of each reference that `erlang:monitor/3` made with an
    /// alias option: an ERTS pid reference holds the pid of its owner
    /// (`erts_get_pid_of_ref`, `erl_bif_unique.h:279`). The owner keeps the
    /// state of the alias in its monitor tree, so a message or an exit
    /// signal through the reference goes to the owner, which checks the
    /// alias when it handles the signal. The entries of a process leave the
    /// table when it exits.
    alias_owners: HashMap<Reference, ProcessId>,
    /// A monitor signal that the current process sent and that waits for
    /// the switch out of the process, its next signal or message to the
    /// same target, or the next monitor signal, as `esdp->pending_signal`
    /// (`erl_proc_sig_queue.c:1101-1170`). The pid is the target.
    pending_monitor: Option<(ProcessId, Monitor)>,
    /// Number for the next unlink operation identifier
    /// (`erts_proc_sig_new_unlink_id`). The values are not observable.
    next_unlink_id: u64,
    /// The process that gets the error reports of the emulator: `logger`,
    /// `undefined`, or a pid, as `system_logger` in `erl_trace.c`. The start
    /// value is `logger` (`erl_trace.c:339`).
    pub(super) system_logger: Term,
    /// The maximum number of frames that `erlang:raise/3` keeps, as
    /// `erts_backtrace_depth`. The start value is 8 (`DEFAULT_BACKTRACE_SIZE`,
    /// `error.h:207`, `erl_init.c:805`), and
    /// `erlang:system_flag(backtrace_depth, Depth)` changes it.
    pub(super) backtrace_depth: usize,
    /// The virtual reductions of the current turn, as `esdp->virtual_reds`.
    /// They use the budget, but the reduction total does not count them
    /// (`bif.h:85-96`). Each switch out sets them to 0
    /// (`erl_process.c:9688`).
    virtual_reductions: i32,
    /// The dirty IO threads of the run and the inbox of their completions.
    dirty_io: DirtyIo,
    /// The delegation records of the capability copies of the run.
    delegations: Delegations,
    /// The terms whose nested values the scheduler still has to release, most
    /// recent first. A process that left the process table moves each term
    /// that it owned here before it drops ([`Self::release_process`]), and
    /// each scheduler step releases at most [`RELEASE_BATCH`] terms of the
    /// list ([`Self::drain_release`]). The list owns each term, so a switch
    /// to another process loses no root. The final drain of a run is the
    /// drop of the scheduler, which releases the terms that are left. The
    /// list grows in chunks ([`ReleaseStack`]), so a push into it copies no
    /// item that a step of the drain already holds.
    release: ReleaseStack,
    /// The policy process of a run with a policy module, or `None` with a
    /// Rust policy.
    policy: Option<PolicyProcess>,
    /// True when the run skips each capability check, as if each effect
    /// entry were `ambient` (section "Capability" of
    /// `docs/effect-inventory.md`). The launcher sets it in
    /// [`Self::without_checks`] before the root process starts, and no BIF
    /// can change it, so a restricted process cannot turn the checks off.
    skip_checks: bool,
}

impl Scheduler {
    /// Return a scheduler with no processes and the atom table of the run.
    /// The table is a copy of the table of the loaded program, so each atom
    /// of the program code keeps its index.
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// let scheduler = Scheduler::new((*program.atoms).clone());
    /// assert_eq!(scheduler.atoms.get("ok"), Atom::from_static("ok"));
    /// ```
    pub(super) fn new(atoms: AtomTable) -> Self {
        let mut resources = Resources::default();
        resources.put(RegexCodes::default());
        resources.put(Ttys::default());
        resources.put(Ports::default());
        Scheduler {
            processes: ProcessTable::default(),
            freed: HashMap::new(),
            runnable: RunQueue::default(),
            next_process_id: ROOT.0 + 1,
            next_reference: 0,
            unique: 0,
            local_node_hash: std::cell::OnceCell::new(),
            root_result: None,
            names: HashMap::new(),
            persistent_terms: PersistentTerms::default(),
            ets: EtsTables::default(),
            resources,
            atoms,
            code: CodeTable::default(),
            nif_libraries: HashSet::new(),
            timers: Timers::new(),
            known_nodes: KnownNodes::default(),
            alias_owners: HashMap::new(),
            pending_monitor: None,
            next_unlink_id: 1,
            system_logger: Term::static_atom("logger"),
            backtrace_depth: 8,
            virtual_reductions: 0,
            dirty_io: DirtyIo::new(),
            delegations: Delegations::default(),
            release: ReleaseStack::new(),
            policy: None,
            skip_checks: false,
        }
    }

    /// Return a scheduler as [`Self::new`] does, for a run that skips each
    /// capability check: the run of a launcher with no policy. Each check
    /// passes, no process gets a capability copy, a `{capabilities, Map}`
    /// spawn option does not reduce the child, and `pbeam_cap:revoke/2`,
    /// `pbeam_cap:policy_revoke/2`, and `pbeam_cap:request/2` have no
    /// effect.
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// let scheduler = Scheduler::without_checks((*program.atoms).clone());
    /// assert!(scheduler.skips_checks());
    /// ```
    pub(super) fn without_checks(atoms: AtomTable) -> Self {
        Scheduler {
            skip_checks: true,
            ..Scheduler::new(atoms)
        }
    }

    /// Return true when the run skips each capability check (see
    /// [`Self::without_checks`]). Each check reads this first.
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// assert!(!Scheduler::new(AtomTable::new()).skips_checks());
    /// ```
    pub(super) fn skips_checks(&self) -> bool {
        self.skip_checks
    }

    /// Create the root process with the initial capabilities
    /// `capabilities`, which the launcher issues. The root process gets
    /// `ProcessId(0)`, and each other process a later pid.
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// let mut scheduler = Scheduler::new(AtomTable::new());
    /// scheduler.start(frame, vec![], AllowAll.initial())?;
    /// assert_eq!(scheduler.schedule_next(), Some(ProcessId(0)));
    /// ```
    pub(super) fn start(
        &mut self,
        frame: Frame,
        arguments: Vec<Term>,
        capabilities: Vec<Capability>,
    ) -> Result<(), BeamError> {
        let id = ROOT;
        let mut root = ProcessState::new(id, Start::Entry(frame), arguments);
        let issued = capabilities
            .into_iter()
            .map(|capability| (Source::Launcher, capability));
        root.security = self.delegations.hold(id, issued);
        self.insert(root);
        Ok(())
    }

    /// Create the root process as [`Self::start`] does, with the boot
    /// authority instead of capabilities, for a run in boot mode (see
    /// `SecurityContext::boot`). The root process enters the runtime with
    /// `pbeam_cap:enter_runtime/2`.
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// scheduler.start_boot(frame, vec![]);
    /// assert!(scheduler.process(ProcessId(0)).unwrap().security.is_boot());
    /// ```
    pub(super) fn start_boot(&mut self, frame: Frame, arguments: Vec<Term>) {
        let mut root = ProcessState::new(ROOT, Start::Entry(frame), arguments);
        root.security = SecurityContext::boot();
        self.insert(root);
    }

    /// Return whether an operation of the current process on the process
    /// `target` crosses from the runtime into the boot: the current process
    /// is not a boot process, and `target` is a live or exiting boot
    /// process. Only a run in boot mode has boot processes. The runtime to
    /// boot gate of `erlang:send/2,3`, the `send` instruction,
    /// `erlang:start_timer/3`, `erlang:link/1`, `erlang:exit/2`, the group
    /// leader BIFs, the process inspection BIFs, and `erlang:unregister/1`
    /// denies such an operation (row "Boot authority" of
    /// `docs/erts-differences.md`).
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// // The current process entered the runtime; `user` is a boot process.
    /// assert!(scheduler.enters_boot(scheduler.whereis("user").unwrap()));
    /// ```
    pub(super) fn enters_boot(&self, target: ProcessId) -> bool {
        !self.current_process().security.is_boot()
            && self
                .processes
                .get(&target)
                .is_some_and(|process| process.security.is_boot())
    }

    /// Register the live boot process `target` as an IO service, for
    /// `pbeam_cap:serve(Pid, io)` of a boot process (see
    /// `SecurityContext::serve_io`). Return `false` and change nothing when
    /// no live boot process has the pid `target`.
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// // In boot mode, before the root process enters the runtime.
    /// assert!(scheduler.serve_io(scheduler.whereis("user").unwrap()));
    /// assert!(scheduler.serves_io(scheduler.whereis("user").unwrap()));
    /// ```
    pub(super) fn serve_io(&mut self, target: ProcessId) -> bool {
        self.processes
            .get_mut(&target)
            .filter(|process| process.exiting.is_none())
            .is_some_and(|process| process.security.serve_io())
    }

    /// Return whether the process `target` is an IO service (see
    /// [`Self::serve_io`]).
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// assert!(!scheduler.serves_io(ProcessId(99)));
    /// ```
    pub(super) fn serves_io(&self, target: ProcessId) -> bool {
        self.processes
            .get(&target)
            .is_some_and(|process| process.security.serves_io())
    }

    /// Store a process and add it to the end of the run queue.
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// scheduler.insert(ProcessState::new(ProcessId(1), Start::Entry(frame), vec![]));
    /// ```
    pub(super) fn insert(&mut self, process: ProcessState) {
        let id = process.id;
        self.processes.insert(process);
        self.enqueue(id);
    }

    /// Put the process `id` at the end of the run queue of its priority
    /// ([`RunQueue::push`]), unless it is in the run queue or runs now: such a process keeps its place, as
    /// `erts_schedule_process` and `erts_proc_sys_schedule` do
    /// (`erl_proc_sig_queue.h:2063-2088`). A process whose dirty IO call
    /// runs goes to the run queue only when it has signals to handle: its
    /// turn then handles them for it, as ERTS sends its pid to
    /// `erts_dirty_process_signal_handler` (`erts_proc_notify_new_sig`,
    /// `erl_proc_sig_queue.h:2062-2089`; `erl_proc_sig_queue.c:979-1008`).
    /// It goes there to run when the call completes
    /// ([`Self::complete_dirty_writes`]).
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// scheduler.enqueue(child);
    /// ```
    fn enqueue(&mut self, id: ProcessId) {
        let current = self.processes.current_id() == Some(id);
        let process = self.processes.get_mut(&id).expect("the process is alive");
        let idle_dirty = process.dirty_running() && !process.mailbox.has_control_signals();
        if !process.in_run_queue && !current && !idle_dirty {
            process.in_run_queue = true;
            self.runnable.push(id, process.priority);
        }
    }

    /// Give the process `id` Erlang work when it has none, as
    /// `erts_proc_notify_new_message` (`erl_proc_sig_queue.h:2080-2088`)
    /// does: a waiting process becomes active and goes to the run queue.
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// scheduler.activate(child);
    /// ```
    fn activate(&mut self, id: ProcessId) {
        let process = self.processes.get_mut(&id).expect("the process is alive");
        if std::mem::take(&mut process.waiting) {
            self.enqueue(id);
        }
    }

    /// Release at most `budget` units of the release work list, and leave the
    /// rest for the next step. A unit is one term, one element of an element
    /// array, one pair of a map, one register or Y slot, one dictionary pair,
    /// one mailbox entry, one monitor, one frame, or one step of a map walk.
    /// One unit pushes at most two terms.
    ///
    /// One term takes its direct children onto the list with
    /// [`Term::take_nested`] and then drops its own storage; the element array
    /// of a tuple or of a fun goes to the list as a group, a map goes there as
    /// a [`MapDrain`] walk, and the containers of an exited process go there
    /// as containers. Thus one call moves at most `budget` units, walks no
    /// node twice, and uses no Rust recursion. The list owns each term, so a
    /// term that this method does not reach stays alive.
    ///
    /// The method calls no guest code, charges no reduction, and changes no
    /// value. A caller passes [`RELEASE_BATCH`], so that a process that waits
    /// to run waits for at most that work (row 11 of
    /// `docs/term-performance-design-codex-v2.md`).
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// scheduler.release.push(Release::Term(Term::tuple(vec![Term::Integer(1)])));
    /// scheduler.drain_release(RELEASE_BATCH);
    /// assert!(scheduler.release.is_empty());
    /// ```
    fn drain_release(&mut self, budget: usize) {
        let mut moved = 0;
        let mut terms = Vec::new();
        while moved < budget {
            let Some(item) = self.release.pop() else {
                return;
            };
            let left = budget - moved;
            match item {
                Release::Term(mut term) => {
                    moved += 1;
                    match term.take_group() {
                        Some(Group::Terms(group)) => self.release.push(Release::Terms(group)),
                        Some(Group::Map(drain)) => self.release.push(Release::Map(drain)),
                        None => {
                            // A cons cell, a binary, or an immediate. A cons
                            // cell moves two terms.
                            terms.clear();
                            term.take_nested(&mut terms);
                            self.release.extend(terms.drain(..).map(Release::Term));
                        }
                    }
                }
                Release::Terms(mut group) => {
                    let take = group.len().min(left);
                    moved += take;
                    if take == group.len() {
                        self.release.extend(group.into_iter().map(Release::Term));
                    } else {
                        // The list takes the terms from the end of the group,
                        // as `Term::take_nested` puts them on a work stack.
                        // The group goes under the moved terms, so that the
                        // next step releases them before it moves more.
                        let rest = group.split_off(group.len() - take);
                        self.release.push(Release::Terms(group));
                        self.release.extend(rest.into_iter().map(Release::Term));
                    }
                }
                Release::Map(mut drain) => {
                    terms.clear();
                    moved += drain.step(&mut terms, left);
                    // The drain goes under the terms that it moved, so that
                    // the next step releases them before it moves more.
                    if !drain.is_empty() {
                        self.release.push(Release::Map(drain));
                    }
                    self.release.extend(terms.drain(..).map(Release::Term));
                }
                Release::Registers(mut registers) => {
                    let take = registers.len().min(left);
                    moved += take;
                    // The registers that this step did not move go under the
                    // terms that it moved, so that the next step releases them
                    // before it moves more.
                    let rest = registers.split_off(registers.len() - take);
                    if !registers.is_empty() {
                        self.release.push(Release::Registers(registers));
                    }
                    for term in rest.into_iter().flatten() {
                        self.release.push(Release::Term(term));
                    }
                }
                Release::Frames(mut frames) => {
                    let Some(frame) = frames.pop() else {
                        continue;
                    };
                    moved += 1;
                    if !frames.is_empty() {
                        self.release.push(Release::Frames(frames));
                    }
                    self.release.push(Release::Slots(frame.stack));
                }
                Release::Slots(mut slots) => {
                    let take = slots.len().min(left);
                    moved += take;
                    let rest = slots.split_off(slots.len() - take);
                    if !slots.is_empty() {
                        self.release.push(Release::Slots(slots));
                    }
                    for slot in rest {
                        match slot {
                            StackSlot::Empty | StackSlot::Catch(_) => {}
                            StackSlot::Term(term) => self.release.push(Release::Term(term)),
                        }
                    }
                }
                Release::Dictionary(mut dictionary) => {
                    let take = dictionary.len().min(left);
                    moved += take;
                    let rest = dictionary.split_off(dictionary.len() - take);
                    if !dictionary.is_empty() {
                        self.release.push(Release::Dictionary(dictionary));
                    }
                    for (key, value) in rest {
                        self.release.push(Release::Term(key));
                        self.release.push(Release::Term(value));
                    }
                }
                Release::Mailbox(mut mailbox) => {
                    terms.clear();
                    moved += mailbox.take_terms_bounded(&mut terms, left);
                    if !mailbox.is_empty() {
                        self.release.push(Release::Mailbox(mailbox));
                    }
                    self.release.extend(terms.drain(..).map(Release::Term));
                }
                Release::Monitors(mut monitors) => {
                    let mut taken = 0;
                    terms.clear();
                    while taken < left {
                        let Some(origin) = monitors.take_value() else {
                            break;
                        };
                        taken += 1;
                        if let Some(monitor) = origin.monitor {
                            terms.push(monitor.item);
                            terms.push(monitor.tag);
                        }
                    }
                    moved += taken;
                    if !monitors.is_empty() {
                        self.release.push(Release::Monitors(monitors));
                    }
                    self.release.extend(terms.drain(..).map(Release::Term));
                }
            }
        }
    }

    /// Move each term of the process `process` to the release work list, and
    /// leave the process with no term. The exit of the process calls this
    /// before the process drops, so that the release of a large graph runs
    /// in bounded scheduler work ([`Self::drain_release`]) and not in the
    /// drop of the process. The list owns each term, so the process loses no
    /// root.
    ///
    /// The process must have left the process table: no guest code can read
    /// its registers, its frames, its dictionary, or its mailbox again.
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// scheduler.release_process(process);
    /// assert!(!scheduler.release.is_empty());
    /// ```
    fn release_process(&mut self, mut process: ProcessState) {
        // The containers of the process go to the list as containers, so that
        // the exit step pays no walk of a large dictionary, mailbox, frame
        // stack, or monitor tree, and the later steps extract their terms in
        // batches.
        if let Some(bif) = process.pending_bif.take() {
            self.release.push(Release::Terms(bif.arguments));
        }
        if let Some(arguments) = process.length_trap.take() {
            self.release.push(Release::Terms(arguments));
        }
        if let Some(reason) = process.exiting.take() {
            self.release.push(Release::Term(reason));
        }
        self.release
            .push(Release::Registers(std::mem::take(&mut process.registers)));
        self.release
            .push(Release::Frames(std::mem::take(&mut process.frames)));
        self.release
            .push(Release::Mailbox(std::mem::take(&mut process.mailbox)));
        self.release
            .push(Release::Dictionary(process.dictionary.take_entries()));
        self.release
            .push(Release::Monitors(std::mem::take(&mut process.monitors)));
    }

    /// Return the number of items of the release work list. The list is
    /// private to the scheduler, and a test of the run loop of another module
    /// reads this count to show that the loop released the work of an exited
    /// process before it waited (row 11 of
    /// `docs/term-performance-design-codex-v2.md`, task
    /// [#559](https://github.com/fahchen/pbeam/issues/559)).
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// assert_eq!(scheduler.release_len(), 0);
    /// ```
    #[cfg(test)]
    pub(super) fn release_len(&self) -> usize {
        self.release.len()
    }

    /// Run one scheduling step, as `erts_schedule` does after it takes a
    /// process from the run queue (`erl_process.c:10318-10460`). The process
    /// becomes the current process with a budget of `CONTEXT_REDS`
    /// reductions (`erl_process.c:10183`).
    ///
    /// - An exiting process runs its exit now. A freed process continues its
    ///   exit at its phase ([`Self::finish_current`]).
    /// - A process whose dirty IO call runs does not run. Its turn handles
    ///   its signals ([`Self::handle_dirty_signals`]).
    /// - A process with signals to handle handles them first. An active
    ///   process, which has Erlang work, uses at most
    ///   `ERTS_SIG_HANDLE_REDS_MAX_PREFERED` (100) reductions for it, and a
    ///   waiting process or a process in a flush uses its budget
    ///   (`erl_process.c:10318-10352`). The handling uses reductions of the
    ///   budget. A signal that ends the process makes it exit in this step.
    /// - A process with no budget left, or with no Erlang work, switches out
    ///   with no Erlang code (`erl_process.c:10375`, `10397`).
    /// - A process whose receive timer expired continues at the instruction
    ///   after its `wait_timeout` with `F_TIMO`, also when a message woke it
    ///   first (`erl_process.c:10451-10458`).
    /// - A step with no runnable process returns [`Turn::Idle`], except when
    ///   the release work list still holds a term: that step returns
    ///   [`Turn::Done`], so that the run loop keeps stepping and the release
    ///   completes (row 11 of `docs/term-performance-design-codex-v2.md`).
    ///
    /// Return [`Turn::Run`] when the current process runs Erlang code now.
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// assert_eq!(scheduler.schedule_next()?, Turn::Run);
    /// assert_eq!(scheduler.current_process_mut().fcalls, CONTEXT_REDS);
    /// ```
    ///
    /// # Errors
    ///
    /// An error of [`Scheduler::handle_signals`].
    pub(super) fn schedule_next(&mut self) -> Result<Turn, BeamError> {
        // The release work of an exiting process runs in this bound, so the
        // process that this step runs waits for the release of at most
        // [`RELEASE_BATCH`] nodes.
        self.drain_release(RELEASE_BATCH);
        let Some(id) = self.runnable.pop() else {
            // A step that releases work reports `Done`, so that the run loop
            // keeps stepping and the release completes. An `Idle` step would
            // let the loop wait for input while most of a dead graph stays
            // retained, and nothing would wake it to finish the release.
            return Ok(if self.release.is_empty() {
                Turn::Idle
            } else {
                Turn::Done
            });
        };
        if let Some((process, phase)) = self.freed.remove(&id) {
            self.exit_links(process, phase, CONTEXT_REDS)?;
            return Ok(Turn::Done);
        }
        if self.processes[&id].dirty_running() {
            self.handle_dirty_signals(id)?;
            return Ok(Turn::Done);
        }
        let process = self.processes.set_current(id);
        process.in_run_queue = false;
        process.fcalls = CONTEXT_REDS;
        if let Some(reason) = process.exiting.clone() {
            self.finish_current(reason, None)?;
            return Ok(Turn::Done);
        }
        if process.mailbox.has_control_signals() {
            let reds = if process.waiting || process.flush != Flush::None {
                process.fcalls
            } else {
                SIGNAL_HANDLING_REDUCTIONS
            };
            let reductions = self.handle_signals(id, reds, reds, HandlingPoint::ScheduleIn)?;
            let process = self.current_process_mut();
            process.fcalls -= reductions;
            if process.fcalls > 0
                && let Some(reason) = process.exiting.clone()
            {
                self.finish_current(reason, None)?;
                return Ok(Turn::Done);
            }
        }
        let process = self.current_process_mut();
        if process.fcalls <= 0 || process.waiting {
            if process.waiting {
                self.park_current();
            } else {
                self.yield_current();
            }
            return Ok(Turn::Done);
        }
        process.hibernated = false;
        if let ReceiveTimer::Expired { resume } = process.receive_timer
            && let Some(frame) = process.frames.last_mut()
        {
            frame.next_instruction = resume;
            process.receive_timer = ReceiveTimer::Timo;
        }
        Ok(Turn::Run)
    }

    /// Expire each timer that is due at the time `now`, in end order. A
    /// process whose receive timer expires gets Erlang work when it waits,
    /// and goes to the end of the run queue when it is not there; a process
    /// with Erlang work stays as it is (`proc_timeout_common`,
    /// `erl_hl_timer.c:692-706`). A BIF timer sends its message to its pid,
    /// or to the process that has its name now, and a name that no process
    /// has drops the message (`bif_timer_timeout`,
    /// `erl_hl_timer.c:1146-1203`).
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// scheduler.expire_timers(scheduler.timers.now());
    /// ```
    pub(super) fn expire_timers(&mut self, now: i64) {
        while let Some(timer) = self.timers.pop_due(now) {
            match timer {
                Timer::Receive(id) => {
                    if self
                        .processes
                        .get_mut(&id)
                        .is_some_and(ProcessState::time_out)
                    {
                        self.enqueue(id);
                    }
                }
                Timer::Bif {
                    reference,
                    receiver,
                    message,
                } => {
                    self.forget_bif_timer(&receiver, reference);
                    let target = match receiver {
                        Receiver::Process(id) => Some(id),
                        Receiver::Name(name) => self.whereis(self.atoms.name_of(&name)),
                    };
                    if let Some(target) = target {
                        self.deliver(target, message);
                    }
                }
                Timer::Accept { port, caller } => self.tcp_accept_timeout(port, caller),
                Timer::SocketReceive { port } => self.tcp_receive_timeout(port),
            }
        }
    }

    /// Wait for at most `timeout`, or without end for `None`, until fd 0
    /// has input, when fd 0 has a read select, until a pipe of a port has
    /// input, until a dirty IO call completes, when a call runs, or until a
    /// handled host signal arrives, when a handler is installed. ERTS waits
    /// for these events in its poll thread (`erl_process.c:3372`), and an
    /// idle scheduler sleeps in its poll until the next timer end
    /// (`erl_process.c:3536-3567`). PBeam has one thread, so it waits only
    /// when no process can run. A signal stops the wait with `EINTR`, or
    /// makes the notify pipe readable when it arrives before the wait.
    ///
    /// When fd 0 has input, its end of file, or an error, the select ends,
    /// and its recipient gets `{select, TTY, Reference, ready_input}`
    /// (`erl_check_io.c:2135-2171`, `prepare_select_msg` at
    /// `erl_check_io.c:699-760`). A message to a process that does not exist
    /// is dropped (`erl_check_io.c:851-856`). When fd 0 is not open, the
    /// select ends without a message, as a select that `erts_poll_control`
    /// cannot add (`erl_check_io.c:1422-1434`). Each ready pipe of a port
    /// goes to [`Self::port_ready`]. A completion only wakes the scheduler;
    /// [`Self::complete_dirty_writes`] takes it. The next scheduling step
    /// reads the signals in the notify pipe (see [`Self::check_signals`]).
    ///
    /// Return `false` when fd 0 has no select, no port waits for a pipe, no
    /// dirty IO call runs, and no handler is installed, and the function
    /// does not wait.
    ///
    /// # Errors
    ///
    /// A ready pipe of a port returns the error of [`Self::port_ready`].
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// // stdin is `/dev/null`: the reader gets its message at once.
    /// assert!(scheduler.wait_events(None)?);
    /// assert!(!scheduler.wait_events(None)?);
    /// ```
    pub(super) fn wait_events(&mut self, timeout: Option<Duration>) -> Result<bool, BeamError> {
        let selected = self.resources.borrow_mut::<Ttys>().input_selected();
        let dirty = self.dirty_io.running();
        let ports = self.resources.borrow_mut::<Ports>().wait_fds();
        let writes = self.resources.borrow_mut::<Ports>().write_fds();
        let fds: Vec<_> = [
            selected.then_some(libc::STDIN_FILENO),
            dirty.then(|| self.dirty_io.wake_fd()),
            signal::notify_fd(),
        ]
        .into_iter()
        .flatten()
        .chain(ports)
        .collect();
        if fds.is_empty() && writes.is_empty() {
            return Ok(false);
        }
        let (readiness, writable) = input::wait(&fds, &writes, timeout);
        if dirty
            && matches!(&readiness, Readiness::Ready(ready) if ready.contains(&self.dirty_io.wake_fd()))
        {
            self.dirty_io.clear_wake();
        }
        // Only fd 0 can be closed: the pipes of a port, the notify pipe, and
        // the wake socket stay open while the scheduler waits for them. The
        // select of fd 0 ends when fd 0 is ready or not open.
        if selected && readiness.ended(libc::STDIN_FILENO) {
            let ttys = self.resources.borrow_mut::<Ttys>();
            let select = ttys.take_input_select().expect("fd 0 has a select");
            if readiness != Readiness::Invalid {
                let message = Term::tuple(vec![
                    Term::static_atom("select"),
                    Term::Reference(select.tty),
                    select.reference,
                    Term::static_atom("ready_input"),
                ]);
                self.deliver(select.recipient, message);
            }
        }
        for fd in readiness.into_ready() {
            self.port_ready(fd)?;
        }
        for fd in writable {
            self.port_writable(fd)?;
        }
        Ok(true)
    }

    /// Pass each handled host signal that arrived since the last scheduling
    /// step to [`Self::notify_signals`]. The check reads one flag and makes
    /// no system call when no signal arrived. ERTS passes a signal at once
    /// from its signal dispatcher thread (`sys.c:1122-1180`); PBeam passes
    /// it at the next scheduling step.
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// scheduler.check_signals()?;
    /// ```
    ///
    /// # Errors
    ///
    /// The errors of [`Self::notify_signals`].
    pub(super) fn check_signals(&mut self) -> Result<(), BeamError> {
        self.notify_signals(&signal::take_notified())
    }

    /// Do the action of each handled host signal, in the order of arrival,
    /// as the ERTS signal dispatcher thread does (`sys.c:1163-1176`). For a
    /// signal of `signum_to_signalterm`, the process registered as
    /// `erl_signal_server` gets `{notify, Signal}`, and nothing happens when
    /// no process has the name (`signal_notify_requested` at
    /// `sys.c:589-607`).
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// scheduler.notify_signals(&[libc::SIGTERM])?;
    /// // erl_signal_server has the message {notify, sigterm}.
    /// ```
    ///
    /// # Errors
    ///
    /// `SIGINT` gives a runtime error: ERTS runs its break handler
    /// (`break_requested` at `sys.c:611-624`, `do_break` at
    /// `break.c:571-687`), which PBeam does not have.
    fn notify_signals(&mut self, signals: &[libc::c_int]) -> Result<(), BeamError> {
        for &number in signals {
            if number == libc::SIGINT {
                return Err(error(
                    "unsupported break of SIGINT: PBeam has no break handler",
                ));
            }
            let name =
                signal::signal_name(number).expect("a handler is set only for a named signal");
            if let Some(server) = self.whereis("erl_signal_server") {
                let message =
                    Term::tuple(vec![Term::static_atom("notify"), Term::static_atom(name)]);
                self.deliver(server, message);
            }
        }
        Ok(())
    }

    /// Run the `prim_tty:write_nif/2` call of the current process on a
    /// dirty IO thread, and switch the process out until the call
    /// completes. The process records the call before the thread gets it,
    /// so the completion always finds the call. ERTS moves the process to a
    /// dirty IO scheduler in the same way (`static_schedule_dirty_nif`,
    /// `erl_nif.c:3437-3477`; `erl_process.c:10244-10262`). The other
    /// processes run while the call runs. `tail` tells whether the NIF call
    /// was a tail call.
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// scheduler.start_dirty_write(write, true);
    /// assert_eq!(scheduler.schedule_next()?, Turn::Idle);
    /// ```
    pub(super) fn start_dirty_write(&mut self, write: IovecWrite, tail: bool) {
        let call = self.dirty_io.next_call();
        let process = self
            .processes
            .current_mut()
            .expect("the scheduler has a current process");
        let id = process.id;
        let record = process.dirty_write.insert(DirtyWrite {
            call,
            tail,
            written: None,
            claim: Claim::default(),
        });
        self.dirty_io.submit(id, call, &record.claim, write);
        // `enqueue` keeps the process out of the run queue while the call
        // runs, unless it has signals to handle (`erl_process.c:9762-9771`).
        self.switch_out(|_| {});
    }

    /// Handle the signals of the process `id`, whose dirty IO call runs, as
    /// `erts_dirty_process_signal_handler` handles them for a process that
    /// runs dirty (`erts_dirty_process_signal_handler.erl:52-89`;
    /// `erts_internal_dirty_process_handle_signals_1`,
    /// `erl_proc_sig_queue.c:8892-8963`). The process does not become the
    /// current process, and its reduction total does not change: ERTS gives
    /// the reductions of the handling to the handler process.
    ///
    /// - A signal that ends the process drops the claim on the call, so a
    ///   call in the queue does not run, and the scheduler drops the result
    ///   of a call that runs. The process goes to the end of the run queue
    ///   and exits in its next turn while the call runs, as
    ///   `set_self_exiting` puts it in a normal run queue
    ///   (`erl_process.c:13484-13548`) and a normal scheduler runs the exit
    ///   of a process that runs dirty (`erl_process.c:10092-10110`).
    /// - A process with signals left goes to the end of the run queue, as
    ///   the handler sends the pid to itself again
    ///   (`erts_dirty_process_signal_handler.erl:83-89`).
    ///
    /// The handling uses at most `CONTEXT_REDS` reductions, about the
    /// budget left to the handler process in its call.
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// scheduler.handle_dirty_signals(writer)?;
    /// ```
    ///
    /// # Errors
    ///
    /// An error of [`Scheduler::handle_signals`].
    fn handle_dirty_signals(&mut self, id: ProcessId) -> Result<(), BeamError> {
        // `enqueue` puts a process whose call runs in the run queue only
        // when it has signals to handle, and only its turn handles them.
        self.processes
            .get_mut(&id)
            .expect("the process is alive")
            .in_run_queue = false;
        self.handle_signals(id, CONTEXT_REDS, CONTEXT_REDS, HandlingPoint::ScheduleIn)?;
        let process = self.processes.get_mut(&id).expect("the process is alive");
        if process.exiting.is_some() {
            process.dirty_write = None;
        }
        // `enqueue` takes the process when it exits or has signals left.
        self.enqueue(id);
        Ok(())
    }

    /// Take each completion from the dirty IO inbox: the process of the
    /// call gets the result and goes to the end of the run queue, as the
    /// end of a dirty NIF call moves the process back to a normal run queue
    /// (`erl_nif.c:462-463`, `507-532`). Its next turn returns the result
    /// (`dirty_nif_finalizer`, `erl_nif.c:3386-3408`) or raises the
    /// exception of the call (`dirty_nif_exception`, `erl_nif.c:3410-3435`).
    ///
    /// The completion of a process that exits or exited is dropped, as
    /// `erts_call_dirty_nif` drops the result of an exiting process
    /// (`erl_nif.c:507-509`, `542`). Process identifiers and call
    /// identifiers are not used again in a run, so a late completion does
    /// not find a new process or a new call.
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// scheduler.complete_dirty_writes();
    /// ```
    ///
    /// # Panics
    ///
    /// A completion of a process that does not exit does not match its
    /// running call, or has no result. Such a process holds the claim on
    /// its call until the completion, and each call completes once.
    pub(super) fn complete_dirty_writes(&mut self) {
        while let Some(Completion {
            process,
            call,
            written,
        }) = self.dirty_io.take_completion()
        {
            let Some(caller) = self
                .processes
                .get_mut(&process)
                .filter(|caller| caller.exiting.is_none())
            else {
                continue;
            };
            let dirty_write = caller
                .dirty_write
                .as_mut()
                .filter(|write| write.call == call && write.written.is_none())
                .expect("a completion resumes the running call of its live process");
            dirty_write.written =
                Some(written.expect("a call runs while its live process holds the claim"));
            self.enqueue(process);
        }
    }

    /// Cancel the BIF timer `reference`, as `cancel_bif_timer` does
    /// (`erl_hl_timer.c:1895-1971`). Return its end tick, or `None` when no
    /// BIF timer with the reference is active. The timer leaves the timer
    /// tree of its receiver (see [`Self::forget_bif_timer`]).
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// assert_eq!(scheduler.cancel_bif_timer(reference), Some(1100));
    /// assert_eq!(scheduler.cancel_bif_timer(reference), None);
    /// ```
    pub(super) fn cancel_bif_timer(&mut self, reference: Reference) -> Option<i64> {
        let (end, timer) = self.timers.cancel_bif(reference)?;
        if let Timer::Bif { receiver, .. } = timer {
            self.forget_bif_timer(&receiver, reference);
        }
        Some(end)
    }

    /// Remove the BIF timer `reference` from the timer tree of the process
    /// `receiver` when the process does not exit, as `bif_timer_timeout` and
    /// `cancel_bif_timer` do (`erl_hl_timer.c:1180-1190`, `1904-1911`). An
    /// exiting process keeps the timer in its tree, and its exit uses the
    /// reductions of the timer.
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// scheduler.forget_bif_timer(&Receiver::Process(id), reference);
    /// ```
    fn forget_bif_timer(&mut self, receiver: &Receiver, reference: Reference) {
        if let Receiver::Process(id) = receiver
            && let Some(process) = self.processes.get_mut(id)
            && process.exiting.is_none()
        {
            process.bif_timers.remove(&TimerReference(reference));
        }
    }

    /// Start the receive timer of the current process for the
    /// `wait_timeout` value `timeout` at the time `now`, as
    /// `erts_set_proc_timer_term` (`erl_hl_timer.c:2786-2805`) does. The
    /// process continues at the instruction index `resume` when the timer
    /// expires. Return false and start no timer for a value that is not an
    /// integer from 0 to 4294967295, or whose end is after
    /// `ERTS_MONOTONIC_END`.
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// assert!(scheduler.set_receive_timer(1_000_000_000, &Term::Integer(100), 7));
    /// assert!(!scheduler.set_receive_timer(1_000_000_000, &Term::Integer(-1), 7));
    /// ```
    pub(super) fn set_receive_timer(&mut self, now: i64, timeout: &Term, resume: usize) -> bool {
        let end = match *timeout {
            Term::Integer(msec) if msec >> 32 == 0 => Timers::timeout_position(now, msec),
            _ => None,
        };
        let Some(end) = end else {
            return false;
        };
        let id = self.current_process_mut().id;
        let key = self.timers.start(end, Timer::Receive(id));
        self.current_process_mut().receive_timer = ReceiveTimer::Set { key, resume };
        true
    }

    /// Stop the receive timer of the current process and clear `F_TIMO`, as
    /// `CANCEL_TIMER` (`erl_process.h:1841-1849`) does in `remove_message`.
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// scheduler.cancel_receive_timer();
    /// assert_eq!(scheduler.current_process_mut().receive_timer, ReceiveTimer::None);
    /// ```
    pub(super) fn cancel_receive_timer(&mut self) {
        let process = self.current_process_mut();
        let state = std::mem::replace(&mut process.receive_timer, ReceiveTimer::None);
        if let ReceiveTimer::Set { key, .. } = state {
            self.timers.cancel(key);
        }
    }

    /// Return the current process for a read, for example a capability
    /// check that does not change the process.
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// scheduler.schedule_next();
    /// assert_eq!(scheduler.current_process().id, ProcessId(0));
    /// ```
    pub(super) fn current_process(&self) -> &ProcessState {
        // ponytail: only the run loop calls this, after `schedule_next` returned a process.
        self.processes
            .current()
            .expect("the scheduler has a current process")
    }

    /// Return the current process.
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// scheduler.schedule_next();
    /// assert_eq!(scheduler.current_process_mut().id, ProcessId(0));
    /// ```
    pub(super) fn current_process_mut(&mut self) -> &mut ProcessState {
        // ponytail: only the run loop calls this, after `schedule_next` returned a process.
        self.processes
            .current_mut()
            .expect("the scheduler has a current process")
    }

    /// Return the current process and the atom table of the run together.
    /// A read of an atom name needs the table at the same time as the
    /// process. The two borrows are of different fields, so they do not
    /// conflict.
    ///
    /// # Panics
    ///
    /// Panic when the scheduler has no current process, as
    /// [`Self::current_process_mut`] does.
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// scheduler.schedule_next();
    /// let (process, atoms) = scheduler.current_process_and_atoms();
    /// assert_eq!(process.id, ProcessId(0));
    /// assert_eq!(atoms.name(506), Some("ok"));
    /// ```
    pub(super) fn current_process_and_atoms(&mut self) -> (&mut ProcessState, &AtomTable) {
        let process = self
            .processes
            .current_mut()
            .expect("the scheduler has a current process");
        (process, &self.atoms)
    }

    /// Return the module of the active frame of the current process from
    /// the code table of the run, with the export targets of its imports.
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// assert_eq!(scheduler.current_module()?.code.name, "process_probe");
    /// ```
    ///
    /// # Errors
    ///
    /// An identity that no loaded module has is a runtime invariant error.
    pub(super) fn current_module(&self) -> Result<&LoadedModule, BeamError> {
        // ponytail: only the run loop calls this, for a process with an active frame.
        let frame = self
            .processes
            .current()
            .and_then(|process| process.frames.last())
            .expect("the current process has an active frame");
        self.code.loaded_by_id(frame.module_id)
    }

    /// Return the pid of each process of the process table, also the
    /// current process and an exiting process, in ascending pid order, as
    /// `erlang:processes/0` lists the process table in index order
    /// (`erts_ptab_list`, `erl_ptab.c:1006-1180`).
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// assert_eq!(scheduler.process_ids(), vec![ProcessId(0), ProcessId(2)]);
    /// ```
    pub(super) fn process_ids(&self) -> Vec<ProcessId> {
        let mut ids: Vec<ProcessId> = self.processes.values().map(|process| process.id).collect();
        ids.sort_unstable();
        ids
    }

    /// Return the process `id` of the process table, also the current
    /// process and an exiting process, as `erts_proc_lookup_raw`
    /// (`erl_process_lock.h:1135-1148`) finds it. Return `None` when no
    /// process has the pid.
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// scheduler.schedule_next()?;
    /// assert_eq!(scheduler.process(ProcessId(0)).map(|process| process.id), Some(ProcessId(0)));
    /// assert!(scheduler.process(ProcessId(99)).is_none());
    /// ```
    pub(super) fn process(&self, id: ProcessId) -> Option<&ProcessState> {
        self.processes.get(&id)
    }

    /// Return the process `id` when it does not exit, as `erts_proc_lookup`
    /// (`erl_process_lock.h:1151-1157`) finds it.
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// assert!(scheduler.live_process(ProcessId(0)).is_some());
    /// ```
    pub(super) fn live_process(&self, id: ProcessId) -> Option<&ProcessState> {
        self.processes
            .get(&id)
            .filter(|process| process.exiting.is_none())
    }

    /// Return whether a process traps in a list insert of `ets:insert/2`
    /// into the table `id`: its pending BIF is the internal export
    /// `ets:insert/6` with the table reference (see `ets::insert_list`).
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// assert!(!scheduler.traps_ets_insert(Reference(0)));
    /// ```
    pub(super) fn traps_ets_insert(&self, id: Reference) -> bool {
        self.processes.values().any(|process| {
            process.pending_bif.as_ref().is_some_and(|pending| {
                (pending.module, pending.function) == ("ets", "insert")
                    && pending.arguments.first() == Some(&Term::Reference(id))
            })
        })
    }

    /// Return the live process `id` for a change, as [`Scheduler::process`]
    /// returns it.
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// let process = scheduler.process_mut(ProcessId(0)).unwrap();
    /// process.mailbox.fetch();
    /// ```
    pub(super) fn process_mut(&mut self, id: ProcessId) -> Option<&mut ProcessState> {
        self.processes.get_mut(&id)
    }

    /// Use `reductions` of the budget of the current process as virtual
    /// reductions, as `ERTS_VBUMP_REDS` (`bif.h:85-96`) does: the budget
    /// goes down, but not below 0, and the virtual reductions get the used
    /// amount.
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// scheduler.current_process_mut().fcalls = 6;
    /// scheduler.vbump_reductions(4);
    /// scheduler.vbump_reductions(4);
    /// assert_eq!(scheduler.current_process_mut().fcalls, 0);
    /// assert_eq!(scheduler.current_reductions(), CONTEXT_REDS as u64 - 6);
    /// ```
    pub(super) fn vbump_reductions(&mut self, reductions: i32) {
        let process = self.current_process_mut();
        let used = reductions.min(process.fcalls.max(0));
        process.fcalls = (process.fcalls - reductions).max(0);
        self.virtual_reductions += used;
    }

    /// Return the reductions that the current process used in its turn, as
    /// `erts_current_reductions` gives them for the calling process
    /// (`beam_common.c:2393-2409`): `REDS_IN - fcalls - virtual_reds`.
    /// `REDS_IN` is always `CONTEXT_REDS` in PBeam.
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// scheduler.schedule_next();
    /// scheduler.current_process_mut().fcalls = -1;
    /// assert_eq!(scheduler.current_reductions(), 4001);
    /// ```
    pub(super) fn current_reductions(&mut self) -> u64 {
        let used = CONTEXT_REDS - self.virtual_reductions - self.current_process_mut().fcalls;
        u64::try_from(used).expect("a turn uses 0 or more reductions")
    }

    /// Switch out the current process: add the reductions of its turn to
    /// its total and set the virtual reductions to 0, as `schedule` does
    /// (`erl_process.c:9678-9700`). Return the process.
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// let process = scheduler.schedule_out();
    /// ```
    fn schedule_out(&mut self) -> &mut ProcessState {
        let used = self.current_reductions();
        self.virtual_reductions = 0;
        let process = self
            .processes
            .take_current()
            .expect("the scheduler has a current process");
        process.reductions += used;
        process
    }

    /// Switch out the current process when it does not exit: send the
    /// pending monitor signal first (`erl_process.c:9665-9668`), then add
    /// the reductions of the turn as [`Self::schedule_out`] does. A process
    /// with Erlang work goes to the end of the run queue, and a process
    /// with signals to handle too (`schedule_out_process`,
    /// `erl_process.c:6695-6851`). `update` changes the state of the
    /// process before the check.
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// scheduler.switch_out(|process| process.waiting = true);
    /// ```
    fn switch_out(&mut self, update: impl FnOnce(&mut ProcessState)) {
        self.send_pending_monitor();
        let process = self.schedule_out();
        update(process);
        if !process.waiting || process.mailbox.has_control_signals() {
            let id = process.id;
            self.enqueue(id);
        }
    }

    /// Stop the current process until a message arrives: it has no Erlang
    /// work. A message makes it active again, and a signal puts it in the
    /// run queue to handle the signal.
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// scheduler.park_current();
    /// ```
    pub(super) fn park_current(&mut self) {
        self.switch_out(|process| process.waiting = true);
    }

    /// Stop the current process after `erlang:hibernate/0`. The process
    /// fetches the messages that arrived. A process with a message in the
    /// inner part stays active and goes to the end of the run queue. Else
    /// it waits until a message arrives. ERTS `erts_hibernate` fetches and
    /// clears `ERTS_PSFLG_ACTIVE` only when the inner part has no message
    /// (`beam_common.c:1568-1602`), and `i_hibernate` always switches the
    /// context (`jit/arm/instr_call.cpp:30-36`).
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// scheduler.hibernate_current();
    /// ```
    pub(super) fn hibernate_current(&mut self) {
        self.switch_out(|process| {
            process.mailbox.fetch();
            if !process.mailbox.has_messages() {
                process.waiting = true;
                process.hibernated = true;
            }
        });
    }

    /// Switch out the current process at the end of its budget, or at
    /// another ERTS switch point. The process stays active and goes to the
    /// end of the run queue, as `schedule_out_process` does
    /// (`erl_process.c:6695-6851`). It keeps its execution state, so it
    /// continues at its next instruction with a new budget. Its reduction
    /// total gets the reductions of the turn, as [`Self::schedule_out`]
    /// adds them.
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// scheduler.yield_current();
    /// assert_eq!(scheduler.schedule_next()?, Turn::Run);
    /// ```
    pub(super) fn yield_current(&mut self) {
        self.switch_out(|_| {});
    }

    /// Create the next process identifier. The root process has
    /// `ProcessId(0)`, so the first pid of this function is `ProcessId(1)`.
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// assert_eq!(Scheduler::new(AtomTable::new()).next_pid()?, ProcessId(1));
    /// ```
    fn next_pid(&mut self) -> Result<ProcessId, BeamError> {
        let value = self.next_process_id;
        self.next_process_id = value
            .checked_add(1)
            .ok_or_else(|| error("process identifier overflow"))?;
        Ok(ProcessId(value))
    }

    /// Create a new unique reference.
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// assert_eq!(scheduler.make_ref()?, Reference(0));
    /// ```
    pub(super) fn make_ref(&mut self) -> Result<Reference, BeamError> {
        let value = self.next_reference;
        let reference =
            Reference::ordinary(value).ok_or_else(|| error("process reference overflow"))?;
        self.next_reference = value + 1;
        Ok(reference)
    }

    /// Return the next value of `erlang:unique_integer/0`, as
    /// `unique_integer_bif` and `bld_unique_integer_term`
    /// (`erl_bif_unique.c:475-578`) make it from the thread id and the
    /// counter of the scheduler. PBeam has one scheduler with thread id 1
    /// and no dirty scheduler that runs Erlang code, so the maximum thread
    /// id is 1, and the counter goes after 1 bit (`init_unique_integer`,
    /// `erl_bif_unique.c:459-470`). The value is `MIN_SMALL` plus
    /// `1 | (Counter << 1)`: `-(1 << 59) + 1` for the first call, and 2 more
    /// for each later call, up to `MAX_SMALL`. ERTS gives other values: its
    /// thread ids include the dirty schedulers of the host, and its boot
    /// makes calls before the first call of the program.
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// assert_eq!(scheduler.unique_integer()?, -(1 << 59) + 1);
    /// assert_eq!(scheduler.unique_integer()?, -(1 << 59) + 3);
    /// ```
    ///
    /// # Errors
    ///
    /// A value above `MAX_SMALL` is a runtime error after 2^59 calls: ERTS
    /// then gives a bignum, which PBeam does not have.
    pub(super) fn unique_integer(&mut self) -> Result<i64, BeamError> {
        let value = i64::try_from((self.unique << 1) | 1)
            .ok()
            .and_then(|raw| SMALL_INTEGER_MIN.checked_add(raw))
            .filter(|value| Term::Integer(*value).small_integer().is_some())
            .ok_or_else(|| error("erlang:unique_integer/0 needs a bignum"))?;
        self.unique += 1;
        Ok(value)
    }

    /// Return whether the exit of the root process completed: the root
    /// process recorded its result, and it is not in the process table and
    /// not in the freed processes. Thus each stage of the exit, also the
    /// links, the monitors, and the TTY monitors ([`Self::exit_links`]), ran
    /// before the run ends, also when the exit switched out at the end of
    /// the budget ([`Self::finish_current`]).
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// scheduler.start(frame, vec![])?;
    /// assert!(!scheduler.root_exited());
    /// scheduler.schedule_next();
    /// scheduler.finish_current(Term::static_atom("normal"), Some(Term::static_atom("ok")))?;
    /// assert!(scheduler.root_exited());
    /// ```
    pub(super) fn root_exited(&self) -> bool {
        let root = ROOT;
        self.root_result.is_some()
            && !self.processes.contains_key(&root)
            && !self.freed.contains_key(&root)
    }

    /// Return the root result and the atom table of the run together.
    /// `None` means the root process did not exit. The table holds each name
    /// that the run created, for example each name of
    /// `erlang:list_to_atom/1`, so a host result that carries the table can
    /// name every atom of the root result.
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// let (root, atoms) = scheduler.into_run_result();
    /// assert_eq!(root, Some(Ok(Term::static_atom("ok"))));
    /// ```
    pub(super) fn into_run_result(self) -> (Option<Result<Term, Term>>, AtomTable) {
        (self.root_result, self.atoms)
    }

    /// Return the bytes of the guest containers that the runtime holds: the
    /// container arrays of each live process and of each freed process, and
    /// the guest terms that they hold; the persistent terms, the ETS
    /// objects, and the messages of the active BIF timers of the node; and
    /// the result or the exit reason of the root process. A guest term that
    /// only the loaded code holds, such as a literal of a decoded
    /// instruction, is not counted. Each shared container is counted once,
    /// as the counting allocator of `docs/term-layout-design.md` section 5
    /// counts each allocation once.
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// assert!(scheduler.live_bytes() > 0);
    /// ```
    pub(super) fn live_bytes(&self) -> usize {
        let mut live = LiveBytes::new();
        let freed = self.freed.values().map(|(process, _)| process);
        for process in self.processes.values().chain(freed) {
            process.account(&mut live);
        }
        self.persistent_terms.account_terms(&mut live);
        self.ets.account_terms(&mut live);
        self.timers.account_terms(&mut live);
        if let Some(result) = &self.root_result {
            match result {
                Ok(term) | Err(term) => live.term(term),
            }
        }
        self.account_release(&mut live);
        live.bytes()
    }

    /// Account the terms that the release work list holds, so that the
    /// live-byte walk of a run counts the graph of a process that left the
    /// process table until the drain releases it. Each item gives the storage
    /// of its container and its terms to the walk, which counts each shared
    /// container once.
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// let mut live = LiveBytes::new();
    /// scheduler.account_release(&mut live);
    /// ```
    pub(super) fn account_release(&self, live: &mut LiveBytes) {
        for item in self.release.iter() {
            match item {
                Release::Term(term) => live.term(term),
                Release::Terms(group) => {
                    live.add(group.capacity() * std::mem::size_of::<Term>());
                    for term in group {
                        live.term(term);
                    }
                }
                Release::Map(drain) => drain.account(live),
                Release::Registers(registers) => {
                    live.add(registers.capacity() * std::mem::size_of::<Option<Term>>());
                    for register in registers.iter().flatten() {
                        live.term(register);
                    }
                }
                Release::Frames(frames) => {
                    live.add(frames.capacity() * std::mem::size_of::<Frame>());
                    for frame in frames {
                        live.add(frame.stack.capacity() * std::mem::size_of::<StackSlot>());
                        for slot in &frame.stack {
                            if let StackSlot::Term(term) = slot {
                                live.term(term);
                            }
                        }
                    }
                }
                Release::Slots(slots) => {
                    live.add(slots.capacity() * std::mem::size_of::<StackSlot>());
                    for slot in slots {
                        if let StackSlot::Term(term) = slot {
                            live.term(term);
                        }
                    }
                }
                Release::Dictionary(dictionary) => {
                    live.add(dictionary.capacity() * std::mem::size_of::<(Term, Term)>());
                    for (key, value) in dictionary {
                        live.term(key);
                        live.term(value);
                    }
                }
                Release::Mailbox(mailbox) => {
                    live.add(mailbox.allocated_bytes());
                    mailbox.account_release_terms(live);
                }
                Release::Monitors(monitors) => {
                    live.add(monitors.allocated_bytes());
                    for origin in monitors.values() {
                        if let Some(monitor) = &origin.monitor {
                            live.term(&monitor.item);
                            live.term(&monitor.tag);
                        }
                    }
                }
            }
        }
        // Every item of the list flushes its own walk, and this closes the
        // walk of the list as a whole.
        live.flush();
    }

    /// Deliver a message that no process sends, such as a timer message or
    /// a reply of a signal, as `erts_queue_message` (`erl_message.c:492-501`)
    /// does. A message to a process that does not exist, or to an exiting
    /// process, is dropped (`erl_message.c:412-426`). The message makes a
    /// waiting process active.
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// scheduler.deliver(target, Term::static_atom("ok"));
    /// ```
    pub(super) fn deliver(&mut self, target: ProcessId, term: Term) {
        let Some(process) = self.processes.get_mut(&target) else {
            return;
        };
        if process.exiting.is_some() {
            return;
        }
        process.mailbox.push(term);
        self.activate(target);
    }

    /// Send a message from the process `from` to `target`, as
    /// `erts_queue_proc_message` (`erl_message.c:531-545`) does. A pending
    /// monitor signal of `from` to the same target goes first
    /// (`prepend_pending_sig_maybe`, `erl_message.c:463-480`). A message to a
    /// process that does not exist, or to an exiting process, is dropped,
    /// and the pending signal waits on. A message to the sender itself marks
    /// it as a process that may have signals from itself.
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// scheduler.send_message(caller, target, Term::static_atom("ok"));
    /// ```
    pub(super) fn send_message(&mut self, from: ProcessId, target: ProcessId, term: Term) {
        let Some(process) = self.processes.get(&target) else {
            return;
        };
        if process.exiting.is_some() {
            return;
        }
        let pending = self.take_pending_monitor(target);
        let process = self
            .processes
            .get_mut(&target)
            .expect("the target is alive");
        process.self_signals |= from == target;
        if let Some(pending) = pending {
            process
                .mailbox
                .push_signal(signals::monitor_signal(pending));
        }
        process.mailbox.push(term);
        self.activate(target);
    }

    /// Spawn a child process for the current process and optionally monitor
    /// or link it. `start` gives its entry frame or its undefined start MFA.
    /// `flags` gives the start values of its `message_queue_data` flag and
    /// of its priority. The link and the monitor go into both
    /// processes at once, as `erl_create_process` does, so no signal goes
    /// to the child.
    ///
    /// First the current process must hold a `process` capability with
    /// `spawn` that permits the module of `start`, and with `link` and
    /// `monitor` when the spawn links or monitors. The child gets the
    /// delegable capabilities of the current process, reduced by
    /// `restrictions` (see `SecurityContext::delegate`). The child of a boot
    /// process is a boot process, and a boot process with `restrictions` is
    /// denied. A denied check or
    /// delegation gives `Ok(Err(Denied))` before the spawn takes a pid or a
    /// reference. In a run that skips the checks, the spawn has no check,
    /// and the child gets no copy and ignores `restrictions`.
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// let (pid, monitor) = scheduler.spawn(Start::Entry(frame), vec![], true, false, StartFlags::default(), None)?.unwrap();
    /// assert!(monitor.is_some());
    /// ```
    pub(super) fn spawn(
        &mut self,
        start: Start,
        arguments: Vec<Term>,
        monitor: bool,
        link: bool,
        flags: StartFlags,
        restrictions: Option<&[Restriction]>,
    ) -> Result<Result<(ProcessId, Option<Reference>), Denied>, BeamError> {
        let module = match &start {
            Start::Entry(frame) => self.code.module_by_id(frame.module_id)?.name.as_str(),
            Start::Undefined { module, .. } => self.atoms.name_of(module),
        };
        let parent = &self.current_process().security;
        let boot = parent.is_boot();
        let check = |needed: bool, operation: Operation, name: Option<&[u8]>| {
            if needed {
                parent.check(CapabilityKind::Process, operation, name)
            } else {
                Ok(())
            }
        };
        let copies = if self.skip_checks {
            Ok(Vec::new())
        } else {
            check(true, Operation::Spawn, Some(module.as_bytes()))
                .and_then(|()| check(link, Operation::Link, None))
                .and_then(|()| check(monitor, Operation::Monitor, None))
                .and_then(|()| parent.delegate(restrictions))
        };
        let copies = match copies {
            Ok(copies) => copies,
            Err(denied) => return Ok(Err(denied)),
        };
        let mut child = ProcessState::new(self.next_pid()?, start, arguments);
        child.off_heap_message_queue = flags.off_heap_message_queue;
        child.priority = flags.priority;
        child.security = if boot {
            SecurityContext::boot()
        } else {
            self.delegations.hold(child.id, copies)
        };
        let parent = self.current_process_mut().id;
        child.parent = Some(parent);
        child.group_leader = self.current_process_mut().group_leader;
        let reference = if monitor {
            let reference = self.make_ref()?;
            child.monitored_by.push(TargetMonitor::Process(Monitor {
                watcher: parent,
                reference,
            }));
            let origin = Origin {
                monitor: Some(OriginMonitor {
                    kind: "process",
                    target: Some(child.id),
                    item: Term::Pid(child.id),
                    tag: Term::static_atom("DOWN"),
                }),
                alias: None,
            };
            self.current_process_mut()
                .monitors
                .insert(reference, origin);
            Some(reference)
        } else {
            None
        };
        if link {
            self.current_process_mut()
                .links
                .insert(LinkId::Process(child.id), 0);
            child.links.insert(LinkId::Process(parent), 0);
        }
        let id = child.id;
        self.insert(child);
        Ok(Ok((id, reference)))
    }

    /// Revoke the initial capabilities of each kind of `kinds` for the
    /// launcher, which issued them: the root process gets a revocation
    /// signal for each of its copies of the kind. The copies of the root
    /// come only from the launcher. Nothing happens after the root process
    /// left the process table, because the run then ends.
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// scheduler.revoke_issued(&[CapabilityKind::Process]);
    /// ```
    pub(super) fn revoke_issued(&mut self, kinds: &[CapabilityKind]) {
        let root = self.processes.get(&ROOT);
        let copies: Vec<_> = root.map_or_else(Vec::new, |root| {
            kinds
                .iter()
                .flat_map(|&kind| root.security.ids(kind))
                .collect()
        });
        for id in copies {
            let signals = self.delegations.revocation(id);
            self.send_revocations(None, signals);
        }
    }

    /// Revoke the copies of `kind` that the current process gave to the
    /// process `holder` at spawn, for `pbeam_cap:revoke/2`. A copy whose
    /// holder lives gets a revocation signal. A copy whose holder exited
    /// sends the revocation on to its receivers at once (see
    /// [`Delegations::revocation`]). Return `Err(Denied)` when the current
    /// process gave `holder` no copy of `kind` that still has a record: a
    /// process can revoke only a capability that it gave. In a run that
    /// skips the checks, the call returns `Ok(())` and has no effect.
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// assert_eq!(scheduler.revoke_given(child, CapabilityKind::Process), Ok(()));
    /// assert_eq!(scheduler.revoke_given(stranger, CapabilityKind::Process), Err(Denied));
    /// ```
    pub(super) fn revoke_given(
        &mut self,
        holder: ProcessId,
        kind: CapabilityKind,
    ) -> Result<(), Denied> {
        if self.skip_checks {
            return Ok(());
        }
        let caller = self.current_process();
        let (id, copies) = (caller.id, caller.security.ids(kind));
        let given = self.delegations.given(&copies, holder);
        if given.is_empty() {
            return Err(Denied);
        }
        for copy in given {
            let signals = self.delegations.revocation(copy);
            self.send_revocations(Some(id), signals);
        }
        Ok(())
    }

    /// Send a revocation signal for each `(holder, copy)` of `signals` from
    /// `sender`, or from the runtime for `None`.
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// scheduler.send_revocations(Some(caller), vec![(child, copy)]);
    /// ```
    fn send_revocations(
        &mut self,
        sender: Option<ProcessId>,
        signals: Vec<(ProcessId, CapabilityId)>,
    ) {
        for (holder, capability) in signals {
            self.send_signal(sender, holder, Signal::Revoke { capability })
                .expect("the holder of a copy that is not released is alive");
        }
    }

    /// Link the current process with the local process `target`, as
    /// `link_opt` (`bif.c:136-186`) does for `erlang:link/1`. A link to the
    /// current process changes nothing. An active link sends no signal. A
    /// new link, or an unlinking link that becomes active again, sends a
    /// link signal to the target, which adds its side of the link when it
    /// handles the signal.
    ///
    /// A target that does not exist or exits (`erts_proc_lookup`) gives
    /// `false` and no link when the current process does not trap exits:
    /// the BIF then raises `noproc` (`bif.c:162-163`, `411-423`). With
    /// `trap_exit` the link is made. A target that exits gets the link
    /// signal and answers it with the link exit signal `noproc` at its exit
    /// (see [`Scheduler::handle_exit_signals`]). For a target that does not
    /// exist, the current process sends itself that signal with no sender
    /// process (`bif.c:180-183`). Return true in each other case.
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// assert!(scheduler.link(child));
    /// assert!(!scheduler.link(dead));
    /// ```
    pub(super) fn link(&mut self, target: ProcessId) -> bool {
        let current = self.current_process_mut().id;
        if target == current {
            return true;
        }
        if self.live_process(target).is_none() && !self.current_process_mut().trap_exit {
            return false;
        }
        let linked = LinkId::Process(target);
        if self.current_process_mut().links.insert(linked, 0) == Some(0) {
            return true;
        }
        let link = Signal::Link { from: current };
        if self.send_signal(Some(current), target, link).is_err() {
            let exit = Signal::Exit {
                from: linked,
                reason: Term::static_atom("noproc"),
                linked: true,
                normal_kills: false,
                alias: None,
            };
            self.send_signal(None, current, exit)
                .expect("the current process gets the exit signal");
        }
        true
    }

    /// Unlink the current process from `target`, as `unlink_1`
    /// (`bif.c:1231-1249`) does for a local pid. Only an active link sends an
    /// unlink signal with a new identifier and becomes unlinking; the target
    /// answers with an unlink acknowledgment, which removes the link. A link
    /// to a process that does not exist goes at once. A target without a
    /// link changes nothing. An `EXIT` message in the queue stays.
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// scheduler.link(child)?;
    /// scheduler.unlink(child);
    /// let unlinking = scheduler.current_process_mut().links.get(&child);
    /// assert!(unlinking.is_some_and(|&id| id != 0));
    /// ```
    pub(super) fn unlink(&mut self, target: ProcessId) {
        let current = self.current_process_mut().id;
        let link = LinkId::Process(target);
        if self.current_process_mut().links.get(&link) != Some(&0) {
            return;
        }
        let id = self.next_unlink_id;
        self.next_unlink_id += 1;
        let unlink = Signal::Unlink { from: current, id };
        if self.send_signal(Some(current), target, unlink).is_ok() {
            self.current_process_mut().links.insert(link, id);
        } else {
            self.current_process_mut().links.remove(&link);
        }
    }

    /// Monitor a target for the current process, as ERTS `monitor`
    /// (`bif.c:758-981`) does after its checks. A process by name is the
    /// process that has the name now (`erts_whereis_name_to_id`). A monitor
    /// of the current process creates no monitor and no alias
    /// (`bif.c:781-784`). Else the monitor goes into the monitor tree of the
    /// current process. A pid gets a monitor signal through the pending
    /// signal slot (see [`Scheduler::send_monitor`]). A name that no process
    /// has, and each port name, give a monitor down signal `noproc` to the
    /// current process at once: PBeam registers no port, so the port lookup
    /// always fails (`bif.c:907-914`). The `DOWN` message gives the pid as
    /// its item, or `{Name, nonode@nohost}` for a name.
    ///
    /// With an alias option, the reference is also an alias of the current
    /// process, and the alias mode goes into the monitor tree.
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// let reference = scheduler.monitor(MonitorTarget::Process(child), options)?;
    /// let gone = scheduler.monitor(MonitorTarget::PortName(name), options)?;
    /// ```
    ///
    /// # Errors
    ///
    /// The `priority` option of a monitor that is not of the current process
    /// is not supported.
    pub(super) fn monitor(
        &mut self,
        target: MonitorTarget,
        options: MonitorOptions,
    ) -> Result<Reference, BeamError> {
        let mut reference = self.make_ref()?;
        let watcher = self.current_process_mut().id;
        if options.alias.is_some() {
            // ERTS makes a pid reference for an alias option (`bif.c:765-767`).
            reference = reference.pid_reference();
            self.alias_owners.insert(reference, watcher);
        }
        let target_port = match target {
            MonitorTarget::Port(port) => Some(port),
            _ => None,
        };
        let (kind, id, item) = match target {
            MonitorTarget::Process(id) => ("process", Some(id), Term::Pid(id)),
            MonitorTarget::ProcessName(name) => {
                let id = self.whereis(self.atoms.name_of(&name));
                ("process", id, named_item(&name))
            }
            MonitorTarget::PortName(name) => ("port", None, named_item(&name)),
            MonitorTarget::Port(port) => ("port", None, Term::Port(port)),
        };
        if id == Some(watcher) {
            return Ok(reference);
        }
        if options.priority {
            // ponytail: no priority messages yet; add them when a measured path uses the option.
            return Err(error("unsupported monitor option priority"));
        }
        let origin = Origin {
            monitor: Some(OriginMonitor {
                kind,
                target: id,
                item,
                tag: options.tag,
            }),
            alias: options.alias,
        };
        self.current_process_mut()
            .monitors
            .insert(reference, origin);
        match (id, target_port) {
            (Some(id), _) if kind == "process" => {
                self.send_monitor(id, Monitor { watcher, reference });
            }
            (_, Some(port)) if self.monitor_port(port, watcher, reference) => {}
            _ => self.send_monitor_down(None, watcher, reference, Term::static_atom("noproc")),
        }
        Ok(reference)
    }

    /// Return the seed of the hash of `term_to_binary/2` with `local`, as
    /// `erts_late_init_external` computes it once for the node
    /// (external.c:136-198): the block hash of the time offset (8 bytes,
    /// little-endian), the host name, and the OS pid in decimal, with the
    /// low 32 bits of the monotonic time as its start value.
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// assert_eq!(scheduler.local_node_hash(), scheduler.local_node_hash());
    /// ```
    pub(in crate::runtime) fn local_node_hash(&self) -> u32 {
        *self.local_node_hash.get_or_init(|| {
            let node = [
                &self.timers.time_offset().to_le_bytes()[..],
                &host::host_name(),
                std::process::id().to_string().as_bytes(),
            ]
            .concat();
            block_hash(&node, self.timers.now() as u32)
        })
    }

    /// Remove a monitor that the current process holds, as the ERTS
    /// `demonitor` function (`bif.c:436-541`) does. Return true when the
    /// monitor tree had the monitor, and false for a reference that is not a
    /// monitor of the current process, also an alias without a monitor. An
    /// `explicit_unalias` alias stays without its monitor; another alias
    /// ends with its monitor (`bif.c:470-492`). A monitor of a process sends
    /// a demonitor signal to it (`bif.c:512-514`). A `DOWN` signal of the
    /// monitor that comes later finds no monitor and is dropped.
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// let reference = scheduler.monitor(MonitorTarget::Process(child), options)?;
    /// assert!(scheduler.demonitor(reference));
    /// assert!(!scheduler.demonitor(reference));
    /// ```
    pub(super) fn demonitor(&mut self, reference: Reference) -> bool {
        let watcher = self.current_process_mut().id;
        let Some(monitor) = self.remove_origin_monitor(watcher, reference) else {
            return false;
        };
        self.end_monitor(watcher, reference, &monitor);
        true
    }

    /// Remove the monitor `reference` of `watcher`, which left the monitor
    /// tree of `watcher`, from its target. A monitor of a process sends a
    /// demonitor signal to it (`bif.c:512-514`). A monitor of a port leaves
    /// the port, as `erts_port_demonitor` does (io.c:2839-2870).
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// let monitor = scheduler.remove_origin_monitor(watcher, reference).unwrap();
    /// scheduler.end_monitor(watcher, reference, &monitor);
    /// ```
    pub(super) fn end_monitor(
        &mut self,
        watcher: ProcessId,
        reference: Reference,
        monitor: &OriginMonitor,
    ) {
        if let (Some(target), "process") = (monitor.target, monitor.kind) {
            let demonitor = Signal::Demonitor { watcher, reference };
            // A target that does not exist drops the signal.
            let _ = self.send_signal(Some(watcher), target, demonitor);
        }
        if let Term::Port(port) = monitor.item {
            self.demonitor_port(port, reference);
        }
    }

    /// Remove the monitor `reference` from the monitor tree of the process
    /// `id` and return it. An `explicit_unalias` alias stays as an alias
    /// without a monitor (`ERTS_ML_STATE_ALIAS_UNALIAS`); another alias ends
    /// with the monitor. Return `None` when the tree has no such monitor.
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// assert!(scheduler.remove_origin_monitor(watcher, reference).is_some());
    /// assert!(scheduler.remove_origin_monitor(watcher, reference).is_none());
    /// ```
    fn remove_origin_monitor(
        &mut self,
        id: ProcessId,
        reference: Reference,
    ) -> Option<OriginMonitor> {
        let monitors = &mut self.processes.get_mut(&id)?.monitors;
        let origin = monitors.get_mut(&reference)?;
        let monitor = origin.monitor.take()?;
        if origin.alias != Some(AliasMode::ExplicitUnalias) {
            monitors.remove(&reference);
        }
        Some(monitor)
    }

    /// Send the message `message` from the current process through the
    /// alias `alias`, as `erts_proc_sig_send_altact_msg`
    /// (`erl_proc_sig_queue.c:2077-2290`) does: the message goes as a signal
    /// to the owner of the alias, which checks the alias when it handles
    /// the signal. A reference that no `erlang:monitor/3` made with an alias
    /// option, and an owner that does not exist or exits, drop the message.
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// scheduler.send_alias_message(alias, Term::static_atom("ok"));
    /// ```
    pub(super) fn send_alias_message(&mut self, alias: Reference, message: Term) {
        let sender = self.current_process_mut().id;
        let Some(&owner) = self.alias_owners.get(&alias) else {
            return;
        };
        if self.live_process(owner).is_some() {
            let signal = Signal::AliasMessage { alias, message };
            self.send_signal(Some(sender), owner, signal)
                .expect("a live process gets the alias message");
        }
    }

    /// Return the owner of the alias reference `reference`: the pid in an
    /// ERTS pid reference (`erts_get_pid_of_ref`, `erl_bif_unique.h:279`).
    /// Return `None` for a reference that no `erlang:monitor/3` made with an
    /// alias option.
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// assert_eq!(scheduler.alias_owner(alias), Some(owner));
    /// assert_eq!(scheduler.alias_owner(plain_reference), None);
    /// ```
    pub(super) fn alias_owner(&self, reference: Reference) -> Option<ProcessId> {
        self.alias_owners.get(&reference).copied()
    }

    /// Give the name `name` to the live process `id`, as `erlang:register/2`
    /// does. Return `false` and change nothing when ERTS
    /// `erts_register_name` (`register.c`) fails: `name` is not an atom or
    /// is `undefined`, `id` is not a pid, the process is not alive, the
    /// process has a name, or another process has the name.
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// let name = Term::Atom(program.atom("pbeam_probe_reg").unwrap());
    /// assert!(scheduler.register(&name, &Term::Pid(ProcessId(0))));
    /// assert!(!scheduler.register(&name, &Term::Pid(ProcessId(0))));
    /// ```
    pub(super) fn register(&mut self, name: &Term, id: &Term) -> bool {
        let (Term::Atom(name), Term::Pid(id)) = (name, id) else {
            return false;
        };
        let name_text = self.atoms.name_of(name);
        match self.processes.get_mut(id) {
            Some(process)
                if !name.is_static("undefined")
                    && process.registered_name.is_none()
                    && !self.names.contains_key(name_text) =>
            {
                process.registered_name = Some(name.clone());
                self.names.insert(name_text.to_owned(), *id);
                true
            }
            _ => false,
        }
    }

    /// Remove the registered name `name` from its process, as
    /// `erlang:unregister/1` does with `erts_unregister_name`
    /// (`bif.c:2324-2335`, `register.c:429-541`). Return `false` and change
    /// nothing when `name` is not an atom or no process has the name.
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// let name = Term::Atom(program.atom("pbeam_probe_reg").unwrap());
    /// assert!(scheduler.register(&name, &Term::Pid(ProcessId(0))));
    /// assert!(scheduler.unregister(&name));
    /// assert!(!scheduler.unregister(&name));
    /// ```
    pub(super) fn unregister(&mut self, name: &Term) -> bool {
        let Term::Atom(name) = name else {
            return false;
        };
        let Some(id) = self.names.remove(self.atoms.name_of(name)) else {
            return false;
        };
        let process = self
            .processes
            .get_mut(&id)
            .expect("a name belongs to a live process");
        process.registered_name = None;
        true
    }

    /// Return the process that has the registered name `name`, as ERTS
    /// `erts_whereis_name_to_id` (`register.c:276-300`) finds it in the
    /// `process_reg` table. Return `None` when no live process has the name.
    /// PBeam registers no port, so a name never gives a port.
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// let name = Term::Atom(program.atom("pbeam_probe_reg").unwrap());
    /// assert_eq!(scheduler.whereis("pbeam_probe_reg"), None);
    /// assert!(scheduler.register(&name, &Term::Pid(ProcessId(0))));
    /// assert_eq!(scheduler.whereis("pbeam_probe_reg"), Some(ProcessId(0)));
    /// ```
    pub(super) fn whereis(&self, name: &str) -> Option<ProcessId> {
        self.names.get(name).copied()
    }

    /// Run the exit of the current process, as `erts_do_exit_process` and
    /// `erts_continue_exit_process` do (`erl_process.c:14170-14833`). The
    /// exit counts its reductions from the reductions left in the turn, and
    /// switches out when they end at one of the checks that a process
    /// without links to ports, distribution, or scheduler blocks can reach.
    /// A switch out uses all reductions of the turn (`BUMP_ALL_REDS`,
    /// `erl_process.c:14832`), and the next turn continues the exit with a
    /// new count and no new bump:
    ///
    /// 1. Bump 100 reductions (`erl_process.c:14229`). Stop the receive
    ///    timer (`erl_process.c:14207-14210`). The root process records its
    ///    result.
    /// 2. Cancel each BIF timer to the pid in the post-order of the timer
    ///    tree of the process (`ERTS_CONTINUE_EXIT_TIMERS`,
    ///    `erl_process.c:14300-14305`). Each timer uses 80 reductions
    ///    (`erl_hl_timer.c:2401-2446`). The walk switches out before a timer
    ///    when no reductions are left, and a walk that ends with no
    ///    reductions left gives 1 (`erl_rbtree.h:1392-1436`). With no
    ///    reductions left after this phase, the process switches out
    ///    (`erl_process.c:14314-14316`). Until it leaves the process table,
    ///    a process that switched out keeps its registered name.
    /// 3. Delete its ETS tables (`ERTS_CONTINUE_EXIT_USING_DB`,
    ///    `erl_process.c:14360-14368`; see [`EtsTables::delete_owned`]). The
    ///    deletion counts from the reductions left in the turn, which the
    ///    timer walk did not change, and the exit switches out when the
    ///    deletion uses them.
    /// 4. Free its registered name, release its capability copies (see
    ///    [`Delegations::release`]), and leave the process table
    ///    (`ERTS_CONTINUE_EXIT_FREE`, `erl_process.c:14406-14476`), which
    ///    uses 50 reductions of the count of the exit. Fetch its signal queue
    ///    (`erl_process.c:14537`). With no reductions left, the process
    ///    switches out (`erl_process.c:14541-14543`).
    /// 5. Send the signals of its links and monitors, and answer its pending
    ///    signals ([`Self::exit_links`]).
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// scheduler.finish_current(Term::static_atom("normal"), Some(result))?;
    /// ```
    ///
    /// # Errors
    ///
    /// The error of [`Self::exit_links`].
    pub(super) fn finish_current(
        &mut self,
        reason: Term,
        result: Option<Term>,
    ) -> Result<(), BeamError> {
        let process = self.current_process_mut();
        let id = process.id;
        let mut next = match process.continue_exit.take() {
            Some(next) => next,
            None => {
                process.bump_reductions(EXIT_REDUCTIONS);
                if let ReceiveTimer::Set { key, .. } = process.receive_timer {
                    self.timers.cancel(key);
                }
                if id == ROOT {
                    let normal = reason == Term::static_atom("normal");
                    self.root_result = Some(match result {
                        Some(result) if normal => Ok(result),
                        _ => Err(reason.clone()),
                    });
                }
                self.current_process_mut()
                    .bif_timers
                    .post_order()
                    .position()
            }
        };
        let process = self.current_process_mut();
        let mut reds = process.fcalls;
        // The tree does not change during the exit: an exiting process
        // keeps each timer that expires or that a BIF cancels, and gets no
        // new timer. Thus the position stays valid across turns.
        let tree = std::mem::take(&mut process.bif_timers);
        let walked = next.is_some();
        while let Some(position) = next {
            if reds <= 0 {
                break;
            }
            let mut walk = tree.post_order_from(position);
            let (&timer, ()) = walk.next().expect("a walk position has an entry");
            next = walk.position();
            // The receiver exits, so the timer has no tree entry to remove.
            let _ = self.timers.cancel_bif(timer.0);
            reds -= TIMER_CANCEL_REDUCTIONS;
        }
        if walked && next.is_none() {
            reds = reds.max(1);
        }
        // The table deletion counts from the reductions left in the turn,
        // not from the count of the exit, and its bump does not change that
        // count (`erl_db.c:5044-5045`, `5118`; `erl_process.c:14360-14366`).
        let deleted = reds > 0 && {
            let left = self.current_process_mut().fcalls;
            let rest = self.ets.delete_owned(id, left);
            if let Some(rest) = rest {
                self.current_process_mut().bump_reductions(left - rest);
            }
            rest.is_some()
        };
        if !deleted {
            let process = self.current_process_mut();
            process.exiting = Some(reason);
            process.bif_timers = tree;
            process.continue_exit = Some(next);
            process.fcalls = 0;
            self.yield_current();
            return Ok(());
        }
        reds -= FREE_REDUCTIONS;
        self.schedule_out();
        let mut process = self
            .processes
            .remove(&id)
            .expect("the exiting process is alive");
        debug_assert!(!process.in_run_queue, "only the current process exits");
        if let Some(name) = &process.registered_name {
            self.names.remove(self.atoms.name_of(name));
        }
        self.delegations
            .release(std::mem::take(&mut process.security));
        self.policy_exited(id, &reason);
        process.mailbox.fetch();
        process.exiting = Some(reason);
        if reds <= 0 {
            self.switch_out_freed(process, ExitPhase::Links(None));
            return Ok(());
        }
        self.exit_links(process, ExitPhase::Links(None), reds)
    }

    /// Switch out the freed process `process`, whose exit continues at
    /// `phase` in its next turn (`erl_process.c:14795-14833`). Send the
    /// pending monitor signal first, as each switch out does
    /// (`erl_process.c:9665-9668`).
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// scheduler.switch_out_freed(process, ExitPhase::Links(None));
    /// ```
    fn switch_out_freed(&mut self, process: ProcessState, phase: ExitPhase) {
        self.send_pending_monitor();
        let id = process.id;
        self.runnable.push(id, process.priority);
        self.freed.insert(id, (process, phase));
    }

    /// Continue the exit of the process that left the process table at
    /// `phase`, with `reds` reductions left, as the phases after
    /// `ERTS_CONTINUE_EXIT_FREE` of `erts_continue_exit_process` do
    /// (`erl_process.c:14545-14599`):
    ///
    /// 1. Send a link exit signal to each linked process, but not through a
    ///    link that is unlinking (`erts_proc_exit_handle_link`,
    ///    `erl_process.c:14068-14165`).
    /// 2. Send a demonitor signal to the target of each monitor that it
    ///    holds, and end each alias (`erl_process.c:13891-13977`).
    /// 3. Send a monitor down signal to each process that monitors it, and
    ///    run each monitor of a TTY resource on it
    ///    (`erl_process.c:13806-13833`).
    /// 4. Answer the signals that it did not handle (see
    ///    [`Scheduler::handle_exit_signals`]).
    /// 5. Send the pending monitor signal, when there is one.
    ///
    /// Each entry of the first three phases uses 1 reduction
    /// (`erl_process.c:13804`, `14165`). A phase switches out the process
    /// before an entry when no reductions are left, and a phase that ends
    /// with no reductions left gives 1 (`erl_rbtree.h:1392-1436`;
    /// `erl_monitor_link.c:517-548`). The exit enters each phase with
    /// reductions left, so the check after each list entry in ERTS is the
    /// same check. The signal phase switches out when its signal count
    /// reaches its limit.
    ///
    /// The links and the monitors that the process holds go in the
    /// post-order of their red-black trees, as `foreach_delete_yielding`
    /// destroys the ERTS trees (`erl_process.c:14545-14572`;
    /// `erl_monitor_link.c:439-460`). The monitors on the process, of
    /// processes and of TTY resources, go in the order that it handled their
    /// monitor signals, as ERTS goes through its list `ERTS_P_LT_MONITORS`
    /// (`erl_proc_sig_queue.c:6555`; `erl_monitor_link.h:652-669`).
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// scheduler.exit_links(process, ExitPhase::Links(None), 3850)?;
    /// ```
    ///
    /// # Errors
    ///
    /// The close of a port with queued output that the process owns (see
    /// [`Self::port_link_exit`]).
    fn exit_links(
        &mut self,
        mut process: ProcessState,
        mut phase: ExitPhase,
        mut reds: i32,
    ) -> Result<(), BeamError> {
        let id = process.id;
        let reason = process
            .exiting
            .clone()
            .expect("a process that left the process table has an exit reason");
        loop {
            phase = match phase {
                ExitPhase::Links(position) => {
                    let mut walk = match position {
                        Some(position) => process.links.post_order_from(position),
                        None => process.links.post_order(),
                    };
                    loop {
                        let position = walk.position();
                        let Some((&linked, &unlinking)) = walk.next() else {
                            break;
                        };
                        if reds <= 0 {
                            self.switch_out_freed(process, ExitPhase::Links(position));
                            return Ok(());
                        }
                        match linked {
                            LinkId::Process(linked) if unlinking == 0 => {
                                let exit = Signal::Exit {
                                    from: LinkId::Process(id),
                                    reason: reason.clone(),
                                    linked: true,
                                    normal_kills: false,
                                    alias: None,
                                };
                                // A linked process that does not exist drops
                                // the signal.
                                let _ = self.send_signal(Some(id), linked, exit);
                            }
                            LinkId::Process(_) => {}
                            LinkId::Port(port) => self.port_link_exit(port, id, &reason)?,
                        }
                        reds -= 1;
                    }
                    ExitPhase::Monitors(None)
                }
                ExitPhase::Monitors(position) => {
                    let mut walk = match position {
                        Some(position) => process.monitors.post_order_from(position),
                        None => process.monitors.post_order(),
                    };
                    loop {
                        let position = walk.position();
                        let Some((&reference, origin)) = walk.next() else {
                            break;
                        };
                        if reds <= 0 {
                            self.switch_out_freed(process, ExitPhase::Monitors(position));
                            return Ok(());
                        }
                        if let Some(OriginMonitor {
                            kind: "process",
                            target: Some(target),
                            ..
                        }) = &origin.monitor
                        {
                            let demonitor = Signal::Demonitor {
                                watcher: id,
                                reference,
                            };
                            // A target that does not exist drops the signal.
                            let _ = self.send_signal(Some(id), *target, demonitor);
                        }
                        if let Some(OriginMonitor {
                            item: Term::Port(port),
                            ..
                        }) = &origin.monitor
                        {
                            self.demonitor_port(*port, reference);
                        }
                        reds -= 1;
                    }
                    ExitPhase::TargetMonitors(0)
                }
                ExitPhase::TargetMonitors(mut done) => {
                    while let Some(&monitor) = process.monitored_by.get(done) {
                        if reds <= 0 {
                            self.switch_out_freed(process, ExitPhase::TargetMonitors(done));
                            return Ok(());
                        }
                        match monitor {
                            TargetMonitor::Process(monitor) => self.send_monitor_down(
                                Some(id),
                                monitor.watcher,
                                monitor.reference,
                                reason.clone(),
                            ),
                            TargetMonitor::Tty(tty) => {
                                self.resources.borrow_mut::<Ttys>().monitor_down(id, tty);
                            }
                            TargetMonitor::Port { port, reference: _ } => {
                                self.tcp_monitor_down(port, id);
                            }
                        }
                        done += 1;
                        reds -= 1;
                    }
                    ExitPhase::Signals
                }
                ExitPhase::Signals => {
                    if !self.handle_exit_signals(&mut process, reds) {
                        self.switch_out_freed(process, ExitPhase::Signals);
                        return Ok(());
                    }
                    break;
                }
            };
            // A walk that ends gives 1 when no reductions are left.
            reds = reds.max(1);
        }
        self.alias_owners.retain(|_, owner| *owner != id);
        self.send_pending_monitor();
        // The process left the process table and no guest code can read it
        // again. Its terms go to the release work list, so that the walk of a
        // large graph runs in bounded scheduler work and not in this step.
        self.release_process(process);
        Ok(())
    }
}

/// Return the `DOWN` item of a monitor by name: `{Name, nonode@nohost}`, as
/// ERTS builds it for a local name (`erl_proc_sig_queue.c:4942-4955`).
///
/// # Examples
///
/// ```rust,ignore
/// assert_eq!(named_item(&name), Term::tuple(vec![Term::Atom(name.clone()), local_node()]));
/// ```
fn named_item(name: &Atom) -> Term {
    Term::tuple(vec![Term::Atom(name.clone()), local_node()])
}

mod known_nodes;
mod policy;
mod ports;
mod process_table;
mod run_queue;
mod signals;
mod tcp_inet;
mod tcp_stream;

use known_nodes::KnownNodes;
pub(super) use policy::PolicyEnd;
use policy::PolicyProcess;
use process_table::ProcessTable;
use run_queue::RunQueue;
pub(super) use signals::{Flushed, HandlingPoint, Received};

#[cfg(test)]
#[path = "../../tests/runtime/scheduler.rs"]
mod tests;

/// The measurement of the scheduler release of a large shared graph of row 11
/// of `docs/term-performance-design-codex-v2.md` and task
/// [#559](https://github.com/fahchen/pbeam/issues/559). It lives beside the
/// scheduler, because the pause is the time inside the private exit and drop
/// paths.
#[cfg(test)]
#[path = "../../tests/runtime/scheduler/release_costs.rs"]
mod release_costs;

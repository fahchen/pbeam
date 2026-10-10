use std::{
    collections::{HashMap, HashSet, VecDeque},
    time::Duration,
};

use super::{
    capability::{
        Capability, CapabilityId, CapabilityKind, Delegations, Denied, Operation, Restriction,
        Source,
    },
    dirty_io::{Claim, Completion, DirtyIo},
    ets::EtsTables,
    host::{
        input::{self, Readiness},
        output::IovecWrite,
        signal,
    },
    local_node,
    persistent_term::PersistentTerms,
    port::Ports,
    process::{
        DirtyWrite, Flush, Frame, LinkId, Monitor, Origin, OriginMonitor, ProcessState,
        ReceiveTimer, Start, TargetMonitor, TimerReference, WalkPosition,
    },
    regex_code::RegexCodes,
    resources::Resources,
    signal::Signal,
    timer::{Receiver, Timer, Timers},
    tty::Ttys,
};
use crate::{
    error::{BeamError, error},
    term::{Atom, AtomTable, ProcessId, Reference, Term},
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
/// switches out at an ERTS switch point. It follows the ERTS model: the
/// process table holds every live process, also an exiting one,
/// `current_process` names the running process, and the run queue holds
/// only processes that wait for a turn. A process can be in the run queue
/// for Erlang work or only to handle its signals.
#[derive(Debug)]
pub(super) struct Scheduler {
    /// All live processes, including the current process.
    processes: HashMap<ProcessId, ProcessState>,
    /// The processes that left the process table in their exit and switched
    /// out before the end of their exit, as ERTS keeps a process in the
    /// `ERTS_PSFLG_FREE` state (`erl_process.c:14469`, `14814-14816`), with
    /// the phase where each exit continues. No pid lookup finds them. The
    /// run queue holds their pids, and each continues its exit in its next
    /// turn.
    freed: HashMap<ProcessId, (ProcessState, ExitPhase)>,
    /// Processes that wait for a turn, in run order. The current process is
    /// not in this queue, as in ERTS.
    runnable: VecDeque<ProcessId>,
    /// The process that runs now, like `current_process` in the ERTS
    /// scheduler data.
    current_process: Option<ProcessId>,
    /// Number for the next process identifier.
    next_process_id: u64,
    /// Number for the next reference.
    next_reference: u64,
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
    /// The modules whose NIF library `erlang:load_nif/2` loaded in the run,
    /// as the `nif` field of the ERTS module instance
    /// (`beam/erl_nif.c:4774-4777`, `4990-4997`). PBeam has one instance of
    /// each module, so the module name identifies it.
    pub(super) nif_libraries: HashSet<String>,
    /// The timers of the run and the run clock. All processes of the run
    /// share them.
    pub(super) timers: Timers,
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
    /// The virtual reductions of the current turn, as `esdp->virtual_reds`.
    /// They use the budget, but the reduction total does not count them
    /// (`bif.h:85-96`). Each switch out sets them to 0
    /// (`erl_process.c:9688`).
    virtual_reductions: i32,
    /// The dirty IO threads of the run and the inbox of their completions.
    dirty_io: DirtyIo,
    /// The delegation records of the capability copies of the run.
    delegations: Delegations,
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
    /// let scheduler = Scheduler::new(program.atoms.clone());
    /// assert_eq!(scheduler.atoms.get("ok"), Atom::from_static("ok"));
    /// ```
    pub(super) fn new(atoms: AtomTable) -> Self {
        let mut resources = Resources::default();
        resources.put(RegexCodes::default());
        resources.put(Ttys::default());
        resources.put(Ports::default());
        Scheduler {
            processes: HashMap::new(),
            freed: HashMap::new(),
            runnable: VecDeque::new(),
            current_process: None,
            next_process_id: ROOT.0 + 1,
            next_reference: 0,
            root_result: None,
            names: HashMap::new(),
            persistent_terms: PersistentTerms::default(),
            ets: EtsTables::default(),
            resources,
            atoms,
            nif_libraries: HashSet::new(),
            timers: Timers::new(),
            alias_owners: HashMap::new(),
            pending_monitor: None,
            next_unlink_id: 1,
            system_logger: Term::static_atom("logger"),
            virtual_reductions: 0,
            dirty_io: DirtyIo::new(),
            delegations: Delegations::default(),
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
    /// let scheduler = Scheduler::without_checks(program.atoms.clone());
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

    /// Store a process and add it to the end of the run queue.
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// scheduler.insert(ProcessState::new(ProcessId(1), Start::Entry(frame), vec![]));
    /// ```
    fn insert(&mut self, process: ProcessState) {
        let id = process.id;
        self.processes.insert(id, process);
        self.enqueue(id);
    }

    /// Put the process `id` at the end of the run queue, unless it is in the
    /// run queue or runs now: such a process keeps its place, as
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
        let current = self.current_process == Some(id);
        let process = self.processes.get_mut(&id).expect("the process is alive");
        let idle_dirty = process.dirty_running() && !process.mailbox.has_control_signals();
        if !process.in_run_queue && !current && !idle_dirty {
            process.in_run_queue = true;
            self.runnable.push_back(id);
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
        let Some(id) = self.runnable.pop_front() else {
            return Ok(Turn::Idle);
        };
        if let Some((process, phase)) = self.freed.remove(&id) {
            self.exit_links(process, phase, CONTEXT_REDS);
            return Ok(Turn::Done);
        }
        if self.processes[&id].dirty_running() {
            self.handle_dirty_signals(id)?;
            return Ok(Turn::Done);
        }
        self.current_process = Some(id);
        let process = self.current_process_mut();
        process.in_run_queue = false;
        process.fcalls = CONTEXT_REDS;
        if let Some(reason) = process.exiting.clone() {
            self.finish_current(reason, None);
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
                self.finish_current(reason, None);
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
                        Receiver::Name(name) => self.whereis(name.name()),
                    };
                    if let Some(target) = target {
                        self.deliver(target, message);
                    }
                }
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
        let fds: Vec<_> = [
            selected.then_some(libc::STDIN_FILENO),
            dirty.then(|| self.dirty_io.wake_fd()),
            signal::notify_fd(),
        ]
        .into_iter()
        .flatten()
        .chain(ports)
        .collect();
        if fds.is_empty() {
            return Ok(false);
        }
        let readiness = input::wait_readable(&fds, timeout);
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
                let message = Term::Tuple(vec![
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
                    Term::Tuple(vec![Term::static_atom("notify"), Term::static_atom(name)]);
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
        let id = self
            .current_process
            .expect("the scheduler has a current process");
        let process = self.processes.get_mut(&id).expect("the process is alive");
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
        self.current_process
            .and_then(|id| self.processes.get(&id))
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
        self.current_process
            .and_then(|id| self.processes.get_mut(&id))
            .expect("the scheduler has a current process")
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
        let id = self
            .current_process
            .take()
            .expect("the scheduler has a current process");
        let process = self.processes.get_mut(&id).expect("the process is alive");
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

    /// Return the root result. `None` means the root process did not exit.
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// assert_eq!(scheduler.into_root_result(), Some(Ok(Term::static_atom("ok"))));
    /// ```
    pub(super) fn into_root_result(self) -> Option<Result<Term, Term>> {
        self.root_result
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
    /// `off_heap_message_queue` sets the start value of its
    /// `message_queue_data` flag. The link and the monitor go into both
    /// processes at once, as `erl_create_process` does, so no signal goes
    /// to the child.
    ///
    /// First the current process must hold a `process` capability with
    /// `spawn` that permits the module of `start`, and with `link` and
    /// `monitor` when the spawn links or monitors. The child gets the
    /// delegable capabilities of the current process, reduced by
    /// `restrictions` (see `SecurityContext::delegate`). A denied check or
    /// delegation gives `Ok(Err(Denied))` before the spawn takes a pid or a
    /// reference. In a run that skips the checks, the spawn has no check,
    /// and the child gets no copy and ignores `restrictions`.
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// let (pid, monitor) = scheduler.spawn(Start::Entry(frame), vec![], true, false, false, None)?.unwrap();
    /// assert!(monitor.is_some());
    /// ```
    pub(super) fn spawn(
        &mut self,
        start: Start,
        arguments: Vec<Term>,
        monitor: bool,
        link: bool,
        off_heap_message_queue: bool,
        restrictions: Option<&[Restriction]>,
    ) -> Result<Result<(ProcessId, Option<Reference>), Denied>, BeamError> {
        let module = match &start {
            Start::Entry(frame) => frame.module.as_str(),
            Start::Undefined { module, .. } => module.name(),
        };
        let parent = &self.current_process().security;
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
        child.off_heap_message_queue = off_heap_message_queue;
        child.security = self.delegations.hold(child.id, copies);
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
        let (kind, id, item) = match target {
            MonitorTarget::Process(id) => ("process", Some(id), Term::Pid(id)),
            MonitorTarget::ProcessName(name) => {
                let id = self.whereis(name.name());
                ("process", id, named_item(&name))
            }
            MonitorTarget::PortName(name) => ("port", None, named_item(&name)),
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
        match id {
            Some(id) if kind == "process" => {
                self.send_monitor(id, Monitor { watcher, reference });
            }
            _ => self.send_monitor_down(None, watcher, reference, Term::static_atom("noproc")),
        }
        Ok(reference)
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
        if let (Some(target), "process") = (monitor.target, monitor.kind) {
            let demonitor = Signal::Demonitor { watcher, reference };
            // A target that does not exist drops the signal.
            let _ = self.send_signal(Some(watcher), target, demonitor);
        }
        true
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
        match self.processes.get_mut(id) {
            Some(process)
                if name.name() != "undefined"
                    && process.registered_name.is_none()
                    && !self.names.contains_key(name.name()) =>
            {
                process.registered_name = Some(name.clone());
                self.names.insert(name.name().to_owned(), *id);
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
        let Some(id) = self.names.remove(name.name()) else {
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
    /// scheduler.finish_current(Term::static_atom("normal"), Some(result));
    /// ```
    pub(super) fn finish_current(&mut self, reason: Term, result: Option<Term>) {
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
            return;
        }
        reds -= FREE_REDUCTIONS;
        self.schedule_out();
        let mut process = self
            .processes
            .remove(&id)
            .expect("the exiting process is alive");
        debug_assert!(!process.in_run_queue, "only the current process exits");
        if let Some(name) = &process.registered_name {
            self.names.remove(name.name());
        }
        self.delegations
            .release(std::mem::take(&mut process.security));
        self.policy_exited(id, &reason);
        process.mailbox.fetch();
        process.exiting = Some(reason);
        if reds <= 0 {
            self.switch_out_freed(process, ExitPhase::Links(None));
            return;
        }
        self.exit_links(process, ExitPhase::Links(None), reds);
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
        self.freed.insert(id, (process, phase));
        self.runnable.push_back(id);
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
    /// scheduler.exit_links(process, ExitPhase::Links(None), 3850);
    /// ```
    fn exit_links(&mut self, mut process: ProcessState, mut phase: ExitPhase, mut reds: i32) {
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
                            return self.switch_out_freed(process, ExitPhase::Links(position));
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
                            LinkId::Port(port) => self.port_link_exit(port, id),
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
                            return self.switch_out_freed(process, ExitPhase::Monitors(position));
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
                        reds -= 1;
                    }
                    ExitPhase::TargetMonitors(0)
                }
                ExitPhase::TargetMonitors(mut done) => {
                    while let Some(&monitor) = process.monitored_by.get(done) {
                        if reds <= 0 {
                            return self.switch_out_freed(process, ExitPhase::TargetMonitors(done));
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
                        }
                        done += 1;
                        reds -= 1;
                    }
                    ExitPhase::Signals
                }
                ExitPhase::Signals => {
                    if !self.handle_exit_signals(&mut process, reds) {
                        return self.switch_out_freed(process, ExitPhase::Signals);
                    }
                    break;
                }
            };
            // A walk that ends gives 1 when no reductions are left.
            reds = reds.max(1);
        }
        self.alias_owners.retain(|_, owner| *owner != id);
        self.send_pending_monitor();
    }
}

/// Return the `DOWN` item of a monitor by name: `{Name, nonode@nohost}`, as
/// ERTS builds it for a local name (`erl_proc_sig_queue.c:4942-4955`).
///
/// # Examples
///
/// ```rust,ignore
/// assert_eq!(named_item(&name), Term::Tuple(vec![Term::Atom(name.clone()), local_node()]));
/// ```
fn named_item(name: &Atom) -> Term {
    Term::Tuple(vec![Term::Atom(name.clone()), local_node()])
}

mod policy;
mod ports;
mod signals;

pub(super) use policy::PolicyEnd;
use policy::PolicyProcess;
pub(super) use signals::{Flushed, HandlingPoint, Received};

#[cfg(test)]
#[path = "../../tests/runtime/scheduler.rs"]
mod tests;

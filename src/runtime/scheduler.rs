use std::collections::{HashMap, HashSet, VecDeque};

use super::{
    ets::EtsTables,
    local_node,
    persistent_term::PersistentTerms,
    process::{Flush, Frame, Monitor, Origin, OriginMonitor, ProcessState, ReceiveTimer, Start},
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
    /// Return value or exit reason of the root process after it exits.
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
        Scheduler {
            processes: HashMap::new(),
            runnable: VecDeque::new(),
            current_process: None,
            next_process_id: 0,
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
        }
    }

    /// Create the root process. The root process gets `ProcessId(0)`.
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// let mut scheduler = Scheduler::new(AtomTable::new());
    /// scheduler.start(frame, vec![])?;
    /// assert_eq!(scheduler.schedule_next(), Some(ProcessId(0)));
    /// ```
    pub(super) fn start(&mut self, frame: Frame, arguments: Vec<Term>) -> Result<(), BeamError> {
        let id = self.next_pid()?;
        self.insert(ProcessState::new(id, Start::Entry(frame), arguments));
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
    /// (`erl_proc_sig_queue.h:2063-2088`).
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// scheduler.enqueue(child);
    /// ```
    fn enqueue(&mut self, id: ProcessId) {
        let current = self.current_process == Some(id);
        let process = self.processes.get_mut(&id).expect("the process is alive");
        if !process.in_run_queue && !current {
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
    /// - An exiting process runs its exit now.
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
    /// An abnormal exit signal through a link to a process that does not
    /// trap exits is not supported (see [`Scheduler::handle_signals`]).
    pub(super) fn schedule_next(&mut self) -> Result<Turn, BeamError> {
        let Some(id) = self.runnable.pop_front() else {
            return Ok(Turn::Idle);
        };
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
                    receiver, message, ..
                } => {
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

    /// Create the next process identifier.
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// assert_eq!(Scheduler::new(AtomTable::new()).next_pid()?, ProcessId(0));
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

    /// Return whether the root process exited.
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
        self.root_result.is_some()
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
    /// # Examples
    ///
    /// ```rust,ignore
    /// let (pid, monitor) = scheduler.spawn(Start::Entry(frame), vec![], true, false, false)?;
    /// assert!(monitor.is_some());
    /// ```
    pub(super) fn spawn(
        &mut self,
        start: Start,
        arguments: Vec<Term>,
        monitor: bool,
        link: bool,
        off_heap_message_queue: bool,
    ) -> Result<(ProcessId, Option<Reference>), BeamError> {
        let mut child = ProcessState::new(self.next_pid()?, start, arguments);
        child.off_heap_message_queue = off_heap_message_queue;
        let parent = self.current_process_mut().id;
        child.group_leader = self.current_process_mut().group_leader;
        let reference = if monitor {
            let reference = self.make_ref()?;
            child.monitored_by.push(Monitor {
                watcher: parent,
                reference,
            });
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
            self.current_process_mut().links.insert(child.id, 0);
            child.links.insert(parent, 0);
        }
        let id = child.id;
        self.insert(child);
        Ok((id, reference))
    }

    /// Link the current process with a live local process, as `link_opt`
    /// (`bif.c:155-184`) does for `erlang:link/1`, and return `true`. A link
    /// to the current process changes nothing. An active link sends no
    /// signal. A new link, or an unlinking link that becomes active again,
    /// sends a link signal to the target, which adds its side of the link
    /// when it handles the signal.
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// assert_eq!(scheduler.link(child)?, Term::static_atom("true"));
    /// ```
    ///
    /// # Errors
    ///
    /// A link to a process that does not exist or exits is not supported:
    /// ERTS raises `noproc`, or with `trap_exit` sends the `noproc` exit
    /// signal (`bif.c:162-163`, `410-424`).
    pub(super) fn link(&mut self, target: ProcessId) -> Result<Term, BeamError> {
        let current = self.current_process_mut().id;
        if target == current {
            return Ok(Term::static_atom("true"));
        }
        if self.live_process(target).is_none() {
            // ponytail: ERTS gives noproc for a missing target; #9 adds it.
            return Err(error(format!(
                "unsupported link to missing process {target:?}"
            )));
        }
        if self.current_process_mut().links.insert(target, 0) == Some(0) {
            return Ok(Term::static_atom("true"));
        }
        self.send_signal(Some(current), target, Signal::Link { from: current })
            .expect("a live process gets the link signal");
        Ok(Term::static_atom("true"))
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
    /// assert_ne!(scheduler.current_process_mut().links[&child], 0);
    /// ```
    pub(super) fn unlink(&mut self, target: ProcessId) {
        let current = self.current_process_mut().id;
        if self.current_process_mut().links.get(&target) != Some(&0) {
            return;
        }
        let id = self.next_unlink_id;
        self.next_unlink_id += 1;
        let unlink = Signal::Unlink { from: current, id };
        if self.send_signal(Some(current), target, unlink).is_ok() {
            self.current_process_mut().links.insert(target, id);
        } else {
            self.current_process_mut().links.remove(&target);
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
    /// current process at once: PBeam has no ports, so the port lookup
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

    /// Return the process that has the registered name `name`, as ERTS
    /// `erts_whereis_name_to_id` (`register.c:276-300`) finds it in the
    /// `process_reg` table. Return `None` when no live process has the name.
    /// PBeam has no ports, so a name never gives a port.
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

    /// Switch out the current process after it exits, and remove it as
    /// [`Self::exit_process`] does.
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// scheduler.finish_current(Term::static_atom("normal"), Some(result));
    /// ```
    pub(super) fn finish_current(&mut self, reason: Term, result: Option<Term>) {
        let id = self.schedule_out().id;
        self.exit_process(id, reason, result);
    }

    /// Remove the process `id` after it exits, as `erts_continue_exit_process`
    /// (`erl_process.c:14270-14833`) does in one turn:
    ///
    /// 1. Stop its receive timer (`erl_process.c:14207-14210`) and the BIF
    ///    timers to its pid (`erl_process.c:14300-14303`), delete its ETS
    ///    tables (`ERTS_CONTINUE_EXIT_USING_DB`, `erl_process.c:14360-14368`),
    ///    and free its registered name (`ERTS_CONTINUE_EXIT_FREE`). Then the
    ///    process leaves the process table (`erl_process.c:14406-14466`).
    /// 2. Fetch its signal queue (`erl_process.c:14537`).
    /// 3. Send a link exit signal to each linked process, but not through a
    ///    link that is unlinking (`erts_proc_exit_handle_link`,
    ///    `erl_process.c:14068-14081`).
    /// 4. Send a demonitor signal to the target of each monitor that it
    ///    holds (`erl_process.c:13891-13898`).
    /// 5. Run its TTY resource monitors, and send a monitor down signal to
    ///    each process that monitors it (`erl_process.c:13806-13815`).
    /// 6. Answer the signals that it did not handle (see
    ///    [`Scheduler::handle_exit_signals`]).
    /// 7. Send the pending monitor signal of the process, when it has one.
    ///
    /// Only the current process exits, so the process is not in the run
    /// queue. The links and the monitors go in the order of their keys;
    /// ERTS goes through its trees in tree order. The root process records
    /// its result.
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// scheduler.exit_process(child, Term::static_atom("killed"), None);
    /// assert!(scheduler.process(child).is_none());
    /// ```
    fn exit_process(&mut self, id: ProcessId, reason: Term, result: Option<Term>) {
        let mut process = self
            .processes
            .remove(&id)
            .expect("the exiting process is alive");
        debug_assert!(!process.in_run_queue, "only the current process exits");
        if let ReceiveTimer::Set { key, .. } = process.receive_timer {
            self.timers.cancel(key);
        }
        self.timers.cancel_process(id);
        self.ets.delete_owned(id);
        if let Some(name) = &process.registered_name {
            self.names.remove(name.name());
        }
        if id == ProcessId(0) {
            let normal = reason == Term::static_atom("normal");
            self.root_result = Some(match result {
                Some(result) if normal => Ok(result),
                _ => Err(reason.clone()),
            });
        }
        process.mailbox.fetch();
        for (&linked, &unlinking) in &process.links {
            if unlinking == 0 {
                let exit = Signal::Exit {
                    from: id,
                    reason: reason.clone(),
                    linked: true,
                    normal_kills: false,
                    alias: None,
                };
                // A linked process that does not exist drops the signal.
                let _ = self.send_signal(Some(id), linked, exit);
            }
        }
        for (&reference, origin) in &process.monitors {
            if let Some(OriginMonitor {
                kind: "process",
                target: Some(target),
                ..
            }) = &origin.monitor
            {
                let target = *target;
                let demonitor = Signal::Demonitor {
                    watcher: id,
                    reference,
                };
                // A target that does not exist drops the signal.
                let _ = self.send_signal(Some(id), target, demonitor);
            }
        }
        // ERTS runs the NIF resource monitors of the process in the monitor
        // stage too (`ERTS_CONTINUE_EXIT_LT_MONITORS`).
        self.resources.borrow_mut::<Ttys>().process_down(id);
        for monitor in &process.monitored_by {
            self.send_monitor_down(Some(id), monitor.watcher, monitor.reference, reason.clone());
        }
        self.handle_exit_signals(&mut process);
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

mod signals;

pub(super) use signals::{Flushed, HandlingPoint, Received};

#[cfg(test)]
mod tests;

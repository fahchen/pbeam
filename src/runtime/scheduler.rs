use std::collections::{HashMap, HashSet, VecDeque};

use super::{
    ets::EtsTables,
    local_node,
    persistent_term::PersistentTerms,
    process::{Frame, Monitor, ProcessState, ReceiveTimer, Start},
    regex_code::RegexCodes,
    resources::Resources,
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

/// An active alias: the process that gets the messages and exit signals to
/// the alias reference, and the mode of the alias. In ERTS the reference
/// holds the pid of the owner (`erts_get_pid_of_ref`,
/// `erl_bif_unique.h:279`), and the monitor of the owner holds the mode.
#[derive(Debug)]
struct Alias {
    /// The process that made the alias.
    owner: ProcessId,
    /// The mode of the alias.
    mode: AliasMode,
}

/// A scheduler on one OS thread. A process runs until it waits, exits, or
/// switches out at an ERTS switch point. It follows the ERTS model: the
/// process table holds every live process, `current_process` names the
/// running process, and the run queue holds only processes that wait for a
/// turn.
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
    /// The active aliases of the run. An alias that ends, and each alias of
    /// a process that exits, leaves the table.
    aliases: HashMap<Reference, Alias>,
    /// The process that gets the error reports of the emulator: `logger`,
    /// `undefined`, or a pid, as `system_logger` in `erl_trace.c`. The start
    /// value is `logger` (`erl_trace.c:339`).
    pub(super) system_logger: Term,
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
            aliases: HashMap::new(),
            system_logger: Term::static_atom("logger"),
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

    /// Store a process and add it to the ready queue.
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// scheduler.insert(ProcessState::new(ProcessId(1), Start::Entry(frame), vec![]));
    /// ```
    fn insert(&mut self, process: ProcessState) {
        self.runnable.push_back(process.id);
        self.processes.insert(process.id, process);
    }

    /// Take the next process from the run queue and make it the current
    /// process with a budget of `CONTEXT_REDS` reductions, as ERTS sets
    /// `p->fcalls` at schedule-in (`erl_process.c:10183`, `10435`). A
    /// process whose receive timer expired continues at the instruction
    /// after its `wait_timeout` with `F_TIMO`, also when a message woke it
    /// first, as ERTS does before it runs the process
    /// (`erl_process.c:10451-10458`).
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// assert_eq!(scheduler.schedule_next(), Some(ProcessId(0)));
    /// assert_eq!(scheduler.current_process_mut().fcalls, CONTEXT_REDS);
    /// ```
    pub(super) fn schedule_next(&mut self) -> Option<ProcessId> {
        self.current_process = self.runnable.pop_front();
        let process = self
            .current_process
            .and_then(|id| self.processes.get_mut(&id))?;
        process.fcalls = CONTEXT_REDS;
        if let ReceiveTimer::Expired { resume } = process.receive_timer
            && let Some(frame) = process.frames.last_mut()
        {
            frame.next_instruction = resume;
            process.receive_timer = ReceiveTimer::Timo;
        }
        self.current_process
    }

    /// Expire each timer that is due at the time `now`, in end order. A
    /// process whose receive timer expires goes to the end of the run queue
    /// when it waits, and a process in the run queue stays there
    /// (`proc_timeout_common`, `erl_hl_timer.c:692-706`). A BIF timer sends
    /// its message to its pid, or to the process that has its name now, and
    /// a name that no process has drops the message (`bif_timer_timeout`,
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
                        self.runnable.push_back(id);
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

    /// Return the live process `id`, also the current process. Return `None`
    /// when no live process has the pid, as `erts_try_lock_sig_free_proc`
    /// (`erl_bif_info.c:1384-1386`) finds no process.
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// scheduler.schedule_next();
    /// assert_eq!(scheduler.process(ProcessId(0)).map(|process| process.id), Some(ProcessId(0)));
    /// assert!(scheduler.process(ProcessId(99)).is_none());
    /// ```
    pub(super) fn process(&self, id: ProcessId) -> Option<&ProcessState> {
        self.processes.get(&id)
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

    /// Stop the current process until a message arrives. A delivery adds it
    /// to the run queue again.
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// scheduler.park_current();
    /// ```
    pub(super) fn park_current(&mut self) {
        self.current_process_mut().wait();
        self.current_process = None;
    }

    /// Stop the current process after `erlang:hibernate/0`. The process
    /// fetches the messages that arrived. A process with a message in its
    /// queue stays active and goes to the end of the run queue. Else it
    /// waits until a message arrives. ERTS `erts_hibernate` fetches and
    /// clears `ERTS_PSFLG_ACTIVE` only when the queue has no message
    /// (`beam_common.c:1568-1602`), `i_hibernate` always switches the
    /// context (`jit/arm/instr_call.cpp:30-36`), and `schedule_out_process`
    /// puts an active process at the end of the run queue
    /// (`erl_process.c:6695`).
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// scheduler.hibernate_current();
    /// ```
    pub(super) fn hibernate_current(&mut self) {
        let process = self.current_process_mut();
        process.mailbox.fetch();
        if process.mailbox.has_messages() {
            let id = process.id;
            self.runnable.push_back(id);
        } else {
            process.wait();
        }
        self.current_process = None;
    }

    /// Switch out the current process at the end of its budget, or at
    /// another ERTS switch point. The process stays active and goes to the
    /// end of the run queue, as `schedule_out_process` does
    /// (`erl_process.c:6695-6851`). It keeps its execution state, so it
    /// continues at its next instruction with a new budget.
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// scheduler.yield_current();
    /// assert_eq!(scheduler.schedule_next(), Some(other));
    /// ```
    pub(super) fn yield_current(&mut self) {
        let id = self
            .current_process
            .take()
            .expect("the scheduler has a current process");
        self.runnable.push_back(id);
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
        self.next_reference = value
            .checked_add(1)
            .ok_or_else(|| error("process reference overflow"))?;
        Ok(Reference(value))
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

    /// Deliver a message to a live process. Add the process to the run queue
    /// when the message wakes it. A message to a process that does not exist
    /// is dropped, as in ERTS.
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
        if process.receive(term) {
            self.runnable.push_back(target);
        }
    }

    /// Spawn a child process for the current process and optionally monitor
    /// or link it. `start` gives its entry frame or its undefined start MFA.
    /// `off_heap_message_queue` sets the start value of its
    /// `message_queue_data` flag.
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
                name: None,
                tag: Term::static_atom("DOWN"),
            });
            Some(reference)
        } else {
            None
        };
        if link {
            self.current_process_mut().links.insert(child.id);
            child.links.insert(parent);
        }
        let id = child.id;
        self.insert(child);
        Ok((id, reference))
    }

    /// Link the current process with a live local process, as
    /// `erlang:link/1` does. Return `true`, as ERTS does.
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// assert_eq!(scheduler.link(child)?, Term::static_atom("true"));
    /// ```
    ///
    /// # Errors
    ///
    /// A link to a process that does not exist is not supported.
    pub(super) fn link(&mut self, target: ProcessId) -> Result<Term, BeamError> {
        let current = self.current_process_mut().id;
        let Some(process) = self.processes.get_mut(&target) else {
            // ponytail: ERTS sends noproc for a dead target; add it when a fixture links to one.
            return Err(error(format!(
                "unsupported link to missing process {target:?}"
            )));
        };
        process.links.insert(current);
        self.current_process_mut().links.insert(target);
        Ok(Term::static_atom("true"))
    }

    /// Remove the link between the current process and `target`, as
    /// `erlang:unlink/1` (`bif.c:1236-1256`) does for a local pid. PBeam has
    /// no signal queue, so both sides lose the link at once. A target
    /// without a link, a target that does not exist, and the current process
    /// itself change nothing. An `EXIT` message in the mailbox stays.
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// scheduler.link(child)?;
    /// scheduler.unlink(child);
    /// assert!(scheduler.current_process_mut().links.is_empty());
    /// ```
    pub(super) fn unlink(&mut self, target: ProcessId) {
        let current = self.current_process_mut().id;
        if let Some(process) = self.processes.get_mut(&target) {
            process.links.remove(&current);
        }
        self.current_process_mut().links.remove(&target);
    }

    /// Monitor a target for the current process, as ERTS `monitor`
    /// (`bif.c:758-981`) does after its checks. A process by name is the
    /// process that has the name now (`erts_whereis_name_to_id`). A monitor
    /// of the current process creates no monitor and no alias
    /// (`bif.c:781-784`). A process that does not exist, a name that no
    /// process has, and each port name give the message
    /// `{Tag, Ref, Type, Item, noproc}` at once: PBeam has no ports, so the
    /// port lookup always fails (`bif.c:910-914`). `Item` is the pid, or
    /// `{Name, nonode@nohost}` for a name.
    ///
    /// With an alias option, the reference is also an active alias of the
    /// current process. The `noproc` `DOWN` message ends a `demonitor` or
    /// `reply_demonitor` alias at once (`erl_proc_sig_queue.c:6463-6490`),
    /// so only an `explicit_unalias` alias stays for it.
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
        let reference = self.make_ref()?;
        let watcher = self.current_process_mut().id;
        let (kind, id, item, name) = match target {
            MonitorTarget::Process(id) => ("process", Some(id), Term::Pid(id), None),
            MonitorTarget::ProcessName(name) => {
                let id = self.whereis(name.name());
                ("process", id, named_item(&name), Some(name))
            }
            MonitorTarget::PortName(name) => ("port", None, named_item(&name), Some(name)),
        };
        if id == Some(watcher) {
            return Ok(reference);
        }
        if options.priority {
            // ponytail: no priority messages yet; add them when a measured path uses the option.
            return Err(error("unsupported monitor option priority"));
        }
        let monitor = Monitor {
            watcher,
            reference,
            name,
            tag: options.tag,
        };
        let mode = match id.and_then(|id| self.processes.get_mut(&id)) {
            Some(process) => {
                process.monitored_by.push(monitor);
                options.alias
            }
            None => {
                let noproc = Term::static_atom("noproc");
                let down = down_message(monitor.tag, reference, kind, item, noproc);
                self.deliver(watcher, down);
                options
                    .alias
                    .filter(|&mode| mode == AliasMode::ExplicitUnalias)
            }
        };
        if let Some(mode) = mode {
            let alias = Alias {
                owner: watcher,
                mode,
            };
            self.aliases.insert(reference, alias);
        }
        Ok(reference)
    }

    /// Remove a monitor that the current process holds, as the ERTS
    /// `demonitor` function does. Return true when an active monitor was
    /// removed, and false when it already fired or never existed. The
    /// removal ends a `demonitor` or `reply_demonitor` alias of the
    /// monitor, and keeps an `explicit_unalias` alias (`bif.c:470-492`).
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// let reference = scheduler.monitor(MonitorTarget::Process(child), options)?;
    /// assert!(scheduler.demonitor(reference));
    /// assert!(!scheduler.demonitor(reference));
    /// ```
    pub(super) fn demonitor(&mut self, reference: Reference) -> bool {
        let current = self.current_process_mut().id;
        let removed = self.remove_monitor(current, reference);
        if removed {
            self.end_monitor_alias(reference);
        }
        removed
    }

    /// Remove the monitor `reference` of `watcher` from its target. Return
    /// true when the monitor was active.
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// assert!(scheduler.remove_monitor(watcher, reference));
    /// assert!(!scheduler.remove_monitor(watcher, reference));
    /// ```
    fn remove_monitor(&mut self, watcher: ProcessId, reference: Reference) -> bool {
        for process in self.processes.values_mut() {
            if let Some(index) = process
                .monitored_by
                .iter()
                .position(|monitor| monitor.watcher == watcher && monitor.reference == reference)
            {
                process.monitored_by.remove(index);
                return true;
            }
        }
        false
    }

    /// End the alias of the monitor `reference` when its monitor ends,
    /// unless it is an `explicit_unalias` alias (`bif.c:470-492`,
    /// `erl_proc_sig_queue.c:6463-6490`).
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// scheduler.end_monitor_alias(reference);
    /// assert_eq!(scheduler.alias_owner(reference), None);
    /// ```
    fn end_monitor_alias(&mut self, reference: Reference) {
        if self
            .aliases
            .get(&reference)
            .is_some_and(|alias| alias.mode != AliasMode::ExplicitUnalias)
        {
            self.aliases.remove(&reference);
        }
    }

    /// Return the owner of the active alias `reference`, or `None` when the
    /// reference is not an active alias. ERTS takes the pid from the
    /// reference (`erts_get_pid_of_ref`, `erl_bif_unique.h:279`), and the
    /// owner ignores a signal through an alias that is not active
    /// (`erl_proc_sig_queue.c:4659-4672`, `6026-6041`).
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// assert_eq!(scheduler.alias_owner(alias), Some(owner));
    /// assert_eq!(scheduler.alias_owner(plain_reference), None);
    /// ```
    pub(super) fn alias_owner(&self, reference: Reference) -> Option<ProcessId> {
        self.aliases.get(&reference).map(|alias| alias.owner)
    }

    /// Return the receiver of a message to the reference `reference`: the
    /// owner of the active alias, or `None` to drop the message. A message
    /// through a `reply_demonitor` alias ends the alias and removes its
    /// monitor (`erl_proc_sig_queue.c:6047-6070`).
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// assert_eq!(scheduler.alias_message_receiver(once), Some(owner));
    /// assert_eq!(scheduler.alias_message_receiver(once), None);
    /// ```
    pub(super) fn alias_message_receiver(&mut self, reference: Reference) -> Option<ProcessId> {
        let alias = self.aliases.get(&reference)?;
        let owner = alias.owner;
        if alias.mode == AliasMode::ReplyDemonitor {
            self.aliases.remove(&reference);
            self.remove_monitor(owner, reference);
        }
        Some(owner)
    }

    /// Set the group leader of a live local process, as
    /// `erts_set_group_leader` does for `erts_internal:group_leader/2`.
    /// Return `false` when the target does not exist.
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// assert!(scheduler.set_group_leader(child, leader));
    /// assert!(!scheduler.set_group_leader(ProcessId(99), leader));
    /// ```
    pub(super) fn set_group_leader(&mut self, target: ProcessId, leader: ProcessId) -> bool {
        let Some(process) = self.processes.get_mut(&target) else {
            return false;
        };
        process.group_leader = leader;
        true
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

    /// Remove the current process after it exits, as [`Self::exit_process`]
    /// does.
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// scheduler.finish_current(Term::static_atom("normal"), Some(result))?;
    /// ```
    ///
    /// # Errors
    ///
    /// An abnormal exit signal to a linked process that does not trap exits
    /// is not supported. The error comes after all exit stages, so the TTY
    /// resource monitors run on each exit, as `tty_monitor_down`
    /// (`prim_tty_nif.c:1235-1241`) does in ERTS.
    pub(super) fn finish_current(
        &mut self,
        reason: Term,
        result: Option<Term>,
    ) -> Result<(), BeamError> {
        let id = self
            .current_process
            .take()
            .expect("the scheduler has a current process");
        self.exit_process(id, reason, result)
    }

    /// Send the exit signal of `erlang:exit(Target, Reason)` from the current
    /// process to the local process `target`, and handle it in the receiver
    /// at once, as the row "Signal queue" of `docs/erts-differences.md`
    /// records. ERTS sends the signal in `send_exit_signal_bif`
    /// (`bif.c:1759-1772`), and the receiver handles it in
    /// `handle_exit_signal` (`erl_proc_sig_queue.c:4740-4770`):
    ///
    /// - A receiver that does not exist drops the signal.
    /// - A receiver that traps exits gets `{'EXIT', Caller, Reason}`, unless
    ///   `Reason` is `kill`.
    /// - Else `normal` is ignored, unless the caller sends it to itself:
    ///   `exit2_suicide` sets `normal_kills` (`erl_proc_sig_queue.c:1807-1809`).
    /// - Else the receiver exits with `Reason`, and `kill` gives `killed`.
    ///
    /// Return `Some(reason)` when the caller is the receiver and must exit
    /// before `exit/2` returns. Another receiver exits here.
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// assert_eq!(scheduler.send_exit(child, Term::static_atom("kill"))?, None);
    /// assert!(scheduler.process(child).is_none());
    /// let caller = scheduler.current_process_mut().id;
    /// assert_eq!(
    ///     scheduler.send_exit(caller, Term::static_atom("kill"))?,
    ///     Some(Term::static_atom("killed"))
    /// );
    /// ```
    ///
    /// # Errors
    ///
    /// An abnormal exit of the receiver to a linked process that does not
    /// trap exits is not supported.
    pub(super) fn send_exit(
        &mut self,
        target: ProcessId,
        reason: Term,
    ) -> Result<Option<Term>, BeamError> {
        let caller = self.current_process_mut().id;
        let Some(process) = self.processes.get(&target) else {
            return Ok(None);
        };
        let kill = reason == Term::static_atom("kill");
        if process.trap_exit && !kill {
            let exit = Term::Tuple(vec![Term::static_atom("EXIT"), Term::Pid(caller), reason]);
            self.deliver(target, exit);
            return Ok(None);
        }
        if reason == Term::static_atom("normal") && target != caller {
            return Ok(None);
        }
        let reason = if kill {
            Term::static_atom("killed")
        } else {
            reason
        };
        if target == caller {
            return Ok(Some(reason));
        }
        self.exit_process(target, reason, None)?;
        Ok(None)
    }

    /// Remove the process `id` after it exits, and take it out of the run
    /// queue. Stop its receive timer and the BIF timers to its pid. Delete
    /// its ETS tables first, then free its registered name, then send `EXIT`
    /// to linked processes that trap exits, then run its TTY resource
    /// monitors and send `DOWN` to monitors. ERTS (`erl_process.c`)
    /// deletes the tables in `ERTS_CONTINUE_EXIT_USING_DB`
    /// (`erl_process.c:14360-14368`), unregisters the name in
    /// `ERTS_CONTINUE_EXIT_FREE`, before `ERTS_CONTINUE_EXIT_LINKS` and
    /// `ERTS_CONTINUE_EXIT_MONITORS`, and sends link exit signals before
    /// monitor down signals. A process that traps exits also receives a
    /// `normal` exit.
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// scheduler.exit_process(child, Term::static_atom("killed"), None)?;
    /// assert!(scheduler.process(child).is_none());
    /// ```
    ///
    /// # Errors
    ///
    /// An abnormal exit signal to a linked process that does not trap exits
    /// is not supported.
    fn exit_process(
        &mut self,
        id: ProcessId,
        reason: Term,
        result: Option<Term>,
    ) -> Result<(), BeamError> {
        let process = self
            .processes
            .remove(&id)
            .expect("the exiting process is alive");
        self.runnable.retain(|&runnable| runnable != id);
        // ERTS cancels the receive timer first (`erl_process.c:14207-14210`),
        // then the BIF timers to the pid (`erl_process.c:14300-14303`).
        if let ReceiveTimer::Set { key, .. } = process.receive_timer {
            self.timers.cancel(key);
        }
        self.timers.cancel_process(id);
        self.ets.delete_owned(process.id);
        if let Some(name) = &process.registered_name {
            self.names.remove(name.name());
        }
        let normal = reason == Term::static_atom("normal");
        if process.id == ProcessId(0) {
            self.root_result = Some(match result {
                Some(result) if normal => Ok(result),
                _ => Err(reason.clone()),
            });
        }
        let mut signals = Ok(());
        for linked in process.links {
            // A missing target counts as trapping; `deliver` then drops the message.
            let trap_exit = self.processes.get_mut(&linked).is_none_or(|target| {
                target.links.remove(&process.id);
                target.trap_exit
            });
            if !trap_exit {
                if !normal {
                    // ponytail: no exit-signal propagation yet; add it when a task needs linked kills.
                    // The other exit stages still run, so fd 0 gets its TTY mode back.
                    signals = Err(error(format!(
                        "unsupported exit signal {reason:?} to a process that does not trap exits"
                    )));
                }
                continue;
            }
            let exit = Term::Tuple(vec![
                Term::static_atom("EXIT"),
                Term::Pid(process.id),
                reason.clone(),
            ]);
            self.deliver(linked, exit);
        }
        // ERTS runs the NIF resource monitors of the process in the monitor
        // stage too (`ERTS_CONTINUE_EXIT_LT_MONITORS`).
        self.resources.borrow_mut::<Ttys>().process_down(process.id);
        for monitor in process.monitored_by {
            let item = monitor
                .name
                .as_ref()
                .map_or(Term::Pid(process.id), named_item);
            self.end_monitor_alias(monitor.reference);
            let down = down_message(
                monitor.tag,
                monitor.reference,
                "process",
                item,
                reason.clone(),
            );
            self.deliver(monitor.watcher, down);
        }
        self.aliases.retain(|_, alias| alias.owner != process.id);
        signals
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

/// Return the message `{Tag, Reference, Type, Item, Reason}` of a monitor.
/// `Tag` is `'DOWN'`, or the `tag` option of `erlang:monitor/3`.
///
/// # Examples
///
/// ```rust,ignore
/// let down = down_message(Term::static_atom("DOWN"), reference, "process", Term::Pid(child), Term::static_atom("normal"));
/// ```
fn down_message(tag: Term, reference: Reference, kind: &str, item: Term, reason: Term) -> Term {
    Term::Tuple(vec![
        tag,
        Term::Reference(reference),
        Term::static_atom(kind),
        item,
        reason,
    ])
}

#[cfg(test)]
mod tests;

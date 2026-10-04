use std::collections::{HashMap, HashSet, VecDeque};

use super::process::{Frame, ProcessState};
use crate::{
    error::{BeamError, error},
    term::{ProcessId, Reference, Term},
};

/// A cooperative scheduler on one OS thread. It follows the ERTS model: the
/// process table holds every live process, `current_process` names the
/// running process, and the run queue holds only processes that wait for a
/// turn.
#[derive(Debug, Default)]
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
    /// Registered names, as the keys of the ERTS `process_reg` table. An
    /// atom name is unique in one atom table, so its text identifies it. The
    /// process of a name is its `registered_name` field; no caller looks up
    /// a process by name yet.
    names: HashSet<String>,
}

impl Scheduler {
    /// Create the root process. The root process gets `ProcessId(0)`.
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// let mut scheduler = Scheduler::default();
    /// scheduler.start(frame, vec![])?;
    /// assert_eq!(scheduler.schedule_next(), Some(ProcessId(0)));
    /// ```
    pub(super) fn start(&mut self, frame: Frame, arguments: Vec<Term>) -> Result<(), BeamError> {
        let id = self.next_pid()?;
        self.insert(ProcessState::new(id, frame, arguments));
        Ok(())
    }

    /// Store a process and add it to the ready queue.
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// scheduler.insert(ProcessState::new(ProcessId(1), frame, vec![]));
    /// ```
    fn insert(&mut self, process: ProcessState) {
        self.runnable.push_back(process.id);
        self.processes.insert(process.id, process);
    }

    /// Take the next process from the run queue and make it the current
    /// process.
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// assert_eq!(scheduler.schedule_next(), Some(ProcessId(0)));
    /// ```
    pub(super) fn schedule_next(&mut self) -> Option<ProcessId> {
        self.current_process = self.runnable.pop_front();
        self.current_process
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

    /// Create the next process identifier.
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// assert_eq!(Scheduler::default().next_pid()?, ProcessId(0));
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
    /// or link it.
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// let (pid, monitor) = scheduler.spawn(frame, vec![], true, false)?;
    /// assert!(monitor.is_some());
    /// ```
    pub(super) fn spawn(
        &mut self,
        frame: Frame,
        arguments: Vec<Term>,
        monitor: bool,
        link: bool,
    ) -> Result<(ProcessId, Option<Reference>), BeamError> {
        let mut child = ProcessState::new(self.next_pid()?, frame, arguments);
        let parent = self.current_process_mut().id;
        child.group_leader = self.current_process_mut().group_leader;
        let reference = if monitor {
            let reference = self.make_ref()?;
            child.monitored_by.push((parent, reference));
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

    /// Monitor a local process for the current process, as
    /// `erlang:monitor(process, Pid)` does. A process that does not exist
    /// gets an immediate `{'DOWN', Ref, process, Pid, noproc}`, as in ERTS.
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// let reference = scheduler.monitor(child)?;
    /// ```
    pub(super) fn monitor(&mut self, target: ProcessId) -> Result<Reference, BeamError> {
        let reference = self.make_ref()?;
        let current = self.current_process_mut().id;
        match self.processes.get_mut(&target) {
            Some(process) => process.monitored_by.push((current, reference)),
            None => self.deliver(
                current,
                Term::Tuple(vec![
                    Term::static_atom("DOWN"),
                    Term::Reference(reference),
                    Term::static_atom("process"),
                    Term::Pid(target),
                    Term::static_atom("noproc"),
                ]),
            ),
        }
        Ok(reference)
    }

    /// Remove a monitor that the current process holds, as the ERTS
    /// `demonitor` function does. Return true when an active monitor was
    /// removed, and false when it already fired or never existed.
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// let reference = scheduler.monitor(child)?;
    /// assert!(scheduler.demonitor(reference));
    /// assert!(!scheduler.demonitor(reference));
    /// ```
    pub(super) fn demonitor(&mut self, reference: Reference) -> bool {
        let current = self.current_process_mut().id;
        for process in self.processes.values_mut() {
            if let Some(index) = process
                .monitored_by
                .iter()
                .position(|&entry| entry == (current, reference))
            {
                process.monitored_by.remove(index);
                return true;
            }
        }
        false
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
                    && !self.names.contains(name.name()) =>
            {
                process.registered_name = Some(name.clone());
                self.names.insert(name.name().to_owned());
                true
            }
            _ => false,
        }
    }

    /// Remove the current process after it exits. Free its registered name
    /// first, then send `EXIT` to linked processes that trap exits, then
    /// `DOWN` to monitors. ERTS (`erl_process.c`) unregisters the name in
    /// `ERTS_CONTINUE_EXIT_FREE`, before `ERTS_CONTINUE_EXIT_LINKS` and
    /// `ERTS_CONTINUE_EXIT_MONITORS`, and sends link exit signals before
    /// monitor down signals. A process that traps exits also receives a
    /// `normal` exit.
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
    /// is not supported.
    pub(super) fn finish_current(
        &mut self,
        reason: Term,
        result: Option<Term>,
    ) -> Result<(), BeamError> {
        let process = self
            .current_process
            .take()
            .and_then(|id| self.processes.remove(&id))
            .expect("the scheduler has a current process");
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
        for linked in process.links {
            // A missing target counts as trapping; `deliver` then drops the message.
            let trap_exit = self.processes.get_mut(&linked).is_none_or(|target| {
                target.links.remove(&process.id);
                target.trap_exit
            });
            if !trap_exit {
                if normal {
                    continue;
                }
                // ponytail: no exit-signal propagation yet; add it when a task needs linked kills.
                return Err(error(format!(
                    "unsupported exit signal {reason:?} to a process that does not trap exits"
                )));
            }
            let exit = Term::Tuple(vec![
                Term::static_atom("EXIT"),
                Term::Pid(process.id),
                reason.clone(),
            ]);
            self.deliver(linked, exit);
        }
        for (watcher, reference) in process.monitored_by {
            let down = Term::Tuple(vec![
                Term::static_atom("DOWN"),
                Term::Reference(reference),
                Term::static_atom("process"),
                Term::Pid(process.id),
                reason.clone(),
            ]);
            self.deliver(watcher, down);
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::Scheduler;
    use crate::term::test_atom;
    use crate::{
        runtime::process::tests::frame,
        term::{ProcessId, Term},
    };

    fn atom(name: &str) -> Term {
        test_atom(name)
    }

    fn first_message(scheduler: &Scheduler, id: u64, index: usize) -> Term {
        scheduler.processes[&ProcessId(id)].mailbox.messages()[index].clone()
    }

    #[test]
    fn keeps_the_current_process_in_the_table_and_out_of_the_run_queue() {
        let mut scheduler = Scheduler::default();
        scheduler.start(frame(), vec![]).unwrap();
        assert_eq!(scheduler.schedule_next(), Some(ProcessId(0)));
        assert!(scheduler.processes.contains_key(&ProcessId(0)));
        let (child, monitor) = scheduler.spawn(frame(), vec![], false, false).unwrap();
        assert_eq!((child, monitor), (ProcessId(1), None));

        scheduler.park_current();
        assert_eq!(scheduler.schedule_next(), Some(ProcessId(1)));
        assert!(scheduler.runnable.is_empty());
        scheduler.deliver(ProcessId(0), atom("ping"));
        scheduler.deliver(ProcessId(1), atom("self"));
        scheduler.deliver(ProcessId(9), atom("lost"));

        assert_eq!(Vec::from(scheduler.runnable.clone()), vec![ProcessId(0)]);
        assert_eq!(first_message(&scheduler, 0, 0), atom("ping"));
        assert_eq!(first_message(&scheduler, 1, 0), atom("self"));
    }

    #[test]
    fn sends_down_and_exit_messages_and_records_the_root_result() {
        let mut scheduler = Scheduler::default();
        scheduler.start(frame(), vec![]).unwrap();
        scheduler.schedule_next();
        scheduler.current_process_mut().trap_exit = true;
        let (monitored, monitor) = scheduler.spawn(frame(), vec![], true, false).unwrap();
        let (linked, _) = scheduler.spawn(frame(), vec![], false, true).unwrap();
        scheduler.park_current();

        scheduler.schedule_next();
        scheduler.finish_current(atom("normal"), None).unwrap();
        scheduler.schedule_next();
        scheduler.finish_current(atom("boom"), None).unwrap();

        assert_eq!(scheduler.schedule_next(), Some(ProcessId(0)));
        assert!(scheduler.current_process_mut().links.is_empty());
        assert_eq!(
            [
                first_message(&scheduler, 0, 0),
                first_message(&scheduler, 0, 1)
            ],
            [
                Term::Tuple(vec![
                    atom("DOWN"),
                    Term::Reference(monitor.unwrap()),
                    atom("process"),
                    Term::Pid(monitored),
                    atom("normal"),
                ]),
                Term::Tuple(vec![atom("EXIT"), Term::Pid(linked), atom("boom")]),
            ]
        );
        scheduler
            .finish_current(atom("normal"), Some(atom("ok")))
            .unwrap();
        assert!(scheduler.processes.is_empty());
        assert_eq!(scheduler.into_root_result(), Some(Ok(atom("ok"))));
    }

    #[test]
    fn links_live_processes_and_ignores_normal_exit_without_trap_exit() {
        let mut scheduler = Scheduler::default();
        scheduler.start(frame(), vec![]).unwrap();
        scheduler.schedule_next();
        let (child, _) = scheduler.spawn(frame(), vec![], false, false).unwrap();
        assert_eq!(scheduler.link(child).unwrap(), atom("true"));
        assert!(scheduler.link(ProcessId(99)).is_err());
        scheduler.park_current();

        scheduler.schedule_next();
        assert!(
            scheduler
                .current_process_mut()
                .links
                .contains(&ProcessId(0))
        );
        scheduler.finish_current(atom("normal"), None).unwrap();

        assert_eq!(scheduler.schedule_next(), None);
        let root = &scheduler.processes[&ProcessId(0)];
        assert!(root.links.is_empty());
        assert!(root.mailbox.messages().is_empty());
    }

    #[test]
    fn rejects_unsupported_exit_signals_and_counter_overflow() {
        let mut scheduler = Scheduler::default();
        scheduler.start(frame(), vec![]).unwrap();
        scheduler.schedule_next();
        scheduler.spawn(frame(), vec![], false, true).unwrap();
        scheduler.park_current();
        scheduler.schedule_next();
        assert!(scheduler.finish_current(atom("boom"), None).is_err());

        let mut scheduler = Scheduler::default();
        scheduler.start(frame(), vec![]).unwrap();
        scheduler.schedule_next();
        scheduler
            .finish_current(atom("boom"), Some(atom("ok")))
            .unwrap();
        assert_eq!(scheduler.into_root_result(), Some(Err(atom("boom"))));

        let mut scheduler = Scheduler {
            next_process_id: u64::MAX,
            next_reference: u64::MAX,
            ..Scheduler::default()
        };
        assert!(scheduler.start(frame(), vec![]).is_err());
        assert!(scheduler.make_ref().is_err());
    }

    #[test]
    fn removes_active_monitors_and_sets_group_leaders() {
        let mut scheduler = Scheduler::default();
        scheduler.start(frame(), vec![]).unwrap();
        scheduler.schedule_next();
        let (child, _) = scheduler.spawn(frame(), vec![], false, false).unwrap();
        assert_eq!(scheduler.processes[&child].group_leader, ProcessId(0));
        assert!(scheduler.set_group_leader(child, child));
        assert_eq!(scheduler.processes[&child].group_leader, child);
        assert!(!scheduler.set_group_leader(ProcessId(99), child));

        let reference = scheduler.monitor(child).unwrap();
        assert_eq!(scheduler.processes[&child].monitored_by.len(), 1);
        assert!(scheduler.demonitor(reference));
        assert!(scheduler.processes[&child].monitored_by.is_empty());
        assert!(!scheduler.demonitor(reference));
    }

    #[test]
    fn frees_the_name_of_a_process_when_it_exits() {
        // ERTS 29.1.1: after a named process exits, register(Name, self())
        // in another process is true.
        let mut scheduler = Scheduler::default();
        scheduler.start(frame(), vec![]).unwrap();
        scheduler.schedule_next();
        let (child, _) = scheduler.spawn(frame(), vec![], false, false).unwrap();
        let name = atom("pbeam_probe_reg");
        assert!(scheduler.register(&name, &Term::Pid(child)));
        assert!(!scheduler.register(&name, &Term::Pid(ProcessId(0))));
        scheduler.park_current();

        scheduler.schedule_next();
        scheduler.finish_current(atom("normal"), None).unwrap();
        assert!(scheduler.names.is_empty());
        assert!(scheduler.register(&name, &Term::Pid(ProcessId(0))));
        assert!(scheduler.names.contains("pbeam_probe_reg"));
        assert_eq!(
            scheduler.processes[&ProcessId(0)]
                .registered_name
                .clone()
                .map(Term::Atom),
            Some(name)
        );
    }
}

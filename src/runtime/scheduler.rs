use std::collections::{HashMap, VecDeque};

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
    /// assert_eq!(scheduler.into_root_result(), Some(Ok(Term::Atom("ok".into()))));
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
    /// scheduler.deliver(target, Term::Atom("ping".into()))?;
    /// ```
    pub(super) fn deliver(&mut self, target: ProcessId, term: Term) -> Result<(), BeamError> {
        let Some(process) = self.processes.get_mut(&target) else {
            return Ok(());
        };
        if process.receive(term)? {
            self.runnable.push_back(target);
        }
        Ok(())
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

    /// Remove the current process after it exits. Send `DOWN` to monitors
    /// and `EXIT` to linked processes that trap exits.
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// scheduler.finish_current(Term::Atom("normal".into()), Some(result))?;
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
        let normal = reason == Term::Atom("normal".into());
        if process.id == ProcessId(0) {
            self.root_result = Some(match result {
                Some(result) if normal => Ok(result),
                _ => Err(reason.clone()),
            });
        }
        for (watcher, reference) in process.monitored_by {
            let down = Term::Tuple(vec![
                Term::Atom("DOWN".into()),
                Term::Reference(reference),
                Term::Atom("process".into()),
                Term::Pid(process.id),
                reason.clone(),
            ]);
            self.deliver(watcher, down)?;
        }
        for linked in process.links {
            let trap_exit = self.processes.get_mut(&linked).map(|target| {
                target.links.remove(&process.id);
                target.trap_exit
            });
            if normal {
                continue;
            }
            if trap_exit == Some(false) {
                // ponytail: no exit-signal propagation yet; add it when a task needs linked kills.
                return Err(error(format!(
                    "unsupported exit signal {reason:?} to a process that does not trap exits"
                )));
            }
            let exit = Term::Tuple(vec![
                Term::Atom("EXIT".into()),
                Term::Pid(process.id),
                reason.clone(),
            ]);
            self.deliver(linked, exit)?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::Scheduler;
    use crate::{
        runtime::process::tests::frame,
        term::{ProcessId, Term},
    };

    fn atom(name: &str) -> Term {
        Term::Atom(name.into())
    }

    fn first_message(scheduler: &Scheduler, id: u64, sequence: u64) -> Term {
        scheduler.processes[&ProcessId(id)]
            .mailbox
            .first_at_or_after(sequence)
            .unwrap()
            .term
            .clone()
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
        scheduler.deliver(ProcessId(0), atom("ping")).unwrap();
        scheduler.deliver(ProcessId(1), atom("self")).unwrap();
        scheduler.deliver(ProcessId(9), atom("lost")).unwrap();

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
}

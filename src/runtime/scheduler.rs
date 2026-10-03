use std::collections::{HashMap, VecDeque};

use super::process::{Frame, ProcessState};
use crate::{
    error::{BeamError, error},
    term::{ProcessId, Reference, Term},
};

/// A cooperative scheduler on one OS thread. The running process is taken
/// out of the process table until it waits or exits.
#[derive(Debug, Default)]
pub(super) struct Scheduler {
    /// Processes that are not running now.
    processes: HashMap<ProcessId, ProcessState>,
    /// Processes that can run, in run order.
    runnable: VecDeque<ProcessId>,
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
    /// assert_eq!(scheduler.take_next().unwrap().id, ProcessId(0));
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

    /// Take the next runnable process from the scheduler.
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// let process = scheduler.take_next();
    /// assert_eq!(process.map(|process| process.id), Some(ProcessId(0)));
    /// ```
    pub(super) fn take_next(&mut self) -> Option<ProcessState> {
        let id = self.runnable.pop_front()?;
        self.processes.remove(&id)
    }

    /// Store a process that waits for a message. A delivery makes it runnable.
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// scheduler.park(process);
    /// ```
    pub(super) fn park(&mut self, mut process: ProcessState) {
        process.wait();
        self.processes.insert(process.id, process);
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

    /// Deliver a message to the current process or another live process.
    /// A message to a process that does not exist is dropped, as in ERTS.
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// scheduler.deliver(&mut current, target, Term::Atom("ping".into()))?;
    /// ```
    pub(super) fn deliver(
        &mut self,
        current: &mut ProcessState,
        target: ProcessId,
        term: Term,
    ) -> Result<(), BeamError> {
        if target == current.id {
            current.receive(term)?;
            return Ok(());
        }
        self.deliver_to(target, term)
    }

    /// Deliver a message to a stored process and wake it when it waits.
    /// A message to a process that does not exist is dropped.
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// scheduler.deliver_to(ProcessId(1), Term::Atom("ping".into()))?;
    /// ```
    fn deliver_to(&mut self, target: ProcessId, term: Term) -> Result<(), BeamError> {
        let Some(process) = self.processes.get_mut(&target) else {
            return Ok(());
        };
        if process.receive(term)? {
            self.runnable.push_back(target);
        }
        Ok(())
    }

    /// Spawn a child process and optionally monitor or link it.
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// let (pid, monitor) = scheduler.spawn(&mut parent, frame, vec![], true, false)?;
    /// assert!(monitor.is_some());
    /// ```
    pub(super) fn spawn(
        &mut self,
        parent: &mut ProcessState,
        frame: Frame,
        arguments: Vec<Term>,
        monitor: bool,
        link: bool,
    ) -> Result<(ProcessId, Option<Reference>), BeamError> {
        let mut child = ProcessState::new(self.next_pid()?, frame, arguments);
        let reference = if monitor {
            let reference = self.make_ref()?;
            child.monitored_by.push((parent.id, reference));
            Some(reference)
        } else {
            None
        };
        if link {
            parent.links.insert(child.id);
            child.links.insert(parent.id);
        }
        let id = child.id;
        self.insert(child);
        Ok((id, reference))
    }

    /// Record a process exit. Send `DOWN` to monitors and `EXIT` to linked
    /// processes that trap exits.
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// scheduler.finish(process, Term::Atom("normal".into()), Some(result))?;
    /// ```
    ///
    /// # Errors
    ///
    /// An abnormal exit signal to a linked process that does not trap exits
    /// is not supported.
    pub(super) fn finish(
        &mut self,
        process: ProcessState,
        reason: Term,
        result: Option<Term>,
    ) -> Result<(), BeamError> {
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
            self.deliver_to(watcher, down)?;
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
            self.deliver_to(linked, exit)?;
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

    #[test]
    fn runs_processes_in_order_and_wakes_a_parked_receiver() {
        let mut scheduler = Scheduler::default();
        scheduler.start(frame(), vec![]).unwrap();
        let mut root = scheduler.take_next().unwrap();
        let (child, monitor) = scheduler
            .spawn(&mut root, frame(), vec![], false, false)
            .unwrap();
        assert_eq!(
            (root.id, child, monitor),
            (ProcessId(0), ProcessId(1), None)
        );

        scheduler.park(root);
        let mut child = scheduler.take_next().unwrap();
        assert!(scheduler.take_next().is_none());
        scheduler
            .deliver(&mut child, ProcessId(0), atom("ping"))
            .unwrap();
        scheduler
            .deliver(&mut child, ProcessId(1), atom("self"))
            .unwrap();
        scheduler
            .deliver(&mut child, ProcessId(9), atom("lost"))
            .unwrap();

        let root = scheduler.take_next().unwrap();
        assert_eq!(
            root.mailbox.first_at_or_after(0).unwrap().term,
            atom("ping")
        );
        assert_eq!(
            child.mailbox.first_at_or_after(0).unwrap().term,
            atom("self")
        );
    }

    #[test]
    fn sends_down_and_exit_messages_and_records_the_root_result() {
        let mut scheduler = Scheduler::default();
        scheduler.start(frame(), vec![]).unwrap();
        let mut root = scheduler.take_next().unwrap();
        root.trap_exit = true;
        let (monitored, monitor) = scheduler
            .spawn(&mut root, frame(), vec![], true, false)
            .unwrap();
        let (linked, _) = scheduler
            .spawn(&mut root, frame(), vec![], false, true)
            .unwrap();
        scheduler.park(root);

        let process = scheduler.take_next().unwrap();
        scheduler.finish(process, atom("normal"), None).unwrap();
        let process = scheduler.take_next().unwrap();
        scheduler.finish(process, atom("boom"), None).unwrap();

        let root = scheduler.take_next().unwrap();
        assert!(root.links.is_empty());
        let messages = [0, 1].map(|sequence| {
            root.mailbox
                .first_at_or_after(sequence)
                .unwrap()
                .term
                .clone()
        });
        assert_eq!(
            messages,
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
            .finish(root, atom("normal"), Some(atom("ok")))
            .unwrap();
        assert_eq!(scheduler.into_root_result(), Some(Ok(atom("ok"))));
    }

    #[test]
    fn rejects_unsupported_exit_signals_and_counter_overflow() {
        let mut scheduler = Scheduler::default();
        scheduler.start(frame(), vec![]).unwrap();
        let mut root = scheduler.take_next().unwrap();
        scheduler
            .spawn(&mut root, frame(), vec![], false, true)
            .unwrap();
        let child = scheduler.take_next().unwrap();
        scheduler.park(root);
        assert!(scheduler.finish(child, atom("boom"), None).is_err());

        let mut scheduler = Scheduler::default();
        scheduler.start(frame(), vec![]).unwrap();
        let root = scheduler.take_next().unwrap();
        scheduler
            .finish(root, atom("boom"), Some(atom("ok")))
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

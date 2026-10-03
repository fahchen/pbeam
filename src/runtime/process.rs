use std::collections::{HashMap, HashSet, VecDeque};

use crate::{
    error::{BeamError, error},
    term::{ProcessId, Reference, Term},
};

/// One active function call: its module, code position, and Y registers.
#[derive(Debug)]
pub(super) struct Frame {
    pub(super) module: String,
    pub(super) function: String,
    pub(super) pc: usize,
    pub(super) stack: Vec<Option<Term>>,
}

#[derive(Debug)]
pub(super) struct Message {
    pub(super) sequence: u64,
    pub(super) term: Term,
}

/// A process mailbox. Each message keeps a stable sequence number, so a
/// receive cursor or receive marker stays valid after other messages leave.
#[derive(Debug, Default)]
pub(super) struct Mailbox {
    messages: VecDeque<Message>,
    next_sequence: u64,
}

impl Mailbox {
    /// Add a term to the end of the mailbox.
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// mailbox.push(Term::Atom("ready".into()))?;
    /// assert_eq!(mailbox.next_sequence(), 1);
    /// ```
    fn push(&mut self, term: Term) -> Result<(), BeamError> {
        let sequence = self.next_sequence;
        self.next_sequence = sequence
            .checked_add(1)
            .ok_or_else(|| error("process mailbox sequence overflow"))?;
        self.messages.push_back(Message { sequence, term });
        Ok(())
    }

    /// Return the first message at or after the receive cursor.
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// assert_eq!(mailbox.first_at_or_after(0).map(|message| &message.term), Some(&term));
    /// ```
    pub(super) fn first_at_or_after(&self, cursor: u64) -> Option<&Message> {
        // ponytail: linear scan; add an index only if large mailboxes matter.
        self.messages
            .iter()
            .find(|message| message.sequence >= cursor)
    }

    /// Remove a message by its sequence number.
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// mailbox.remove(0);
    /// assert!(mailbox.first_at_or_after(0).is_none());
    /// ```
    pub(super) fn remove(&mut self, sequence: u64) {
        self.messages.retain(|message| message.sequence != sequence);
    }

    /// Return the sequence number that the next message will get.
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// assert_eq!(Mailbox::default().next_sequence(), 0);
    /// ```
    pub(super) fn next_sequence(&self) -> u64 {
        self.next_sequence
    }
}

/// The execution state and mailbox of one process.
#[derive(Debug)]
pub(super) struct ProcessState {
    pub(super) id: ProcessId,
    pub(super) frames: Vec<Frame>,
    pub(super) registers: Vec<Option<Term>>,
    pub(super) mailbox: Mailbox,
    pub(super) receive_cursor: u64,
    pub(super) current_message: Option<u64>,
    /// Mailbox positions saved by `recv_marker_reserve`, keyed by marker or
    /// by the bound reference.
    pub(super) receive_markers: HashMap<Reference, u64>,
    pub(super) waiting: bool,
    pub(super) trap_exit: bool,
    links: HashSet<ProcessId>,
    monitored_by: Vec<(ProcessId, Reference)>,
}

impl ProcessState {
    /// Create a runnable process with one entry frame and argument registers.
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// let process = ProcessState::new(ProcessId(0), frame, vec![]);
    /// assert!(!process.waiting);
    /// ```
    fn new(id: ProcessId, frame: Frame, arguments: Vec<Term>) -> Self {
        Self {
            id,
            frames: vec![frame],
            registers: arguments.into_iter().map(Some).collect(),
            mailbox: Mailbox::default(),
            receive_cursor: 0,
            current_message: None,
            receive_markers: HashMap::new(),
            waiting: false,
            trap_exit: false,
            links: HashSet::new(),
            monitored_by: Vec::new(),
        }
    }
}

/// A cooperative scheduler on one OS thread. The running process is taken
/// out of the process table until it waits or exits.
#[derive(Debug, Default)]
pub(super) struct Scheduler {
    processes: HashMap<ProcessId, ProcessState>,
    runnable: VecDeque<ProcessId>,
    next_process_id: u64,
    next_reference: u64,
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
        process.waiting = true;
        self.processes.insert(process.id, process);
    }

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
            return current.mailbox.push(term);
        }
        self.deliver_to(target, term)
    }

    fn deliver_to(&mut self, target: ProcessId, term: Term) -> Result<(), BeamError> {
        let Some(process) = self.processes.get_mut(&target) else {
            return Ok(());
        };
        process.mailbox.push(term)?;
        if process.waiting {
            process.waiting = false;
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

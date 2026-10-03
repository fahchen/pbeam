use std::collections::{HashMap, HashSet, VecDeque};

use crate::term::{ProcessId, Reference, Term};

/// One active function call: its module, code position, and Y registers.
#[derive(Debug)]
pub(super) struct Frame {
    /// Name of the module that owns the code.
    pub(super) module: String,
    /// Name of the function, for error messages.
    pub(super) function: String,
    /// Index of the next instruction to run in the module code.
    pub(super) next_instruction: usize,
    /// Y registers. `allocate` sets the size and `deallocate` clears them.
    pub(super) stack: Vec<Option<Term>>,
}

/// One mailbox message.
#[derive(Debug)]
pub(super) struct Message {
    /// Arrival order in the mailbox. It does not change when other messages
    /// leave the mailbox.
    pub(super) sequence: u64,
    /// The message term.
    pub(super) term: Term,
}

/// A process mailbox. Each message keeps a stable sequence number, so a
/// receive cursor or receive marker stays valid after other messages leave.
#[derive(Debug, Default)]
pub(super) struct Mailbox {
    /// Messages in arrival order.
    messages: VecDeque<Message>,
    /// Sequence number for the next message.
    next_sequence: u64,
}

impl Mailbox {
    /// Add a term to the end of the mailbox.
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// mailbox.push(Term::Atom("ready".into()));
    /// assert_eq!(mailbox.next_sequence(), 1);
    /// ```
    fn push(&mut self, term: Term) {
        let sequence = self.next_sequence;
        // ponytail: like ERTS, the mailbox has no message limit; a u64 lasts
        // about 584 years at one message per nanosecond.
        self.next_sequence += 1;
        self.messages.push_back(Message { sequence, term });
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
    /// Identifier of this process.
    pub(super) id: ProcessId,
    /// Call stack. The last frame is the active function.
    pub(super) frames: Vec<Frame>,
    /// X registers. All frames of the process share them.
    pub(super) registers: Vec<Option<Term>>,
    /// Messages that this process did not receive yet.
    pub(super) mailbox: Mailbox,
    /// Sequence number where `loop_rec` starts its search.
    pub(super) receive_cursor: u64,
    /// Sequence number of the message that `loop_rec` selected.
    pub(super) current_message: Option<u64>,
    /// Mailbox positions saved by `recv_marker_reserve`, keyed by marker or
    /// by the bound reference.
    pub(super) receive_markers: HashMap<Reference, u64>,
    /// True while the process waits in `wait` for a new message.
    waiting: bool,
    /// True when exit signals from linked processes become `EXIT` messages.
    pub(super) trap_exit: bool,
    /// Processes linked to this process.
    pub(super) links: HashSet<ProcessId>,
    /// Watchers that get a `DOWN` message when this process exits.
    pub(super) monitored_by: Vec<(ProcessId, Reference)>,
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
    pub(super) fn new(id: ProcessId, frame: Frame, arguments: Vec<Term>) -> Self {
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

    /// Mark the process as waiting for a new message.
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// process.wait();
    /// assert!(process.receive(Term::Atom("ping".into())));
    /// ```
    pub(super) fn wait(&mut self) {
        self.waiting = true;
    }

    /// Add a message to the mailbox. Return true when the message wakes a
    /// waiting process, so the scheduler must run it again.
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// assert!(!process.receive(Term::Atom("ping".into())));
    /// ```
    pub(super) fn receive(&mut self, term: Term) -> bool {
        self.mailbox.push(term);
        std::mem::take(&mut self.waiting)
    }
}

#[cfg(test)]
pub(super) mod tests {
    use super::{Frame, Mailbox, ProcessState};
    use crate::term::{ProcessId, Term};

    pub(in crate::runtime) fn frame() -> Frame {
        Frame {
            module: "fixture".into(),
            function: "main".into(),
            next_instruction: 0,
            stack: Vec::new(),
        }
    }

    #[test]
    fn keeps_message_sequences_after_removal() {
        let mut mailbox = Mailbox::default();
        mailbox.push(Term::Atom("first".into()));
        mailbox.push(Term::Atom("second".into()));
        mailbox.remove(0);

        let message = mailbox.first_at_or_after(0).unwrap();
        assert_eq!(
            (message.sequence, &message.term),
            (1, &Term::Atom("second".into()))
        );
        assert!(mailbox.first_at_or_after(2).is_none());
        assert_eq!(mailbox.next_sequence(), 2);
    }

    #[test]
    fn wakes_only_a_waiting_process() {
        let mut process = ProcessState::new(ProcessId(0), frame(), vec![Term::Integer(1)]);
        assert_eq!(process.registers, vec![Some(Term::Integer(1))]);
        assert!(!process.receive(Term::Atom("early".into())));
        process.wait();
        assert!(process.receive(Term::Atom("late".into())));
        assert!(!process.receive(Term::Atom("later".into())));
    }
}

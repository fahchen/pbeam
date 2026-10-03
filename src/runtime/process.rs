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

/// The execution state and mailbox of one process.
#[derive(Debug)]
pub(super) struct ProcessState {
    /// Identifier of this process.
    pub(super) id: ProcessId,
    /// Call stack. The last frame is the active function.
    pub(super) frames: Vec<Frame>,
    /// X registers. All frames of the process share them.
    pub(super) registers: Vec<Option<Term>>,
    /// Messages that this process did not receive yet, in arrival order.
    pub(super) mailbox: VecDeque<Term>,
    /// Mailbox index of the next message that `loop_rec` examines.
    pub(super) receive_cursor: usize,
    /// Mailbox index of the message that `loop_rec` selected.
    pub(super) current_message: Option<usize>,
    /// Mailbox indexes saved by `recv_marker_reserve`, keyed by marker or by
    /// the bound reference.
    pub(super) receive_markers: HashMap<Reference, usize>,
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
            mailbox: VecDeque::new(),
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
        self.mailbox.push_back(term);
        std::mem::take(&mut self.waiting)
    }

    /// Remove the message at a mailbox index and reset the receive cursor,
    /// as ERTS does in `remove_message`. Receive markers after the removed
    /// message move back by one, so they keep their message.
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// process.remove_message(0);
    /// assert_eq!(process.receive_cursor, 0);
    /// ```
    pub(super) fn remove_message(&mut self, index: usize) {
        self.mailbox.remove(index);
        self.receive_cursor = 0;
        for position in self.receive_markers.values_mut() {
            if *position > index {
                *position -= 1;
            }
        }
    }
}

#[cfg(test)]
pub(super) mod tests {
    use super::{Frame, ProcessState};
    use crate::term::{ProcessId, Reference, Term};

    pub(in crate::runtime) fn frame() -> Frame {
        Frame {
            module: "fixture".into(),
            function: "main".into(),
            next_instruction: 0,
            stack: Vec::new(),
        }
    }

    #[test]
    fn moves_receive_markers_after_a_removed_message() {
        let mut process = ProcessState::new(ProcessId(0), frame(), vec![]);
        for name in ["first", "second", "third"] {
            process.receive(Term::Atom(name.into()));
        }
        process.receive_cursor = 2;
        process.receive_markers.insert(Reference(1), 0);
        process.receive_markers.insert(Reference(2), 2);
        process.remove_message(1);

        assert_eq!(
            Vec::from(process.mailbox.clone()),
            vec![Term::Atom("first".into()), Term::Atom("third".into())]
        );
        assert_eq!(process.receive_cursor, 0);
        assert_eq!(process.receive_markers[&Reference(1)], 0);
        assert_eq!(process.receive_markers[&Reference(2)], 1);
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

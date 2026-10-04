use std::collections::HashSet;

use super::mailbox::Mailbox;
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
    pub(super) stack: Vec<StackSlot>,
}

/// One Y register of a frame.
#[derive(Debug)]
pub(super) enum StackSlot {
    /// Allocated, but no instruction wrote it yet.
    Empty,
    /// A term that a BEAM instruction wrote.
    Term(Term),
    /// A catch tag that `try` or `catch` wrote: the index of the first
    /// instruction after the handler label in the module of the frame.
    Catch(usize),
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
    /// Messages and receive markers, with the receive save pointer.
    pub(super) mailbox: Mailbox,
    /// True while the process waits in `wait` for a new message.
    waiting: bool,
    /// True when exit signals from linked processes become `EXIT` messages.
    pub(super) trap_exit: bool,
    /// Processes linked to this process.
    pub(super) links: HashSet<ProcessId>,
    /// Watchers that get a `DOWN` message when this process exits.
    pub(super) monitored_by: Vec<(ProcessId, Reference)>,
    /// Group leader that receives the IO requests of this process. A new
    /// process starts as its own group leader, as `init` does in ERTS; a
    /// spawned process gets the group leader of its parent.
    pub(super) group_leader: ProcessId,
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
            waiting: false,
            trap_exit: false,
            links: HashSet::new(),
            monitored_by: Vec::new(),
            group_leader: id,
        }
    }

    /// Mark the process as waiting for a new message.
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// process.wait();
    /// assert!(process.receive(Term::static_atom("ok")));
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
    /// assert!(!process.receive(Term::static_atom("ok")));
    /// ```
    pub(super) fn receive(&mut self, term: Term) -> bool {
        self.mailbox.push(term);
        std::mem::take(&mut self.waiting)
    }
}

#[cfg(test)]
pub(super) mod tests {
    use super::{Frame, ProcessState};
    use crate::term::test_atom;
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
    fn wakes_only_a_waiting_process() {
        let mut process = ProcessState::new(ProcessId(0), frame(), vec![Term::Integer(1)]);
        assert_eq!(process.registers, vec![Some(Term::Integer(1))]);
        assert!(!process.receive(test_atom("early")));
        process.wait();
        assert!(process.receive(test_atom("late")));
        assert!(!process.receive(test_atom("later")));
    }
}

use std::collections::HashSet;

use super::{mailbox::Mailbox, timer::TimerKey};
use crate::term::{Atom, ProcessId, Reference, Term};

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

/// The receive timeout state of a process, as the ERTS process flags
/// `F_TIMO` and `F_INSLPQUEUE`, the process timer `common.timer`, and the
/// resume address in `def_arg_reg[0]` (`erl_process.h:1841-1849`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum ReceiveTimer {
    /// No flag: the next `wait_timeout` reads its timeout.
    None,
    /// `F_TIMO`: the receive waits with `infinity`, or its timer expired and
    /// the process continues at `timeout`.
    Timo,
    /// `F_INSLPQUEUE`: the receive timer with the key `key` is active.
    /// `resume` is the index of the instruction after `wait_timeout`, as
    /// `def_arg_reg[0]`.
    Set { key: TimerKey, resume: usize },
    /// `ERTS_PTMR_TIMEDOUT`: the receive timer expired. The process
    /// continues at `resume` the next time it runs.
    Expired { resume: usize },
}

/// A monitor of another process on this process, as the target part of an
/// ERTS monitor (`ErtsMonitorData`, `erl_monitor_link.h`).
#[derive(Debug)]
pub(super) struct Monitor {
    /// The process that gets the `DOWN` message.
    pub(super) watcher: ProcessId,
    /// The reference of the monitor.
    pub(super) reference: Reference,
    /// The registered name of a monitor by name. The `DOWN` message then
    /// gives `{Name, nonode@nohost}` as its item, not the pid
    /// (`erl_proc_sig_queue.c:4942-4955`).
    pub(super) name: Option<Atom>,
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
    /// The receive timeout state.
    pub(super) receive_timer: ReceiveTimer,
    /// True when exit signals from linked processes become `EXIT` messages.
    pub(super) trap_exit: bool,
    /// Processes linked to this process.
    pub(super) links: HashSet<ProcessId>,
    /// Monitors that send a `DOWN` message when this process exits.
    pub(super) monitored_by: Vec<Monitor>,
    /// Group leader that receives the IO requests of this process. A new
    /// process starts as its own group leader, as `init` does in ERTS; a
    /// spawned process gets the group leader of its parent.
    pub(super) group_leader: ProcessId,
    /// Process dictionary of `put/2` and `get/1`, as key and value pairs in
    /// insertion order. Keys compare with exact equality (`=:=`).
    pub(super) dictionary: Vec<(Term, Term)>,
    /// Name that `erlang:register/2` gave this process, as
    /// `common.u.alive.reg` in ERTS.
    pub(super) registered_name: Option<Atom>,
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
            receive_timer: ReceiveTimer::None,
            trap_exit: false,
            links: HashSet::new(),
            monitored_by: Vec::new(),
            group_leader: id,
            dictionary: Vec::new(),
            registered_name: None,
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

    /// Mark the receive timer as expired, as `proc_timeout_common`
    /// (`erl_hl_timer.c:692-706`) does. Return true when the process waited,
    /// so the scheduler must run it again. A process that a message woke
    /// stays in the run queue.
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// process.receive_timer = ReceiveTimer::Set { key: (1100, 0), resume: 7 };
    /// process.wait();
    /// assert!(process.time_out());
    /// assert_eq!(process.receive_timer, ReceiveTimer::Expired { resume: 7 });
    /// ```
    pub(super) fn time_out(&mut self) -> bool {
        if let ReceiveTimer::Set { resume, .. } = self.receive_timer {
            self.receive_timer = ReceiveTimer::Expired { resume };
        }
        std::mem::take(&mut self.waiting)
    }
}

#[cfg(test)]
pub(super) mod tests {
    use super::{Frame, ProcessState, ReceiveTimer};
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

    #[test]
    fn expires_the_receive_timer_and_wakes_only_a_waiting_process() {
        let mut process = ProcessState::new(ProcessId(0), frame(), vec![]);
        process.receive_timer = ReceiveTimer::Set {
            key: (1100, 0),
            resume: 7,
        };
        process.wait();
        assert!(process.time_out());
        assert_eq!(process.receive_timer, ReceiveTimer::Expired { resume: 7 });
        assert!(!process.time_out());
        assert_eq!(process.receive_timer, ReceiveTimer::Expired { resume: 7 });
    }
}

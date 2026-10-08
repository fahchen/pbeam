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

/// The start of a new process. ERTS starts each process with `i_apply_only`
/// of its MFA (`erl_process.c:12616`; JIT `jit/beam_jit_main.cpp:258`), so
/// the process calls the export entry of the function when it first runs.
#[derive(Debug)]
pub(super) enum Start {
    /// The entry frame of an exported function.
    Entry(Frame),
    /// A function that its loaded module does not export, or a start with
    /// more than 1023 arguments. When the process first runs, it raises
    /// `system_limit` for more than 1023 arguments, as the `apply` of ERTS
    /// does. Else it calls the export entry, and thus the error handler.
    Undefined {
        /// The module of the start MFA.
        module: Atom,
        /// The function of the start MFA.
        function: Atom,
    },
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
    /// The first element of the `DOWN` message: `'DOWN'`, or the `tag`
    /// option of `erlang:monitor/3` (`erl_proc_sig_queue.c:6417-6429`).
    pub(super) tag: Term,
}

/// The execution state and mailbox of one process.
#[derive(Debug)]
pub(super) struct ProcessState {
    /// Identifier of this process.
    pub(super) id: ProcessId,
    /// Call stack. The last frame is the active function.
    pub(super) frames: Vec<Frame>,
    /// The module and the function of a `Start::Undefined` start until the
    /// process first runs. The process then has no frame, and the arity is
    /// the number of argument registers.
    pub(super) undefined_start: Option<(Atom, Atom)>,
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
    /// True when the `message_queue_data` flag is `off_heap`
    /// (`FS_OFF_HEAP_MSGQ`), false for `on_heap`. PBeam has no process heap,
    /// so the flag does not change where the messages are kept.
    pub(super) off_heap_message_queue: bool,
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
    /// The reductions that remain in the current turn, as `p->fcalls` and
    /// the JIT `FCALLS` register. `Scheduler::schedule_next` sets it to
    /// `CONTEXT_REDS`. A cost point subtracts from it, and the process
    /// switches out at the ERTS switch points. A `return` can make it -1.
    pub(super) fcalls: i32,
}

impl ProcessState {
    /// Create a runnable process with argument registers. An entry start
    /// gives one frame. An undefined start gives no frame and keeps the
    /// module and the function in `undefined_start`.
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// let process = ProcessState::new(ProcessId(0), Start::Entry(frame), vec![]);
    /// assert!(!process.waiting);
    /// let start = Start::Undefined { module: module.clone(), function: function.clone() };
    /// let process = ProcessState::new(ProcessId(1), start, vec![Term::Integer(1)]);
    /// assert_eq!(process.undefined_start, Some((module, function)));
    /// ```
    pub(super) fn new(id: ProcessId, start: Start, arguments: Vec<Term>) -> Self {
        let (frames, undefined_start) = match start {
            Start::Entry(frame) => (vec![frame], None),
            Start::Undefined { module, function } => (Vec::new(), Some((module, function))),
        };
        Self {
            id,
            frames,
            undefined_start,
            registers: arguments.into_iter().map(Some).collect(),
            mailbox: Mailbox::default(),
            waiting: false,
            receive_timer: ReceiveTimer::None,
            trap_exit: false,
            off_heap_message_queue: false,
            links: HashSet::new(),
            monitored_by: Vec::new(),
            group_leader: id,
            dictionary: Vec::new(),
            registered_name: None,
            fcalls: 0,
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

    /// Use `reductions` of the budget of the current turn, as
    /// `ERTS_VBUMP_REDS` (`bif.h:85-96`) does: the budget goes down, but not
    /// below 0. ERTS also adds the used amount to `virtual_reds`, which only
    /// the reduction total of the process reads. PBeam keeps no reduction
    /// total yet (#230).
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// process.fcalls = 6;
    /// process.vbump_reductions(4);
    /// assert_eq!(process.fcalls, 2);
    /// process.vbump_reductions(4);
    /// assert_eq!(process.fcalls, 0);
    /// ```
    pub(super) fn vbump_reductions(&mut self, reductions: i32) {
        self.fcalls = (self.fcalls - reductions).max(0);
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
    use super::{Frame, ProcessState, ReceiveTimer, Start};
    use crate::term::test_atom;
    use crate::term::{Atom, ProcessId, Term};

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
        let mut process =
            ProcessState::new(ProcessId(0), Start::Entry(frame()), vec![Term::Integer(1)]);
        assert_eq!(process.registers, vec![Some(Term::Integer(1))]);
        assert_eq!(process.frames.len(), 1);
        assert_eq!(process.undefined_start, None);
        assert!(!process.receive(test_atom("early")));
        process.wait();
        assert!(process.receive(test_atom("late")));
        assert!(!process.receive(test_atom("later")));
    }

    #[test]
    fn expires_the_receive_timer_and_wakes_only_a_waiting_process() {
        let mut process = ProcessState::new(ProcessId(0), Start::Entry(frame()), vec![]);
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

    #[test]
    fn uses_reductions_down_to_zero_as_erts_vbump_reds() {
        let mut process = ProcessState::new(ProcessId(0), Start::Entry(frame()), vec![]);
        process.fcalls = 6;
        process.vbump_reductions(4);
        assert_eq!(process.fcalls, 2);
        process.vbump_reductions(4);
        assert_eq!(process.fcalls, 0);
    }

    #[test]
    fn keeps_the_mfa_of_an_undefined_start_without_a_frame() {
        let module = Atom::from_static("erlang").unwrap();
        let function = Atom::from_static("ok").unwrap();
        let start = Start::Undefined {
            module: module.clone(),
            function: function.clone(),
        };
        let process = ProcessState::new(ProcessId(1), start, vec![Term::Integer(1)]);
        assert!(process.frames.is_empty());
        assert_eq!(process.undefined_start, Some((module, function)));
        assert_eq!(process.registers, vec![Some(Term::Integer(1))]);
    }
}

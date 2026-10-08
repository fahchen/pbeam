mod tree;

pub(super) use tree::{MonitorLinkTree, TimerReference};

use super::{mailbox::Mailbox, scheduler::AliasMode, timer::TimerKey};
use crate::{
    error::{BeamError, error},
    term::{Atom, ProcessId, Reference, Term},
};

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

/// A monitor of this process that another process holds: the target part
/// of an ERTS monitor (`ERTS_P_LT_MONITORS`, `erl_monitor_link.h`). The
/// watcher keeps the rest in its [`Origin`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct Monitor {
    /// The process that gets the monitor down signal.
    pub(super) watcher: ProcessId,
    /// The reference of the monitor.
    pub(super) reference: Reference,
}

/// An entry of the monitor tree of a process (`ERTS_P_MONITORS`): the
/// origin part of a monitor that this process holds, an alias, or both.
#[derive(Debug, Clone, PartialEq)]
pub(super) struct Origin {
    /// The monitor, or `None` for an alias without a monitor
    /// (`ERTS_MON_TYPE_ALIAS`): an `explicit_unalias` alias after its
    /// monitor ended (`bif.c:470-482`, `erl_proc_sig_queue.c:6463-6480`).
    pub(super) monitor: Option<OriginMonitor>,
    /// The mode of the alias of the reference, or `None` when the
    /// reference is not an active alias (`ERTS_ML_STATE_ALIAS_MASK`).
    pub(super) alias: Option<AliasMode>,
}

/// The monitor of an [`Origin`]: what the watcher needs to make the
/// `DOWN` message and to remove the monitor.
#[derive(Debug, Clone, PartialEq)]
pub(super) struct OriginMonitor {
    /// The third element of the `DOWN` message: `process` or `port`.
    pub(super) kind: &'static str,
    /// The monitored process, or `None` for a name that no process had and
    /// for a port name (`other.item` is `undefined`).
    pub(super) target: Option<ProcessId>,
    /// The fourth element of the `DOWN` message: the pid, or
    /// `{Name, nonode@nohost}` for a monitor by name
    /// (`erl_proc_sig_queue.c:4942-4955`).
    pub(super) item: Term,
    /// The first element of the `DOWN` message: `'DOWN'`, or the `tag`
    /// option of `erlang:monitor/3` (`erl_proc_sig_queue.c:6417-6429`).
    pub(super) tag: Term,
}

/// The state of a BIF that handles the signals of its caller up to a flush
/// signal (`FS_FLUSHING_SIGS` and `FS_FLUSHED_SIGS`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Flush {
    /// No flush runs.
    None,
    /// The flush signal is in the queue (`FS_FLUSHING_SIGS`).
    Flushing,
    /// The handling reached the flush signal (`FS_FLUSHED_SIGS`).
    Flushed,
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
    /// The signal queue: messages, receive markers, and signals, with the
    /// receive save pointer.
    pub(super) mailbox: Mailbox,
    /// True while the process has no Erlang work: it waits in `wait` for a
    /// new message, or after `erlang:hibernate/0` (no `ERTS_PSFLG_ACTIVE`).
    pub(super) waiting: bool,
    /// True while the process is in the run queue (`ERTS_PSFLG_IN_RUNQ`). A
    /// waiting process can be in the run queue to handle its signals.
    pub(super) in_run_queue: bool,
    /// The exit reason of a process that a signal ended
    /// (`erts_set_self_exiting`, `erl_process.c:13485-13548`). A process
    /// that does not exit at once exits when it runs next. Messages to it
    /// are dropped.
    pub(super) exiting: Option<Term>,
    /// True when the process may have sent a message or a signal to itself
    /// that it did not handle (`ERTS_PXSFLG_MAYBE_SELF_SIGS`). A handling
    /// pass that leaves no incoming signal clears it
    /// (`erl_proc_sig_queue.c:7117-7126`).
    pub(super) self_signals: bool,
    /// True after `erlang:hibernate/0` made the process wait
    /// (`F_HIBERNATED`). A handling pass that leaves messages in the inner
    /// part then makes it active (`erl_proc_sig_queue.c:7152-7154`). ERTS
    /// clears the flag at the next garbage collection, and PBeam, which has
    /// no garbage collection, when the process runs Erlang code again.
    pub(super) hibernated: bool,
    /// The state of a flush of the signals of the process.
    pub(super) flush: Flush,
    /// The receive timeout state.
    pub(super) receive_timer: ReceiveTimer,
    /// True when exit signals from linked processes become `EXIT` messages.
    pub(super) trap_exit: bool,
    /// True when the `message_queue_data` flag is `off_heap`
    /// (`FS_OFF_HEAP_MSGQ`), false for `on_heap`. PBeam has no process heap,
    /// so the flag does not change where the messages are kept.
    pub(super) off_heap_message_queue: bool,
    /// The link tree (`ERTS_P_LINKS`): processes linked to this process, by
    /// pid, with the identifier of an unlink that waits for its
    /// acknowledgment, or 0 for an active link (`ErtsILink.unlinking`). A
    /// link that is unlinking sends no exit signal
    /// (`erl_process.c:14068-14081`).
    pub(super) links: MonitorLinkTree<ProcessId, u64>,
    /// The monitor tree: the monitors that this process holds and its
    /// aliases, by reference (`ERTS_P_MONITORS`).
    pub(super) monitors: MonitorLinkTree<Reference, Origin>,
    /// Monitors that send a monitor down signal when this process exits, in
    /// creation order (`ERTS_P_LT_MONITORS`).
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
    /// The reductions of the turns that ended, as `p->reds`. Each switch
    /// out adds the reductions of the turn, except the virtual reductions
    /// (`erl_process.c:9678-9700`).
    pub(super) reductions: u64,
    /// A BIF call that continues when the process runs again, as the ERTS
    /// `BIF_TRAP` to the export of a BIF sets `c_p->i` to the export
    /// (`jit/arm/instr_bif.cpp:525-536`, `676-701`). `BeamProgram::run` runs
    /// it first.
    pub(super) pending_bif: Option<PendingBif>,
    /// The trap state of the guard BIF `length/1`: the arguments of its
    /// trap export, the rest of the list and the count so far. ERTS
    /// `i_length` keeps them in the X registers above the live registers
    /// and runs the instruction again (`jit/arm/instr_bif.cpp:198-273`).
    pub(super) length_trap: Option<Vec<Term>>,
    /// The BIF timers to this process, as the ERTS tree `p->bif_timers`
    /// (`erl_hl_timer.c:579-617`). `erlang:start_timer/3` to the pid adds a
    /// timer, and a timer that expires or that `erlang:cancel_timer/2`
    /// cancels leaves the tree when the process does not exit
    /// (`erl_hl_timer.c:1180-1190`, `1832-1858`, `1895-1915`). The exit
    /// cancels the timers in the post-order of the tree.
    pub(super) bif_timers: MonitorLinkTree<TimerReference, ()>,
    /// The state of an exit that switched out before it left the process
    /// table, as `p->u.terminate` holds the state of a continued exit
    /// (`erl_process.c:14276-14290`): the BIF timers that the exit did not
    /// cancel yet, the next timer last. `None` until the exit switches out.
    /// The exit then continues with no new bump.
    pub(super) continue_exit: Option<Vec<TimerReference>>,
}

/// A BIF call that continues after a switch: the export of a BIF and the
/// arguments that the BIF trap gives it.
#[derive(Debug)]
pub(super) struct PendingBif {
    /// Module of the BIF export, for example `lists`.
    pub(super) module: &'static str,
    /// Function of the BIF export, for example `member`.
    pub(super) function: &'static str,
    /// Arguments of the BIF export, in `x0` onwards.
    pub(super) arguments: Vec<Term>,
    /// True when the BIF call was a tail call through the export: its
    /// value then returns from the frame with no reduction.
    pub(super) tail: bool,
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
            in_run_queue: false,
            exiting: None,
            self_signals: false,
            hibernated: false,
            flush: Flush::None,
            receive_timer: ReceiveTimer::None,
            trap_exit: false,
            off_heap_message_queue: false,
            links: MonitorLinkTree::default(),
            monitors: MonitorLinkTree::default(),
            monitored_by: Vec::new(),
            group_leader: id,
            dictionary: Vec::new(),
            registered_name: None,
            fcalls: 0,
            reductions: 0,
            pending_bif: None,
            length_trap: None,
            bif_timers: MonitorLinkTree::default(),
            continue_exit: None,
        }
    }

    /// Use `reductions` of the budget of the current turn, as `BUMP_REDS`
    /// (`bif.h:71-78`) does: the budget goes down, but not below 0.
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// process.fcalls = 6;
    /// process.bump_reductions(4);
    /// assert_eq!(process.fcalls, 2);
    /// process.bump_reductions(4);
    /// assert_eq!(process.fcalls, 0);
    /// ```
    pub(super) fn bump_reductions(&mut self, reductions: i32) {
        self.fcalls = (self.fcalls - reductions).max(0);
    }

    /// Use all reductions of the budget of the current turn, as
    /// `BUMP_ALL_REDS` (`bif.h:80-83`) does before a BIF traps to its export.
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// process.bump_all_reductions();
    /// assert_eq!(process.fcalls, 0);
    /// ```
    pub(super) fn bump_all_reductions(&mut self) {
        self.fcalls = 0;
    }

    /// Return the reductions that a BIF can use, as `ERTS_BIF_REDS_LEFT`
    /// gives them in the JIT: the budget of the turn (`bif.h:66-67`,
    /// `218-219`). A BIF runs with a budget of 0 or more: a `return` that
    /// makes the budget -1 switches out the process first. A hashmap call
    /// of `erts_internal:map_next/3` can also make the budget negative, but
    /// `maps.erl` calls it only before a call or a return, which switch out
    /// the process first.
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// process.fcalls = 6;
    /// assert_eq!(process.bif_reductions_left()?, 6);
    /// ```
    ///
    /// # Errors
    ///
    /// Return a runtime error when the budget is negative: code that calls
    /// `erts_internal:map_next/3` directly and then calls a BIF that reads
    /// the budget before the next call or return. Each ERTS BIF reads a
    /// negative budget in its own way, and PBeam does not implement this
    /// yet (row "Map iteration" in `docs/erts-differences.md`).
    pub(super) fn bif_reductions_left(&self) -> Result<usize, BeamError> {
        usize::try_from(self.fcalls).map_err(|_| {
            error(
                "unsupported negative reduction budget in a BIF after erts_internal:map_next/3 \
                 (see \"Map iteration\" in docs/erts-differences.md)",
            )
        })
    }

    /// Return the result of `erlang:process_info/2` for the item `item` of
    /// this process, as `process_info_aux` (`erl_bif_info.c`) gives it, or
    /// `None` for an item that PBeam does not support:
    ///
    /// - `registered_name` gives `{registered_name, Name}`, or `[]` without a
    ///   name.
    /// - `message_queue_len` gives `{message_queue_len, Messages}`: the
    ///   caller counts the messages as `mq_len` (`erl_bif_info.c:1742-1754`).
    /// - `reductions` gives `{reductions, N}`: the reduction total and
    ///   `current`, the reductions of the current turn when the process runs
    ///   (`erl_bif_info.c:2114-2121`).
    /// - `current_stacktrace` gives `{current_stacktrace, []}` for the
    ///   calling process (`caller`), because PBeam records no stack frames.
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// let length = process.info(&Term::static_atom("message_queue_len"), 0, 2, false)?;
    /// assert_eq!(length, Some(Term::Tuple(vec![Term::static_atom("message_queue_len"), Term::Integer(2)])));
    /// ```
    ///
    /// # Errors
    ///
    /// A reduction total above the PBeam integer range is an error.
    pub(super) fn info(
        &self,
        item: &Term,
        current: u64,
        messages: usize,
        caller: bool,
    ) -> Result<Option<Term>, BeamError> {
        let Term::Atom(name) = item else {
            return Ok(None);
        };
        let value = match (name.name(), &self.registered_name) {
            // ponytail: PBeam records no stack frames; see "Exception stack trace" in docs/erts-differences.md.
            ("current_stacktrace", _) if caller => Term::List(Vec::new()),
            ("registered_name", Some(registered)) => Term::Atom(registered.clone()),
            ("registered_name", None) => return Ok(Some(Term::List(Vec::new()))),
            ("message_queue_len", _) => Term::Integer(messages as i64),
            ("reductions", _) => Term::Integer(
                i64::try_from(self.reductions + current)
                    .map_err(|_| error("the reduction total exceeds the PBeam integer range"))?,
            ),
            _ => return Ok(None),
        };
        Ok(Some(Term::Tuple(vec![item.clone(), value])))
    }

    /// Mark the receive timer as expired, as `proc_timeout_common`
    /// (`erl_hl_timer.c:692-706`) does. Return true when the process waited
    /// and now has Erlang work, so the scheduler must run it. A process that
    /// a message woke, or an exiting process, stays as it is.
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// process.receive_timer = ReceiveTimer::Set { key: (1100, 0), resume: 7 };
    /// process.waiting = true;
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
    fn expires_the_receive_timer_and_wakes_only_a_waiting_process() {
        let mut process = ProcessState::new(ProcessId(0), Start::Entry(frame()), vec![]);
        process.receive_timer = ReceiveTimer::Set {
            key: (1100, 0),
            resume: 7,
        };
        process.waiting = true;
        assert!(process.time_out());
        assert_eq!(process.receive_timer, ReceiveTimer::Expired { resume: 7 });
        assert!(!process.time_out());
        assert_eq!(process.receive_timer, ReceiveTimer::Expired { resume: 7 });
    }

    #[test]
    fn uses_reductions_down_to_zero_as_erts_bump_reds() {
        let mut process = ProcessState::new(ProcessId(0), Start::Entry(frame()), vec![]);
        process.fcalls = 6;
        process.bump_reductions(4);
        assert_eq!(process.fcalls, 2);
        process.bump_reductions(4);
        assert_eq!(process.fcalls, 0);
    }

    #[test]
    fn returns_a_runtime_error_for_the_negative_budget_of_a_bif() {
        let mut process = ProcessState::new(ProcessId(0), Start::Entry(frame()), vec![]);
        process.fcalls = 0;
        assert_eq!(process.bif_reductions_left(), Ok(0));
        process.fcalls = -1;
        let failure = process.bif_reductions_left().unwrap_err().to_string();
        assert!(failure.starts_with("unsupported negative reduction budget in a BIF"));
        assert!(failure.contains("\"Map iteration\""));
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

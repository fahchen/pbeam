mod dictionary;
mod tree;

pub(super) use dictionary::Dictionary;
pub(super) use tree::{MonitorLinkTree, TimerReference, WalkPosition};

use std::collections::BTreeMap;

use super::{
    capability::SecurityContext, dirty_io::Claim, host::output::IovecWritten, mailbox::Mailbox,
    scheduler::AliasMode, timer::TimerKey,
};
use crate::{
    beam::{ModuleId, NameId},
    error::{BeamError, error},
    term::{Atom, AtomTable, LiveBytes, PortId, ProcessId, Reference, Term},
};

/// The other end of a link: a process or a port, as the `other.item` of an
/// ERTS link, which is a pid or a port (`erl_monitor_link.h`). A link tree
/// orders its entries by the `Eterm` word of the item (see the `TreeKey`
/// implementation).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum LinkId {
    /// A local process.
    Process(ProcessId),
    /// A local port.
    Port(PortId),
}

impl LinkId {
    /// Return the term of the identifier: the pid or the port, as the
    /// second element of an `EXIT` message.
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// assert_eq!(LinkId::Process(ProcessId(1)).term(), Term::Pid(ProcessId(1)));
    /// assert_eq!(LinkId::Port(PortId(2)).term(), Term::Port(PortId(2)));
    /// ```
    pub(super) fn term(self) -> Term {
        match self {
            LinkId::Process(id) => Term::Pid(id),
            LinkId::Port(id) => Term::Port(id),
        }
    }
}

/// One active function call: its module, code position, and Y registers. The
/// module and the function are identities, and the code table resolves their
/// names when a diagnostic needs them.
#[derive(Clone, Debug)]
pub(super) struct Frame {
    /// Identity of the module that owns the code. The run loop compares it,
    /// and the code table gives the name.
    pub(super) module_id: ModuleId,
    /// Identity of the function in the name table of the module, for error
    /// messages.
    pub(super) function: NameId,
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
    /// A BIF, a function that its loaded module does not export, or a start
    /// with more than 1023 arguments. When the process first runs, it raises
    /// `system_limit` for more than 1023 arguments, as the `apply` of ERTS
    /// does. Else it calls the export entry, and thus the BIF or the error
    /// handler.
    Undefined {
        /// The module of the start MFA.
        module: Atom,
        /// The function of the start MFA.
        function: Atom,
    },
}

/// One Y register of a frame.
#[derive(Clone, Debug)]
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

/// An entry of the target monitor list of a process (`ERTS_P_LT_MONITORS`,
/// `erl_monitor_link.h`): a monitor on the process that another process or
/// a resource holds.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum TargetMonitor {
    /// A monitor that another process holds (`ERTS_MON_TYPE_PROC`).
    Process(Monitor),
    /// A monitor that the TTY resource with this reference holds
    /// (`ERTS_MON_TYPE_RESOURCE`, `erl_nif.c:3957-3989`).
    Tty(Reference),
    /// A monitor that the file resource with this reference holds
    /// (`ERTS_MON_TYPE_RESOURCE`, `prim_file_nif.c:541-570`).
    File(Reference),
    /// A monitor that a port holds (`ERTS_MON_TYPE_PORT`): the listen port
    /// of an accept that waits.
    Port {
        /// The port that holds the monitor.
        port: PortId,
        /// The reference of the monitor.
        reference: Reference,
    },
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

/// The start MFA of a process. An entry start keeps the identity of the
/// loaded code, and a start that no loaded module exports keeps the atoms of
/// its MFA. Thus the process creation reads no name, and a reader resolves
/// the module name and the function name from the code table.
#[derive(Debug, Clone)]
pub(super) enum InitialCall {
    /// The entry function of a loaded module.
    Entry {
        /// Identity of the module that owns the code.
        module: ModuleId,
        /// Identity of the function in the name table of the module.
        function: NameId,
        /// The number of start arguments.
        arity: usize,
    },
    /// A start MFA that no loaded module exports, or a start with more than
    /// `APPLY_MAX_ARGUMENTS` arguments.
    Undefined {
        /// The module of the start MFA.
        module: Atom,
        /// The function of the start MFA.
        function: Atom,
        /// The number of start arguments.
        arity: usize,
    },
}

/// The priority of a process, from `PRIORITY_MAX` to `PRIORITY_LOW`
/// (`erl_process.h:124-127`). A process starts with `normal`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(super) enum Priority {
    /// `max`: runs before each other priority.
    Max,
    /// `high`: runs before `normal` and `low`.
    High,
    /// `normal`: the start priority.
    #[default]
    Normal,
    /// `low`: shares the `normal` queue, at a lower rate.
    Low,
}

impl Priority {
    /// Return the priority of the atom name `name`, or `None` for a name
    /// that is not a priority (`erts_set_process_priority`,
    /// `erl_process.c:9460-9466`).
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// assert_eq!(Priority::from_name("max"), Some(Priority::Max));
    /// assert_eq!(Priority::from_name("foo"), None);
    /// ```
    pub(super) fn from_name(name: &str) -> Option<Self> {
        match name {
            "max" => Some(Self::Max),
            "high" => Some(Self::High),
            "normal" => Some(Self::Normal),
            "low" => Some(Self::Low),
            _ => None,
        }
    }

    /// Return the atom name of the priority (`erts_get_process_priority`,
    /// `erl_process.c:9443-9453`).
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// assert_eq!(Priority::Low.name(), "low");
    /// ```
    pub(super) fn name(self) -> &'static str {
        match self {
            Self::Max => "max",
            Self::High => "high",
            Self::Normal => "normal",
            Self::Low => "low",
        }
    }
}

/// The process flags that the spawn options set at the start of a process
/// (`erts_parse_spawn_opts`, `erl_process.c:12222-12434`).
#[derive(Clone, Copy, Debug, Default)]
pub(super) struct StartFlags {
    /// The `message_queue_data` flag is `off_heap` (`SPO_OFF_HEAP_MSGQ`),
    /// else `on_heap`.
    pub(super) off_heap_message_queue: bool,
    /// The priority of the option `{priority, Level}` (`SPO_PRIORITY`).
    pub(super) priority: Priority,
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
    /// Float registers, which `fmove`, `fconv`, and the float arithmetic
    /// instructions read and write. They grow as necessary. The compiler
    /// writes a float register before each read in one function, and no
    /// call keeps a value in one, as in ERTS.
    pub(super) float_registers: Vec<f64>,
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
    /// True when the `async_dist` flag is set (`F_ASYNC_DIST`,
    /// `erl_process.h:1656`). PBeam has no distribution, so the flag has no
    /// other effect.
    pub(super) async_dist: bool,
    /// The node monitors of the process (`ERTS_MON_TYPE_NODES`): the count
    /// of each by the bits of its options (`erts_monitor_nodes`,
    /// `dist.c:6731-6845`). PBeam has no distribution, so a node monitor
    /// never gets a message.
    pub(super) node_monitors: BTreeMap<u8, u64>,
    /// The priority of the process (`ERTS_PSFLGS_USR_PRIO`). The scheduler
    /// puts the process into the run queue of this priority.
    pub(super) priority: Priority,
    /// The link tree (`ERTS_P_LINKS`): processes and ports linked to this
    /// process, by pid or port, with the identifier of an unlink that waits for its
    /// acknowledgment, or 0 for an active link (`ErtsILink.unlinking`). A
    /// link that is unlinking sends no exit signal
    /// (`erl_process.c:14068-14081`).
    pub(super) links: MonitorLinkTree<LinkId, u64>,
    /// The monitor tree: the monitors that this process holds and its
    /// aliases, by reference (`ERTS_P_MONITORS`).
    pub(super) monitors: MonitorLinkTree<Reference, Origin>,
    /// The monitors on this process, in the order that the process handled
    /// their monitor signals (`ERTS_P_LT_MONITORS`,
    /// `erl_proc_sig_queue.c:6544-6562`). The exit of the process sends a
    /// monitor down signal for each process monitor and runs each TTY
    /// monitor, in this order.
    pub(super) monitored_by: Vec<TargetMonitor>,
    /// Group leader that receives the IO requests of this process. A new
    /// process starts as its own group leader, as `init` does in ERTS; a
    /// spawned process gets the group leader of its parent.
    pub(super) group_leader: ProcessId,
    /// Process dictionary, as the ERTS linear hash table of
    /// `erl_process_dict.c`. Keys compare with exact equality (`=:=`).
    pub(super) dictionary: Dictionary,
    /// Name that `erlang:register/2` gave this process, as
    /// `common.u.alive.reg` in ERTS.
    pub(super) registered_name: Option<Atom>,
    /// The module that the export entry calls for a function that a module
    /// does not export, as the process-specific error handler of
    /// `erts_proc_get_error_handler` (`erl_process.h:2420-2430`). `None`
    /// means the default `error_handler`: ERTS keeps the default in the
    /// process specific data also as a `NULL` pointer
    /// (`erts_proc_set_error_handler`, `erl_process.h:2432-2446`).
    pub(super) error_handler: Option<String>,
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
    /// A BIF trap whose target had no loaded export at the trap. `run`
    /// resolves the target when the process runs again, after the switch
    /// out of the trap.
    pub(super) pending_trap: Option<PendingTrap>,
    /// The result of a list insert of `ets:insert/2` or `ets:insert_new/2`
    /// that ended while the process trapped in it, until the process reads
    /// it, as `ctx->return_value` (`erl_db.c:2180-2184`, `2239-2244`). The
    /// state of the call lives with the process, so the result stays after
    /// the table goes away (`erl_db.c:2142-2150`).
    pub(super) insert_result: Option<Term>,
    /// The dirty IO call of the process, from the switch out at the call
    /// until `BeamProgram::run` returns its result, or until a signal ends
    /// the process. ERTS runs the process on a dirty IO scheduler for this
    /// time (`erl_nif.c:426-543`). While the call runs, the process does not
    /// run Erlang code, and the scheduler handles its signals for it, as
    /// `erts_dirty_process_signal_handler` does
    /// (`erts_dirty_process_signal_handler.erl:52-89`).
    pub(super) dirty_write: Option<DirtyWrite>,
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
    /// (`erl_process.c:14276-14290`): the position of the next BIF timer
    /// in the post-order walk of `bif_timers`, or `None` after the last
    /// timer, as the yield state of the ERTS walk keeps the next node
    /// (`erl_rbtree.h:1392-1396`). `None` until the exit switches out. The
    /// exit then continues with no new bump.
    pub(super) continue_exit: Option<Option<WalkPosition>>,
    /// The copies of the capabilities of the process. The root process gets
    /// the capabilities of the policy of the run, and a child gets its copies
    /// from its parent at spawn (`Scheduler::spawn`). A handled revocation
    /// signal deletes a copy. The exit releases the copies when the process
    /// leaves the process table (`Delegations::release`). A new process has
    /// none.
    pub(super) security: SecurityContext,
    /// The process that spawned this process, as `p->parent`
    /// (`erl_process.c:12755-12760`), or `None` for the root process and the
    /// policy process, which the launcher makes. The policy module gets it
    /// in the requester map of `grant/3`.
    pub(super) parent: Option<ProcessId>,
    /// The start MFA of the process, and its arity, as `p->u.initial`
    /// (`erl_process.c:12979-12981`). The policy module gets it in the
    /// requester map of `initial/1` and `grant/3`. The code table resolves
    /// the names of an entry start when a reader needs them.
    pub(super) initial_call: InitialCall,
    /// Whether the next `erlang:apply/3` of the process is a body apply with
    /// no frame, as ERTS runs it from `beam_run_process`. `None` before the
    /// first apply: a start at `erlang:apply/3` runs the `call_ext_only` of
    /// its `erlang.beam` body as that apply. The wake-up of
    /// `erlang:hibernate/3` sets `Some(true)`, because ERTS also applies its
    /// MFA from `beam_run_process` (`bif.c:1403-1427`). Each other apply
    /// sets `Some(false)`.
    pub(super) frameless_apply: Option<bool>,
    /// `true` while the first apply of a start at `erlang:apply/3` runs. ERTS
    /// starts the process at `beam_run_process`, a body apply with no frame
    /// (`erl_process.c:12616-12624`), so the stack trace of a BIF that this
    /// apply calls does not hold the first frame of the process.
    pub(super) start_apply: bool,
    /// The answer of the policy process to the `pbeam_cap:request/2` call
    /// of the process: `true` for a grant and `false` for a denial, or
    /// `None` while the call waits. The grant signal sets it, and the call
    /// takes it.
    pub(super) policy_reply: Option<bool>,
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

/// A BIF trap that waits for the next turn of the process: the trap target
/// had no loaded export, and ERTS switches out at the trap before its export
/// stub runs (`context_switch_simplified`, `jit/arm/instr_bif.cpp:525-536`,
/// `676-701`). The arguments are in `x0` onwards.
#[derive(Debug)]
pub(super) struct PendingTrap {
    /// Module of the target, for example `re`.
    pub(super) module: &'static str,
    /// Function of the target, for example `grun`.
    pub(super) function: &'static str,
    /// Arity of the target, and the number of arguments in `x0` onwards.
    pub(super) arity: u32,
    /// True when the BIF call was a tail call through the target.
    pub(super) tail: bool,
}

/// A `prim_tty:write_nif/2` call that runs on a dirty IO thread.
#[derive(Debug)]
pub(super) struct DirtyWrite {
    /// The identifier of the call in the run. The completion of the call
    /// gives the same identifier.
    pub(super) call: u64,
    /// True when the NIF call was a tail call: its value then returns from
    /// the frame.
    pub(super) tail: bool,
    /// The result of the host write, or `None` while the call runs.
    pub(super) written: Option<IovecWritten>,
    /// The claim of the process on the call. A dirty IO thread runs the
    /// call only while the claim exists. The claim goes when this record
    /// goes: at the result, at the start of the exit, or at the end of the
    /// run.
    pub(super) claim: Claim,
}

impl Frame {
    /// Return the bytes of the Y-register slots of the frame: the capacity
    /// of the slot array times the size of one slot. The number excludes
    /// the header of the `Vec`, as the accounting of task
    /// [#559](https://github.com/fahchen/pbeam/issues/559) excludes it.
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// let frame = Frame { module_id: 0, function: 0, next_instruction: 0, stack: Vec::new() };
    /// assert_eq!(frame.allocated_bytes(), 0);
    /// ```
    pub(super) fn allocated_bytes(&self) -> usize {
        self.stack.capacity() * std::mem::size_of::<StackSlot>()
    }
}

impl ProcessState {
    /// Create a runnable process with argument registers. An entry start
    /// gives one frame. An undefined start gives no frame and keeps the
    /// module and the function in `undefined_start`. The arity of the start
    /// MFA is the number of argument registers.
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
        let arity = arguments.len();
        let (frames, undefined_start, initial_call) = match start {
            Start::Entry(frame) => {
                let initial_call = InitialCall::Entry {
                    module: frame.module_id,
                    function: frame.function,
                    arity,
                };
                (vec![frame], None, initial_call)
            }
            Start::Undefined { module, function } => {
                let initial_call = InitialCall::Undefined {
                    module: module.clone(),
                    function: function.clone(),
                    arity,
                };
                (Vec::new(), Some((module, function)), initial_call)
            }
        };
        Self {
            id,
            frames,
            undefined_start,
            registers: arguments.into_iter().map(Some).collect(),
            float_registers: Vec::new(),
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
            async_dist: false,
            node_monitors: BTreeMap::new(),
            priority: Priority::Normal,
            links: MonitorLinkTree::default(),
            monitors: MonitorLinkTree::default(),
            monitored_by: Vec::new(),
            group_leader: id,
            dictionary: Dictionary::default(),
            registered_name: None,
            error_handler: None,
            fcalls: 0,
            reductions: 0,
            pending_bif: None,
            pending_trap: None,
            insert_result: None,
            dirty_write: None,
            length_trap: None,
            bif_timers: MonitorLinkTree::default(),
            continue_exit: None,
            security: SecurityContext::default(),
            parent: None,
            initial_call,
            frameless_apply: None,
            start_apply: false,
            policy_reply: None,
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
    /// - `group_leader` gives `{group_leader, Pid}`, the group leader of the
    ///   process (`erl_bif_info.c:1866-1870`).
    /// - `priority` gives `{priority, Level}`, the priority that
    ///   `process_flag/2` or the spawn option set
    ///   (`erts_get_process_priority`, `erl_process.c:9443-9453`).
    /// - `reductions` gives `{reductions, N}`: the reduction total and
    ///   `current`, the reductions of the current turn when the process runs
    ///   (`erl_bif_info.c:2114-2121`).
    /// - `current_stacktrace` gives `{current_stacktrace, []}` for the
    ///   calling process (`caller`), because PBeam records no stack frames.
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// let length = process.info(&atoms, &Term::static_atom("message_queue_len"), 0, 2, false, None)?;
    /// assert_eq!(length, Some(Term::tuple(vec![Term::static_atom("message_queue_len"), Term::Integer(2)])));
    /// ```
    ///
    /// # Errors
    ///
    /// A reduction total above the PBeam integer range is an error.
    pub(super) fn info(
        &self,
        atoms: &AtomTable,
        item: &Term,
        current: u64,
        messages: usize,
        caller: bool,
        registered_name: Option<&Atom>,
    ) -> Result<Option<Term>, BeamError> {
        let Term::Atom(name) = item else {
            return Ok(None);
        };
        let value = match (atoms.name_of(name), &self.registered_name) {
            // ponytail: PBeam records no stack frames; see "Exception stack trace" in docs/erts-differences.md.
            ("current_stacktrace", _) if caller => Term::nil(),
            ("registered_name", _) => match registered_name {
                Some(registered) => Term::Atom(registered.clone()),
                None => return Ok(Some(Term::nil())),
            },
            ("message_queue_len", _) => Term::Integer(messages as i64),
            ("group_leader", _) => Term::Pid(self.group_leader),
            ("priority", _) => Term::static_atom(self.priority.name()),
            ("links", _) => {
                // An entry of a link that waits for its unlink
                // acknowledgment stays in the tree with a nonzero value,
                // and ERTS reports only active links (`collect_one_link`,
                // `erl_bif_info.c:574-585`). ERTS walks the tree and
                // prepends each link, so the list is the reverse of the
                // walk (`erl_bif_info.c:1764-1775`).
                let links: Vec<Term> = self
                    .links
                    .post_order()
                    .filter(|(_, unlinking)| **unlinking == 0)
                    .map(|(id, _)| id.term())
                    .collect();
                Term::list_from(links.into_iter().rev())
            }
            ("reductions", _) => Term::Integer(
                i64::try_from(self.reductions + current)
                    .map_err(|_| error("the reduction total exceeds the PBeam integer range"))?,
            ),
            _ => return Ok(None),
        };
        Ok(Some(Term::tuple(vec![item.clone(), value])))
    }

    /// Return true while the dirty IO call of the process runs: the
    /// process waits for its completion and does not run Erlang code.
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// assert!(!process.dirty_running());
    /// ```
    pub(super) fn dirty_running(&self) -> bool {
        self.dirty_write
            .as_ref()
            .is_some_and(|write| write.written.is_none())
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

    /// Return the bytes of the container arrays of the process: the frame
    /// array and the Y-register slots of each frame, the register array,
    /// the dictionary, the target monitors, the mailbox, the link and
    /// monitor trees, and the BIF timers. The number excludes the header of
    /// each `Vec` and of each tree node, as the accounting of task
    /// [#559](https://github.com/fahchen/pbeam/issues/559) excludes them.
    /// The terms that the containers hold are the work of
    /// [`Self::account`].
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// let process = ProcessState::new(ProcessId(0), Start::Undefined { module, function }, vec![]);
    /// assert!(process.allocated_bytes() > 0);
    /// ```
    pub(super) fn allocated_bytes(&self) -> usize {
        let mut bytes = self.frames.capacity() * std::mem::size_of::<Frame>()
            + self.registers.capacity() * std::mem::size_of::<Option<Term>>()
            + self.dictionary_bytes()
            + self.pending_bif_bytes()
            + self.length_trap_bytes()
            + self.monitored_by.capacity() * std::mem::size_of::<TargetMonitor>()
            + self.mailbox.allocated_bytes()
            + self.links.allocated_bytes()
            + self.monitors.allocated_bytes()
            + self.bif_timers.allocated_bytes();
        for frame in &self.frames {
            bytes += frame.allocated_bytes();
        }
        bytes
    }

    /// Return the bytes of the process dictionary: the capacity of its pair,
    /// chain, and bucket arrays times the size of one item. The number
    /// excludes the headers of the arrays, as the accounting of task
    /// [#559](https://github.com/fahchen/pbeam/issues/559) excludes it.
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// let mut process = ProcessState::new(ProcessId(0), Start::Entry(frame), vec![]);
    /// process.dictionary.put(Term::Integer(1), Term::Integer(2));
    /// assert!(process.dictionary_bytes() > 0);
    /// ```
    pub(super) fn dictionary_bytes(&self) -> usize {
        self.dictionary.allocated_bytes()
    }

    /// Return the bytes of the argument vector of the pending BIF call of the
    /// process: the capacity of the vector times the size of one argument, or
    /// 0 when the process has no pending call. The vector survives the
    /// scheduling step at the call. The number excludes the header of the
    /// `Vec`, as the accounting of task
    /// [#559](https://github.com/fahchen/pbeam/issues/559) excludes it.
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// let mut process = ProcessState::new(ProcessId(0), Start::Entry(frame), vec![]);
    /// process.pending_bif = Some(PendingBif { module: "lists", function: "member", arguments: vec![Term::Integer(1)], tail: false });
    /// assert!(process.pending_bif_bytes() > 0);
    /// ```
    pub(super) fn pending_bif_bytes(&self) -> usize {
        self.pending_bif.as_ref().map_or(0, |pending| {
            pending.arguments.capacity() * std::mem::size_of::<Term>()
        })
    }

    /// Return the bytes of the state vector of the `length/1` trap of the
    /// process: the capacity of the vector times the size of one term, or 0
    /// when the process has no trap. The vector survives the scheduling step
    /// at the trap. The number excludes the header of the `Vec`, as the
    /// accounting of task [#559](https://github.com/fahchen/pbeam/issues/559)
    /// excludes it.
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// let mut process = ProcessState::new(ProcessId(0), Start::Entry(frame), vec![]);
    /// process.length_trap = Some(vec![Term::Integer(1), Term::nil()]);
    /// assert!(process.length_trap_bytes() > 0);
    /// ```
    pub(super) fn length_trap_bytes(&self) -> usize {
        self.length_trap
            .as_ref()
            .map_or(0, |trap| trap.capacity() * std::mem::size_of::<Term>())
    }

    /// Count the bytes of the containers of the process and of each guest
    /// term that they hold. The terms are in the Y-register slots, the X
    /// registers, the dictionary, the exit reason of an exiting process, the
    /// mailbox entries, the origins of the monitor tree, the arguments of a
    /// pending BIF, and the state of the `length/1` trap. The walk reaches
    /// the exit reason for a live process and for a freed process, because
    /// the scheduler counts both through this method.
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// let mut live = LiveBytes::new();
    /// process.account(&mut live);
    /// assert!(live.bytes() > 0);
    /// ```
    pub(super) fn account(&self, live: &mut LiveBytes) {
        live.add(self.allocated_bytes());
        for frame in &self.frames {
            for slot in &frame.stack {
                if let StackSlot::Term(term) = slot {
                    live.term(term);
                }
            }
        }
        for register in self.registers.iter().flatten() {
            live.term(register);
        }
        self.dictionary.account(live);
        if let Some(reason) = &self.exiting {
            live.term(reason);
        }
        self.mailbox.account_terms(live);
        for origin in self.monitors.values() {
            if let Some(monitor) = &origin.monitor {
                live.term(&monitor.item);
                live.term(&monitor.tag);
            }
        }
        if let Some(pending) = &self.pending_bif {
            for argument in &pending.arguments {
                live.term(argument);
            }
        }
        if let Some(trap) = &self.length_trap {
            for term in trap {
                live.term(term);
            }
        }
    }
}

#[cfg(test)]
#[path = "../../tests/runtime/process.rs"]
pub(super) mod tests;

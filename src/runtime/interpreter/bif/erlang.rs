//! Bodies of the `erlang` BIFs that do not fit on one table line.

use super::BifResult::{Raise, Trap, Value};
use super::*;
use crate::runtime::capability;
use crate::runtime::host;
use crate::runtime::interpreter::code_table::Finish;
use crate::runtime::local_node;
use crate::runtime::process::LinkId;
use crate::runtime::process::{Flush, TimerReference};
use crate::runtime::scheduler::{AliasMode, CONTEXT_REDS, Flushed, MonitorOptions, MonitorTarget};
use crate::runtime::signal::Signal;
use crate::runtime::time::{self, TimeValue};
use crate::runtime::timer::{Receiver, Timer, Timers};
use crate::term::{Fun, MAX_ATOM_CHARACTERS, MAX_TUPLE_SIZE, ProcessId, Reference};

/// Run `erlang:node/1`: return the node of a pid. A term that is not a pid
/// gives a runtime error.
///
/// # Examples
///
/// ```rust,ignore
/// let result = node(&Term::Pid(pid))?;
/// assert!(matches!(result, BifResult::Value(Term::Atom(atom)) if atom.name() == "nonode@nohost"));
/// assert!(node(&Term::Integer(1)).is_err());
/// ```
pub(super) fn node(term: &Term) -> Result<BifResult, BeamError> {
    match term {
        Term::Pid(_) => Ok(Value(local_node())),
        _ => Err(error(format!(
            "unsupported erlang:node/1 argument {term:?}: not a pid"
        ))),
    }
}

/// The flags of ERTS `process_flag_2` (`bif.c:2028-2237`) that need a
/// subsystem that PBeam does not have: distribution, the error handler,
/// priorities, schedulers, the garbage collector, tracing, and saved calls.
/// The tuple `{monitor_nodes, Options}` is also such a flag.
const UNSUPPORTED_PROCESS_FLAGS: &[&str] = &[
    "async_dist",
    "error_handler",
    "priority",
    "scheduler",
    "fullsweep_after",
    "min_heap_size",
    "min_bin_vheap_size",
    "max_heap_size",
    "sensitive",
    "monitor_nodes",
    "save_calls",
];

/// Run `erlang:process_flag(Flag, Value)` as ERTS `process_flag_2`
/// (`bif.c:2028-2237`) does for the flags `trap_exit` and
/// `message_queue_data`: set the flag of the calling process and return the
/// old value. `trap_exit` takes `true` or `false`. `message_queue_data`
/// takes `on_heap` or `off_heap` (`erts_change_message_queue_management`,
/// `erl_message.c:1022-1100`), and its start value is `on_heap`. Each other
/// value raises `badarg`. A flag that ERTS does not know raises `badarg`,
/// also a tuple that is not `{monitor_nodes, Options}`.
///
/// Each flag of `UNSUPPORTED_PROCESS_FLAGS` except `max_heap_size` first
/// checks its value as ERTS does, and raises `badarg` for a value that is
/// not valid: `async_dist`, `sensitive`, and `monitor_nodes` take a boolean
/// (`erts_monitor_nodes`, `dist.c:6740-6741`), `error_handler` takes an
/// atom, `priority` takes `low`, `normal`, `high`, or `max`
/// (`erts_set_process_priority`, `erl_process.c:9460-9466`), `save_calls`
/// takes a small integer from 0 to 10000 (`process_flag_aux`,
/// `bif.c:1907-1915`), and `scheduler`, `fullsweep_after`, `min_heap_size`,
/// and `min_bin_vheap_size` take a small integer of 0 or more.
///
/// PBeam has no process heap, so `message_queue_data` does not change where
/// the messages are kept.
///
/// # Examples
///
/// ```rust,ignore
/// let old = process_flag(&mut context, &Term::static_atom("trap_exit"), &Term::static_atom("true"))?;
/// assert!(matches!(old, BifResult::Value(Term::Atom(atom)) if atom.name() == "false"));
/// let old = process_flag(&mut context, &Term::static_atom("message_queue_data"), &Term::static_atom("off_heap"))?;
/// assert!(matches!(old, BifResult::Value(Term::Atom(atom)) if atom.name() == "on_heap"));
/// assert!(matches!(process_flag(&mut context, &foo, &Term::static_atom("true"))?, BifResult::Raise { .. }));
/// assert!(process_flag(&mut context, &Term::static_atom("priority"), &Term::static_atom("high")).is_err());
/// ```
///
/// # Errors
///
/// Each flag of `UNSUPPORTED_PROCESS_FLAGS` gives a runtime error for a
/// value that passes the check, and `max_heap_size` and
/// `{monitor_nodes, Options}` give it for each value. ERTS raises `badarg`
/// for a `scheduler` value above the scheduler count, which PBeam does not
/// model, and PBeam gives the runtime error.
pub(super) fn process_flag(
    context: &mut BifContext<'_>,
    flag: &Term,
    value: &Term,
) -> Result<BifResult, BeamError> {
    let process = context.scheduler.current_process_mut();
    let term = value;
    let value = match value {
        Term::Atom(value) => Some(value.name()),
        _ => None,
    };
    Ok(match flag {
        Term::Atom(flag) if flag.name() == "trap_exit" => {
            let old = Term::static_atom(&process.trap_exit.to_string());
            match value {
                Some("true") => process.trap_exit = true,
                Some("false") => process.trap_exit = false,
                _ => return Ok(BifResult::badarg()),
            }
            Value(old)
        }
        Term::Atom(flag) if flag.name() == "message_queue_data" => {
            let old = Term::static_atom(if process.off_heap_message_queue {
                "off_heap"
            } else {
                "on_heap"
            });
            match value {
                Some("on_heap") => process.off_heap_message_queue = false,
                Some("off_heap") => process.off_heap_message_queue = true,
                _ => return Ok(BifResult::badarg()),
            }
            Value(old)
        }
        Term::Atom(flag) if UNSUPPORTED_PROCESS_FLAGS.contains(&flag.name()) => {
            let small = term.small_integer();
            let valid = match flag.name() {
                "async_dist" | "sensitive" | "monitor_nodes" => {
                    matches!(value, Some("true" | "false"))
                }
                "error_handler" => value.is_some(),
                "priority" => matches!(value, Some("low" | "normal" | "high" | "max")),
                "save_calls" => small.is_some_and(|calls| (0..=10_000).contains(&calls)),
                "max_heap_size" => true,
                _ => small.is_some_and(|size| size >= 0),
            };
            if !valid {
                return Ok(BifResult::badarg());
            }
            return Err(error(format!(
                "unsupported erlang:process_flag/2 flag {}",
                flag.name()
            )));
        }
        Term::Tuple(elements) if matches!(elements.as_ref(), [Term::Atom(tag), _] if tag.name() == "monitor_nodes") =>
        {
            return Err(error(
                "unsupported erlang:process_flag/2 flag {monitor_nodes, Options}",
            ));
        }
        _ => BifResult::badarg(),
    })
}

/// The items of ERTS `system_flag_2` (`bif.c:4972-5304`) that need a
/// subsystem that PBeam does not have: schedulers, the garbage collector,
/// tracing, the time offset, statistics, the CPU topology, and the memory
/// allocators.
const UNSUPPORTED_SYSTEM_FLAGS: &[&str] = &[
    "multi_scheduling",
    "schedulers_online",
    "fullsweep_after",
    "min_heap_size",
    "min_bin_vheap_size",
    "max_heap_size",
    "trace_control_word",
    "sequential_tracer",
    "reset_seq_trace",
    "scheduler_wall_time",
    "dirty_cpu_schedulers_online",
    "microstate_accounting",
    "outstanding_system_requests_limit",
    "scheduling_statistics",
    "internal_cpu_topology",
    "cpu_topology",
    "scheduler_bind_type",
    "erts_alloc",
];

/// The minimum heap size in words, `H_MIN_SIZE` of ERTS. ERTS starts it at
/// `H_DEFAULT_SIZE` (`erl_vm.h:66`, `erl_init.c:808`). Only the emulator
/// flag `+hms` and the item `min_heap_size` change it, and PBeam has
/// neither, so the value stays 233.
const MIN_HEAP_SIZE: i64 = 233;

/// The largest value of `backtrace_depth`, `MAX_BACKTRACE_SIZE` of ERTS
/// (`error.h:206`). `system_flag(backtrace_depth, Depth)` sets a larger
/// depth to this value (`bif.c:5108-5115`).
const MAX_BACKTRACE_SIZE: i64 = 64;

/// Run `erlang:system_flag(Item, Value)` as ERTS `system_flag_2`
/// (`bif.c:4972-5304`) does for the items `system_logger`, `debug_flags`,
/// and `backtrace_depth`. `system_logger` takes `logger`, `undefined`, or a
/// local pid, sets the node value, and returns the old value
/// (`erts_set_system_logger`, `erl_trace.c:2486-2491`). The start value is
/// `logger` (`erl_trace.c:339`). Each other value raises `badarg`.
/// `debug_flags` returns `true` for each value. `backtrace_depth` takes a
/// small integer of 0 or more, sets the depth of the run to at most 64, and
/// returns the old depth (`bif.c:5108-5115`). Each other value raises
/// `badarg`. An item that ERTS does not
/// know raises `badarg`, also `time_offset` with a value that is not
/// `finalize`. Each item of `UNSUPPORTED_SYSTEM_FLAGS` first checks its
/// value as ERTS does (`unsupported_system_flag_value`), and raises
/// `badarg` for a value that is not valid.
///
/// After the check of the item and the value, the caller needs a `runtime`
/// capability that permits `system_flag`, else the call raises
/// `error:capability_denied` and sets nothing. The check covers each item
/// that ERTS accepts, also `debug_flags`. A value that only the missing
/// subsystem rejects, such as a scheduler count above the host count, gives
/// `capability_denied` to a denied caller where ERTS raises `badarg`.
///
/// PBeam writes no error report (row "Error report" in
/// `docs/erts-differences.md`), so the system logger value has no other
/// effect.
///
/// # Examples
///
/// ```rust,ignore
/// let old = system_flag(&mut context, &Term::static_atom("system_logger"), &Term::Pid(pid))?;
/// assert!(matches!(old, BifResult::Value(Term::Atom(atom)) if atom.name() == "logger"));
/// assert!(matches!(system_flag(&mut context, &foo, &Term::Integer(1))?, BifResult::Raise { .. }));
/// let depth = system_flag(&mut context, &Term::static_atom("backtrace_depth"), &Term::Integer(100))?;
/// assert!(matches!(depth, BifResult::Value(Term::Integer(8))));
/// let depth = system_flag(&mut context, &Term::static_atom("backtrace_depth"), &Term::static_atom("foo"))?;
/// assert!(matches!(depth, BifResult::Raise { .. }));
/// assert!(system_flag(&mut context, &Term::static_atom("trace_control_word"), &Term::Integer(0)).is_err());
/// ```
///
/// # Errors
///
/// Each item of `UNSUPPORTED_SYSTEM_FLAGS` with a value that passes the
/// check, and `time_offset` with `finalize`, give a runtime error. A new
/// `capability_denied` atom in a full atom table gives the error of
/// `AtomTable::put`.
pub(super) fn system_flag(
    context: &mut BifContext<'_>,
    item: &Term,
    value: &Term,
) -> Result<BifResult, BeamError> {
    let Term::Atom(item) = item else {
        return Ok(BifResult::badarg());
    };
    let logger = match value {
        Term::Pid(_) => true,
        Term::Atom(logger) => matches!(logger.name(), "logger" | "undefined"),
        _ => false,
    };
    let valid = match (item.name(), value) {
        ("system_logger", _) => logger,
        ("debug_flags", _) => true,
        ("backtrace_depth", _) => value.small_integer().is_some_and(|depth| depth >= 0),
        ("time_offset", Term::Atom(finalize)) => finalize.name() == "finalize",
        (name, _) if UNSUPPORTED_SYSTEM_FLAGS.contains(&name) => {
            unsupported_system_flag_value(name, value)
        }
        _ => false,
    };
    if !valid {
        return Ok(BifResult::badarg());
    }
    let scheduler = &mut *context.scheduler;
    let check = denial(
        scheduler,
        CapabilityKind::Runtime,
        Operation::SystemFlag,
        None,
    );
    if let Some(denied) = check? {
        return Ok(denied);
    }
    Ok(match (item.name(), value.small_integer()) {
        ("system_logger", _) => Value(std::mem::replace(
            &mut scheduler.system_logger,
            value.clone(),
        )),
        ("debug_flags", _) => Value(Term::static_atom("true")),
        ("backtrace_depth", Some(depth)) => {
            let depth = depth.min(MAX_BACKTRACE_SIZE) as usize;
            let old = std::mem::replace(&mut scheduler.backtrace_depth, depth);
            Value(Term::Integer(old as i64))
        }
        (name, _) => {
            return Err(error(format!(
                "unsupported erlang:system_flag/2 item {name}"
            )));
        }
    })
}

/// Tell whether ERTS `system_flag_2` (`bif.c:4972-5304`) accepts `value` for
/// the item `name` of `UNSUPPORTED_SYSTEM_FLAGS` before it uses the missing
/// subsystem. ERTS raises `badarg` for a value that this check rejects:
///
/// - `multi_scheduling` takes `block`, `unblock`, `block_normal`, or
///   `unblock_normal`.
/// - `schedulers_online` and `dirty_cpu_schedulers_online` take a small
///   integer of 1 or more (`erts_set_schedulers_online`,
///   `erl_process.c:8241-8242`).
/// - `fullsweep_after`, `min_heap_size`, and `min_bin_vheap_size` take a
///   small integer of 0 or more.
/// - `trace_control_word` takes an integer from 0 to `u32::MAX`
///   (`db_set_trace_control_word`, `erl_db_util.c:1091-1103`).
/// - `sequential_tracer` takes `false`, a pid, or `{Module, State}` with an
///   atom `Module` (`erts_term_to_tracer`, `erl_trace.c:3090-3129`).
/// - `scheduler_wall_time` takes `true` or `false`, and
///   `microstate_accounting` takes `true`, `false`, or `reset`.
/// - `outstanding_system_requests_limit` takes an integer from 1 to
///   `2^27 - 1` (`erts_set_outstanding_system_requests_limit`,
///   `beam_bif_load.c:115-126`).
/// - `scheduling_statistics` takes `disable`, `enable`, or `clear`.
/// - `internal_cpu_topology` takes `undefined` or a non-empty proper list of
///   tuples `{cpu, Node, Processor, ProcessorNode, Core, Thread, Logical}`
///   (`erts_set_cpu_topology`, `erl_cpu_topology.c:1348-1400`).
/// - `erts_alloc` takes `{Allocator, Parameter, Value}` with two atoms and
///   an integer (`erts_alloc_set_dyn_param`, `erl_alloc.c:3445-3462`).
/// - `max_heap_size` takes a small integer `Size`, or a map with a small
///   integer `size` key and the optional boolean keys `kill`,
///   `error_logger`, and `include_shared_binaries` (`erts_max_heap_size`,
///   `erl_gc.c:3824-3872`). `Size` is 0 or at least the minimum heap size
///   (`bif.c:5089-5090`).
///
/// `reset_seq_trace`, `cpu_topology`, and `scheduler_bind_type` accept each
/// value here. ERTS also rejects a value that this check accepts when the
/// value needs the state of the missing subsystem: the scheduler count, the
/// tracer module, the allocator names, the CPU topology checks, and the CPU
/// binding support of the host.
///
/// # Examples
///
/// ```rust,ignore
/// assert!(!unsupported_system_flag_value("min_heap_size", &Term::static_atom("foo")));
/// assert!(unsupported_system_flag_value("min_heap_size", &Term::Integer(8)));
/// assert!(!unsupported_system_flag_value("schedulers_online", &Term::Integer(0)));
/// ```
fn unsupported_system_flag_value(name: &str, value: &Term) -> bool {
    let atom = match value {
        Term::Atom(atom) => Some(atom.name()),
        _ => None,
    };
    let integer = match value {
        Term::Integer(integer) => Some(*integer),
        _ => None,
    };
    let small = value.small_integer();
    let cpu = |cpu: &Term| match cpu {
        Term::Tuple(elements) => {
            matches!(elements.as_ref(), [Term::Atom(tag), _, _, _, _, _, _] if tag.name() == "cpu")
        }
        _ => false,
    };
    match name {
        "multi_scheduling" => matches!(
            atom,
            Some("block" | "unblock" | "block_normal" | "unblock_normal")
        ),
        "schedulers_online" | "dirty_cpu_schedulers_online" => {
            small.is_some_and(|count| count >= 1)
        }
        "fullsweep_after" | "min_heap_size" | "min_bin_vheap_size" => {
            small.is_some_and(|size| size >= 0)
        }
        "trace_control_word" => integer.is_some_and(|word| u32::try_from(word).is_ok()),
        "sequential_tracer" => match value {
            Term::Pid(_) => true,
            Term::Tuple(elements) => matches!(elements.as_ref(), [Term::Atom(_), _]),
            _ => atom == Some("false"),
        },
        "scheduler_wall_time" => matches!(atom, Some("true" | "false")),
        "microstate_accounting" => matches!(atom, Some("true" | "false" | "reset")),
        "outstanding_system_requests_limit" => {
            integer.is_some_and(|limit| (1..1 << 27).contains(&limit))
        }
        "scheduling_statistics" => matches!(atom, Some("disable" | "enable" | "clear")),
        "internal_cpu_topology" => match value.proper_list() {
            Some(cpus) => !value.is_nil() && cpus.clone().all(cpu),
            None => atom == Some("undefined"),
        },
        "erts_alloc" => match value {
            Term::Tuple(elements) => matches!(
                elements.as_ref(),
                [Term::Atom(_), Term::Atom(_), Term::Integer(_)]
            ),
            _ => false,
        },
        "max_heap_size" => {
            let size = match value {
                Term::Map(map) => {
                    let flag = |key| {
                        map.get(&Term::static_atom(key)).is_none_or(
                            |flag| matches!(flag, Term::Atom(flag) if matches!(flag.name(), "true" | "false")),
                        )
                    };
                    if !["kill", "error_logger", "include_shared_binaries"]
                        .into_iter()
                        .all(flag)
                    {
                        return false;
                    }
                    map.get(&Term::static_atom("size"))
                        .and_then(Term::small_integer)
                }
                _ => small,
            };
            size.is_some_and(|size| size == 0 || size >= MIN_HEAP_SIZE)
        }
        _ => true,
    }
}

/// Run `erlang:error(Reason)` as ERTS `error_1` (`bif.c:1436-1440`) does:
/// raise an exception of class `error` with `Reason`. The first frame of
/// the stack trace is the frame of the function that calls the BIF, with
/// its arity: ERTS `handle_error` starts with the arguments `am_true`
/// (`beam_common.c:454`), and `erts_build_mfa_item` puts the arity in the
/// frame when the arguments are not a list (`utils.c:3448-3455`). Thus
/// this is [`error_2`] with arguments that are not a list.
///
/// `error_handler:crash/1` calls `tl/1` on this stack trace
/// (`error_handler.erl:172-178`).
///
/// # Examples
///
/// ```rust,ignore
/// // In `tail/1`: erlang:error(r).
/// let result = error_1(context, &Term::static_atom("r"))?;
/// // The stack trace is [{Module, tail, 1, []}].
/// assert!(matches!(result, BifResult::RaiseWithStacktrace { .. }));
/// ```
///
/// # Errors
///
/// A caller instruction with no `func_info` before it is a runtime
/// invariant error.
pub(super) fn error_1(context: &mut BifContext<'_>, reason: &Term) -> Result<BifResult, BeamError> {
    error_2(context, reason, &Term::static_atom("true"))
}

/// Run `erlang:error(Reason, Args)` as ERTS `error_2` (`bif.c:1448-1454`)
/// does: raise an exception of class `error` with `Reason`. ERTS puts
/// `Args` in the first frame of the stack trace, the frame of the function
/// that calls the BIF (`beam_common.c:478-495`, `903-911`, `1011-1027`). The
/// frame is `{Module, Function, Args, Location}` when `Args` is a list, and
/// `{Module, Function, Arity, Location}` for each other term
/// (`erts_build_mfa_item`, `utils.c:3448-3455`).
///
/// PBeam keeps no line table and no caller frames (row "Exception stack
/// trace" in `docs/erts-differences.md`), so the stack trace is only this
/// frame, with the location `[]`. OTP code such as `re:ucompile/2` and
/// `unicode:fake_stacktrace/3` matches this first frame.
///
/// # Examples
///
/// ```rust,ignore
/// // In `tail/1`: erlang:error(r, [a]) and erlang:error(r, none).
/// let result = error_2(context, &Term::static_atom("r"), &args)?;
/// // The stack trace is [{Module, tail, [a], []}] or [{Module, tail, 1, []}].
/// assert!(matches!(result, BifResult::RaiseWithStacktrace { .. }));
/// ```
///
/// # Errors
///
/// A caller instruction with no `func_info` before it is a runtime
/// invariant error.
pub(super) fn error_2(
    context: &mut BifContext<'_>,
    reason: &Term,
    args: &Term,
) -> Result<BifResult, BeamError> {
    let caller = current_frame(context.scheduler.current_process_mut())
        .module
        .clone();
    let module = Rc::clone(context.scheduler.code.module(&caller)?);
    let (module, function, arity) = current_function(context.scheduler, &module)?;
    let args = if args.is_list() {
        args.clone()
    } else {
        Term::Integer(arity.into())
    };
    let frame = Term::tuple(vec![module, function, args, Term::nil()]);
    Ok(BifResult::RaiseWithStacktrace {
        class: ExceptionClass::Error,
        reason: reason.clone(),
        stacktrace: Term::list_from(vec![frame]),
    })
}

/// Run `erlang:raise(Class, Reason, Stacktrace)` as ERTS `raise_3`
/// (`bif.c:1549-1692`) does: raise an exception of class `error`, `exit`,
/// or `throw` with `Reason`. A class that is not `error`, `exit`, or
/// `throw`, or a stack trace that is not valid, makes the BIF return the
/// atom `badarg`; it does not raise. A valid stack trace is a proper list of
/// `{Fun, Args}`, `{Fun, Args, Location}`, `{M, F, A}`, or
/// `{M, F, A, Location}`, where `Fun` is a fun, `M` and `F` are atoms, and
/// `Location` is a list.
///
/// The exception gets the frames of `Stacktrace`. When the list has more
/// frames than `depth`, the run value of `backtrace_depth`, it keeps the
/// first `depth` frames, and one frame for the depth 0
/// (`bif.c:1626-1637`). A `{Fun, Args}` frame becomes `{Fun, Args, []}`,
/// and a `{M, F, A}` frame becomes `{M, F, A, []}` (`bif.c:1661-1687`).
///
/// # Examples
///
/// ```rust,ignore
/// let frames = Term::list_from(vec![mfa]); // [{m, f, 1}]
/// let result = raise(8, &Term::static_atom("throw"), &reason, &frames);
/// // The stack trace is [{m, f, 1, []}].
/// assert!(matches!(result, BifResult::RaiseWithStacktrace { class: ExceptionClass::Throw, .. }));
/// let result = raise(8, &Term::static_atom("foo"), &reason, &Term::nil());
/// assert!(matches!(result, BifResult::Value(Term::Atom(atom)) if atom.name() == "badarg"));
/// ```
pub(super) fn raise(depth: usize, class: &Term, reason: &Term, stacktrace: &Term) -> BifResult {
    let Some(class) = ExceptionClass::from_term(class) else {
        return Value(Term::static_atom("badarg"));
    };
    let location = |location: &Term| location.is_list();
    let frame = |frame: &Term| match frame {
        Term::Tuple(elements) => match elements.as_ref() {
            [Term::Fun(_), _] => true,
            [Term::Fun(_), _, place] => location(place),
            [Term::Atom(_), Term::Atom(_), _] => true,
            [Term::Atom(_), Term::Atom(_), _, place] => location(place),
            _ => false,
        },
        _ => false,
    };
    let Some(frames) = stacktrace.proper_list() else {
        return Value(Term::static_atom("badarg"));
    };
    if !frames.clone().all(frame) {
        return Value(Term::static_atom("badarg"));
    }
    let count = frames.clone().count();
    let kept = if depth < count { depth.max(1) } else { count };
    let frames = frames.take(kept).map(|frame| match frame {
        Term::Tuple(elements)
            if matches!(elements.as_ref(), [Term::Fun(_), _] | [Term::Atom(_), _, _]) =>
        {
            let mut elements: Vec<Term> = elements.iter().cloned().collect();
            elements.push(Term::nil());
            Term::tuple(elements)
        }
        _ => frame.clone(),
    });
    BifResult::RaiseWithStacktrace {
        class,
        reason: reason.clone(),
        stacktrace: Term::list_from(frames),
    }
}

/// Run `erlang:link(Pid)` for a local pid, as ERTS `link_opt`
/// (`bif.c:136-186`, `411-423`) does through [`Scheduler::link`]: return
/// `true`, or raise `noproc` for a process that does not exist or exits
/// when the caller does not trap exits.
///
/// The caller needs a `process` capability that permits `link`, also for
/// its own pid. Without it, the call raises `error:capability_denied` and
/// makes no link and no signal. The check comes before the process lookup,
/// so a denied call gives the same result for each pid. Then the runtime to
/// boot gate denies a link to a boot process in the same way (see
/// `boot_denial`).
///
/// # Examples
///
/// ```rust,ignore
/// assert!(matches!(link(scheduler, child)?, BifResult::Value(Term::Atom(atom)) if atom.name() == "true"));
/// assert!(matches!(link(scheduler, dead)?, BifResult::Raise { .. }));
/// ```
///
/// # Errors
///
/// A new `capability_denied` atom in a full atom table gives the error of
/// `AtomTable::put`.
pub(super) fn link(scheduler: &mut Scheduler, target: ProcessId) -> Result<BifResult, BeamError> {
    if let Some(denied) = denial(scheduler, CapabilityKind::Process, Operation::Link, None)? {
        return Ok(denied);
    }
    if let Some(denied) = boot_denial(scheduler, target)? {
        return Ok(denied);
    }
    Ok(if scheduler.link(target) {
        Value(Term::static_atom("true"))
    } else {
        Raise {
            class: ExceptionClass::Error,
            reason: Term::static_atom("noproc"),
        }
    })
}

/// Run `erlang:exit(Id, Reason)`: send an exit signal to a local pid, as
/// ERTS `send_exit_signal_bif` (`bif.c:1750-1772`) does, and return `true`.
/// The receiver acts on the signal when it handles it (see
/// [`Scheduler::handle_signals`]).
///
/// A reference sends the signal to the owner of the alias, with the caller
/// as the sender, and the owner ignores it when the alias is not active
/// (`erl_proc_sig_queue.c:1650-1658`, `4659-4672`). A reference that no
/// `erlang:monitor/3` made with an alias option sends nothing. The
/// reference of a NIF resource also gives `true`: in ERTS it is a magic
/// reference, and `is_internal_ref` is true for it (`erl_term.h:1064-1065`).
///
/// A signal to the caller with `kill`, or without `trap_exit`, ends the
/// caller before `exit/2` returns (`exit2_suicide`): `normal` then also ends
/// it. The caller fetches its signals and traps to `erts_internal:await_exit/0`,
/// which handles them until the signal ends the caller (`bif.c:1698-1716`).
///
/// Each other term raises `error:badarg` (`bif.c:1864-1866`). PBeam cannot
/// represent a remote pid.
///
/// A signal to another process, also through an alias of another process,
/// and a signal to a port need a `process` capability that permits
/// `exit_other`. Without it, the call raises `error:capability_denied` and
/// sends nothing. A signal to the caller, also through an alias of the
/// caller, has no check. A reference that is not an alias sends nothing and
/// has no check: in ERTS it holds no pid (`erl_proc_sig_queue.c:1647-1655`).
/// After the check, the runtime to boot gate denies a signal to a boot
/// process, also through its alias, in the same way (see `boot_denial`).
///
/// # Examples
///
/// ```rust,ignore
/// let result = exit(&mut context, &Term::Reference(reference), &boom)?;
/// assert!(matches!(result, BifResult::Value(Term::Atom(atom)) if atom.name() == "true"));
/// let result = exit(&mut context, &Term::Pid(caller), &kill)?;
/// assert!(matches!(result, BifResult::TrapBif { function: "await_exit", .. }));
/// assert!(matches!(exit(&mut context, &foo, &boom)?, BifResult::Raise { .. }));
/// ```
///
/// # Errors
///
/// A port after the check returns a runtime error (row "Ports" of
/// `docs/erts-differences.md`). A new `capability_denied` atom in a full
/// atom table gives the error of `AtomTable::put`.
pub(super) fn exit(
    context: &mut BifContext<'_>,
    id: &Term,
    reason: &Term,
) -> Result<BifResult, BeamError> {
    let scheduler = &mut *context.scheduler;
    let caller = scheduler.current_process_mut();
    let (caller_id, trap_exit) = (caller.id, caller.trap_exit);
    let (target, alias) = match id {
        Term::Pid(target) => (Some(*target), None),
        Term::Reference(reference) => {
            let Some(owner) = scheduler.alias_owner(*reference) else {
                return Ok(Value(Term::static_atom("true")));
            };
            (Some(owner), Some(*reference))
        }
        Term::Port(_) => (None, None),
        _ => return Ok(BifResult::badarg()),
    };
    if target != Some(caller_id) {
        let check = denial(
            scheduler,
            CapabilityKind::Process,
            Operation::ExitOther,
            None,
        );
        if let Some(denied) = check? {
            return Ok(denied);
        }
    }
    let Some(target) = target else {
        return Err(unsupported_port("erlang:exit/2"));
    };
    if let Some(denied) = boot_denial(scheduler, target)? {
        return Ok(denied);
    }
    let suicide = alias.is_none()
        && target == caller_id
        && (*reason == Term::static_atom("kill") || !trap_exit);
    let exit = Signal::Exit {
        from: LinkId::Process(caller_id),
        reason: reason.clone(),
        linked: false,
        normal_kills: suicide,
        alias,
    };
    // A receiver that does not exist drops the signal.
    let _ = scheduler.send_signal(Some(caller_id), target, exit);
    if !suicide {
        return Ok(Value(Term::static_atom("true")));
    }
    scheduler.current_process_mut().mailbox.fetch();
    Ok(BifResult::TrapBif {
        module: "erts_internal",
        function: "await_exit",
        arguments: Vec::new(),
    })
}

/// Run `erlang:demonitor(Reference, Options)` as ERTS `demonitor_2`
/// (bif.c) does. `Options` is a list of `flush` and `info` in any order,
/// with repeats. An options term that is not such a list raises `badarg`,
/// then a `Reference` that is not a reference raises `badarg`. The checks
/// come before the monitor is removed, so a call that raises leaves the
/// monitor active.
/// `erlang:demonitor/1` passes `[]`, as ERTS `demonitor_1` (bif.c) does,
/// so it returns `true`.
///
/// The result is `true`. With `info`, it is `false` when no active monitor
/// was removed. With `flush` and no active monitor removed, the DOWN
/// message can be queued, so the BIF traps into
/// `erts_internal:flush_monitor_messages(Reference, false, Result)`. ERTS
/// gives `Multi = true` only for a time-offset monitor, which PBeam does
/// not support.
///
/// # Examples
///
/// ```rust,ignore
/// let result = demonitor(&mut context, &Term::Reference(reference), &Term::nil());
/// assert!(matches!(result, BifResult::Value(Term::Atom(atom)) if atom.name() == "true"));
/// let info = Term::list_from(vec![Term::static_atom("info")]);
/// let result = demonitor(&mut context, &Term::Reference(reference), &info);
/// assert!(matches!(result, BifResult::Value(Term::Atom(atom)) if atom.name() == "false"));
/// let flush = Term::list_from(vec![Term::static_atom("flush")]);
/// let result = demonitor(&mut context, &Term::Reference(gone), &flush);
/// assert!(matches!(result, BifResult::Trap { function: "flush_monitor_messages", .. }));
/// let result = demonitor(&mut context, &Term::Integer(1), &Term::nil());
/// assert!(matches!(result, BifResult::Raise { .. }));
/// ```
pub(super) fn demonitor(
    context: &mut BifContext<'_>,
    reference: &Term,
    options: &Term,
) -> BifResult {
    let Some(options) = options.proper_list() else {
        return BifResult::badarg();
    };
    let (mut flush, mut info) = (false, false);
    for option in options {
        match option {
            Term::Atom(atom) if atom.name() == "flush" => flush = true,
            Term::Atom(atom) if atom.name() == "info" => info = true,
            _ => return BifResult::badarg(),
        }
    }
    let Term::Reference(reference) = reference else {
        return BifResult::badarg();
    };
    if context.scheduler.demonitor(*reference) {
        return Value(Term::static_atom("true"));
    }
    let result = Term::static_atom(if info { "false" } else { "true" });
    if !flush {
        return Value(result);
    }
    Trap {
        module: "erts_internal",
        function: "flush_monitor_messages",
        arguments: vec![
            Term::Reference(*reference),
            Term::static_atom("false"),
            result,
        ],
    }
}

/// Run `erlang:monitor(Type, Target, Options)` as ERTS `monitor_3`
/// (`bif.c:988-997`) and `monitor` (`bif.c:758-981`) do;
/// `erlang:monitor/2` passes `[]` as `monitor_2` (`bif.c:983-986`) does.
/// The options come first (see [`monitor_options`]). The type `process`
/// accepts a pid, a registered name, and `{Name, nonode@nohost}`; the type
/// `port` accepts a registered name and `{Name, nonode@nohost}`. PBeam is a
/// node that is not alive, so `{Name, Node}` with another node raises
/// `badarg` (`bif.c:881-882`). Each other target, and each other type,
/// raises `badarg`. A monitor of the caller creates no monitor and no
/// alias, and each port name gives the `noproc` `DOWN` message (see
/// [`Scheduler::monitor`]). PBeam registers no port.
///
/// After these checks, the caller needs a `process` capability that permits
/// `monitor`, also for its own pid. Without it, the call raises
/// `error:capability_denied`, makes no reference, and adds no monitor. The
/// check comes before the name lookup and before the runtime error of a
/// part that PBeam does not support.
///
/// # Examples
///
/// ```rust,ignore
/// let result = monitor(&mut context, &process, &Term::Pid(child), &Term::nil())?;
/// assert!(matches!(result, BifResult::Value(Term::Reference(_))));
/// assert!(matches!(monitor(&mut context, &port, &Term::Pid(child), &Term::nil())?, BifResult::Raise { .. }));
/// assert!(matches!(monitor(&mut context, &process, &Term::Pid(child), &x)?, BifResult::Raise { .. }));
/// assert!(monitor(&mut context, &time_offset, &clock_service, &Term::nil()).is_err());
/// ```
///
/// # Errors
///
/// A `time_offset` monitor of `clock_service`, a monitor of the type `port`
/// with a port, and the `priority` option of a monitor that is not of the
/// caller, are not supported. A new `capability_denied` atom in a full atom
/// table gives the error of `AtomTable::put`.
pub(super) fn monitor(
    context: &mut BifContext<'_>,
    kind: &Term,
    target: &Term,
    options: &Term,
) -> Result<BifResult, BeamError> {
    let Some(options) = monitor_options(options) else {
        return Ok(BifResult::badarg());
    };
    let Term::Atom(kind) = kind else {
        return Ok(BifResult::badarg());
    };
    // An `Err` is the runtime error of a valid target that PBeam does not
    // support. It comes after the capability check.
    let target = match (kind.name(), target) {
        ("process", Term::Pid(id)) => Some(Ok(MonitorTarget::Process(*id))),
        ("process", target) => local_name(target).map(|name| Ok(MonitorTarget::ProcessName(name))),
        ("port", Term::Port(_)) => Some(Err(unsupported_port("erlang:monitor/2,3"))),
        ("port", target) => local_name(target).map(|name| Ok(MonitorTarget::PortName(name))),
        ("time_offset", Term::Atom(service)) if service.name() == "clock_service" => {
            // ponytail: no time offset service yet; add it when a measured path monitors it.
            Some(Err(error("unsupported time_offset monitor")))
        }
        _ => None,
    };
    let Some(target) = target else {
        return Ok(BifResult::badarg());
    };
    let scheduler = &mut *context.scheduler;
    if let Some(denied) = denial(scheduler, CapabilityKind::Process, Operation::Monitor, None)? {
        return Ok(denied);
    }
    let reference = scheduler.monitor(target?, options)?;
    Ok(Value(Term::Reference(reference)))
}

/// Read the options of `erlang:monitor/3` as ERTS `erts_monitor_opts`
/// (`bif.c:701-756`) does. The options are a proper list of `priority`,
/// `{alias, explicit_unalias | demonitor | reply_demonitor}`, and
/// `{tag, Tag}`; the last `alias` and the last `tag` option win. Return
/// `None` for each other element and for a tail that is not `[]`, which
/// `monitor_3` rejects with `badarg`.
///
/// # Examples
///
/// ```rust,ignore
/// let alias = Term::tuple(vec![Term::static_atom("alias"), Term::static_atom("demonitor")]);
/// let options = monitor_options(&Term::list_from(vec![alias])).unwrap();
/// assert_eq!(options.alias, Some(AliasMode::Demonitor));
/// assert!(monitor_options(&Term::static_atom("x")).is_none());
/// ```
fn monitor_options(options: &Term) -> Option<MonitorOptions> {
    let options = options.proper_list()?;
    let mut result = MonitorOptions {
        alias: None,
        tag: Term::static_atom("DOWN"),
        priority: false,
    };
    for option in options {
        match option {
            Term::Atom(option) if option.name() == "priority" => result.priority = true,
            Term::Tuple(pair) => match pair.as_ref() {
                [Term::Atom(key), Term::Atom(mode)] if key.name() == "alias" => {
                    result.alias = Some(match mode.name() {
                        "explicit_unalias" => AliasMode::ExplicitUnalias,
                        "demonitor" => AliasMode::Demonitor,
                        "reply_demonitor" => AliasMode::ReplyDemonitor,
                        _ => return None,
                    });
                }
                [Term::Atom(key), tag] if key.name() == "tag" => result.tag = tag.clone(),
                _ => return None,
            },
            _ => return None,
        }
    }
    Some(result)
}

/// Return the name of a local monitor target by name: an atom, or
/// `{Name, nonode@nohost}` with an atom name. Return `None` for each other
/// term, which `monitor` (`bif.c:875-896`, `938-950`) rejects with `badarg`.
///
/// # Examples
///
/// ```rust,ignore
/// assert_eq!(local_name(&Term::tuple(vec![name.clone(), local_node()])), Some(atom));
/// assert_eq!(local_name(&Term::Integer(1)), None);
/// ```
fn local_name(target: &Term) -> Option<Atom> {
    match target {
        Term::Atom(name) => Some(name.clone()),
        Term::Tuple(pair) => match pair.as_ref() {
            [Term::Atom(name), node] if *node == local_node() => Some(name.clone()),
            _ => None,
        },
        _ => None,
    }
}

/// Run `erlang:spawn/3`: start a process that runs `module:function` with the
/// elements of `arguments`, as ERTS `spawn_3` does with default spawn options.
/// A `module` or `function` that is not an atom, and `arguments` that are not
/// a proper list, raise `badarg`.
///
/// # Examples
///
/// ```rust,ignore
/// let result = spawn(&mut context, &module, &function, &Term::nil())?;
/// assert!(matches!(result, BifResult::Value(Term::Pid(_))));
/// ```
pub(super) fn spawn(
    context: &mut BifContext<'_>,
    module: &Term,
    function: &Term,
    arguments: &Term,
) -> Result<BifResult, BeamError> {
    create_process(
        context.scheduler,
        module,
        function,
        arguments,
        SpawnOptions::default(),
    )
}

/// Run `erlang:spawn_link/3`: start a linked process, as ERTS `spawn_link_3`
/// does by adding `SPO_LINK` to the default spawn options.
///
/// # Examples
///
/// ```rust,ignore
/// let result = spawn_link(&mut context, &module, &function, &Term::nil())?;
/// assert!(matches!(result, BifResult::Value(Term::Pid(_))));
/// ```
pub(super) fn spawn_link(
    context: &mut BifContext<'_>,
    module: &Term,
    function: &Term,
    arguments: &Term,
) -> Result<BifResult, BeamError> {
    let options = SpawnOptions {
        link: true,
        ..SpawnOptions::default()
    };
    create_process(context.scheduler, module, function, arguments, options)
}

/// Run `erlang:spawn_opt/4`, which `erlang:spawn_monitor/3` in `erlang.beam`
/// calls with the options `[monitor]`: start a process with the spawn
/// options in `options`, as ERTS `spawn_opt_4` (`bif.c`) does. The option
/// `link` links the new process and `monitor` monitors it. With `monitor`
/// the result is `{Pid, Reference}`, else `Pid`. The option
/// `{message_queue_data, on_heap | off_heap}` sets the start value of the
/// `message_queue_data` flag of the new process, and the last such option
/// wins (`erl_process.c:12305-12319`). `options` that are not a proper list,
/// and a `message_queue_data` option with another value, raise `badarg`, as
/// `erts_parse_spawn_opts` (`erl_process.c:12222-12434`) does. The PBeam
/// option `{capabilities, Map}` restricts the capabilities of the new process
/// (see `capability::restrictions`), and the last such option wins. A `Map`
/// that is not valid raises `badarg`. Any other option gives a runtime
/// error. After the options, a `module` or `function` that is not an atom
/// raises `badarg`, as in `spawn/3`.
///
/// # Examples
///
/// ```rust,ignore
/// let result = spawn_opt(&mut context, &module, &function, &Term::nil(), &Term::list_from(vec![monitor]))?;
/// assert!(matches!(result, BifResult::Value(Term::Tuple(_))));
/// assert!(matches!(spawn_opt(&mut context, &module, &function, &Term::nil(), &link_x)?, BifResult::Raise { .. }));
/// assert!(spawn_opt(&mut context, &module, &function, &Term::nil(), &Term::list_from(vec![priority])).is_err());
/// ```
pub(super) fn spawn_opt(
    context: &mut BifContext<'_>,
    module: &Term,
    function: &Term,
    arguments: &Term,
    options: &Term,
) -> Result<BifResult, BeamError> {
    let Some(options) = options.proper_list() else {
        return Ok(BifResult::badarg());
    };
    let mut spawn_options = SpawnOptions::default();
    for option in options {
        match option {
            Term::Atom(option) if option.name() == "link" => spawn_options.link = true,
            Term::Atom(option) if option.name() == "monitor" => spawn_options.monitor = true,
            Term::Tuple(option) if matches!(option.as_ref(), [Term::Atom(name), _] if name.name() == "message_queue_data") =>
            {
                spawn_options.off_heap_message_queue = match &option[1] {
                    Term::Atom(value) if value.name() == "on_heap" => false,
                    Term::Atom(value) if value.name() == "off_heap" => true,
                    _ => return Ok(BifResult::badarg()),
                };
            }
            Term::Tuple(option) if matches!(option.as_ref(), [Term::Atom(name), _] if name.name() == "capabilities") =>
            {
                let Some(restrictions) = capability::restrictions(&option[1]) else {
                    return Ok(BifResult::badarg());
                };
                spawn_options.capabilities = Some(restrictions);
            }
            // ponytail: ERTS accepts more options; see "Spawn options" in docs/erts-differences.md.
            _ => return Err(error(format!("unsupported spawn option {option:?}"))),
        }
    }
    create_process(
        context.scheduler,
        module,
        function,
        arguments,
        spawn_options,
    )
}

/// Run `erlang:put/2`: set the value of `key` in the process dictionary of
/// the calling process. Return the old value, or `undefined` for a new key,
/// as ERTS `pd_hash_put` (`erl_process_dict.c`) does. Keys compare with
/// exact equality.
///
/// # Examples
///
/// ```rust,ignore
/// let key = Term::static_atom("ok");
/// assert!(matches!(put(&mut context, &key, &Term::Integer(1)), BifResult::Value(Term::Atom(atom)) if atom.name() == "undefined"));
/// assert!(matches!(put(&mut context, &key, &Term::Integer(2)), BifResult::Value(Term::Integer(1))));
/// ```
pub(super) fn put(context: &mut BifContext<'_>, key: &Term, value: &Term) -> BifResult {
    let dictionary = &mut context.scheduler.current_process_mut().dictionary;
    let old = match dictionary.iter_mut().find(|(other, _)| other == key) {
        Some((_, old)) => std::mem::replace(old, value.clone()),
        None => {
            dictionary.push((key.clone(), value.clone()));
            Term::static_atom("undefined")
        }
    };
    Value(old)
}

/// Run `erlang:get/1`: return the value of `key` in the process dictionary
/// of the calling process, or `undefined` when the key has no value. Keys
/// compare with exact equality.
///
/// # Examples
///
/// ```rust,ignore
/// let key = Term::static_atom("ok");
/// assert!(matches!(get(&mut context, &key), BifResult::Value(Term::Atom(atom)) if atom.name() == "undefined"));
/// put(&mut context, &key, &Term::Integer(1));
/// assert!(matches!(get(&mut context, &key), BifResult::Value(Term::Integer(1))));
/// ```
pub(super) fn get(context: &mut BifContext<'_>, key: &Term) -> BifResult {
    let dictionary = &context.scheduler.current_process_mut().dictionary;
    Value(
        dictionary
            .iter()
            .find(|(other, _)| other == key)
            .map_or_else(
                || Term::static_atom("undefined"),
                |(_, value)| value.clone(),
            ),
    )
}

/// Run `erlang:'++'/2`: append `right` to the proper list `left`, as ERTS
/// `append` (`erl_bif_lists.c:266-338`) does. `right` can be any term, so
/// `[1] ++ x` gives the improper list `[1|x]`, and an empty `left` gives
/// `right` itself with no cost. A `left` that is not a list raises
/// `badarg`. A nonempty `left` goes to [`append_chunk`], which can trap.
///
/// # Examples
///
/// ```rust,ignore
/// let x = Term::static_atom("x");
/// assert!(matches!(append(&mut process, &Term::nil(), &x)?, BifResult::Value(term) if term == x));
/// let list = append(&mut process, &Term::list_from(vec![Term::Integer(1)]), &x)?;
/// assert!(matches!(list, BifResult::Value(list) if list == Term::cons(Term::Integer(1), x.clone())));
/// assert!(matches!(append(&mut process, &x, &Term::nil())?, BifResult::Raise { .. }));
/// ```
///
/// # Errors
///
/// Return the runtime error of [`ProcessState::bif_reductions_left`] for a
/// negative budget.
pub(super) fn append(
    process: &mut ProcessState,
    left: &Term,
    right: &Term,
) -> Result<BifResult, BeamError> {
    match left {
        _ if left.is_nil() => Ok(Value(right.clone())),
        _ if left.is_list() => append_chunk(process, left, right, left),
        _ => Ok(BifResult::badarg()),
    }
}

/// Walk the next chunk of `left` from `rest`, the cons cell after the
/// elements done, as ERTS `append_continue` does
/// (`erl_bif_lists.c:237-251`). For a `right` of `[]`, `append_empty_rhs`
/// only walks the list, 32 elements for each reduction
/// (`erl_bif_lists.c:61-89`). Else `lists_append_onheap` copies 60 elements
/// for each reduction (`erl_bif_lists.c:176-235`). Each chunk takes as many
/// elements as the budget permits, and bumps the reductions for the
/// elements that it took. When elements remain, the BIF traps to its export
/// with the rest of `left`, as the ERTS state keeps its `iterator`
/// (`erl_bif_lists.c:47-55`, `282-289`, `316-320`). At the end, an improper
/// `left` raises `badarg`. Else a `right` of `[]` gives `left` itself, as
/// `append_empty_rhs` gives `lhs_original`, and another `right` gives a copy
/// of the elements of `left` with the tail `right`. PBeam makes the copy at
/// the end, in O(n) time, because a cons cell does not change after it is
/// made.
///
/// ERTS copies on the heap only while the heap has free words. When the
/// heap is full, it continues in `lists_append_alloc` with 40 elements for
/// each reduction (`erl_bif_lists.c:111-174`, `219-221`), and a garbage
/// collection can follow. PBeam has no process heap and always uses the
/// cost of the heap copy (see "Garbage collection reductions" in
/// `docs/erts-differences.md`).
///
/// # Examples
///
/// ```rust,ignore
/// process.fcalls = 1;
/// let left = Term::list_from((0..61).map(Term::Integer));
/// let trap = append_chunk(&mut process, &left, &Term::list_from(vec![Term::Integer(61)]), &left)?;
/// assert!(matches!(trap, BifResult::TrapBif { function: "++", .. }));
/// assert_eq!(process.fcalls, 0);
/// ```
///
/// # Errors
///
/// Return the runtime error of [`ProcessState::bif_reductions_left`] for a
/// negative budget.
pub(super) fn append_chunk(
    process: &mut ProcessState,
    left: &Term,
    right: &Term,
    rest: &Term,
) -> Result<BifResult, BeamError> {
    let mut elements = rest.list_iter().expect("'++' walks a list");
    let per_reduction = if right.is_nil() { 32 } else { 60 };
    let budget = per_reduction * process.bif_reductions_left()?;
    let walked = elements.by_ref().take(budget).count();
    process.bump_reductions((walked / per_reduction) as i32);
    let rest = elements.rest();
    if rest.is_nonempty_list() {
        return Ok(BifResult::TrapBif {
            module: "erlang",
            function: "++",
            arguments: vec![left.clone(), right.clone(), rest.clone()],
        });
    }
    Ok(match rest {
        _ if !rest.is_nil() => BifResult::badarg(),
        _ if right.is_nil() => Value(left.clone()),
        _ => {
            let elements = left.list_iter().expect("'++' appends to a list");
            Value(Term::list_with_tail(
                elements.cloned().collect(),
                right.clone(),
            ))
        }
    })
}

/// Count the elements of `list` and add `count`, as ERTS
/// `erts_trapping_length_1` does (`erl_bif_guard.c:270-313`) for
/// `erlang:length/1` (`erl_bif_guard.c:210-247`) and for the `i_length`
/// instruction of the guard BIF. A chunk takes at most 16 elements for each
/// reduction of the budget. When the list ends in the chunk, the BIF bumps
/// one reduction for each 16 elements and returns the count. Else it uses
/// all reductions and traps to its export with the rest of the list and the
/// count so far. A list that does not end in `[]` raises `badarg` with no
/// cost.
///
/// # Examples
///
/// ```rust,ignore
/// process.fcalls = 10;
/// let list = Term::list_from((0..32).map(Term::Integer));
/// assert!(matches!(length(&mut process, &list, 0)?, BifResult::Value(Term::Integer(32))));
/// assert_eq!(process.fcalls, 8);
/// process.fcalls = 1;
/// let trap = length(&mut process, &list, 0)?;
/// assert!(matches!(trap, BifResult::TrapBif { function: "length", .. }));
/// ```
///
/// # Errors
///
/// Return the runtime error of [`ProcessState::bif_reductions_left`] for a
/// negative budget.
pub(super) fn length(
    process: &mut ProcessState,
    list: &Term,
    count: i64,
) -> Result<BifResult, BeamError> {
    let Some(mut elements) = list.list_iter() else {
        return Ok(BifResult::badarg());
    };
    let max_iter = 16 * process.bif_reductions_left()?;
    let counted = elements.by_ref().take(max_iter).count();
    let rest = elements.rest();
    if rest.is_nonempty_list() {
        process.bump_all_reductions();
        return Ok(BifResult::TrapBif {
            module: "erlang",
            function: "length",
            arguments: vec![rest.clone(), Term::Integer(count + counted as i64)],
        });
    }
    if !rest.is_nil() {
        return Ok(BifResult::badarg());
    }
    process.bump_reductions((counted / 16) as i32);
    Ok(Value(Term::Integer(count + counted as i64)))
}

/// Run `erlang:binary_part/3`: return the `length` bytes of a binary from
/// byte `start`, as ERTS `binary_part_3` (`erl_bif_guard.c:455-458`) and
/// `erts_binary_part` (`erl_bif_binary.c:1954-2012`) do. A negative length
/// takes the bytes before `start`. A term that is not a bitstring, a start
/// that is not a non-negative integer, a length that is not an integer,
/// and a part that is not in the binary raise `badarg`. ERTS raises
/// `badarg` for the length `-2^63` because its negation does not fit in a
/// signed word (`erl_bif_binary.c:1974-1977`); PBeam gives the same result
/// because 2^63 is more than each start. In a guard, the `gc_bif3`
/// instruction goes to its fail label instead. ERTS returns a sub-binary of
/// the argument; PBeam returns a copy of the bytes. PBeam has no bitstring
/// that is not a binary and no bignum, so these ERTS `badarg` inputs cannot
/// occur (rows "Bitstring terms" and "Integer range" of
/// `docs/erts-differences.md`).
///
/// # Examples
///
/// ```rust,ignore
/// let hello = Term::binary(b"hello".to_vec());
/// let part = binary_part(&hello, &Term::Integer(5), &Term::Integer(-2));
/// assert!(matches!(part, BifResult::Value(Term::Binary(bytes)) if bytes == b"lo"));
/// let past = binary_part(&hello, &Term::Integer(1), &Term::Integer(5));
/// assert!(matches!(past, BifResult::Raise { .. }));
/// ```
pub(super) fn binary_part(binary: &Term, start: &Term, length: &Term) -> BifResult {
    let (Term::Binary(bytes), Term::Integer(start), Term::Integer(length)) =
        (binary, start, length)
    else {
        return BifResult::badarg();
    };
    let Ok(start) = u64::try_from(*start) else {
        return BifResult::badarg();
    };
    let (start, length) = if *length < 0 {
        let length = length.unsigned_abs();
        let Some(start) = start.checked_sub(length) else {
            return BifResult::badarg();
        };
        (start, length)
    } else {
        (start, *length as u64)
    };
    // The start is below 2^63 and the length is at most 2^63, so the sum
    // does not overflow.
    let end = start + length;
    if end > bytes.len() as u64 {
        return BifResult::badarg();
    }
    // ERTS makes a sub-binary that refers to the source
    // (`binary_part_3` in `erl_bif_binary.c`; `binary_referenced_byte_size_1`,
    // `erl_bif_binary.c:2508-2530`), so the part shares the bytes of the
    // source and keeps their offset.
    Value(Term::Binary(
        bytes.bits(start as usize * 8, end as usize * 8),
    ))
}

/// Run `erlang:byte_size/1`: return the number of bytes of a binary, as
/// ERTS `byte_size_1` (`erl_bif_guard.c:376-393`) and the JIT
/// `emit_bif_byte_size` (`jit/arm/instr_guard_bifs.cpp:438-509`) do. A
/// term that is not a bitstring raises `badarg`. In a guard, the `gc_bif`
/// instruction goes to its fail label instead. ERTS rounds the size of a
/// bitstring up to the next byte. PBeam has no bitstring that is not a
/// binary (row "Bitstring terms" of `docs/erts-differences.md`).
///
/// # Examples
///
/// ```rust,ignore
/// let hello = Term::binary(b"hello".to_vec());
/// assert!(matches!(byte_size(&hello), BifResult::Value(Term::Integer(5))));
/// assert!(matches!(byte_size(&Term::static_atom("foo")), BifResult::Raise { .. }));
/// ```
pub(super) fn byte_size(term: &Term) -> BifResult {
    match term {
        Term::Binary(bytes) => Value(Term::Integer(bytes.len() as i64)),
        _ => BifResult::badarg(),
    }
}

/// Run `erlang:tl/1`: return the tail of the first cons cell of a nonempty
/// list, as ERTS `tl_1` (`bif.c:2931-2938`) does. The tail of a proper list
/// is a proper list, and the tail of `[1|x]` is `x`. `[]` and a term that is
/// not a list raise `badarg`.
///
/// # Examples
///
/// ```rust,ignore
/// let x = Term::static_atom("x");
/// let proper = Term::list_from(vec![Term::Integer(1), Term::Integer(2)]);
/// assert!(matches!(tl(&proper), BifResult::Value(rest) if rest == Term::list_from([Term::Integer(2)])));
/// let improper = Term::list_with_tail(vec![Term::Integer(1)], x.clone());
/// assert!(matches!(tl(&improper), BifResult::Value(term) if term == x));
/// assert!(matches!(tl(&Term::nil()), BifResult::Raise { .. }));
/// ```
pub(super) fn tl(list: &Term) -> BifResult {
    match list.cons_cell() {
        Some((_, tail)) => Value(tail.clone()),
        None => BifResult::badarg(),
    }
}

/// Run `erlang:split_binary/2`: return `{Left, Right}`, the bytes of a
/// binary before and from `position`, as ERTS `split_binary_2` does
/// (`binary.c:476-517`). A term that is not a binary, and a position that is
/// not an integer from 0 to the byte size, raise `badarg`.
///
/// # Examples
///
/// ```rust,ignore
/// let pair = split_binary(&Term::binary(b"abc".to_vec()), &Term::Integer(1));
/// assert!(matches!(pair, BifResult::Value(Term::Tuple(parts)) if parts == [Term::binary(b"a".to_vec()), Term::binary(b"bc".to_vec())]));
/// assert!(matches!(split_binary(&Term::binary(vec![]), &Term::Integer(1)), BifResult::Raise { .. }));
/// ```
pub(super) fn split_binary(binary: &Term, position: &Term) -> BifResult {
    match (binary, position) {
        (Term::Binary(bytes), Term::Integer(at)) if (0..=bytes.len() as i64).contains(at) => {
            let (left, right) = bytes.split_at(*at as usize);
            Value(Term::tuple(vec![
                Term::binary(left.to_vec()),
                Term::binary(right.to_vec()),
            ]))
        }
        _ => BifResult::badarg(),
    }
}

/// Run `erlang:integer_to_list/1`: return the decimal characters of an
/// integer, as ERTS `integer_to_list_1` and `integer_to_list`
/// (`bif.c:3266-3339`) do. A term that is not an integer raises `badarg`.
/// [`integer_digits`] gives the digits and charges a bignum.
///
/// # Examples
///
/// ```rust,ignore
/// process.fcalls = 10;
/// let list = integer_to_list(&mut process, &Term::Integer(-12))?;
/// assert!(matches!(list, BifResult::Value(codes) if codes == Term::list_from([45, 49, 50].map(Term::Integer))));
/// assert_eq!(process.fcalls, 10);
/// integer_to_list(&mut process, &Term::Integer(1 << 59))?;
/// assert_eq!(process.fcalls, 9);
/// assert!(matches!(integer_to_list(&mut process, &Term::nil())?, BifResult::Raise { .. }));
/// process.fcalls = 0;
/// assert!(integer_to_list(&mut process, &Term::Integer(1 << 59)).is_err());
/// ```
///
/// # Errors
///
/// Return the runtime error of [`integer_digits`].
pub(super) fn integer_to_list(
    process: &mut ProcessState,
    term: &Term,
) -> Result<BifResult, BeamError> {
    let Some(digits) = integer_digits(process, term, 10, "integer_to_list/1")? else {
        return Ok(BifResult::badarg());
    };
    let codes = digits.bytes().map(|byte| Term::Integer(i64::from(byte)));
    Ok(Value(Term::list_from(codes)))
}

/// Run `erlang:integer_to_binary/1`: return the decimal characters of an
/// integer as a binary, as ERTS `integer_to_binary_1` and
/// `integer_to_binary` (`binary.c:80-145`) do. A term that is not an
/// integer raises `badarg`. [`integer_digits`] gives the digits and charges
/// a bignum.
///
/// # Examples
///
/// ```rust,ignore
/// process.fcalls = 10;
/// let binary = integer_to_binary(&mut process, &Term::Integer(-12))?;
/// assert!(matches!(binary, BifResult::Value(Term::Binary(bytes)) if bytes == b"-12"));
/// assert_eq!(process.fcalls, 10);
/// integer_to_binary(&mut process, &Term::Integer(1 << 59))?;
/// assert_eq!(process.fcalls, 9);
/// assert!(matches!(integer_to_binary(&mut process, &Term::static_atom("a"))?, BifResult::Raise { .. }));
/// ```
///
/// # Errors
///
/// Return the runtime error of [`integer_digits`].
pub(super) fn integer_to_binary(
    process: &mut ProcessState,
    term: &Term,
) -> Result<BifResult, BeamError> {
    let Some(digits) = integer_digits(process, term, 10, "integer_to_binary/1")? else {
        return Ok(BifResult::badarg());
    };
    Ok(Value(Term::binary(digits.into_bytes())))
}

/// Run `erlang:integer_to_binary/2`: return the characters of an integer in
/// `base` as a binary, as ERTS `integer_to_binary_2` and
/// `integer_to_binary` (`binary.c:80-117`, `147-182`) do. A digit above 9
/// is an upper-case letter (`Sint_to_buf`, `utils.c:2994-3030`). A first
/// argument that is not an integer, and a base that is not a small integer
/// from 2 to 36, raise `badarg`. [`integer_digits`] gives the digits and
/// charges a bignum.
///
/// # Examples
///
/// ```rust,ignore
/// let binary = integer_to_binary_2(&mut process, &Term::Integer(-255), &Term::Integer(16))?;
/// assert!(matches!(binary, BifResult::Value(Term::Binary(bytes)) if bytes == b"-FF"));
/// assert!(matches!(integer_to_binary_2(&mut process, &Term::Integer(1), &Term::Integer(37))?, BifResult::Raise { .. }));
/// ```
///
/// # Errors
///
/// Return the runtime error of [`integer_digits`].
pub(super) fn integer_to_binary_2(
    process: &mut ProcessState,
    term: &Term,
    base: &Term,
) -> Result<BifResult, BeamError> {
    // ERTS checks both arguments before the work (`binary.c:152-159`). Each
    // failed check raises `badarg` with no other effect, so the order of the
    // checks does not change the result.
    let Some(base) = base
        .small_integer()
        .and_then(|base| u32::try_from(base).ok())
        .filter(|base| (2..=36).contains(base))
    else {
        return Ok(BifResult::badarg());
    };
    let Some(digits) = integer_digits(process, term, base, "integer_to_binary/2")? else {
        return Ok(BifResult::badarg());
    };
    Ok(Value(Term::binary(digits.into_bytes())))
}

/// Return the characters of an integer in `base`, from 2 to 36, for the BIF
/// `erlang:Name`, or `None` for a term that is not an integer. A digit
/// above 9 is an upper-case letter, as `Sint_to_buf` (`utils.c:2994-3030`)
/// writes it. ERTS `integer_to_list` (`bif.c:3266-3311`) and
/// `integer_to_binary` (`binary.c:80-117`) charge the same cost.
///
/// A small integer costs no reduction. A bignum costs one reduction for each
/// 16 digits of [`big_integer_estimate`] (`bif.c:3283-3299`,
/// `binary.c:92-108`). In the PBeam integer range, a bignum is from 2^59 to
/// 2^63 - 1 or from -2^63 to -2^59 - 1. It costs 1 in base 10 and 4 in
/// base 2.
///
/// # Examples
///
/// ```rust,ignore
/// process.fcalls = 10;
/// assert_eq!(integer_digits(&mut process, &Term::Integer(-12), 10, "integer_to_list/1")?.as_deref(), Some("-12"));
/// assert_eq!(integer_digits(&mut process, &Term::Integer(255), 16, "integer_to_binary/2")?.as_deref(), Some("FF"));
/// assert_eq!(process.fcalls, 10);
/// integer_digits(&mut process, &Term::Integer(1 << 59), 10, "integer_to_list/1")?;
/// assert_eq!(process.fcalls, 9);
/// assert_eq!(integer_digits(&mut process, &Term::nil(), 10, "integer_to_list/1")?, None);
/// ```
///
/// # Errors
///
/// Return a runtime error for a bignum that costs more than the reductions
/// left. ERTS then runs the BIF again on a dirty CPU scheduler
/// (`bif.c:3289-3296`, `3323-3336`; `binary.c:98-105`, `129-142`,
/// `163-178`), and PBeam has no dirty scheduler. Also return the runtime
/// error of [`ProcessState::bif_reductions_left`] for a negative budget.
fn integer_digits(
    process: &mut ProcessState,
    term: &Term,
    base: u32,
    name: &str,
) -> Result<Option<String>, BeamError> {
    /// `DIGITS_PER_RED` (`bif.c:3283`, `binary.c:92`).
    const DIGITS_PER_RED: usize = 16;
    let Term::Integer(value) = term else {
        return Ok(None);
    };
    if term.small_integer().is_none() {
        let cost = big_integer_estimate(*value, base) / DIGITS_PER_RED;
        if cost > process.bif_reductions_left()? {
            // ponytail: no dirty scheduler; see "Scheduling" in docs/erts-differences.md.
            return Err(error(format!(
                "unsupported erlang:{name} of {value} with {} reductions left: ERTS runs it on a dirty CPU scheduler",
                process.fcalls
            )));
        }
        process.bump_reductions(cost as i32);
    }
    let mut magnitude = value.unsigned_abs();
    let mut digits = Vec::new();
    loop {
        let digit = (magnitude % u64::from(base)) as u32;
        digits.push(char::from_digit(digit, base).map_or('0', |digit| digit.to_ascii_uppercase()));
        magnitude /= u64::from(base);
        if magnitude == 0 {
            break;
        }
    }
    if *value < 0 {
        digits.push('-');
    }
    Ok(Some(digits.iter().rev().collect()))
}

/// Return the number of digits in `base` that ERTS `big_integer_estimate`
/// (`big.c:2589-2597`) gives for a bignum, with the sign and a null byte:
/// `(lg + 1) / lg2(base) + 1`, rounded down, where `lg` is the index of the
/// highest set bit of the magnitude (`I_lg`, `big.c:2226-2237`) and
/// `lg2(base)` is the rounded value of `lg2_lookup` (`big.c:4087-4092`),
/// then one more for a negative sign and one for the null byte.
///
/// # Examples
///
/// ```rust,ignore
/// // 2^59: lg 59, 60 / 3.32193 + 1 = 19.06, then the null byte.
/// assert_eq!(big_integer_estimate(1 << 59, 10), 20);
/// assert_eq!(big_integer_estimate(i64::MIN, 10), 22);
/// // 2^63 - 1 in base 2: lg 62, 63 / 1.0 + 1 = 64, then the null byte.
/// assert_eq!(big_integer_estimate(i64::MAX, 2), 65);
/// ```
fn big_integer_estimate(value: i64, base: u32) -> usize {
    /// `lg2_lookup` (`big.c:4087-4092`): the rounded `log2` of each base
    /// from 2 to 36.
    #[allow(clippy::approx_constant, reason = "ERTS uses these rounded values")]
    const LG2_LOOKUP: [f64; 35] = [
        1.0, 1.58496, 2.0, 2.32193, 2.58496, 2.80735, 3.0, 3.16993, 3.32193, 3.45943, 3.58496,
        3.70044, 3.80735, 3.90689, 4.0, 4.08746, 4.16993, 4.24793, 4.32193, 4.39232, 4.45943,
        4.52356, 4.58496, 4.64386, 4.70044, 4.75489, 4.80735, 4.85798, 4.90689, 4.9542, 5.0,
        5.04439, 5.08746, 5.12928, 5.16993,
    ];
    let lg = u64::BITS - 1 - value.unsigned_abs().leading_zeros();
    let digits = (f64::from(lg + 1) / LG2_LOOKUP[base as usize - 2] + 1.0) as usize;
    digits + usize::from(value < 0) + 1
}

/// Run `erlang:atom_to_list/1`: return the Unicode code points of the atom
/// name, as ERTS `atom_to_list_1` (`bif.c:3186-3210`) does. The atom `''`
/// gives `[]`. A term that is not an atom raises `badarg`.
///
/// # Examples
///
/// ```rust,ignore
/// let list = atom_to_list(&Term::static_atom("ok"));
/// assert!(matches!(list, BifResult::Value(codes) if codes == Term::list_from([Term::Integer(111), Term::Integer(107)])));
/// assert!(matches!(atom_to_list(&Term::static_atom("")), BifResult::Value(codes) if codes.is_nil()));
/// assert!(matches!(atom_to_list(&Term::nil()), BifResult::Raise { .. }));
/// ```
pub(super) fn atom_to_list(term: &Term) -> BifResult {
    match term {
        Term::Atom(atom) => {
            Value(Term::list_from(atom.name().chars().map(|character| {
                Term::Integer(i64::from(u32::from(character)))
            })))
        }
        _ => BifResult::badarg(),
    }
}

/// Run `erlang:atom_to_binary/2`: return the text of an atom as a binary,
/// as ERTS `atom_to_binary_2` (`erl_unicode.c:1928-1960`) does. With
/// `utf8` or `unicode`, the binary holds the UTF-8 bytes of the name. With
/// `latin1`, it holds one byte for each character. A term that is not an
/// atom, an encoding that is not `latin1`, `utf8`, or `unicode`, and
/// `latin1` for a name with a character above 255 raise `badarg`.
///
/// # Examples
///
/// ```rust,ignore
/// let pi = Term::Atom(atoms.put("\u{3c0}")?);
/// assert!(matches!(atom_to_binary(&pi, &Term::static_atom("utf8")), BifResult::Value(Term::Binary(bytes)) if bytes == [207, 128]));
/// assert!(matches!(atom_to_binary(&pi, &Term::static_atom("latin1")), BifResult::Raise { .. }));
/// let e_acute = Term::Atom(atoms.put("\u{e9}")?);
/// assert!(matches!(atom_to_binary(&e_acute, &Term::static_atom("latin1")), BifResult::Value(Term::Binary(bytes)) if bytes == [233]));
/// assert!(matches!(atom_to_binary(&Term::static_atom("ok"), &Term::static_atom("foo")), BifResult::Raise { .. }));
/// ```
pub(super) fn atom_to_binary(term: &Term, encoding: &Term) -> BifResult {
    let (Term::Atom(atom), Term::Atom(encoding)) = (term, encoding) else {
        return BifResult::badarg();
    };
    match encoding.name() {
        "utf8" | "unicode" => Value(Term::binary(atom.name().as_bytes().to_vec())),
        "latin1" => atom
            .name()
            .chars()
            .map(|character| u8::try_from(u32::from(character)).ok())
            .collect::<Option<Vec<u8>>>()
            .map_or_else(BifResult::badarg, |bytes| Value(Term::binary(bytes))),
        _ => BifResult::badarg(),
    }
}

/// Run `erlang:list_to_atom/1`: return the atom whose name has the code
/// points of a list, and put a new atom into the atom table of the run, as
/// ERTS `list_to_atom_1` (`bif.c:3217-3236`) does with
/// `erts_unicode_list_to_buf` (`utils.c:2862-2942`). ERTS counts each
/// element before it checks it, so a 256th element raises `system_limit`,
/// also when it is not a character. ERTS checks the tail after the 255th
/// element, so an improper list of 300 characters raises `system_limit`.
/// An element that is not an integer in `0..=0x10FFFF` outside
/// `0xD800..=0xDFFF`, a tail that is not `[]`, and a term that is not a
/// list, raise `badarg`.
///
/// # Examples
///
/// ```rust,ignore
/// let mut atoms = AtomTable::new();
/// let ok = list_to_atom(&mut atoms, &Term::list_from(vec![Term::Integer(111), Term::Integer(107)]))?;
/// assert!(matches!(ok, BifResult::Value(atom) if atom == Term::static_atom("ok")));
/// let surrogate = Term::list_from(vec![Term::Integer(0xD800)]);
/// assert!(matches!(list_to_atom(&mut atoms, &surrogate)?, BifResult::Raise { .. }));
/// let long = Term::list_from(vec![Term::Integer(97); 256]);
/// assert!(matches!(list_to_atom(&mut atoms, &long)?, BifResult::Raise { .. }));
/// let improper = Term::list_with_tail(vec![Term::Integer(97)], Term::static_atom("x"));
/// assert!(matches!(list_to_atom(&mut atoms, &improper)?, BifResult::Raise { .. }));
/// ```
///
/// # Errors
///
/// A new atom in a full atom table gives the error of `AtomTable::put`,
/// where ERTS stops the emulator.
pub(super) fn list_to_atom(atoms: &mut AtomTable, term: &Term) -> Result<BifResult, BeamError> {
    let Some(mut elements) = term.list_iter() else {
        return Ok(BifResult::badarg());
    };
    let mut name = String::new();
    for (count, element) in elements.by_ref().enumerate() {
        if count == MAX_ATOM_CHARACTERS {
            return Ok(Raise {
                class: ExceptionClass::Error,
                reason: Term::static_atom("system_limit"),
            });
        }
        let character = match element {
            Term::Integer(code) => u32::try_from(*code).ok().and_then(char::from_u32),
            _ => None,
        };
        let Some(character) = character else {
            return Ok(BifResult::badarg());
        };
        name.push(character);
    }
    if !elements.rest().is_nil() {
        return Ok(BifResult::badarg());
    }
    Ok(Value(Term::Atom(atoms.put(&name)?)))
}

/// Run `erlang:system_info/1`. The item `os_type` returns `{unix, Flavor}`,
/// as ERTS `system_info_1` (`erl_bif_info.c:3026-3027`) does with `os_type`
/// `"unix"` (`sys/unix/sys.c:835`) and the OS flavor of the host. ERTS makes
/// both atoms at emulator start (`os_info_init` in
/// `erl_bif_info.c:6418-6440`); PBeam puts them into the atom table of the
/// run at this call.
///
/// # Examples
///
/// ```rust,ignore
/// let mut atoms = AtomTable::new();
/// let os_type = system_info(&mut atoms, &Term::static_atom("os_type"))?;
/// // macOS: {unix, darwin}.
/// assert!(matches!(os_type, BifResult::Value(Term::Tuple(_))));
/// assert!(system_info(&mut atoms, &Term::static_atom("otp_release")).is_err());
/// ```
///
/// # Errors
///
/// Each other item gives a runtime error, also an item that ERTS rejects
/// with `badarg`. A new atom in a full atom table gives the error of
/// `AtomTable::put`.
pub(super) fn system_info(atoms: &mut AtomTable, item: &Term) -> Result<BifResult, BeamError> {
    match item {
        Term::Atom(atom) if atom.name() == "os_type" => Ok(Value(Term::tuple(vec![
            Term::Atom(atoms.put("unix")?),
            Term::Atom(atoms.put(&host::os_flavor())?),
        ]))),
        _ => Err(error(format!(
            "unsupported erlang:system_info/1 argument {item:?}"
        ))),
    }
}

/// Run `erlang:'+'/2`, `erlang:'-'/2`, `erlang:'*'/2`, `erlang:'div'/2`, or
/// `erlang:'rem'/2` with `operation`, the checked `i64` sum, difference,
/// product, or quotient, or the remainder, as ERTS `erts_mixed_plus`,
/// `erts_mixed_minus`, `stimes_2`, `intdiv_2`, and `rem_2`
/// (`erl_arith.c:370-389`, `erl_arith.c:585-602`, `erl_arith.c:117-143`,
/// `erl_arith.c:145-161`) do for two integers. The JIT runs `*` with
/// `emit_i_mul_add`, `div` with `emit_i_int_div` and `emit_div_rem`, and
/// `rem` with `emit_i_rem` (`jit/arm/instr_arith.cpp:542`,
/// `jit/arm/instr_arith.cpp:968`, `jit/arm/instr_arith.cpp:1067`,
/// `jit/arm/instr_arith.cpp:1078`) with the same results. An operand that is
/// not a number raises `badarith`. PBeam has no float term, so each number
/// is an integer. A result outside the `i64` range gives a runtime error,
/// where ERTS makes a bignum. The `rem` operation never returns `None`, so
/// `rem` never gives this error (see [`int_rem`]).
///
/// # Examples
///
/// ```rust,ignore
/// let sum = arith("+", &Term::Integer(1), &Term::Integer(2), i64::checked_add)?;
/// assert!(matches!(sum, BifResult::Value(Term::Integer(3))));
/// let product = arith("*", &Term::Integer(-3), &Term::Integer(4), i64::checked_mul)?;
/// assert!(matches!(product, BifResult::Value(Term::Integer(-12))));
/// let atom = Term::static_atom("a");
/// assert!(matches!(arith("-", &atom, &Term::Integer(1), i64::checked_sub)?, BifResult::Raise { .. }));
/// assert!(arith("+", &Term::Integer(i64::MAX), &Term::Integer(1), i64::checked_add).is_err());
/// ```
pub(super) fn arith(
    name: &str,
    left: &Term,
    right: &Term,
    operation: fn(i64, i64) -> Option<i64>,
) -> Result<BifResult, BeamError> {
    let (Term::Integer(left), Term::Integer(right)) = (left, right) else {
        return Ok(BifResult::badarith());
    };
    // ponytail: a result outside i64 needs a bignum term; see "Integer range" in docs/erts-differences.md.
    let result = operation(*left, *right).ok_or_else(|| {
        error(format!(
            "unsupported erlang:{name}/2 result of {left} and {right}: needs a bignum"
        ))
    })?;
    Ok(Value(Term::Integer(result)))
}

/// Run `erlang:'div'/2`: return the integer quotient of `left` and `right`,
/// truncated toward zero, as ERTS `intdiv_2` (`erl_arith.c:127-143`) does.
/// A `right` of `0` raises `badarith` before the operand types are checked
/// (`erl_arith.c:131-133`), so `a div 0` raises `badarith` too. Each other
/// case runs as [`arith`].
///
/// # Examples
///
/// ```rust,ignore
/// let quotient = int_div(&Term::Integer(-7), &Term::Integer(2))?;
/// assert!(matches!(quotient, BifResult::Value(Term::Integer(-3))));
/// assert!(matches!(int_div(&Term::Integer(1), &Term::Integer(0))?, BifResult::Raise { .. }));
/// assert!(int_div(&Term::Integer(i64::MIN), &Term::Integer(-1)).is_err());
/// ```
pub(super) fn int_div(left: &Term, right: &Term) -> Result<BifResult, BeamError> {
    if *right == Term::Integer(0) {
        return Ok(BifResult::badarith());
    }
    arith("div", left, right, i64::checked_div)
}

/// Run `erlang:'rem'/2`: return the integer remainder of `left` and `right`,
/// with the sign of `left`, as ERTS `rem_2` (`erl_arith.c:145-161`) and the
/// JIT `emit_i_rem` (`jit/arm/instr_arith.cpp:1078-1087`) do. A `right` of
/// `0` raises `badarith` before the operand types are checked
/// (`erl_arith.c:149-151`), so `a rem 0` raises `badarith` too. Each other
/// case runs as [`arith`]. `i64::MIN rem -1` gives `0`, as the bignum path
/// `erts_int_div_rem` (`erl_arith.c:1172`) does, so no result leaves the
/// `i64` range.
///
/// # Examples
///
/// ```rust,ignore
/// let remainder = int_rem(&Term::Integer(-7), &Term::Integer(3))?;
/// assert!(matches!(remainder, BifResult::Value(Term::Integer(-1))));
/// let remainder = int_rem(&Term::Integer(i64::MIN), &Term::Integer(-1))?;
/// assert!(matches!(remainder, BifResult::Value(Term::Integer(0))));
/// assert!(matches!(int_rem(&Term::static_atom("a"), &Term::Integer(0))?, BifResult::Raise { .. }));
/// ```
pub(super) fn int_rem(left: &Term, right: &Term) -> Result<BifResult, BeamError> {
    if *right == Term::Integer(0) {
        return Ok(BifResult::badarith());
    }
    arith("rem", left, right, |left, right| {
        Some(left.wrapping_rem(right))
    })
}

/// The reductions that ERTS `bump_reds_linear` (`erl_arith.c:56-72`) uses
/// for a bignum operation in the PBeam integer range: a bignum of one
/// 64-bit digit needs fewer than `ERTS_ARITH_WORDS_PER_REDUCTION` (16,
/// `global.h:1041`) heap words, so it costs `0 / 16 + 1`.
const BIGNUM_OPERATION_REDUCTIONS: i32 = 1;

/// Run `erlang:'-'/1`: return the negation of an integer, as ERTS
/// `sminus_1` and `erts_unary_minus` (`erl_arith.c:107-110`,
/// `erl_arith.c:509-567`) do. The JIT `gc_bif` `-`/1 calls the same
/// `erts_unary_minus` for each operand that it does not negate inline
/// (`jit/arm/instr_arith.cpp:224`, `280`). A small integer costs no
/// reduction, also when its negation is the bignum 2^59. A bignum operand
/// costs 1 reduction (`erl_arith.c:537-551`). An operand that is not a
/// number raises `badarith`. PBeam has no float term.
///
/// # Examples
///
/// ```rust,ignore
/// process.fcalls = 10;
/// assert!(matches!(negate(&mut process, &Term::Integer(5))?, BifResult::Value(Term::Integer(-5))));
/// assert_eq!(process.fcalls, 10);
/// negate(&mut process, &Term::Integer(1 << 59))?;
/// assert_eq!(process.fcalls, 9);
/// assert!(matches!(negate(&mut process, &Term::static_atom("a"))?, BifResult::Raise { .. }));
/// assert!(negate(&mut process, &Term::Integer(i64::MIN)).is_err());
/// ```
///
/// # Errors
///
/// Return a runtime error for `-(2^63)`, whose negation is outside the
/// `i64` range, where ERTS makes a bignum.
pub(super) fn negate(process: &mut ProcessState, term: &Term) -> Result<BifResult, BeamError> {
    let Term::Integer(value) = term else {
        return Ok(BifResult::badarith());
    };
    // ponytail: a result outside i64 needs a bignum term; see "Integer range" in docs/erts-differences.md.
    let result = value.checked_neg().ok_or_else(|| {
        error(format!(
            "unsupported erlang:-/1 result of {value}: needs a bignum"
        ))
    })?;
    if term.small_integer().is_none() {
        process.bump_reductions(BIGNUM_OPERATION_REDUCTIONS);
    }
    Ok(Value(Term::Integer(result)))
}

/// Run `erlang:'band'/2`: return the bitwise and of two integers, as ERTS
/// `band_2` and `erts_band` (`erl_arith.c:164-170`,
/// `erl_arith.c:1252-1278`) do. The JIT `gc_bif` `band` calls the same
/// `erts_band` for operands that are not both small integers
/// (`jit/arm/instr_arith.cpp:1168`, `1236`). Two small integers cost no
/// reduction. A bignum operand costs 1 reduction, also when the result is
/// a small integer (`erl_arith.c:1276`). An operand that is not an integer
/// raises `badarith`. The result of two `i64` values is an `i64` value.
///
/// # Examples
///
/// ```rust,ignore
/// process.fcalls = 10;
/// let result = band(&mut process, &Term::Integer(12), &Term::Integer(10));
/// assert!(matches!(result, BifResult::Value(Term::Integer(8))));
/// assert_eq!(process.fcalls, 10);
/// band(&mut process, &Term::Integer(1 << 59), &Term::Integer(-1));
/// assert_eq!(process.fcalls, 9);
/// assert!(matches!(band(&mut process, &Term::Integer(1), &Term::static_atom("a")), BifResult::Raise { .. }));
/// ```
pub(super) fn band(process: &mut ProcessState, left: &Term, right: &Term) -> BifResult {
    let (Term::Integer(left_value), Term::Integer(right_value)) = (left, right) else {
        return BifResult::badarith();
    };
    if left.small_integer().is_none() || right.small_integer().is_none() {
        process.bump_reductions(BIGNUM_OPERATION_REDUCTIONS);
    }
    Value(Term::Integer(left_value & right_value))
}

/// Convert `value`, a time in the native unit, to `unit` with
/// [`time::time_unit_conversion`], as `erlang:monotonic_time/1` and
/// `os:system_time/1` do (`erl_time_sup.c:2268-2350`). A converted time
/// is the BIF result, and a unit that is not valid raises `badarg`. A trap
/// calls `erlang:convert_time_unit(Value, 1000000000, Unit)`
/// (`erts_convert_time_unit_trap`, `bif.c:5557`), which the pinned
/// `erlang.beam` runs (`erlang.erl:5299-5340`).
///
/// # Examples
///
/// ```rust,ignore
/// let result = convert_time(1_500_000_000, 0, &Term::static_atom("second"));
/// assert!(matches!(result, BifResult::Value(Term::Integer(1))));
/// let result = convert_time(1_000_000_000, 1, &Term::static_atom("perf_counter"));
/// assert!(matches!(result, BifResult::Trap { function: "convert_time_unit", .. }));
/// assert!(matches!(convert_time(1, 0, &Term::Integer(0)), BifResult::Raise { .. }));
/// ```
pub(super) fn convert_time(value: i64, offset_multiplier: i64, unit: &Term) -> BifResult {
    match time::time_unit_conversion(value, offset_multiplier, unit) {
        TimeValue::Value(value) => Value(Term::Integer(value)),
        TimeValue::Trap(native) => Trap {
            module: "erlang",
            function: "convert_time_unit",
            arguments: vec![
                Term::Integer(native),
                Term::Integer(time::TIME_UNIT),
                unit.clone(),
            ],
        },
        TimeValue::Badarg => BifResult::badarg(),
    }
}

/// Run `erlang:list_to_tuple(List)`: return a tuple with the elements of
/// the proper list `List`, as `list_to_tuple_1` (`bif.c:3789-3812`) does.
/// `[]` gives `{}`. A term that is not a proper list, and a list with more
/// than [`MAX_TUPLE_SIZE`] elements, raise `badarg`. The BIF uses no
/// reduction.
///
/// # Examples
///
/// ```rust,ignore
/// let list = Term::list_from(vec![Term::static_atom("a"), Term::Integer(1)]);
/// assert!(matches!(list_to_tuple(&list), BifResult::Value(Term::Tuple(elements)) if elements.len() == 2));
/// assert!(matches!(list_to_tuple(&Term::static_atom("a")), BifResult::Raise { .. }));
/// ```
pub(super) fn list_to_tuple(list: &Term) -> BifResult {
    match list.proper_list() {
        Some(elements) if elements.clone().count() <= MAX_TUPLE_SIZE => {
            Value(Term::tuple(elements.cloned().collect()))
        }
        _ => BifResult::badarg(),
    }
}

/// Run `erlang:tuple_to_list(Tuple)`: return a proper list with the
/// elements of `Tuple` in order, as `tuple_to_list_1` (`bif.c:3762-3783`)
/// does. `{}` gives `[]`. A term that is not a tuple raises `badarg`. The
/// BIF uses no reduction.
///
/// # Examples
///
/// ```rust,ignore
/// let tuple = Term::tuple(vec![Term::static_atom("ok"), Term::Integer(1)]);
/// assert!(matches!(tuple_to_list(&tuple), BifResult::Value(list) if list.proper_list().is_some_and(|elements| elements.len() == 2)));
/// assert!(matches!(tuple_to_list(&Term::tuple(vec![])), BifResult::Value(list) if list.is_nil()));
/// assert!(matches!(tuple_to_list(&Term::nil()), BifResult::Raise { .. }));
/// ```
pub(super) fn tuple_to_list(tuple: &Term) -> BifResult {
    match tuple {
        Term::Tuple(elements) => Value(Term::list_from(elements.iter().cloned())),
        _ => BifResult::badarg(),
    }
}

/// Run `erlang:posixtime_to_universaltime/1`: return the UTC date and time
/// `{{Year, Month, Day}, {Hour, Minute, Second}}` of `time`, the seconds
/// since 1970-01-01 00:00:00 UTC, with [`time::seconds_to_univ`]. Each
/// integer gives a date, and each other term raises `badarg`, as in ERTS
/// `posixtime_to_universaltime_1` (`bif.c:4025-4050`).
///
/// # Examples
///
/// ```rust,ignore
/// // {{1970,1,2},{0,0,0}}
/// assert!(matches!(posixtime_to_universaltime(&Term::Integer(86_400)), BifResult::Value(_)));
/// assert!(matches!(posixtime_to_universaltime(&Term::nil()), BifResult::Raise { .. }));
/// ```
pub(super) fn posixtime_to_universaltime(time: &Term) -> BifResult {
    let Term::Integer(time) = time else {
        return BifResult::badarg();
    };
    Value(date_time(time::seconds_to_univ(*time)))
}

/// Run `erlang:universaltime_to_localtime/1`: return the local date and time
/// of a UTC date and time `{{Year, Month, Day}, {Hour, Minute, Second}}` in
/// the host time zone, as ERTS `universaltime_to_localtime_1`
/// (`bif.c:3975-3995`) does. A term that [`time::time_to_parts`] does not
/// read, a value that [`time::univ_to_clock`] does not accept, and a
/// failed `localtime_r` ([`host::clock::local_time`]) raise `badarg`.
///
/// # Examples
///
/// ```rust,ignore
/// let triple = |a, b, c| Term::tuple(vec![Term::Integer(a), Term::Integer(b), Term::Integer(c)]);
/// // TZ=EST5EDT,M3.2.0,M11.1.0: {{2026,1,15},{7,0,0}}
/// let date = Term::tuple(vec![triple(2026, 1, 15), triple(12, 0, 0)]);
/// assert!(matches!(universaltime_to_localtime(&date), BifResult::Value(_)));
/// let date = Term::tuple(vec![triple(1901, 12, 31), triple(23, 59, 59)]);
/// assert!(matches!(universaltime_to_localtime(&date), BifResult::Raise { .. }));
/// ```
pub(super) fn universaltime_to_localtime(date: &Term) -> BifResult {
    time::time_to_parts(date)
        .and_then(time::univ_to_clock)
        .and_then(host::clock::local_time)
        .map_or_else(BifResult::badarg, |local| Value(date_time(local)))
}

/// Return the term `{{Year, Month, Day}, {Hour, Minute, Second}}` of a date
/// and time.
///
/// # Examples
///
/// ```rust,ignore
/// // {{1970,1,2},{0,0,0}}
/// let term = date_time((1970, 1, 2, 0, 0, 0));
/// ```
fn date_time((year, month, day, hour, minute, second): time::DateTime) -> Term {
    Term::tuple(vec![
        Term::tuple([year, month, day].map(Term::Integer).to_vec()),
        Term::tuple([hour, minute, second].map(Term::Integer).to_vec()),
    ])
}

/// Run `erlang:element/2`: return the element at the 1-based `index` of a
/// tuple. An `index` that is not an integer or is outside the tuple, and a
/// `tuple` that is not a tuple, raise `badarg`, as in ERTS `element_2`
/// (`bif.c`).
///
/// # Examples
///
/// ```rust,ignore
/// let tuple = Term::tuple(vec![Term::static_atom("ok"), Term::Integer(1)]);
/// assert!(matches!(element(&Term::Integer(2), &tuple), BifResult::Value(Term::Integer(1))));
/// assert!(matches!(element(&Term::Integer(3), &tuple), BifResult::Raise { .. }));
/// assert!(matches!(element(&Term::Integer(1), &Term::nil()), BifResult::Raise { .. }));
/// ```
pub(super) fn element(index: &Term, tuple: &Term) -> BifResult {
    let (Term::Integer(index), Term::Tuple(elements)) = (index, tuple) else {
        return BifResult::badarg();
    };
    usize::try_from(*index)
        .ok()
        .and_then(|index| index.checked_sub(1))
        .and_then(|index| elements.get(index))
        .map_or_else(BifResult::badarg, |element| Value(element.clone()))
}

/// Run `erlang:is_function/2`: return `true` when `term` is a fun of
/// `arity` arguments, as ERTS `erl_is_function` (`erl_bif_op.c:237-267`)
/// does. An `arity` that is negative or not an integer raises `badarg`. A
/// non-negative arity that no fun has gives `false`. The `is_function2`
/// instruction runs the same check and passes only when it returns `true`
/// (`jit/arm/instr_common.cpp:1266-1283`).
///
/// # Examples
///
/// ```rust,ignore
/// // fun(X) -> {Tag, X} end
/// assert!(matches!(is_function(&pair, &Term::Integer(1)), BifResult::Value(value) if value == Term::static_atom("true")));
/// assert!(matches!(is_function(&pair, &Term::Integer(0)), BifResult::Value(value) if value == Term::static_atom("false")));
/// assert!(matches!(is_function(&Term::static_atom("notfun"), &Term::Integer(-1)), BifResult::Raise { .. }));
/// ```
pub(in crate::runtime::interpreter) fn is_function(term: &Term, arity: &Term) -> BifResult {
    let &Term::Integer(arity) = arity else {
        return BifResult::badarg();
    };
    if arity < 0 {
        return BifResult::badarg();
    }
    let matches = matches!(term, Term::Fun(fun) if i64::from(fun.target().arity()) == arity);
    Value(Term::static_atom(if matches { "true" } else { "false" }))
}

/// Send `message` to `to` and return the message, as ERTS `erl_send`
/// (`bif.c:2845-2908`) does for the `send` instruction and for
/// `erlang:send/2` (`send_2`, `bif.c:2801-2804`). A local send then uses
/// reductions, and can return the message after a switch (see
/// [`local_send_result`]). A `to` that [`deliver_to`] rejects raises
/// `badarg`, and a caller without the `send` operation raises
/// `error:capability_denied`.
///
/// # Examples
///
/// ```rust,ignore
/// assert!(matches!(send(scheduler, &Term::Pid(pid), ok.clone())?, BifResult::Value(value) if value == ok));
/// assert!(matches!(send(scheduler, &Term::static_atom("nobody"), ok.clone())?, BifResult::Raise { .. }));
/// let remote = Term::tuple(vec![Term::static_atom("name"), Term::static_atom("a@b")]);
/// assert!(matches!(send(scheduler, &remote, ok.clone())?, BifResult::Value(_)));
/// ```
///
/// # Errors
///
/// A port as `to` returns a runtime error after the capability check: ERTS
/// runs the port command of the message (`erl_send`, `bif.c:2872-2899`). A
/// new `capability_denied` atom in a full atom table gives the error of
/// `AtomTable::put`.
pub(in crate::runtime::interpreter) fn send(
    scheduler: &mut Scheduler,
    to: &Term,
    message: Term,
) -> Result<BifResult, BeamError> {
    Ok(
        match deliver_to(scheduler, to, message.clone(), "erlang:send/2")? {
            Ok(Sent::Local) => local_send_result(scheduler, message),
            Ok(Sent::NoConnect) => Value(message),
            Err(raise) => raise,
        },
    )
}

/// Run `erlang:send(To, Message, Options)` as ERTS `send_3`
/// (`bif.c:2697-2799`) does. The options come first: a proper list of
/// `noconnect`, `nosuspend`, and `priority`. Each other element, and a tail
/// that is not `[]`, raise `badarg`, and nothing is sent. Then the message
/// goes to `to` as [`deliver_to`] gives it, and a `to` that it rejects
/// raises `badarg`. A caller without the `send` operation raises
/// `error:capability_denied`. The result is `noconnect` for `{Name, Node}` on another
/// node with `noconnect`, and `ok` for each other send. A local send uses
/// reductions, and can return `ok` after a switch (see
/// [`local_send_result`]).
///
/// PBeam has no distribution and sends no message to a port, so
/// `nosuspend` changes nothing.
/// A `priority` message is an ordinary message unless its receiver gets it
/// through a priority alias (`erl_proc_sig_queue.c:6021-6045`). Only
/// `erlang:alias/1` makes a priority alias, so `priority` also changes
/// nothing. ERTS gives the `badarg` of an option the `error_info` cause
/// `badopt`, which PBeam does not model.
///
/// # Examples
///
/// ```rust,ignore
/// let noconnect = Term::list_from(vec![Term::static_atom("noconnect")]);
/// assert!(matches!(send3(scheduler, &Term::Pid(pid), ok.clone(), &noconnect), BifResult::Value(Term::Atom(atom)) if atom.name() == "ok"));
/// let remote = Term::tuple(vec![Term::static_atom("name"), Term::static_atom("a@b")]);
/// assert!(matches!(send3(scheduler, &remote, ok.clone(), &noconnect), BifResult::Value(Term::Atom(atom)) if atom.name() == "noconnect"));
/// assert!(matches!(send3(scheduler, &Term::Pid(pid), ok, &Term::static_atom("x"))?, BifResult::Raise { .. }));
/// ```
///
/// # Errors
///
/// A port as `to` after valid options and the capability check returns a
/// runtime error: ERTS runs the port command of the message (`send_3`,
/// `bif.c:2733-2739`). A new `capability_denied` atom in a full atom table
/// gives the error of `AtomTable::put`.
pub(super) fn send3(
    scheduler: &mut Scheduler,
    to: &Term,
    message: &Term,
    options: &Term,
) -> Result<BifResult, BeamError> {
    let Some(options) = options.proper_list() else {
        return Ok(BifResult::badarg());
    };
    let mut connect = true;
    for option in options {
        match option {
            Term::Atom(option) if option.name() == "noconnect" => connect = false,
            Term::Atom(option) if matches!(option.name(), "nosuspend" | "priority") => {}
            _ => return Ok(BifResult::badarg()),
        }
    }
    Ok(
        match deliver_to(scheduler, to, message.clone(), "erlang:send/3")? {
            Ok(Sent::Local) => local_send_result(scheduler, Term::static_atom("ok")),
            Ok(Sent::NoConnect) if !connect => Value(Term::static_atom("noconnect")),
            Ok(Sent::NoConnect) => Value(Term::static_atom("ok")),
            Err(raise) => raise,
        },
    )
}

/// The reductions of a local send: `erl_send` and `send_3` use them with
/// `ERTS_VBUMP_REDS` after `do_send` gives a result that is 0 or more
/// (`bif.c:2861-2866`, `2741-2746`). A send to another node gives
/// `SEND_NOCONNECT` and uses none.
const SEND_REDUCTIONS: i32 = 4;

/// Use [`SEND_REDUCTIONS`] of the budget of the caller after a local send.
/// Return `value`, or return it after a switch when the budget is then 0,
/// as `ERTS_IS_PROC_OUT_OF_REDS` and `ERTS_BIF_PREP_YIELD_RETURN` do
/// (`bif.c:2863-2864`, `2876-2879`; `bif.h:69`, `445-452`).
///
/// # Examples
///
/// ```rust,ignore
/// scheduler.current_process_mut().fcalls = 5;
/// assert!(matches!(local_send_result(scheduler, ok.clone()), BifResult::Value(_)));
/// assert!(matches!(local_send_result(scheduler, ok), BifResult::YieldReturn(_)));
/// ```
fn local_send_result(scheduler: &mut Scheduler, value: Term) -> BifResult {
    scheduler.vbump_reductions(SEND_REDUCTIONS);
    if scheduler.current_process_mut().fcalls == 0 {
        BifResult::YieldReturn(value)
    } else {
        Value(value)
    }
}

/// Run `erlang:bump_reductions/1`: use `reductions` of the budget of the
/// caller, but not more than `CONTEXT_REDS`, and return `true`, as
/// `bump_reductions_1` (`bif.c:5403-5415`) does with `BIF_RET2`
/// (`bif.h:71-78`, `221-224`). The budget does not go below 0, and the
/// process switches out at the next cost point. A value that is not a small
/// integer, or a negative value, raises `badarg`.
///
/// # Examples
///
/// ```rust,ignore
/// scheduler.current_process_mut().fcalls = 4000;
/// assert!(matches!(bump_reductions(scheduler, &Term::Integer(10)), BifResult::Value(_)));
/// assert_eq!(scheduler.current_process_mut().fcalls, 3990);
/// assert!(matches!(bump_reductions(scheduler, &Term::Integer(-1)), BifResult::Raise { .. }));
/// ```
pub(super) fn bump_reductions(scheduler: &mut Scheduler, reductions: &Term) -> BifResult {
    match reductions.small_integer() {
        Some(value) if value >= 0 => {
            let value = value.min(i64::from(CONTEXT_REDS)) as i32;
            scheduler.current_process_mut().bump_reductions(value);
            Value(Term::static_atom("true"))
        }
        _ => BifResult::badarg(),
    }
}

/// The result of a send that does not raise, as ERTS `do_send`
/// (`bif.c:2458-2695`) gives it to `erl_send` and `send_3`.
enum Sent {
    /// The message went to a local process, or a local send dropped it.
    Local,
    /// The message was to `{Name, Node}` on another node, and was dropped
    /// (`SEND_NOCONNECT`).
    NoConnect,
}

/// The destination of a send, as ERTS `do_send` (`bif.c:2458-2695`) reads
/// it before it looks up a name or a process.
enum Destination<'a> {
    /// A local pid.
    Pid(ProcessId),
    /// A reference, which can be an alias.
    Alias(Reference),
    /// A registered name. A name that no process has raises `badarg`.
    Name(&'a str),
    /// `{Name, Node}` with the local node. A name that no process has drops
    /// the message.
    NodeName(&'a str),
    /// `{Name, Node}` with another node.
    Remote,
    /// A port.
    Port,
}

/// Return the destination of `to`, or `None` for a term that `do_send`
/// rejects with `SEND_BADARG` before any lookup: a term that is not a pid,
/// a reference, an atom, a port, or a 2-tuple of atoms
/// (`bif.c:2614-2621`, `2668-2674`).
///
/// # Examples
///
/// ```rust,ignore
/// assert!(matches!(destination(&Term::Pid(pid)), Some(Destination::Pid(_))));
/// let remote = Term::tuple(vec![Term::static_atom("name"), Term::static_atom("a@b")]);
/// assert!(matches!(destination(&remote), Some(Destination::Remote)));
/// assert!(destination(&Term::Integer(1)).is_none());
/// ```
fn destination(to: &Term) -> Option<Destination<'_>> {
    Some(match to {
        Term::Pid(pid) => Destination::Pid(*pid),
        Term::Reference(reference) => Destination::Alias(*reference),
        Term::Atom(name) => Destination::Name(name.name()),
        Term::Port(_) => Destination::Port,
        Term::Tuple(elements) => match elements.as_ref() {
            [Term::Atom(name), node @ Term::Atom(_)] if *node == local_node() => {
                Destination::NodeName(name.name())
            }
            [Term::Atom(_), Term::Atom(_)] => Destination::Remote,
            _ => return None,
        },
        _ => return None,
    })
}

/// Give `message` to the process that `to` names, as ERTS `do_send`
/// (`bif.c:2458-2695`) does, for `bif`, the BIF name of a runtime error.
/// This is the one send path of the `send` instruction, `erlang:send/2`,
/// and `erlang:send/3`. Return `Err` with the exception to raise.
///
/// - A `to` that [`destination`] rejects raises `badarg`.
/// - Then the caller needs a `process` capability that permits `send`.
///   Without it, the send raises `error:capability_denied` and sends
///   nothing. The check comes before the name lookup, so a denied send
///   gives the same result for each name.
/// - After the lookup, the runtime to boot gate denies a message to a boot
///   process, by its pid or its name, in the same way, unless the boot
///   process is an IO service and the message is an IO request that the
///   caller may make (see `io_service::send_denial`). A message to an alias
///   has no gate: the owner gave the alias as a reply channel, for example
///   for the reply of `gen:call/4`.
/// - A pid gets the message from the caller (see
///   [`Scheduler::send_message`]). ERTS drops a message to a process that
///   does not exist or exits.
/// - A reference sends the message as a signal to the owner of the alias,
///   which drops it when the alias is not active
///   (`erl_proc_sig_queue.c:6009-6075`; see
///   [`Scheduler::send_alias_message`]).
/// - An atom gives the message to its registered process. A name that no
///   process has raises `badarg`. PBeam registers no port.
/// - `{Name, Node}` with the local node gives the message to the registered
///   process, or drops it when no process has the name. Another node drops
///   the message and gives [`Sent::NoConnect`], as ERTS does on a node that
///   is not alive (`remote_send`, `bif.c:2397-2400`).
///
/// # Examples
///
/// ```rust,ignore
/// assert!(matches!(deliver_to(scheduler, &Term::Pid(pid), ok.clone(), "erlang:send/2")?, Ok(Sent::Local)));
/// let remote = Term::tuple(vec![Term::static_atom("name"), Term::static_atom("a@b")]);
/// assert!(matches!(deliver_to(scheduler, &remote, ok.clone(), "erlang:send/2")?, Ok(Sent::NoConnect)));
/// assert!(deliver_to(scheduler, &Term::static_atom("nobody"), ok, "erlang:send/2")?.is_err());
/// ```
///
/// # Errors
///
/// A port after the check returns a runtime error that names `bif`. A new
/// `capability_denied` atom in a full atom table gives the error of
/// `AtomTable::put`.
fn deliver_to(
    scheduler: &mut Scheduler,
    to: &Term,
    message: Term,
    bif: &str,
) -> Result<Result<Sent, BifResult>, BeamError> {
    let Some(destination) = destination(to) else {
        return Ok(Err(BifResult::badarg()));
    };
    if let Some(denied) = denial(scheduler, CapabilityKind::Process, Operation::Send, None)? {
        return Ok(Err(denied));
    }
    let target = match destination {
        Destination::Pid(pid) => Some(pid),
        Destination::Alias(reference) => {
            scheduler.send_alias_message(reference, message);
            return Ok(Ok(Sent::Local));
        }
        Destination::Name(name) => match scheduler.whereis(name) {
            Some(target) => Some(target),
            None => return Ok(Err(BifResult::badarg())),
        },
        Destination::NodeName(name) => scheduler.whereis(name),
        Destination::Remote => return Ok(Ok(Sent::NoConnect)),
        Destination::Port => return Err(unsupported_port(bif)),
    };
    if let Some(target) = target {
        if let Some(denied) = io_service::send_denial(scheduler, target, &message)? {
            return Ok(Err(denied));
        }
        let caller = scheduler.current_process_mut().id;
        scheduler.send_message(caller, target, message);
    }
    Ok(Ok(Sent::Local))
}

/// Run `erlang:unregister(Name)`: remove the registered name `Name` and
/// return `true`, as ERTS `unregister_1` (`bif.c:2324-2335`) does through
/// `Scheduler::unregister`. A `Name` that is not an atom, and a name that no
/// process has, raise `badarg`. The entry is `ambient`, but the runtime to
/// boot gate denies the name of a boot process (see `boot_denial`): the
/// call raises `error:capability_denied` and the name stays.
///
/// # Examples
///
/// ```rust,ignore
/// let result = unregister(scheduler, &Term::static_atom("worker"))?;
/// assert!(matches!(result, BifResult::Value(Term::Atom(atom)) if atom.name() == "true"));
/// ```
///
/// # Errors
///
/// A new `capability_denied` atom in a full atom table gives the error of
/// `AtomTable::put`.
pub(super) fn unregister(scheduler: &mut Scheduler, name: &Term) -> Result<BifResult, BeamError> {
    if let Term::Atom(atom) = name
        && let Some(owner) = scheduler.whereis(atom.name())
        && let Some(denied) = boot_denial(scheduler, owner)?
    {
        return Ok(denied);
    }
    Ok(if scheduler.unregister(name) {
        Value(Term::static_atom("true"))
    } else {
        BifResult::badarg()
    })
}

/// The largest arity of a BEAM function (`MAX_ARG` in `erl_vm.h:54`).
const MAX_ARG: i64 = 255;

/// Run `erlang:make_fun/3`: return the external fun of
/// `module:function/arity`, as ERTS `make_fun_3` (`bif.c:4467-4485`) does.
/// The module and the function must be atoms, and the arity must be an
/// integer in `0..=255` ([`MAX_ARG`]), else the call raises
/// `badarg`. The function does not need to exist.
///
/// # Examples
///
/// ```rust,ignore
/// let module = Term::static_atom("erlang");
/// let function = Term::static_atom("self");
/// assert!(matches!(make_fun(&module, &function, &Term::Integer(0)), BifResult::Value(Term::Fun(_))));
/// assert!(matches!(make_fun(&module, &function, &Term::Integer(256)), BifResult::Raise { .. }));
/// assert!(matches!(make_fun(&Term::Integer(1), &function, &Term::Integer(0)), BifResult::Raise { .. }));
/// ```
pub(super) fn make_fun(module: &Term, function: &Term, arity: &Term) -> BifResult {
    match (module, function, arity.small_integer()) {
        (Term::Atom(module), Term::Atom(function), Some(arity @ 0..=MAX_ARG)) => Value(Term::Fun(
            Fun::external(module.clone(), function.clone(), arity as u32),
        )),
        _ => BifResult::badarg(),
    }
}

/// The item atoms that `erlang:process_info/2` accepts (`pi_arg2ix` in
/// `erl_bif_info.c`). The tuple `{dictionary, Key}` is also an item.
const PROCESS_INFO_ITEMS: [&str; 40] = [
    "registered_name",
    "current_function",
    "initial_call",
    "status",
    "messages",
    "message_queue_len",
    "links",
    "monitors",
    "monitored_by",
    "dictionary",
    "trap_exit",
    "error_handler",
    "heap_size",
    "stack_size",
    "memory",
    "garbage_collection",
    "group_leader",
    "reductions",
    "priority",
    "trace",
    "binary",
    "sequential_trace_token",
    "catchlevel",
    "backtrace",
    "last_calls",
    "total_heap_size",
    "suspending",
    "min_heap_size",
    "min_bin_vheap_size",
    "max_heap_size",
    "current_location",
    "current_stacktrace",
    "message_queue_data",
    "garbage_collection_info",
    "magic_ref",
    "fullsweep_after",
    "parent",
    "async_dist",
    "label",
    "priority_messages",
];

/// Return whether a term is one `erlang:process_info/2` item: an atom of
/// `PROCESS_INFO_ITEMS` or a tuple `{dictionary, Key}`, as `pi_arg2ix`
/// (`erl_bif_info.c`) accepts.
///
/// # Examples
///
/// ```rust,ignore
/// assert!(is_process_info_item(&Term::static_atom("registered_name")));
/// let lookup = Term::tuple(vec![Term::static_atom("dictionary"), Term::Integer(1)]);
/// assert!(is_process_info_item(&lookup));
/// assert!(!is_process_info_item(&Term::static_atom("ok")));
/// ```
fn is_process_info_item(item: &Term) -> bool {
    match item {
        Term::Atom(name) => PROCESS_INFO_ITEMS.contains(&name.name()),
        Term::Tuple(elements) => {
            matches!(elements.as_ref(), [Term::Atom(key), _] if key.name() == "dictionary")
        }
        _ => false,
    }
}

/// Run `erlang:process_info/2` as `process_info_bif`
/// (`erl_bif_info.c:1273-1502`) does. First raise `badarg` when `item` is
/// not an item or a proper list of items, then raise `badarg` when
/// `process` is not a pid. The items that PBeam supports are in
/// [`ProcessState::info`].
///
/// For another process, the caller then needs the `process` capability with
/// `inspect`, else the BIF raises `error:capability_denied` before it looks
/// up the process (row "Capability checks" of `docs/erts-differences.md`).
///
/// - For the caller, first handle the signals that the caller may have sent
///   to itself, up to a flush signal (`pi_maybe_flush_signals`,
///   `erl_bif_info.c:1198-1268`); without such signals only fetch. The BIF
///   runs again after a switch when its reductions are used up, and a
///   signal can end the caller.
/// - Another process that does not exist gives `undefined`
///   (`erl_bif_info.c:1384-1386`). A process that exits, or that has
///   signals or messages that it did not handle, gets a process-info
///   request: the save pointer of the caller goes to the end of its queue
///   (`erts_msgq_set_save_end`), and the BIF traps to
///   `erts_internal:await_result/1` for the reply (`erl_bif_info.c:1478-1500`).
/// - Else the BIF reads the item, and traps to
///   `erlang:bif_handle_signals_return/2` when the caller has incoming
///   signals (`erl_bif_info.c:1441-1443`).
///
/// A read of an item costs the caller one reduction after the read
/// (`erl_bif_info.c:1424`, `1601`).
///
/// # Examples
///
/// ```rust,ignore
/// let process = Term::Pid(context.scheduler.current_process_mut().id);
/// let name = process_info(&mut context, &process, &Term::static_atom("registered_name"))?;
/// assert!(matches!(name, BifResult::Value(name) if name.is_nil()));
/// let dead = process_info(&mut context, &Term::Pid(ProcessId(99)), &name_item)?;
/// assert!(matches!(dead, BifResult::Value(Term::Atom(atom)) if atom.name() == "undefined"));
/// let bogus = process_info(&mut context, &process, &Term::static_atom("bogus_item"))?;
/// assert!(matches!(bogus, BifResult::Raise { .. }));
/// assert!(process_info(&mut context, &process, &Term::static_atom("status")).is_err());
/// ```
///
/// # Errors
///
/// A list of items, another item, and the `current_stacktrace` of another
/// process give a runtime error.
pub(super) fn process_info(
    context: &mut BifContext<'_>,
    process: &Term,
    item: &Term,
) -> Result<BifResult, BeamError> {
    let valid = match item.proper_list() {
        Some(items) => items.clone().all(is_process_info_item),
        None => is_process_info_item(item),
    };
    let (true, Term::Pid(id)) = (valid, process) else {
        return Ok(BifResult::badarg());
    };
    let scheduler = &mut *context.scheduler;
    if let Some(denied) = other_process_denial(scheduler, *id, Operation::Inspect)? {
        return Ok(denied);
    }
    let caller = scheduler.current_process_mut();
    // ponytail: lists of items, other items, and the frames of another process need more state; see "BIF arguments" in docs/erts-differences.md.
    let unsupported = || {
        error(format!(
            "unsupported erlang:process_info/2 arguments {process:?}, {item:?}"
        ))
    };
    if *id == caller.id {
        if caller.flush == Flush::None && !caller.self_signals {
            if caller.mailbox.has_outer_messages() {
                caller.mailbox.fetch();
            }
        } else {
            match scheduler.flush_signals()? {
                Flushed::Done => {}
                Flushed::Exited(reason) => return Ok(BifResult::Exited(reason)),
                Flushed::Yield => {
                    return Ok(BifResult::TrapBif {
                        module: "erlang",
                        function: "process_info",
                        arguments: vec![process.clone(), item.clone()],
                    });
                }
            }
        }
        let current = scheduler.current_reductions();
        let caller = scheduler.current_process_mut();
        let messages = caller.mailbox.message_count();
        let info = caller
            .info(item, current, messages, true)?
            .ok_or_else(unsupported)?;
        caller.bump_reductions(1);
        return Ok(Value(info));
    }
    let Some(target) = scheduler.process(*id) else {
        return Ok(Value(Term::static_atom("undefined")));
    };
    let messages = target.mailbox.message_count();
    let info = target
        .info(item, 0, messages, false)?
        .ok_or_else(unsupported)?;
    if target.exiting.is_some() || target.mailbox.has_incoming() {
        let caller = scheduler.current_process_mut().id;
        let reference = scheduler.make_ref()?;
        scheduler.current_process_mut().mailbox.set_save_end();
        let request = Signal::ProcessInfo {
            requester: caller,
            reference,
            item: item.clone(),
        };
        scheduler
            .send_signal(Some(caller), *id, request)
            .expect("the process is in the process table");
        return Ok(Trap {
            module: "erts_internal",
            function: "await_result",
            arguments: vec![Term::Reference(reference)],
        });
    }
    scheduler.current_process_mut().bump_reductions(1);
    Ok(signals_return(scheduler, process, info))
}

/// Run `erlang:is_process_alive/1` as `is_process_alive_1`
/// (`erl_bif_info.c:4060-4103`) does. The caller is alive. A pid that no
/// process has gives `false`. A process that exits, or that has signals or
/// messages that it did not handle, gets an is-alive request through
/// `erts_internal:is_process_alive/1`, so that the answer comes after the
/// signals that the caller sent before. Else the result is `true`. A
/// result traps to `erlang:bif_handle_signals_return/2` when the caller has
/// incoming signals. A term that is not a pid raises `badarg`. PBeam cannot
/// represent a remote pid. For another process, the caller needs the
/// `process` capability with `inspect`, else the BIF raises
/// `error:capability_denied` before it looks up the process.
///
/// # Examples
///
/// ```rust,ignore
/// let alive = is_process_alive(&mut context, &Term::Pid(child))?;
/// assert!(matches!(alive, BifResult::Value(Term::Atom(atom)) if atom.name() == "true"));
/// assert!(matches!(is_process_alive(&mut context, &Term::Integer(1))?, BifResult::Raise { .. }));
/// ```
///
/// # Errors
///
/// The errors of `AtomTable::put` for a full atom table.
pub(super) fn is_process_alive(
    context: &mut BifContext<'_>,
    pid: &Term,
) -> Result<BifResult, BeamError> {
    let Term::Pid(id) = pid else {
        return Ok(BifResult::badarg());
    };
    let scheduler = &mut *context.scheduler;
    if *id == scheduler.current_process_mut().id {
        return Ok(Value(Term::static_atom("true")));
    }
    if let Some(denied) = other_process_denial(scheduler, *id, Operation::Inspect)? {
        return Ok(denied);
    }
    let alive = match scheduler.process(*id) {
        None => "false",
        Some(target) if target.exiting.is_some() || target.mailbox.has_incoming() => {
            return Ok(Trap {
                module: "erts_internal",
                function: "is_process_alive",
                arguments: vec![pid.clone()],
            });
        }
        Some(_) => "true",
    };
    Ok(signals_return(scheduler, pid, Term::static_atom(alive)))
}

/// Return `value` as the result of a BIF that read the state of the process
/// `from`, or, when the caller has incoming signals, trap to
/// `erlang:bif_handle_signals_return/2`, which handles the signals of the
/// caller up to a flush signal and then returns `value`
/// (`ERTS_BIF_HANDLE_SIGNALS_FROM_RETURN`, `bif.h:527-532`). The signals
/// that `from` sent to the caller before the read then come before the
/// result.
///
/// # Examples
///
/// ```rust,ignore
/// let result = signals_return(scheduler, &Term::Pid(child), Term::static_atom("true"));
/// ```
pub(super) fn signals_return(scheduler: &mut Scheduler, from: &Term, value: Term) -> BifResult {
    if scheduler.current_process_mut().mailbox.has_incoming() {
        return BifResult::TrapBif {
            module: "erlang",
            function: "bif_handle_signals_return",
            arguments: vec![from.clone(), value],
        };
    }
    Value(value)
}

/// Run `erlang:function_exported/3`: return `true` when a loaded module
/// exports the function or the function is a BIF, as ERTS
/// `function_exported_3` (`bif.c`) does with `erts_find_function` and
/// `erts_is_builtin`. A `module` or `name` that is not an atom, and an
/// `arity` that is not a small integer, raise `badarg`. ERTS reads the
/// small-integer arity into a C `int`, so only its low 32 bits count, and a
/// negative arity finds no function.
///
/// # Examples
///
/// ```rust,ignore
/// let erlang = Term::static_atom("erlang");
/// let exported = function_exported(&scheduler.code, &erlang, &Term::static_atom("self"), &Term::Integer(0));
/// assert!(matches!(exported, BifResult::Value(Term::Atom(atom)) if atom.name() == "true"));
/// let exported = function_exported(&scheduler.code, &erlang, &Term::static_atom("self"), &Term::Integer(1 << 59));
/// assert!(matches!(exported, BifResult::Raise { .. }));
/// ```
pub(super) fn function_exported(
    code: &CodeTable,
    module: &Term,
    name: &Term,
    arity: &Term,
) -> BifResult {
    let (Term::Atom(module), Term::Atom(name), Some(arity)) = (module, name, arity.small_integer())
    else {
        return BifResult::badarg();
    };
    let (module, name) = (module.name(), name.name());
    let exported = u32::try_from(arity as i32).is_ok_and(|arity| {
        code.get(module)
            .is_some_and(|loaded| loaded.exports.contains_key(&(name.to_owned(), arity)))
            || is_bif(module, name, arity)
            // `erts_is_builtin` (`beam_common.c`) also counts these
            // instruction calls, which are not `bif.tab` entries.
            || matches!(
                (module, name, arity),
                ("erlang", "apply", 2 | 3) | ("erlang", "yield", 0)
            )
    });
    Value(Term::static_atom(if exported { "true" } else { "false" }))
}

/// Run `erlang:finish_loading(PreparedCodeList)`, as ERTS
/// `finish_loading_1` (`beam_bif_load.c:340-508`) does. A term that is not a
/// proper list, an element that is not a reference, and a reference with
/// no prepared code raise `badarg`, also a reference whose code loaded
/// already. Then the caller needs a `code` capability that permits `load`
/// for each module of the list (row "Capability checks" of
/// `docs/erts-differences.md`), after the argument checks and before any
/// module loads: a denied call loads no module and gives
/// `error:capability_denied`. An empty list needs the `load` operation and
/// compares no module name. A list that names one module more than one time
/// returns `{duplicated, Modules}` and keeps each prepared entry (see
/// [`CodeTable::finish`]). Else each module goes into the code table of
/// the run, and the BIF returns `ok` after the calling process switches
/// out: ERTS suspends the caller until the new code is committed and then
/// returns the value (`beam_bif_load.c:555-566`). An empty list also
/// returns `ok` this way.
///
/// # Examples
///
/// ```rust,ignore
/// let loaded = finish_loading(&mut scheduler, &Term::list_from(vec![Term::Reference(prepared)]))?;
/// assert!(matches!(loaded, BifResult::YieldReturn(_)));
/// assert!(matches!(finish_loading(&mut scheduler, &Term::list_from(vec![test_atom("a")]))?, BifResult::Raise { .. }));
/// ```
///
/// # Errors
///
/// A module that is loaded already gives the runtime error of
/// [`CodeTable::finish`]: PBeam has no code replacement. A new
/// `capability_denied` atom in a full atom table gives the error of
/// `AtomTable::put`.
pub(super) fn finish_loading(
    scheduler: &mut Scheduler,
    prepared: &Term,
) -> Result<BifResult, BeamError> {
    let Some(elements) = prepared.proper_list() else {
        return Ok(BifResult::badarg());
    };
    let mut references = Vec::new();
    for element in elements {
        let Term::Reference(reference) = element else {
            return Ok(BifResult::badarg());
        };
        references.push(*reference);
    }
    let Some(names) = scheduler.code.prepared_names(&references) else {
        return Ok(BifResult::badarg());
    };
    if let Some(denied) = denial(scheduler, CapabilityKind::Code, Operation::Load, None)? {
        return Ok(denied);
    }
    for name in &names {
        let check = denial(
            scheduler,
            CapabilityKind::Code,
            Operation::Load,
            Some(name.as_bytes()),
        );
        if let Some(denied) = check? {
            return Ok(denied);
        }
    }
    match scheduler.code.finish(&references, names)? {
        Finish::Loaded => Ok(BifResult::YieldReturn(Term::static_atom("ok"))),
        Finish::Duplicated(names) => {
            let mut modules = Vec::with_capacity(names.len());
            for name in names {
                modules.push(Term::Atom(scheduler.atoms.put(&name)?));
            }
            Ok(Value(Term::tuple(vec![
                Term::static_atom("duplicated"),
                Term::list_from(modules),
            ])))
        }
    }
}

/// Run `erlang:module_loaded/1`, as ERTS `module_loaded_1`
/// (`beam_bif_load.c:738-755`) does: return `true` when `module` has current
/// code and no `-on_load` function waits to run, and `false` for an atom
/// that names no loaded module. A `module` that is not an atom raises
/// `badarg`. Then the caller needs a `code` capability that permits
/// `inspect` for the module name (row "Capability checks" of
/// `docs/erts-differences.md`), after the atom check and before the code
/// table lookup. The PBeam loader does not load a module that has an `on_load`
/// instruction (`unsupported BEAM opcode 149`), so each loaded module counts.
///
/// # Examples
///
/// ```rust,ignore
/// let loaded = module_loaded(&mut context, &Term::static_atom("erlang"))?;
/// assert!(matches!(loaded, BifResult::Value(Term::Atom(atom)) if atom.name() == "true"));
/// assert!(matches!(module_loaded(&mut context, &Term::Integer(1))?, BifResult::Raise { .. }));
/// ```
///
/// # Errors
///
/// A new `capability_denied` atom in a full atom table gives the error of
/// `AtomTable::put`.
pub(super) fn module_loaded(
    context: &mut BifContext<'_>,
    module: &Term,
) -> Result<BifResult, BeamError> {
    let Term::Atom(module) = module else {
        return Ok(BifResult::badarg());
    };
    let check = denial(
        context.scheduler,
        CapabilityKind::Code,
        Operation::Inspect,
        Some(module.name().as_bytes()),
    );
    if let Some(denied) = check? {
        return Ok(denied);
    }
    let loaded = context.scheduler.code.get(module.name()).is_some();
    Ok(Value(Term::static_atom(if loaded {
        "true"
    } else {
        "false"
    })))
}

/// The preloaded modules of ERTS, in the order of `PRELOAD_BEAM` and
/// `ESOCK_PRELOAD_BEAM` (`erts/emulator/Makefile.in:804-828`).
const PRELOADED: [&str; 22] = [
    "erts_code_purger",
    "erl_init",
    "init",
    "prim_buffer",
    "prim_eval",
    "prim_inet",
    "prim_file",
    "zlib",
    "socket_registry",
    "prim_socket",
    "prim_net",
    "prim_zip",
    "erl_prim_loader",
    "erlang",
    "erts_internal",
    "erl_tracer",
    "erts_literal_area_collector",
    "erts_trace_cleaner",
    "erts_dirty_process_signal_handler",
    "atomics",
    "counters",
    "persistent_term",
];

/// Run `erlang:pre_loaded/0`, as ERTS `pre_loaded_0` and `erts_preloaded`
/// (`beam_bif_load.c:757-760`, `erl_init.c:459-484`) do: return the names
/// of the preloaded modules of the emulator, with the last module of the
/// preload list first. The list is fixed: PBeam has no module that is built
/// into the runtime, and a run loads the files of these modules with the
/// other start modules, so the BIF returns the ERTS list also when the run
/// did not load one of them.
///
/// # Examples
///
/// ```rust,ignore
/// let BifResult::Value(modules) = pre_loaded(&mut scheduler)? else { unreachable!() };
/// // [persistent_term, counters, atomics, ..., erl_init, erts_code_purger]
/// ```
///
/// # Errors
///
/// The atom table cannot take a new module name.
pub(super) fn pre_loaded(scheduler: &mut Scheduler) -> Result<BifResult, BeamError> {
    let mut modules = Vec::with_capacity(PRELOADED.len());
    for name in PRELOADED.iter().rev() {
        modules.push(Term::Atom(scheduler.atoms.put(name)?));
    }
    Ok(Value(Term::list_from(modules)))
}

/// Run `erlang:get_module_info(Module, What)`, which `Module:module_info/1`
/// calls, as ERTS `get_module_info_2` and `module_info_1`
/// (`erl_bif_info.c:5918-5945`, `5986-6030`) do. A `Module` that is not an
/// atom or that names no loaded module raises `badarg`. The key `module`
/// returns `Module`. A term that is not a key of `get_module_info/3`
/// (`md5`, `exports`, `functions`, `nifs`, `attributes`, `compile`,
/// `native_addresses`, `native`, or `module`) raises `badarg`. The caller
/// needs a `code` capability that permits `inspect` for the module name
/// (row "Capability checks" of `docs/erts-differences.md`), after the atom
/// checks of `Module` and `What` and before the code table lookup, so a
/// denied call does not tell whether a module is loaded. This entry of the
/// inventory was `later` until #465.
///
/// # Examples
///
/// ```rust,ignore
/// let lists = Term::static_atom("lists");
/// let info = get_module_info(&mut context, &lists, &Term::static_atom("module"))?;
/// assert!(matches!(info, BifResult::Value(Term::Atom(atom)) if atom.name() == "lists"));
/// ```
///
/// # Errors
///
/// Each other key of `get_module_info/3` is the runtime error
/// `erlang:get_module_info/2 key K is not implemented`. A new
/// `capability_denied` atom in a full atom table gives the error of
/// `AtomTable::put`.
pub(super) fn get_module_info(
    context: &mut BifContext<'_>,
    module: &Term,
    what: &Term,
) -> Result<BifResult, BeamError> {
    let Term::Atom(name) = module else {
        return Ok(BifResult::badarg());
    };
    let Term::Atom(key) = what else {
        return Ok(BifResult::badarg());
    };
    let check = denial(
        context.scheduler,
        CapabilityKind::Code,
        Operation::Inspect,
        Some(name.name().as_bytes()),
    );
    if let Some(denied) = check? {
        return Ok(denied);
    }
    if context.scheduler.code.get(name.name()).is_none() {
        return Ok(BifResult::badarg());
    }
    match key.name() {
        "module" => Ok(Value(module.clone())),
        key @ ("md5" | "exports" | "functions" | "nifs" | "attributes" | "compile"
        | "native_addresses" | "native") => Err(error(format!(
            "erlang:get_module_info/2 key {key} is not implemented"
        ))),
        _ => Ok(BifResult::badarg()),
    }
}

/// Run `erlang:start_timer(Time, Dest, Msg)`, as ERTS `start_timer_3` and
/// `setup_bif_timer` (`erl_hl_timer.c:2605-2620`, `1804-1871`) do:
///
/// - `Time` goes through the relative branch of `parse_timeout_pos`
///   ([`Timers::timeout_position`]). A term that is not an integer, a
///   negative integer, an end after `ERTS_MONOTONIC_END`, and a 64-bit
///   overflow raise `badarg`.
/// - Then a `Dest` that is not a pid or an atom raises `badarg`.
/// - Then the caller needs a `process` capability that permits `send`, also
///   for its own pid. Without it, the call raises `error:capability_denied`,
///   makes no reference, and starts no timer.
/// - Then the runtime to boot gate denies a timer for a boot process, or for
///   a name that a boot process has now, in the same way (see
///   `boot_denial`). A name gets its process when the timer expires, so a
///   name that a boot process registers later is not denied.
/// - Then the BIF makes a new reference and returns it. A pid that is not
///   alive gets no timer, also a pid whose exit started, as
///   `erts_pid2proc_opt` finds no exiting process
///   (`erl_process_lock.c:970-984`). The timer of a pid goes into the timer
///   tree of the process (`erl_hl_timer.c:1832-1858`).
///
/// When the timer expires, `Dest` gets `{timeout, Reference, Msg}`.
///
/// # Examples
///
/// ```rust,ignore
/// let result = start_timer(&mut context, &Term::Integer(100), &Term::Pid(pid), &msg)?;
/// assert!(matches!(result, BifResult::Value(Term::Reference(_))));
/// let result = start_timer(&mut context, &Term::Integer(-1), &Term::Pid(pid), &msg)?;
/// assert!(matches!(result, BifResult::Raise { .. }));
/// ```
///
/// # Errors
///
/// A reference counter overflow gives a runtime error. A new
/// `capability_denied` atom in a full atom table gives the error of
/// `AtomTable::put`.
pub(super) fn start_timer(
    context: &mut BifContext<'_>,
    time: &Term,
    receiver: &Term,
    message: &Term,
) -> Result<BifResult, BeamError> {
    let scheduler = &mut *context.scheduler;
    let end = match *time {
        Term::Integer(msec) => Timers::timeout_position(scheduler.timers.now(), msec),
        _ => None,
    };
    let Some(end) = end else {
        return Ok(BifResult::badarg());
    };
    let receiver = match receiver {
        Term::Pid(id) => Receiver::Process(*id),
        Term::Atom(name) => Receiver::Name(name.clone()),
        _ => return Ok(BifResult::badarg()),
    };
    if let Some(denied) = denial(scheduler, CapabilityKind::Process, Operation::Send, None)? {
        return Ok(denied);
    }
    let target = match &receiver {
        Receiver::Process(id) => Some(*id),
        Receiver::Name(name) => scheduler.whereis(name.name()),
    };
    if let Some(target) = target
        && let Some(denied) = boot_denial(scheduler, target)?
    {
        return Ok(denied);
    }
    let reference = scheduler.make_ref()?;
    if !matches!(receiver, Receiver::Process(id) if scheduler.live_process(id).is_none()) {
        if let Receiver::Process(id) = receiver
            && let Some(process) = scheduler.process_mut(id)
        {
            process.bif_timers.insert(TimerReference(reference), ());
        }
        let message = Term::tuple(vec![
            Term::static_atom("timeout"),
            Term::Reference(reference),
            message.clone(),
        ]);
        scheduler.timers.start(
            end,
            Timer::Bif {
                reference,
                receiver,
                message,
            },
        );
    }
    Ok(Value(Term::Reference(reference)))
}

/// Run `erlang:cancel_timer(TimerRef, Options)`, as ERTS `cancel_timer_2`
/// (`erl_hl_timer.c:2649-2659`) does:
///
/// - `parse_bif_timer_options` (`erl_hl_timer.c:2352-2398`) reads a list of
///   `{async, Bool}` and `{info, Bool}`, and the last value of an option
///   wins. The defaults are `async` false and `info` true. Each other
///   element, also `{abs, Bool}`, raises `badarg`.
/// - Then a `TimerRef` that is not a reference raises `badarg`
///   (`access_bif_timer`, `erl_hl_timer.c:2237-2340`).
/// - Then `access_btm` (`erl_hl_timer.c:1922-1971`) cancels the active
///   timer and gives the milliseconds left (`get_time_left`), or `false`
///   when no timer is active.
/// - `access_sched_local_btm` (`erl_hl_timer.c:2078-2147`) gives the
///   result: `info` false gives `ok`. Else `async` false gives the
///   milliseconds left or `false` (`return_info`, `erl_hl_timer.c:1975-1989`),
///   and `async` true sends `{cancel_timer, TimerRef, Result}` to the caller
///   and gives `ok` (`send_async_info`, `erl_hl_timer.c:1992-2034`).
///
/// Each reference that PBeam can represent is a reference of the local
/// scheduler, so the `no_timer_result` path (`erl_hl_timer.c:2209-2234`)
/// never runs.
///
/// # Examples
///
/// ```rust,ignore
/// let result = cancel_timer(&mut context, &Term::Reference(reference), &Term::nil())?;
/// assert!(matches!(result, BifResult::Value(Term::Integer(100))));
/// let result = cancel_timer(&mut context, &Term::Reference(reference), &Term::nil())?;
/// assert!(matches!(result, BifResult::Value(Term::Atom(atom)) if atom.name() == "false"));
/// ```
pub(super) fn cancel_timer(
    context: &mut BifContext<'_>,
    reference: &Term,
    options: &Term,
) -> Result<BifResult, BeamError> {
    let (Some((asynchronous, info)), Term::Reference(reference)) =
        (timer_options(options), reference)
    else {
        return Ok(BifResult::badarg());
    };
    let scheduler = &mut *context.scheduler;
    let result = match scheduler.cancel_bif_timer(*reference) {
        Some(end) => Term::Integer(Timers::time_left(scheduler.timers.now(), end)),
        None => Term::static_atom("false"),
    };
    if !info {
        return Ok(Value(Term::static_atom("ok")));
    }
    if !asynchronous {
        return Ok(Value(result));
    }
    let caller = scheduler.current_process_mut().id;
    let message = Term::tuple(vec![
        Term::static_atom("cancel_timer"),
        Term::Reference(*reference),
        result,
    ]);
    scheduler.deliver(caller, message);
    Ok(Value(Term::static_atom("ok")))
}

/// Read the options of `erlang:cancel_timer/2` as `parse_bif_timer_options`
/// (`erl_hl_timer.c:2352-2398`) does without the `abs` option. Return
/// `(async, info)`, or `None` for an option list that is not valid.
///
/// # Examples
///
/// ```rust,ignore
/// assert_eq!(timer_options(&Term::nil()), Some((false, true)));
/// assert_eq!(timer_options(&Term::static_atom("foo")), None);
/// ```
fn timer_options(options: &Term) -> Option<(bool, bool)> {
    let options = options.proper_list()?;
    let (mut asynchronous, mut info) = (false, true);
    for option in options {
        let Term::Tuple(fields) = option else {
            return None;
        };
        let [Term::Atom(name), Term::Atom(value)] = fields.as_ref() else {
            return None;
        };
        let value = match value.name() {
            "true" => true,
            "false" => false,
            _ => return None,
        };
        match name.name() {
            "async" => asynchronous = value,
            "info" => info = value,
            _ => return None,
        }
    }
    Some((asynchronous, info))
}

/// The exit status of a halt whose flush timeout ends, as
/// `ERTS_HALT_FLUSH_TIMEOUT_EXIT_CODE` (`erl_init.c:75`).
const HALT_FLUSH_TIMEOUT_STATUS: i32 = 255;

/// Run `erlang:halt(Status, Options)`, as `halt_2` (`bif.c:4309-4398`)
/// does. `erlang:halt/0` and `erlang:halt/1` call it with the options `[]`
/// (`erlang.erl:3357-3358`, `3383-3388`).
///
/// The options are checked first. Each element of a proper list must be
/// `{flush, true | false}` or `{flush_timeout, T}`, else the call raises
/// `badarg` (`bif.c:4316-4343`). `T` is a non-negative small integer, which
/// `term_to_Uint32` truncates to 32 bits (`big.c:3648-3663`). The last
/// option of each kind holds. ERTS gives this `badarg` the `error_info`
/// cause `badopt`, which PBeam does not model.
///
/// A non-negative integer `Status` stops the run with the status code
/// `Status` masked to `INT_MAX` (`bif.c:4346-4362`, `big.c:3545-3565`), as
/// the error [`BeamError::Halt`]. Each other process stops at once, as in a
/// halting ERTS scheduler (`erl_process.c:15220-15255`). PBeam writes each
/// output with a host call before the call returns, so a flush has no port
/// output to wait for. With a flush and a timeout of 0, the ERTS flush
/// timer ends before the flush, and the node exits with status 255
/// (`erl_process.c:15244-15248`, `erl_init.c:2771-2774`; measured 5 of 5).
///
/// `abort` (a core dump) and a slogan `Status` (a crash dump with the
/// string as its slogan, see [`is_slogan`]) give a runtime error: PBeam
/// writes no core dump and no crash dump. Each other `Status` raises
/// `badarg` (`bif.c:4379`, `4388-4393`).
///
/// After the checks of the options and of `Status`, the caller needs a
/// `runtime` capability that permits `halt`, else the call raises
/// `error:capability_denied` and the run goes on. ERTS checks the same
/// arguments before it stops (`bif.c:4316-4389`).
///
/// # Examples
///
/// ```rust,ignore
/// assert_eq!(halt(&mut scheduler, &Term::Integer(259), &Term::nil()), Err(BeamError::Halt(259)));
/// assert!(matches!(halt(&mut scheduler, &Term::Integer(-1), &Term::nil()), Ok(BifResult::Raise { .. })));
/// assert!(halt(&mut scheduler, &Term::static_atom("abort"), &Term::nil()).is_err());
/// ```
///
/// # Errors
///
/// A halt gives [`BeamError::Halt`]. `abort` and a slogan give a runtime
/// error. A new `capability_denied` atom in a full atom table gives the
/// error of `AtomTable::put`.
pub(super) fn halt(
    scheduler: &mut Scheduler,
    status: &Term,
    options: &Term,
) -> Result<BifResult, BeamError> {
    let Some(options) = options.proper_list() else {
        return Ok(BifResult::badarg());
    };
    let mut flush = true;
    let mut timeout = None;
    for option in options {
        match option {
            Term::Tuple(pair) => match pair.as_ref() {
                [Term::Atom(name), Term::Atom(value)]
                    if name.name() == "flush" && matches!(value.name(), "true" | "false") =>
                {
                    flush = value.name() == "true";
                }
                [Term::Atom(name), value] if name.name() == "flush_timeout" => {
                    match value.small_integer() {
                        Some(value @ 0..) => timeout = Some(value & i64::from(u32::MAX)),
                        _ => return Ok(BifResult::badarg()),
                    }
                }
                _ => return Ok(BifResult::badarg()),
            },
            _ => return Ok(BifResult::badarg()),
        }
    }
    let stop = match status {
        Term::Integer(code) if *code >= 0 => {
            if flush && timeout == Some(0) {
                BeamError::Halt(HALT_FLUSH_TIMEOUT_STATUS)
            } else {
                let code = i32::try_from(code & i64::from(i32::MAX)).expect("masked to INT_MAX");
                BeamError::Halt(code)
            }
        }
        Term::Atom(abort) if abort.name() == "abort" => {
            error("unsupported erlang:halt/2 with abort: PBeam writes no core dump")
        }
        _ if is_slogan(status) => {
            error("unsupported erlang:halt/2 with a crash dump slogan: PBeam writes no crash dump")
        }
        _ => return Ok(BifResult::badarg()),
    };
    if let Some(denied) = denial(scheduler, CapabilityKind::Runtime, Operation::Halt, None)? {
        return Ok(denied);
    }
    Err(stop)
}

/// The most characters of a crash dump slogan, as `HALT_MSG_SIZE`
/// (`bif.c:4373`).
const HALT_MSG_SIZE: usize = 1023;

/// Return whether `status` is a crash dump slogan of `erlang:halt/2`: a list
/// for which `erts_unicode_list_to_buf` with at most `HALT_MSG_SIZE`
/// characters does not give a type error (`bif.c:4372-4380`,
/// `utils.c:2862-2942`). Each of the first 1023 elements must be a code
/// point outside `0xD800..=0xDFFF`. The tail must be `[]`, unless the list
/// has more than 1023 elements: ERTS cuts the slogan before it reads the
/// tail.
///
/// # Examples
///
/// ```rust,ignore
/// assert!(is_slogan(&Term::list_from(vec![Term::Integer(65)])));
/// assert!(!is_slogan(&Term::list_from(vec![Term::static_atom("foo")])));
/// assert!(!is_slogan(&Term::list_with_tail(vec![Term::Integer(65)], Term::static_atom("foo"))));
/// ```
fn is_slogan(status: &Term) -> bool {
    let Some(mut elements) = status.list_iter() else {
        return false;
    };
    let characters = elements.by_ref().take(HALT_MSG_SIZE).all(|element| {
        matches!(element, Term::Integer(code)
            if u32::try_from(*code).ok().and_then(char::from_u32).is_some())
    });
    // The rest is `[]` after the last element of a proper list, and a cons
    // cell after 1023 elements of a longer list.
    characters && elements.rest().is_list()
}

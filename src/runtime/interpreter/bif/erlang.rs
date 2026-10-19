//! Bodies of the `erlang` BIFs that do not fit on one table line.

use super::BifResult::{Raise, Trap, Value};
use super::*;
use crate::beam::code::Instruction;
use crate::runtime::capability;
use crate::runtime::host;
use crate::runtime::interpreter::DEFAULT_ERROR_HANDLER;
use crate::runtime::interpreter::code_table::{CodeTable, Finish};
use crate::runtime::local_node;
use crate::runtime::port::Ports;
use crate::runtime::process::InitialCall;
use crate::runtime::process::{Flush, Origin, Priority, TimerReference};
use crate::runtime::process::{LinkId, ProcessState};
use crate::runtime::scheduler::PTAB_SIZE;
use crate::runtime::scheduler::{AliasMode, CONTEXT_REDS, Flushed, MonitorOptions, MonitorTarget};
use crate::runtime::signal::Signal;
use crate::runtime::time::{self, TimeValue};
use crate::runtime::timer::{Receiver, Timer, Timers};
use num_bigint::BigInt;
use num_traits::FromPrimitive;
use std::io::Write;

use crate::term::{
    Atom, AtomTable, Fun, MAX_ATOM_CHARACTERS, MAX_ATOM_SZ_LIMIT, MAX_TUPLE_SIZE, Map, ProcessId,
    Reference, SMALL_INTEGER_MAX, SMALL_INTEGER_MIN, debug_term, external, print_term,
};

/// Run `erlang:node/1`: return the node of a pid. A term that is not a pid
/// gives a runtime error.
///
/// # Examples
///
/// ```rust,ignore
/// let atoms = AtomTable::new();
/// let result = node(&Term::Pid(pid), &atoms)?;
/// assert!(matches!(result, BifResult::Value(Term::Atom(atom)) if atom.is_static("nonode@nohost"));
/// assert!(node(&Term::Integer(1), &atoms).is_err());
/// ```
pub(super) fn node(term: &Term, atoms: &AtomTable) -> Result<BifResult, BeamError> {
    match term {
        Term::Pid(_) => Ok(Value(local_node())),
        _ => Err(error(format!(
            "unsupported erlang:node/1 argument {}: not a pid",
            debug_term(term, atoms)
        ))),
    }
}

/// Run `erlang:nodes(NodeTypes)` and `erlang:nodes(NodeTypes, Options)` as
/// ERTS `nodes` does (`dist.c:6261-6476`) on a node that is not alive.
/// `NodeTypes` is one node type or a proper list of them: `visible`,
/// `hidden`, `connected`, `this`, and `known`. PBeam has no connection, so
/// `visible`, `hidden`, and `connected` give no node. `this` and `known`
/// give the local node `nonode@nohost` last. Before it, `known` gives the
/// not connected entry of each node that a send used, from the oldest to
/// the newest (see `Scheduler::known_nodes`). `Options` (`None` for
/// `nodes/1`) is a map of the keys `connection_id` and `node_type`, each
/// `true` or `false`. With options, the result holds `{Node, Info}`, and
/// `Info` maps `connection_id` to `undefined` and `node_type` to `known`
/// or `this` for each key that is `true`. Each other term raises `badarg`.
/// A result of more than 10 nodes costs one reduction for each 10 nodes.
///
/// # Examples
///
/// ```rust,ignore
/// assert_eq!(nodes(&Term::static_atom("this"), None, scheduler), Value(Term::list_from([local_node()])));
/// assert_eq!(nodes(&Term::static_atom("visible"), None, scheduler), Value(Term::nil()));
/// assert!(matches!(nodes(&Term::Integer(1), None, scheduler), BifResult::Raise { .. }));
/// ```
pub(super) fn nodes(
    node_types: &Term,
    options: Option<&Term>,
    scheduler: &mut Scheduler,
) -> BifResult {
    let one = Term::list_from([node_types.clone()]);
    let node_types = if matches!(node_types, Term::Atom(_)) {
        &one
    } else {
        node_types
    };
    let Some(node_types) = node_types.proper_list() else {
        return BifResult::badarg();
    };
    let (mut this, mut known) = (false, false);
    for node_type in node_types {
        match node_type {
            Term::Atom(name) if name.is_static("this") => this = true,
            Term::Atom(name) if name.is_static("known") => (this, known) = (true, true),
            Term::Atom(name) if name.is_one_of(&["visible", "hidden", "connected"]) => {}
            _ => return BifResult::badarg(),
        }
    }
    let (mut connection_id, mut node_type) = (false, false);
    match options {
        None => {}
        Some(Term::Map(options)) => {
            for (option, value) in options.iter() {
                let flag = match option {
                    Term::Atom(name) if name.is_static("connection_id") => &mut connection_id,
                    Term::Atom(name) if name.is_static("node_type") => &mut node_type,
                    _ => return BifResult::badarg(),
                };
                *flag = match value {
                    Term::Atom(value) if value.is_static("true") => true,
                    Term::Atom(value) if value.is_static("false") => false,
                    _ => return BifResult::badarg(),
                };
            }
        }
        Some(_) => return BifResult::badarg(),
    }
    let mut names: Vec<(Term, &str)> = Vec::new();
    if known {
        names.extend(
            scheduler
                .known_nodes()
                .into_iter()
                .map(|name| (Term::Atom(name), "known")),
        );
    }
    if this {
        names.push((local_node(), "this"));
    }
    // ERTS charges one reduction for each 10 nodes of a result of more than
    // 10 nodes (`dist.c:6465-6468`).
    if names.len() > 10 {
        scheduler
            .current_process_mut()
            .bump_reductions((names.len() / 10) as i32);
    }
    let atoms = &scheduler.atoms;
    Value(Term::list_from(names.into_iter().map(|(name, kind)| {
        if options.is_none() {
            return name;
        }
        let mut info = Vec::new();
        if connection_id {
            info.push((
                Term::static_atom("connection_id"),
                Term::static_atom("undefined"),
            ));
        }
        if node_type {
            info.push((Term::static_atom("node_type"), Term::static_atom(kind)));
        }
        // ponytail: each key is an atom, so `from_pairs` cannot fail.
        let info = Map::from_pairs(info, atoms).expect("atom keys fit each map form");
        Term::tuple(vec![name, Term::Map(info)])
    })))
}

/// The flags of ERTS `process_flag_2` (`bif.c:2028-2237`) that need a
/// subsystem that PBeam does not have: the error handler, schedulers,
/// the garbage collector, tracing, and saved calls.
const UNSUPPORTED_PROCESS_FLAGS: &[&str] = &[
    "scheduler",
    "fullsweep_after",
    "min_heap_size",
    "min_bin_vheap_size",
    "max_heap_size",
    "sensitive",
    "save_calls",
];

/// The option bits of a node monitor, `ERTS_NODES_MON_OPT_*` of ERTS
/// (`dist.c:6698-6702`): the node type `visible`, the node type `hidden`,
/// `nodedown_reason`, the info map, and `connection_id`.
const NODES_MONITOR_VISIBLE: u8 = 1 << 0;
const NODES_MONITOR_HIDDEN: u8 = 1 << 1;
const NODES_MONITOR_DOWN_REASON: u8 = 1 << 2;
const NODES_MONITOR_INFO_MAP: u8 = 1 << 3;
const NODES_MONITOR_CONNECTION_ID: u8 = 1 << 4;

/// Return the key of a node monitor with `options`, as
/// `erts_monitor_nodes` makes it (`dist.c:6735-6802`): `[]` gives 0, and a
/// map gives the info map bit and the bit of each option. The option
/// `list => true` clears the info map bit, `connection_id` and
/// `nodedown_reason` take `true` or `false`, and `node_type` takes
/// `visible`, `hidden`, or `all`. A map key that is not an option, an
/// option value that is not valid, and a term that is not `[]` or a map
/// give `None`.
///
/// # Examples
///
/// ```rust,ignore
/// assert_eq!(nodes_monitor_key(&Term::nil()), Some(0));
/// let map = Map::from_pairs(vec![(Term::static_atom("node_type"), Term::static_atom("all"))], &atoms)?;
/// assert_eq!(nodes_monitor_key(&Term::Map(map)), Some(0b1011));
/// assert_eq!(nodes_monitor_key(&Term::Integer(1)), None);
/// ```
fn nodes_monitor_key(options: &Term) -> Option<u8> {
    let options = match options {
        Term::Map(options) => options,
        _ => return options.is_nil().then_some(0),
    };
    let mut key = NODES_MONITOR_INFO_MAP;
    for (option, value) in options.iter() {
        let (Term::Atom(option), Term::Atom(value)) = (option, value) else {
            return None;
        };
        let on = value.is_static("true");
        let boolean = on || value.is_static("false");
        let node_type = option.is_static("node_type");
        key = match () {
            _ if option.is_static("list") && on => key & !NODES_MONITOR_INFO_MAP,
            _ if option.is_static("connection_id") && boolean => {
                key | if on { NODES_MONITOR_CONNECTION_ID } else { 0 }
            }
            _ if option.is_static("nodedown_reason") && boolean => {
                key | if on { NODES_MONITOR_DOWN_REASON } else { 0 }
            }
            _ if node_type && value.is_static("visible") => key | NODES_MONITOR_VISIBLE,
            _ if node_type && value.is_static("hidden") => key | NODES_MONITOR_HIDDEN,
            _ if node_type && value.is_static("all") => {
                key | NODES_MONITOR_VISIBLE | NODES_MONITOR_HIDDEN
            }
            _ => return None,
        };
    }
    Some(key)
}

/// Set or clear a node monitor of `process` with `options`, as
/// `erts_monitor_nodes` does (`dist.c:6731-6845`), and return the old count
/// of the monitor with the same key. `true` adds 1 to the count, and
/// `false` removes the monitor. A value that is not `true` or `false` and
/// options that `nodes_monitor_key` rejects raise `badarg`.
///
/// # Examples
///
/// ```rust,ignore
/// assert!(matches!(monitor_nodes(process, &Term::nil(), Some("true")), BifResult::Value(Term::Integer(0))));
/// assert!(matches!(monitor_nodes(process, &Term::nil(), Some("true")), BifResult::Value(Term::Integer(1))));
/// assert!(matches!(monitor_nodes(process, &Term::nil(), Some("false")), BifResult::Value(Term::Integer(2))));
/// ```
fn monitor_nodes(process: &mut ProcessState, options: &Term, value: Option<&str>) -> BifResult {
    let (Some(on @ ("true" | "false")), Some(key)) = (value, nodes_monitor_key(options)) else {
        return BifResult::badarg();
    };
    let old = if on == "true" {
        let count = process.node_monitors.entry(key).or_default();
        *count += 1;
        *count - 1
    } else {
        process.node_monitors.remove(&key).unwrap_or(0)
    };
    Value(Term::from_bigint(BigInt::from(old)))
}

/// Set a boolean process flag to `value` and return the old value, or raise
/// `badarg` for a value that is not `true` or `false`.
///
/// # Examples
///
/// ```rust,ignore
/// let mut flag = false;
/// assert!(matches!(boolean_flag(&mut flag, Some("true")), BifResult::Value(_)));
/// assert!(flag);
/// ```
fn boolean_flag(flag: &mut bool, value: Option<&str>) -> BifResult {
    let old = Term::static_atom(&flag.to_string());
    match value {
        Some("true") => *flag = true,
        Some("false") => *flag = false,
        _ => return BifResult::badarg(),
    }
    Value(old)
}

/// Run `erlang:process_flag(Flag, Value)` as ERTS `process_flag_2`
/// (`bif.c:2028-2237`) does for the flags `trap_exit`, `async_dist`,
/// `message_queue_data`, `error_handler`, `priority`, `monitor_nodes`, and
/// `{monitor_nodes, Options}`: set the flag of the calling process and
/// return the old value. `trap_exit` and `async_dist` take `true` or
/// `false` (`bif.c:2032-2044`), and the start value of `async_dist` is
/// `false`, the value without `+pad true` (`erl_process.c:12552-12553`).
/// `monitor_nodes` and `{monitor_nodes, Options}` take `true` or `false`
/// and return the old count of the node monitor (`bif.c:2202-2227`,
/// `monitor_nodes`).
/// `message_queue_data` takes `on_heap` or `off_heap`
/// (`erts_change_message_queue_management`, `erl_message.c:1022-1100`), and
/// its start value is `on_heap`. `error_handler` takes an atom and sets the
/// module that the export entry calls for a function that a module does not
/// export (`process_flag_2`, `bif.c:2045-2051`;
/// `erts_proc_set_error_handler`, `erl_process.h:2432-2446`); its start
/// value is the atom `error_handler`. `priority` takes `max`, `high`,
/// `normal`, or `low` (`bif.c:2052-2056`; `erts_set_process_priority`,
/// `erl_process.c:9456-9547`), and its start value is `normal`. The
/// scheduler reads the new priority when the process next goes into the
/// run queue. Each other value raises `badarg`. A
/// flag that ERTS does not know raises `badarg`, also a tuple that is not
/// `{monitor_nodes, Options}`. PBeam has no distribution, so `async_dist`
/// has no other effect and a node monitor never gets a message.
///
/// Each flag of `UNSUPPORTED_PROCESS_FLAGS` except `max_heap_size` first
/// checks its value as ERTS does, and raises `badarg` for a value that is
/// not valid: `sensitive` takes a boolean, `save_calls` takes a small integer from 0 to
/// 10000 (`process_flag_aux`, `bif.c:1907-1915`), and `scheduler`,
/// `fullsweep_after`, `min_heap_size`, and `min_bin_vheap_size` take a small
/// integer of 0 or more.
///
/// PBeam has no process heap, so `message_queue_data` does not change where
/// the messages are kept.
///
/// # Examples
///
/// ```rust,ignore
/// let old = process_flag(&mut context, &Term::static_atom("trap_exit"), &Term::static_atom("true"))?;
/// assert!(matches!(old, BifResult::Value(Term::Atom(atom)) if atom.is_static("false"));
/// let old = process_flag(&mut context, &Term::static_atom("message_queue_data"), &Term::static_atom("off_heap"))?;
/// assert!(matches!(old, BifResult::Value(Term::Atom(atom)) if atom.is_static("on_heap"));
/// let old = process_flag(&mut context, &Term::static_atom("error_handler"), &Term::static_atom("lists"))?;
/// assert!(matches!(old, BifResult::Value(Term::Atom(atom)) if atom.is_static("error_handler"));
/// assert!(matches!(process_flag(&mut context, &foo, &Term::static_atom("true"))?, BifResult::Raise { .. }));
/// let old = process_flag(&mut context, &Term::static_atom("priority"), &Term::static_atom("high"))?;
/// assert!(matches!(old, BifResult::Value(Term::Atom(atom)) if atom.is_static("normal"));
/// assert!(process_flag(&mut context, &Term::static_atom("sensitive"), &Term::static_atom("true")).is_err());
/// ```
///
/// # Errors
///
/// Each flag of `UNSUPPORTED_PROCESS_FLAGS` gives a runtime error for a
/// value that passes the check, and `max_heap_size` gives it for each
/// value. ERTS raises `badarg`
/// for a `scheduler` value above the scheduler count, which PBeam does not
/// model, and PBeam gives the runtime error.
pub(super) fn process_flag(
    context: &mut BifContext<'_>,
    flag: &Term,
    value: &Term,
) -> Result<BifResult, BeamError> {
    let term = value;
    let value = match value {
        Term::Atom(value) => Some(context.scheduler.atoms.name_of(value).to_owned()),
        _ => None,
    };
    let value = value.as_deref();
    let process = context.scheduler.current_process_mut();
    Ok(match flag {
        Term::Atom(flag) if flag.is_static("trap_exit") => {
            boolean_flag(&mut process.trap_exit, value)
        }
        Term::Atom(flag) if flag.is_static("async_dist") => {
            boolean_flag(&mut process.async_dist, value)
        }
        Term::Atom(flag) if flag.is_static("monitor_nodes") => {
            monitor_nodes(process, &Term::nil(), value)
        }
        Term::Tuple(elements) if matches!(elements.as_ref(), [Term::Atom(tag), _] if tag.is_static("monitor_nodes")) => {
            monitor_nodes(process, &elements[1], value)
        }
        Term::Atom(flag) if flag.is_static("message_queue_data") => {
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
        Term::Atom(flag) if flag.is_static("error_handler") => {
            // `process_flag_2` takes an atom for `error_handler` and gives
            // `badarg` for each other value (`bif.c:2045-2051`), and
            // `erts_proc_set_error_handler` returns the old handler
            // (`erl_process.h:2432-2446`).
            let Some(value) = value else {
                return Ok(BifResult::badarg());
            };
            // The name of the old handler goes into the atom table of the
            // run, which holds the name of a handler that a module gives.
            let (name, handler) = {
                let process = context.scheduler.current_process_mut();
                (
                    process
                        .error_handler
                        .clone()
                        .unwrap_or_else(|| DEFAULT_ERROR_HANDLER.to_owned()),
                    (value != DEFAULT_ERROR_HANDLER).then(|| value.to_owned()),
                )
            };
            let old = Term::Atom(context.scheduler.atoms.put(&name)?);
            context.scheduler.current_process_mut().error_handler = handler;
            Value(old)
        }
        Term::Atom(flag) if flag.is_static("priority") => {
            let Some(priority) = value.and_then(Priority::from_name) else {
                return Ok(BifResult::badarg());
            };
            let old = std::mem::replace(&mut process.priority, priority);
            Value(Term::static_atom(old.name()))
        }
        Term::Atom(flag)
            if UNSUPPORTED_PROCESS_FLAGS.contains(&context.scheduler.atoms.name_of(flag)) =>
        {
            let small = term.small_integer();
            let valid = if flag.is_static("sensitive") {
                matches!(value, Some("true" | "false"))
            } else if flag.is_static("save_calls") {
                small.is_some_and(|calls| (0..=10_000).contains(&calls))
            } else {
                // The name `max_heap_size` takes every value. Each other name
                // needs a small integer that is not negative.
                flag.is_static("max_heap_size") || small.is_some_and(|size| size >= 0)
            };
            if !valid {
                return Ok(BifResult::badarg());
            }
            return Err(error(format!(
                "unsupported erlang:process_flag/2 flag {}",
                context.scheduler.atoms.name_of(flag)
            )));
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
/// assert!(matches!(old, BifResult::Value(Term::Atom(atom)) if atom.is_static("logger"));
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
        Term::Atom(logger) => logger.is_one_of(&["logger", "undefined"]),
        _ => false,
    };
    let atoms = &context.scheduler.atoms;
    let valid = match (atoms.name_of(item), value) {
        ("system_logger", _) => logger,
        ("debug_flags", _) => true,
        ("backtrace_depth", _) => value.small_integer().is_some_and(|depth| depth >= 0),
        ("time_offset", Term::Atom(finalize)) => atoms.name_of(finalize) == "finalize",
        (name, _) if UNSUPPORTED_SYSTEM_FLAGS.contains(&name) => {
            unsupported_system_flag_value(name, atoms, value)
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
    Ok(
        match (scheduler.atoms.name_of(item), value.small_integer()) {
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
        },
    )
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
/// assert!(!unsupported_system_flag_value("min_heap_size", &AtomTable::new(), &Term::static_atom("foo")));
/// assert!(unsupported_system_flag_value("min_heap_size", &AtomTable::new(), &Term::Integer(8)));
/// assert!(!unsupported_system_flag_value("schedulers_online", &AtomTable::new(), &Term::Integer(0)));
/// ```
fn unsupported_system_flag_value(name: &str, atoms: &AtomTable, value: &Term) -> bool {
    let atom = match value {
        Term::Atom(atom) => Some(atoms.name_of(atom)),
        _ => None,
    };
    let integer = match value {
        Term::Integer(integer) => Some(*integer),
        _ => None,
    };
    let small = value.small_integer();
    let cpu = |cpu: &Term| match cpu {
        Term::Tuple(elements) => {
            matches!(elements.as_ref(), [Term::Atom(tag), _, _, _, _, _, _] if tag.is_static("cpu"))
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
                            |flag| matches!(flag, Term::Atom(flag) if flag.is_one_of(&["true", "false"])),
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

/// Run `erlang:exit(Reason)` as ERTS `exit_1` (`bif.c:1536-1540`) does:
/// raise an exception of class `exit` with `Reason`. The stack trace is the
/// stack trace of [`error_1`]: ERTS `save_stacktrace` saves it for each
/// class, and the first frame is the function that calls the BIF, with its
/// arity (`beam_common.c:454`, `880-935`). An uncaught `exit` exits the
/// process with `Reason` alone.
///
/// # Examples
///
/// ```rust,ignore
/// // In `tail/1`: exit(r).
/// let result = exit_1(context, &Term::static_atom("r"))?;
/// // The stack trace is [{Module, tail, 1, []}].
/// assert!(matches!(result, BifResult::RaiseWithStacktrace { class: ExceptionClass::Exit, .. }));
/// ```
///
/// # Errors
///
/// A caller instruction with no `func_info` before it is a runtime
/// invariant error.
pub(super) fn exit_1(context: &mut BifContext<'_>, reason: &Term) -> Result<BifResult, BeamError> {
    raise_with_args(
        context,
        ExceptionClass::Exit,
        reason,
        &Term::static_atom("true"),
        Term::nil(),
    )
}

/// Run `erlang:throw(Reason)` as ERTS `throw_1` (`bif.c:4132-4136`) does:
/// raise an exception of class `throw` with `Reason`, with the stack trace
/// of [`exit_1`]. An uncaught `throw` exits the process with
/// `{{nocatch, Reason}, Stacktrace}`.
///
/// # Examples
///
/// ```rust,ignore
/// // In `tail/1`: throw(r).
/// let result = throw_1(context, &Term::static_atom("r"))?;
/// // The stack trace is [{Module, tail, 1, []}].
/// assert!(matches!(result, BifResult::RaiseWithStacktrace { class: ExceptionClass::Throw, .. }));
/// ```
///
/// # Errors
///
/// A caller instruction with no `func_info` before it is a runtime
/// invariant error.
pub(super) fn throw_1(context: &mut BifContext<'_>, reason: &Term) -> Result<BifResult, BeamError> {
    raise_with_args(
        context,
        ExceptionClass::Throw,
        reason,
        &Term::static_atom("true"),
        Term::nil(),
    )
}

/// Run `erlang:error(Reason, Args)` as ERTS `error_2` (`bif.c:1448-1454`)
/// does: raise an exception of class `error` with `Reason`. ERTS puts
/// `Args` in the first frame of the stack trace, the frame of the function
/// that calls the BIF (`beam_common.c:478-495`, `903-911`, `1011-1027`). The
/// frame is `{Module, Function, Args, Location}` when `Args` is a list, and
/// `{Module, Function, Arity, Location}` for each other term
/// (`erts_build_mfa_item`, `utils.c:3448-3455`).
///
/// The caller frames follow, as `mfa_stacktrace` gathers them. PBeam keeps
/// no line table (row "Exception stack trace" in
/// `docs/erts-differences.md`), so each location is `[]`. OTP code such as
/// `re:ucompile/2` and `unicode:fake_stacktrace/3` matches the first frame.
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
    raise_with_args(context, ExceptionClass::Error, reason, args, Term::nil())
}

/// Run `erlang:error(Reason, Args, Options)` as ERTS `error_3`
/// (`bif.c:1462-1503`) does: raise an exception of class `error` with
/// `Reason`, as `erlang:error(Reason, Args)` does. When `Options` is a
/// proper list that is not empty and each element is `{error_info, Map}`
/// with a map `Map`, the location of the first frame is
/// `[{error_info, Map}]` of the last element (`save_stacktrace`,
/// `beam_common.c:1030-1051`). ERTS puts `{file, File}` and `{line, Line}`
/// before it (`erts_build_mfa_item`, `utils.c:3430-3445`); PBeam keeps no
/// line table. For each other `Options` term, the location is `[]`, and the
/// BIF does not raise `badarg`.
///
/// # Examples
///
/// ```rust,ignore
/// // In `tail/1`: erlang:error(r, [a], [{error_info, #{}}]).
/// let result = error_3(context, &Term::static_atom("r"), &args, &options)?;
/// // The stack trace is [{Module, tail, [a], [{error_info, #{}}]} | Callers].
/// assert!(matches!(result, BifResult::RaiseWithStacktrace { .. }));
/// ```
///
/// # Errors
///
/// A caller instruction with no `func_info` before it is a runtime
/// invariant error.
pub(super) fn error_3(
    context: &mut BifContext<'_>,
    reason: &Term,
    args: &Term,
    options: &Term,
) -> Result<BifResult, BeamError> {
    raise_with_args(
        context,
        ExceptionClass::Error,
        reason,
        args,
        error_info_location(options),
    )
}

/// Return the location of the first frame of the stack trace of
/// `erlang:error(Reason, Args, Options)`, as ERTS `error_3`
/// (`bif.c:1462-1503`) reads `Options`: `[{error_info, Map}]` of the last
/// element when `Options` is a proper list that is not empty and each
/// element is `{error_info, Map}` with a map `Map`, else `[]`. The tail
/// call of the fun `fun erlang:error/3` also uses it.
///
/// # Examples
///
/// ```rust,ignore
/// // [{error_info, #{a => 1}}, {error_info, #{b => 2}}] gives [{error_info, #{b => 2}}].
/// assert_eq!(error_info_location(&options), Term::list_from(vec![last]));
/// assert_eq!(error_info_location(&Term::static_atom("bad")), Term::nil());
/// ```
pub(in crate::runtime::interpreter) fn error_info_location(options: &Term) -> Term {
    let error_info = options
        .proper_list()
        .and_then(|mut options| {
            options.try_fold(None, |_, option| match option {
                Term::Tuple(pair)
                    if matches!(pair.as_ref(), [Term::Atom(tag), Term::Map(_)] if tag.is_static("error_info")) =>
                {
                    Some(Some(option.clone()))
                }
                _ => None,
            })
        })
        .flatten();
    Term::list_from(error_info)
}

/// Raise an exception of `class` with `Reason` and the stack trace of the
/// frames of the process, whose first frame holds `Args` and `Location`,
/// for `erlang:error/1,2,3`, `erlang:exit/1`, and `erlang:throw/1`.
fn raise_with_args(
    context: &mut BifContext<'_>,
    class: ExceptionClass,
    reason: &Term,
    args: &Term,
    location: Term,
) -> Result<BifResult, BeamError> {
    // A process that starts at the BIF has no frame. Its tail apply from the
    // initial function of ERTS (`i_apply_only`, `erl_process.c:12616`) puts
    // the continuation of `beam_normal_exit` as the caller
    // (`apply_bif_error_adjustment`, `beam_common.c:1345-1365`), and the stack
    // trace has no frame.
    if context.scheduler.current_process_mut().frames.is_empty() {
        return Ok(BifResult::RaiseWithStacktrace {
            class,
            reason: reason.clone(),
            stacktrace: Term::nil(),
        });
    }
    let module_id = current_frame(context.scheduler.current_process_mut()).module_id;
    let module = Arc::clone(context.scheduler.code.module_by_id(module_id)?);
    let (module, function, arity) = current_function(context.scheduler, &module)?;
    let args = if args.is_list() {
        args.clone()
    } else {
        Term::Integer(arity.into())
    };
    // ERTS saves the current frame and at most `backtrace_depth - 1` caller
    // frames of the process when the exception starts
    // (`save_stacktrace`, `beam_common.c:880-935`), and the frame of the
    // function that calls the BIF gets `Args`
    // (`beam_common.c:454-495`, `1011-1027`; `erts_build_mfa_item`,
    // `utils.c:3448-3455`). The stack trace of the exception keeps at most
    // `backtrace_depth` frames, and one frame for the depth 0
    // (`bif.c:1626-1637`). `error_handler:crash/1` calls `tl/1` on this
    // stack trace (`error_handler.erl:172-178`).
    let depth = context.scheduler.backtrace_depth;
    let mut frames = mfa_stacktrace(context.scheduler, 0)?;
    frames.truncate(depth.max(1));
    // The apply of the first frame of a process that starts at
    // `erlang:apply/2,3` gives no frame, as in ERTS.
    if let Some(first) = frames.first_mut() {
        *first = Term::tuple(vec![module, function, args, location]);
    }
    Ok(BifResult::RaiseWithStacktrace {
        class,
        reason: reason.clone(),
        stacktrace: Term::list_from(frames),
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
/// assert!(matches!(result, BifResult::Value(Term::Atom(atom)) if atom.is_static("badarg"));
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

/// Run `erlang:link(Id)` for a local pid or port, as ERTS `link_opt`
/// (`bif.c:136-232`, `411-423`) does through [`Scheduler::link`] and
/// [`Scheduler::link_port`]: return `true`, or raise `noproc` for a process
/// that does not exist or exits, or a port that is not open, when the
/// caller does not trap exits. A caller that traps exits gets `{'EXIT',
/// Port, noproc}` from a port that is not open. Each other term raises
/// `badarg`.
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
/// assert!(matches!(link(scheduler, &Term::Pid(child))?, BifResult::Value(Term::Atom(atom)) if atom.is_static("true"));
/// assert!(matches!(link(scheduler, &Term::Pid(dead))?, BifResult::Raise { .. }));
/// ```
///
/// # Errors
///
/// A new `capability_denied` atom in a full atom table gives the error of
/// `AtomTable::put`.
pub(super) fn link(scheduler: &mut Scheduler, target: &Term) -> Result<BifResult, BeamError> {
    let target = match *target {
        Term::Pid(pid) => LinkId::Process(pid),
        Term::Port(port) => {
            if scheduler.remote_port_owner(port).is_some() {
                return Err(error(
                    "unsupported: a port of another scheduler in a run of execute_on_schedulers",
                ));
            }
            LinkId::Port(port)
        }
        _ => return Ok(BifResult::badarg()),
    };
    if let Some(denied) = denial(scheduler, CapabilityKind::Process, Operation::Link, None)? {
        return Ok(denied);
    }
    let linked = match target {
        LinkId::Process(target) => {
            if let Some(denied) = boot_denial(scheduler, target)? {
                return Ok(denied);
            }
            scheduler.link(target)
        }
        LinkId::Port(port) => scheduler.link_port(port),
    };
    Ok(if linked {
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
/// assert!(matches!(result, BifResult::Value(Term::Atom(atom)) if atom.is_static("true"));
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
/// assert!(matches!(result, BifResult::Value(Term::Atom(atom)) if atom.is_static("true"));
/// let info = Term::list_from(vec![Term::static_atom("info")]);
/// let result = demonitor(&mut context, &Term::Reference(reference), &info);
/// assert!(matches!(result, BifResult::Value(Term::Atom(atom)) if atom.is_static("false"));
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
            Term::Atom(atom) if atom.is_static("flush") => flush = true,
            Term::Atom(atom) if atom.is_static("info") => info = true,
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
    // The alias owners stay with each scheduler, so a message to an alias
    // of another scheduler would be dropped.
    if context.scheduler.shared().is_some() && options.alias.is_some() {
        return Err(error(
            "unsupported: an alias from a monitor in a run of execute_on_schedulers",
        ));
    }
    let Term::Atom(kind) = kind else {
        return Ok(BifResult::badarg());
    };
    // An `Err` is the runtime error of a valid target that PBeam does not
    // support. It comes after the capability check.
    let target = match (context.scheduler.atoms.name_of(kind), target) {
        ("process", Term::Pid(id)) => Some(Ok(MonitorTarget::Process(*id))),
        ("process", target) => local_name(target).map(|name| Ok(MonitorTarget::ProcessName(name))),
        ("port", Term::Port(port)) if context.scheduler.remote_port_owner(*port).is_some() => {
            Some(Err(error(
                "unsupported: a port of another scheduler in a run of execute_on_schedulers",
            )))
        }
        ("port", Term::Port(port)) => Some(Ok(MonitorTarget::Port(*port))),
        ("port", target) => local_name(target).map(|name| Ok(MonitorTarget::PortName(name))),
        ("time_offset", Term::Atom(service)) if service.is_static("clock_service") => {
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
            Term::Atom(option) if option.is_static("priority") => result.priority = true,
            Term::Tuple(pair) => match pair.as_ref() {
                [Term::Atom(key), Term::Atom(mode)] if key.is_static("alias") => {
                    result.alias = Some(if mode.is_static("explicit_unalias") {
                        AliasMode::ExplicitUnalias
                    } else if mode.is_static("demonitor") {
                        AliasMode::Demonitor
                    } else if mode.is_static("reply_demonitor") {
                        AliasMode::ReplyDemonitor
                    } else {
                        return None;
                    });
                }
                [Term::Atom(key), tag] if key.is_static("tag") => result.tag = tag.clone(),
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
/// `link` links the new process and `monitor` monitors it. The option
/// `{monitor, MonitorOpts}` monitors it with the `tag` of `MonitorOpts`,
/// which `code_server:schedule_on_load/4` gives (code_server.erl:1396), and
/// `MonitorOpts` that `erlang:monitor/3` rejects raise `badarg`
/// (`erl_process.c:12406-12418`). An `alias` or `priority` monitor option
/// gives a runtime error. With a monitor the result is `{Pid, Reference}`,
/// else `Pid`. The option
/// `{message_queue_data, on_heap | off_heap}` sets the start value of the
/// `message_queue_data` flag of the new process, and the last such option
/// wins (`erl_process.c:12305-12319`). The option `{priority, Level}` sets
/// the start priority of the new process, else it is `normal`
/// (`erl_process.c:12246`, `12275-12287`). `options` that are not a proper
/// list, and a `message_queue_data` or `priority` option with another
/// value, raise `badarg` with the `error_info` cause `badopt`, as
/// `erts_parse_spawn_opts` (`erl_process.c:12222-12434`) and
/// `spawn_opt_4` (`bif.c:1040-1051`) do. The PBeam
/// option `{capabilities, Map}` restricts the capabilities of the new process
/// (see `capability::restrictions`), and the last such option wins. A `Map`
/// that is not valid raises the same `badarg`. Any other option gives a runtime
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
        return Ok(BifResult::badopt());
    };
    let mut spawn_options = SpawnOptions::default();
    for option in options {
        match option {
            Term::Atom(option) if option.is_static("link") => spawn_options.link = true,
            Term::Atom(option) if option.is_static("monitor") => {
                spawn_options.monitor = Some(Term::static_atom("DOWN"));
            }
            Term::Tuple(option) if matches!(option.as_ref(), [Term::Atom(name), _] if name.is_static("monitor")) =>
            {
                let Some(monitor) = monitor_options(&option[1]) else {
                    return Ok(BifResult::badopt());
                };
                if monitor.alias.is_some() || monitor.priority {
                    return Err(error(format!(
                        "unsupported spawn option {}",
                        debug_term(&Term::Tuple(option.clone()), &context.scheduler.atoms)
                    )));
                }
                spawn_options.monitor = Some(monitor.tag);
            }
            Term::Tuple(option) if matches!(option.as_ref(), [Term::Atom(name), _] if name.is_static("message_queue_data")) =>
            {
                spawn_options.flags.off_heap_message_queue = match &option[1] {
                    Term::Atom(value) if value.is_static("on_heap") => false,
                    Term::Atom(value) if value.is_static("off_heap") => true,
                    _ => return Ok(BifResult::badopt()),
                };
            }
            Term::Tuple(option) if matches!(option.as_ref(), [Term::Atom(name), _] if name.is_static("priority")) =>
            {
                // `erts_parse_spawn_opts` takes `max`, `high`, `normal`, or
                // `low`, and gives `badarg` for each other value
                // (`erl_process.c:12275-12287`).
                let priority = match &option[1] {
                    Term::Atom(value) => {
                        Priority::from_name(context.scheduler.atoms.name_of(value))
                    }
                    _ => None,
                };
                let Some(priority) = priority else {
                    return Ok(BifResult::badopt());
                };
                spawn_options.flags.priority = priority;
            }
            Term::Tuple(option) if matches!(option.as_ref(), [Term::Atom(name), _] if context.scheduler.atoms.name_of(name) == "capabilities") =>
            {
                let Some(restrictions) =
                    capability::restrictions(&context.scheduler.atoms, &option[1])
                else {
                    return Ok(BifResult::badopt());
                };
                spawn_options.capabilities = Some(restrictions);
            }
            // ponytail: ERTS accepts more options; see "Spawn options" in docs/erts-differences.md.
            _ => {
                return Err(error(format!(
                    "unsupported spawn option {}",
                    debug_term(option, &context.scheduler.atoms)
                )));
            }
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
/// as ERTS `pd_hash_put` (`erl_process_dict.c:628-760`) does. Keys compare
/// with exact equality.
///
/// # Examples
///
/// ```rust,ignore
/// let key = Term::static_atom("ok");
/// assert!(matches!(put(&mut context, &key, &Term::Integer(1)), BifResult::Value(Term::Atom(atom)) if atom.is_static("undefined"));
/// assert!(matches!(put(&mut context, &key, &Term::Integer(2)), BifResult::Value(Term::Integer(1))));
/// ```
pub(super) fn put(context: &mut BifContext<'_>, key: &Term, value: &Term) -> BifResult {
    let dictionary = &mut context.scheduler.current_process_mut().dictionary;
    Value(or_undefined(dictionary.put(key.clone(), value.clone())))
}

/// Run `erlang:get/1`: return the value of `key` in the process dictionary
/// of the calling process, or `undefined` when the key has no value, as ERTS
/// `erts_pd_hash_get` (`erl_process_dict.c:463-498`) does. Keys compare with
/// exact equality.
///
/// # Examples
///
/// ```rust,ignore
/// let key = Term::static_atom("ok");
/// assert!(matches!(get(&mut context, &key), BifResult::Value(Term::Atom(atom)) if atom.is_static("undefined"));
/// put(&mut context, &key, &Term::Integer(1));
/// assert!(matches!(get(&mut context, &key), BifResult::Value(Term::Integer(1))));
/// ```
pub(super) fn get(context: &mut BifContext<'_>, key: &Term) -> BifResult {
    let dictionary = &context.scheduler.current_process_mut().dictionary;
    Value(or_undefined(dictionary.get(key).cloned()))
}

/// Run `erlang:get/0`: return the `{Key, Value}` pairs of the process
/// dictionary of the calling process, in the order of ERTS `pd_hash_get_all`
/// (`erl_process_dict.c:578-626`).
///
/// # Examples
///
/// ```rust,ignore
/// put(&mut context, &Term::Integer(1), &Term::Integer(2));
/// // get_all(&mut context) is [{1,2}].
/// ```
pub(super) fn get_all(context: &mut BifContext<'_>) -> BifResult {
    let dictionary = &context.scheduler.current_process_mut().dictionary;
    Value(Term::list_from(dictionary.pairs().map(|(key, value)| {
        Term::tuple(vec![key.clone(), value.clone()])
    })))
}

/// Run `erlang:get_keys/0`: return the keys of the process dictionary of the
/// calling process, in the order of ERTS `pd_hash_get_all_keys`
/// (`erl_process_dict.c:508-539`).
///
/// # Examples
///
/// ```rust,ignore
/// put(&mut context, &Term::Integer(1), &Term::Integer(2));
/// // get_keys(&mut context) is [1].
/// ```
pub(super) fn get_keys(context: &mut BifContext<'_>) -> BifResult {
    let dictionary = &context.scheduler.current_process_mut().dictionary;
    Value(Term::list_from(
        dictionary.pairs().map(|(key, _)| key.clone()),
    ))
}

/// Run `erlang:get_keys/1`: return the keys whose value is exactly equal to
/// `value`, in the order of ERTS `pd_hash_get_keys`
/// (`erl_process_dict.c:542-575`).
///
/// # Examples
///
/// ```rust,ignore
/// put(&mut context, &Term::Integer(1), &Term::Integer(2));
/// // get_keys_of(&mut context, &Term::Integer(2)) is [1].
/// ```
pub(super) fn get_keys_of(context: &mut BifContext<'_>, value: &Term) -> BifResult {
    let dictionary = &context.scheduler.current_process_mut().dictionary;
    Value(Term::list_from(
        dictionary
            .pairs()
            .filter(|(_, other)| other == value)
            .map(|(key, _)| key.clone()),
    ))
}

/// Run `erlang:erase/0`: remove all pairs of the process dictionary of the
/// calling process and return them as `get/0` does, as ERTS `erase_0`
/// (`erl_process_dict.c:363-371`) does.
///
/// # Examples
///
/// ```rust,ignore
/// put(&mut context, &Term::Integer(1), &Term::Integer(2));
/// // erase_all(&mut context) is [{1,2}], and the dictionary is empty.
/// ```
pub(super) fn erase_all(context: &mut BifContext<'_>) -> BifResult {
    let dictionary = &mut context.scheduler.current_process_mut().dictionary;
    Value(Term::list_from(
        dictionary
            .erase_all()
            .into_iter()
            .map(|(key, value)| Term::tuple(vec![key, value])),
    ))
}

/// Run `erlang:erase/1`: remove `key` from the process dictionary of the
/// calling process and return its value, or `undefined`, as ERTS
/// `pd_hash_erase` (`erl_process_dict.c:385-436`) does.
///
/// # Examples
///
/// ```rust,ignore
/// put(&mut context, &Term::Integer(1), &Term::Integer(2));
/// assert!(matches!(erase(&mut context, &Term::Integer(1)), BifResult::Value(Term::Integer(2))));
/// ```
pub(super) fn erase(context: &mut BifContext<'_>, key: &Term) -> BifResult {
    let dictionary = &mut context.scheduler.current_process_mut().dictionary;
    Value(or_undefined(dictionary.erase(key)))
}

/// Return `value`, or `undefined` for no value.
///
/// # Examples
///
/// ```rust,ignore
/// assert!(matches!(or_undefined(None), Term::Atom(atom) if atom.is_static("undefined")));
/// ```
fn or_undefined(value: Option<Term>) -> Term {
    value.unwrap_or_else(|| Term::static_atom("undefined"))
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

/// Run `erlang:bit_size/1`: return the number of bits of a binary, as
/// ERTS `bit_size_1` (`erl_bif_guard.c:354-371`) and the JIT
/// `emit_bif_bit_size` (`jit/arm/instr_guard_bifs.cpp:336-441`) do. A term
/// that is not a bitstring raises `badarg`. In a guard, the `gc_bif`
/// instruction goes to its fail label instead. PBeam has no bitstring that
/// is not a binary (row "Bitstring terms" of `docs/erts-differences.md`),
/// so the size is 8 times the byte size. The size comes from the view, so
/// a binary that an append built is not joined.
///
/// # Examples
///
/// ```rust,ignore
/// let hello = Term::binary(b"hello".to_vec());
/// assert!(matches!(bit_size(&hello), BifResult::Value(Term::Integer(40))));
/// assert!(matches!(bit_size(&Term::static_atom("foo")), BifResult::Raise { .. }));
/// ```
pub(super) fn bit_size(term: &Term) -> BifResult {
    match term {
        Term::Binary(bytes) => Value(Term::Integer(bytes.bit_size() as i64)),
        _ => BifResult::badarg(),
    }
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

/// The bytes of a chunk of `erlang:binary_to_list/1,3` for each reduction,
/// as ERTS `ERTS_B2L_BYTES_PER_REDUCTION` (`binary.c:182`).
const B2L_BYTES_PER_REDUCTION: usize = 256;

/// The budget of a call of `erlang:binary_to_list/1,3` that runs in one
/// chunk after a short budget, as ERTS `B2L_MIN_EXEC_REDS`
/// (`binary.c:38`).
const B2L_MIN_EXEC_REDS: usize = CONTEXT_REDS as usize / 4;

/// The budget at or below which `erlang:binary_to_list/1,3` switches out
/// before a short binary, as ERTS `B2L_RESCHED_REDS` (`binary.c:39`).
const B2L_RESCHED_REDS: usize = CONTEXT_REDS as usize / 40;

/// Run `erlang:binary_to_list/1`, as ERTS `binary_to_list_1`
/// (`binary.c:306-352`): `<<>>` gives `[]`, and a term that is not a
/// binary, or a bitstring that is not a whole number of bytes, raises
/// `badarg`. [`b2l_start`] makes the list.
///
/// # Examples
///
/// ```rust,ignore
/// let list = binary_to_list_1(&mut scheduler, &Term::binary(vec![1, 2]))?;
/// assert!(matches!(list, BifResult::Value(codes) if codes == Term::list_from([1, 2].map(Term::Integer))));
/// ```
///
/// # Errors
///
/// Return the runtime error of [`ProcessState::bif_reductions_left`] for a
/// negative budget.
pub(super) fn binary_to_list_1(
    scheduler: &mut Scheduler,
    term: &Term,
) -> Result<BifResult, BeamError> {
    match term {
        Term::Binary(binary) if binary.bit_size() == 0 => Ok(Value(Term::nil())),
        Term::Binary(binary) if binary.bit_size() % 8 == 0 => {
            let size = binary.bit_size() / 8;
            b2l_start(
                scheduler,
                "binary_to_list",
                vec![term.clone()],
                binary,
                0,
                size,
                size,
            )
        }
        _ => Ok(BifResult::badarg()),
    }
}

/// Run `erlang:binary_to_list/3`, as ERTS `binary_to_list_3`
/// (`binary.c:354-411`): the bytes from the 1-based position `start` to
/// `stop`. A term that is not a binary, a bitstring that is not a whole
/// number of bytes, a position that is not an integer, and a position
/// outside `1 =< start =< stop =< byte_size(Binary)` raise `badarg`. A
/// bignum position is above each byte size. ERTS decides on one chunk with
/// the size of the whole binary, not of the part (`binary.c:385-398`), and
/// [`b2l_start`] does the same.
///
/// # Examples
///
/// ```rust,ignore
/// let list = binary_to_list_3(&mut scheduler, &Term::binary(vec![1, 2, 3]), &Term::Integer(2), &Term::Integer(3))?;
/// assert!(matches!(list, BifResult::Value(codes) if codes == Term::list_from([2, 3].map(Term::Integer))));
/// ```
///
/// # Errors
///
/// Return the runtime error of [`ProcessState::bif_reductions_left`] for a
/// negative budget.
pub(super) fn binary_to_list_3(
    scheduler: &mut Scheduler,
    term: &Term,
    start: &Term,
    stop: &Term,
) -> Result<BifResult, BeamError> {
    let (Term::Binary(binary), Term::Integer(first), Term::Integer(last)) = (term, start, stop)
    else {
        return Ok(BifResult::badarg());
    };
    let bits = binary.bit_size();
    let size = (bits / 8) as i64;
    if bits % 8 != 0 || *first < 1 || *last > size || last < first {
        return Ok(BifResult::badarg());
    }
    let arguments = vec![term.clone(), start.clone(), stop.clone()];
    let length = (last - first + 1) as usize;
    b2l_start(
        scheduler,
        "binary_to_list",
        arguments,
        binary,
        *first as usize - 1,
        length,
        size as usize,
    )
}

/// Start the list of the `length` bytes at `offset` of `binary`, of `size`
/// bytes, for `erlang:<bif>` with `arguments`, as ERTS `binary_to_list`
/// does (`binary.c:258-290`, `329-351`). A binary of fewer than 256 bytes
/// for each reduction left is one chunk. A binary of fewer than
/// `B2L_MIN_EXEC_REDS * 256` bytes is also one chunk with a budget of 1000.
/// With 100 or fewer reductions left, it first switches out: the call
/// uses the rest of its budget as virtual reductions and runs again from
/// the start in the next turn (`ERTS_BIF_YIELD1`, `ERTS_BIF_YIELD3`). One
/// chunk costs one reduction for each 256 bytes. Each other binary goes
/// to [`b2l_chunk`].
///
/// # Errors
///
/// Return the runtime error of [`ProcessState::bif_reductions_left`] for a
/// negative budget.
fn b2l_start(
    scheduler: &mut Scheduler,
    bif: &'static str,
    arguments: Vec<Term>,
    binary: &Binary,
    offset: usize,
    length: usize,
    size: usize,
) -> Result<BifResult, BeamError> {
    let process = scheduler.current_process_mut();
    let reds_left = process.bif_reductions_left()?;
    let short = size < B2L_MIN_EXEC_REDS * B2L_BYTES_PER_REDUCTION;
    if size >= reds_left * B2L_BYTES_PER_REDUCTION && !short {
        return Ok(b2l_chunk(
            process,
            binary,
            offset,
            length,
            Term::nil(),
            reds_left,
        ));
    }
    if size >= reds_left * B2L_BYTES_PER_REDUCTION && reds_left <= B2L_RESCHED_REDS {
        let fcalls = process.fcalls;
        scheduler.vbump_reductions(fcalls);
        return Ok(BifResult::TrapBif {
            module: "erlang",
            function: bif,
            arguments,
        });
    }
    process.bump_reductions(((length - 1) / B2L_BYTES_PER_REDUCTION + 1) as i32);
    Ok(Value(b2l_cons(
        binary,
        offset,
        offset + length,
        Term::nil(),
    )))
}

/// Make the list of the last chunk of the `remaining` bytes at `offset` of
/// the binary `binary` in front of the list `tail`, with the budget
/// `reds_left`, as ERTS `binary_to_list_chunk` does (`binary.c:201-256`):
/// the chunk has `(reds_left + 1) * 256` bytes or fewer, and costs one
/// reduction for each 256 bytes. When bytes remain, the call traps to the
/// export `erts_internal:binary_to_list_continue`, which continues with
/// the budget of the next turn (`binary.c:292-304`). ERTS keeps the state
/// in a magic reference, and PBeam keeps the binary, the offset, the
/// remaining bytes, and the list so far as terms. The trap disables the
/// garbage collection of the process in ERTS, and a return after it can
/// switch out for a pending collection; PBeam has no process heap, so the
/// last chunk returns its list.
///
/// # Examples
///
/// ```rust,ignore
/// let list = b2l_chunk(&mut process, &Binary::from(vec![1, 2]), 0, 2, Term::nil(), 4000);
/// assert!(matches!(list, BifResult::Value(codes) if codes == Term::list_from([1, 2].map(Term::Integer))));
/// ```
pub(super) fn b2l_chunk(
    process: &mut ProcessState,
    binary: &Binary,
    offset: usize,
    remaining: usize,
    tail: Term,
    reds_left: usize,
) -> BifResult {
    let size = ((reds_left + 1) * B2L_BYTES_PER_REDUCTION).min(remaining);
    process.bump_reductions(((size - 1) / B2L_BYTES_PER_REDUCTION + 1) as i32);
    let rest = remaining - size;
    let list = b2l_cons(binary, offset + rest, offset + remaining, tail);
    if rest == 0 {
        return Value(list);
    }
    BifResult::TrapBif {
        module: "erts_internal",
        function: "binary_to_list_continue",
        arguments: vec![
            Term::Binary(binary.clone()),
            Term::Integer(offset as i64),
            Term::Integer(rest as i64),
            list,
        ],
    }
}

/// Return the list of the bytes from byte `start` to byte `end` of
/// `binary` in front of the list `tail`, as ERTS `erts_bin_bytes_to_list`
/// does. [`Binary::byte_runs`] reads the range from the chunks of a binary
/// that an append built and does not join them. One call does O(log
/// chunks) work to find the chunks of the range, O(1) work for each chunk of
/// the range, and one cons for each byte of the range, and the reductions of
/// the caller pay for the bytes.
pub(super) fn b2l_cons(binary: &Binary, start: usize, end: usize, tail: Term) -> Term {
    binary
        .byte_runs(start, end)
        .into_iter()
        .rev()
        .fold(tail, |tail, run| {
            run.iter().rev().fold(tail, |tail, byte| {
                Term::cons(Term::Integer(i64::from(*byte)), tail)
            })
        })
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

/// Run `erlang:integer_to_list/2`: return the characters of an integer in
/// `base` as a proper list, as ERTS `integer_to_list_2` and
/// `integer_to_list` (`bif.c:3266-3311`, `3341-3374`) do. A digit above 9
/// is an upper-case letter (`Sint_to_buf`, `utils.c:2994-3030`). A first
/// argument that is not an integer, and a base that is not a small integer
/// from 2 to 36, raise `badarg`. [`integer_digits`] gives the digits and
/// charges a bignum.
///
/// # Examples
///
/// ```rust,ignore
/// let list = integer_to_list_2(&mut process, &Term::Integer(-255), &Term::Integer(16))?;
/// assert!(matches!(list, BifResult::Value(codes) if codes.proper_list().is_some_and(|codes| codes.len() == 4)));
/// assert!(matches!(integer_to_list_2(&mut process, &Term::Integer(1), &Term::Integer(37))?, BifResult::Raise { .. }));
/// ```
///
/// # Errors
///
/// Return the runtime error of [`integer_digits`].
pub(super) fn integer_to_list_2(
    process: &mut ProcessState,
    term: &Term,
    base: &Term,
) -> Result<BifResult, BeamError> {
    // ERTS checks both arguments before the work (`bif.c:3346-3353`). Each
    // failed check raises `badarg` with no other effect, so the order of the
    // checks does not change the result.
    let Some(base) = base
        .small_integer()
        .and_then(|base| u32::try_from(base).ok())
        .filter(|base| (2..=36).contains(base))
    else {
        return Ok(BifResult::badarg());
    };
    let Some(digits) = integer_digits(process, term, base, "integer_to_list/2")? else {
        return Ok(BifResult::badarg());
    };
    Ok(Value(Term::list_from(
        digits.bytes().map(|byte| Term::Integer(i64::from(byte))),
    )))
}

/// Run `erlang:pid_to_list/1` and `erlang:port_to_list/1`: return the
/// characters of the text of a pid or a port as a proper list, as ERTS
/// `pid_to_list_1` and `port_to_list_1` (`bif.c:4505-4517`) do.
/// `term2list_dsprintf` (`bif.c:4442-4457`) prints the term with `%T`
/// ([`print_term`]) and turns each byte of the text into an integer
/// (`buf_to_intlist`, `utils.c:3039-3053`). ERTS prints a pid as
/// `<0.Number.Serial>` and a port as `#Port<0.Number>`. A term that is not
/// a pid, for [`pid_to_list`], and a term that is not a port, for
/// [`port_to_list`], raise `badarg`. `atoms` is the table of the run that
/// owns `term`.
///
/// # Examples
///
/// ```rust,ignore
/// let atoms = AtomTable::new();
/// let text = pid_to_list(&Term::Pid(pid), &atoms);
/// assert!(matches!(text, BifResult::Value(codes) if codes.proper_list().is_some_and(|codes| codes.len() > 0)));
/// assert!(matches!(pid_to_list(&Term::nil(), &atoms), BifResult::Raise { .. }));
/// ```
pub(super) fn pid_to_list(term: &Term, atoms: &AtomTable) -> BifResult {
    match term {
        Term::Pid(_) => term_text_list(term, atoms),
        _ => BifResult::badarg(),
    }
}

/// Run `erlang:port_to_list/1`, with the text, the checks, and the result
/// of [`pid_to_list`] for a term that is a port (`bif.c:4512-4517`).
///
/// # Examples
///
/// ```rust,ignore
/// let atoms = AtomTable::new();
/// let text = port_to_list(&Term::Port(port), &atoms);
/// assert!(matches!(text, BifResult::Value(codes) if codes.proper_list().is_some_and(|codes| codes.len() > 0)));
/// assert!(matches!(port_to_list(&Term::Pid(pid), &atoms), BifResult::Raise { .. }));
/// ```
pub(super) fn port_to_list(term: &Term, atoms: &AtomTable) -> BifResult {
    match term {
        Term::Port(_) => term_text_list(term, atoms),
        _ => BifResult::badarg(),
    }
}

/// Return the characters of the `%T` text of `term` as a proper list, the
/// result of [`pid_to_list`] and [`port_to_list`] for a term of the
/// accepted kind. [`print_term`] gives the text, and each byte of the text
/// becomes an integer, as `buf_to_intlist` (`utils.c:3039-3053`) does.
///
/// # Examples
///
/// ```rust,ignore
/// let atoms = AtomTable::new();
/// // [60, 48, 46, 49, 46, 48, 62] is the text of the pid `<0.1.0>`.
/// assert!(matches!(term_text_list(&Term::Pid(pid), &atoms), BifResult::Value(codes) if codes.proper_list().is_some_and(|codes| codes.len() == 7)));
/// ```
fn term_text_list(term: &Term, atoms: &AtomTable) -> BifResult {
    Value(Term::list_from(
        print_term(term, atoms)
            .into_iter()
            .map(|byte| Term::Integer(i64::from(byte))),
    ))
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
/// `binary.c:92-108`). A bignum from 2^59 to 2^63 - 1 costs 1 in base 10
/// and 4 in base 2.
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
    if !term.is_integer() {
        return Ok(None);
    }
    if term.small_integer().is_none() {
        let value = term.bigint().expect("the term is an integer");
        let cost =
            big_integer_estimate(value.bits(), value.sign() == num_bigint::Sign::Minus, base)
                / DIGITS_PER_RED;
        if cost > process.bif_reductions_left()? {
            // ponytail: no dirty scheduler; see "Scheduling" in docs/erts-differences.md.
            return Err(error(format!(
                "unsupported erlang:{name} of a bignum of {} bits with {} reductions left: ERTS runs it on a dirty CPU scheduler",
                value.bits(),
                process.fcalls
            )));
        }
        process.bump_reductions(cost as i32);
    }
    let Term::Integer(value) = term else {
        // `erts_big_to_list` writes a digit above 9 as an upper-case letter,
        // as `Sint_to_buf` does.
        let value = term.bigint().expect("the term is an integer");
        return Ok(Some(value.to_str_radix(base).to_ascii_uppercase()));
    };
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
/// // 2^59 has 60 bits: lg 59, 60 / 3.32193 + 1 = 19.06, then the null byte.
/// assert_eq!(big_integer_estimate(60, false, 10), 20);
/// assert_eq!(big_integer_estimate(64, true, 10), 22);
/// // 2^63 - 1 in base 2: lg 62, 63 / 1.0 + 1 = 64, then the null byte.
/// assert_eq!(big_integer_estimate(63, false, 2), 65);
/// ```
fn big_integer_estimate(bits: u64, negative: bool, base: u32) -> usize {
    /// `lg2_lookup` (`big.c:4087-4092`): the rounded `log2` of each base
    /// from 2 to 36.
    #[allow(clippy::approx_constant, reason = "ERTS uses these rounded values")]
    const LG2_LOOKUP: [f64; 35] = [
        1.0, 1.58496, 2.0, 2.32193, 2.58496, 2.80735, 3.0, 3.16993, 3.32193, 3.45943, 3.58496,
        3.70044, 3.80735, 3.90689, 4.0, 4.08746, 4.16993, 4.24793, 4.32193, 4.39232, 4.45943,
        4.52356, 4.58496, 4.64386, 4.70044, 4.75489, 4.80735, 4.85798, 4.90689, 4.9542, 5.0,
        5.04439, 5.08746, 5.12928, 5.16993,
    ];
    // `lg + 1` is the number of bits of the magnitude.
    let digits = (bits as f64 / LG2_LOOKUP[base as usize - 2] + 1.0) as usize;
    digits + usize::from(negative) + 1
}

/// Run `erlang:atom_to_list/1`: return the Unicode code points of the atom
/// name, as ERTS `atom_to_list_1` (`bif.c:3186-3210`) does. The atom `''`
/// gives `[]`. A term that is not an atom raises `badarg`. `atoms` is the
/// table of the run that owns `term`.
///
/// # Examples
///
/// ```rust,ignore
/// let atoms = AtomTable::new();
/// let list = atom_to_list(&atoms, &Term::static_atom("ok"));
/// assert!(matches!(list, BifResult::Value(codes) if codes == Term::list_from([Term::Integer(111), Term::Integer(107)])));
/// assert!(matches!(atom_to_list(&atoms, &Term::static_atom("")), BifResult::Value(codes) if codes.is_nil()));
/// assert!(matches!(atom_to_list(&atoms, &Term::nil()), BifResult::Raise { .. }));
/// ```
pub(super) fn atom_to_list(atoms: &AtomTable, term: &Term) -> BifResult {
    match term {
        Term::Atom(atom) => {
            Value(Term::list_from(atoms.name_of(atom).chars().map(
                |character| Term::Integer(i64::from(u32::from(character))),
            )))
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
/// assert!(matches!(atom_to_binary(&atoms, &pi, &Term::static_atom("utf8")), BifResult::Value(Term::Binary(bytes)) if bytes == [207, 128]));
/// assert!(matches!(atom_to_binary(&atoms, &pi, &Term::static_atom("latin1")), BifResult::Raise { .. }));
/// let e_acute = Term::Atom(atoms.put("\u{e9}")?);
/// assert!(matches!(atom_to_binary(&atoms, &e_acute, &Term::static_atom("latin1")), BifResult::Value(Term::Binary(bytes)) if bytes == [233]));
/// assert!(matches!(atom_to_binary(&atoms, &Term::static_atom("ok"), &Term::static_atom("foo")), BifResult::Raise { .. }));
/// ```
pub(super) fn atom_to_binary(atoms: &AtomTable, term: &Term, encoding: &Term) -> BifResult {
    let (Term::Atom(atom), Term::Atom(encoding)) = (term, encoding) else {
        return BifResult::badarg();
    };
    if encoding.is_one_of(&["utf8", "unicode"]) {
        Value(Term::binary(atoms.name_of(atom).as_bytes().to_vec()))
    } else if encoding.is_static("latin1") {
        atoms
            .name_of(atom)
            .chars()
            .map(|character| u8::try_from(u32::from(character)).ok())
            .collect::<Option<Vec<u8>>>()
            .map_or_else(BifResult::badarg, |bytes| Value(Term::binary(bytes)))
    } else {
        BifResult::badarg()
    }
}

/// The reason that a list does not give an atom name.
enum NameError {
    /// The term is not a proper list of characters: `badarg` in
    /// `list_to_atom/1` and `list_to_existing_atom/1`.
    Badarg,
    /// The list holds more than [`MAX_ATOM_CHARACTERS`] elements:
    /// `system_limit` in `list_to_atom/1` (`bif.c:3228-3234`) and `badarg`
    /// in `list_to_existing_atom/1` (`bif.c:3245-3248`).
    TooLong,
}

/// Return the name that the code points of `term` give, as
/// `erts_unicode_list_to_buf` (`utils.c:2862-2925`) writes it for
/// `list_to_atom_1` and `list_to_existing_atom_1` (`bif.c:3217-3236`,
/// `3239-3258`). ERTS counts each element before it checks it, so a 256th
/// element gives [`NameError::TooLong`], also when it is not a character.
/// ERTS checks the tail after the 255th element, so an improper list of 300
/// characters gives [`NameError::TooLong`]. An element that is not an
/// integer in `0..=0x10FFFF` outside `0xD800..=0xDFFF`, a tail that is not
/// `[]`, and a term that is not a list, give [`NameError::Badarg`].
///
/// # Examples
///
/// ```rust,ignore
/// assert_eq!(list_name(&Term::nil()), Ok(String::new()));
/// assert!(list_name(&Term::static_atom("ok")).is_err());
/// ```
fn list_name(term: &Term) -> Result<String, NameError> {
    let Some(mut elements) = term.list_iter() else {
        return Err(NameError::Badarg);
    };
    let mut name = String::new();
    for (count, element) in elements.by_ref().enumerate() {
        if count == MAX_ATOM_CHARACTERS {
            return Err(NameError::TooLong);
        }
        let character = match element {
            Term::Integer(code) => u32::try_from(*code).ok().and_then(char::from_u32),
            _ => None,
        };
        let Some(character) = character else {
            return Err(NameError::Badarg);
        };
        name.push(character);
    }
    if !elements.rest().is_nil() {
        return Err(NameError::Badarg);
    }
    Ok(name)
}

/// Run `erlang:list_to_atom/1`: return the atom whose name has the code
/// points of a list, and put a new atom into the atom table of the run, as
/// ERTS `list_to_atom_1` (`bif.c:3217-3236`) does. ERTS counts each element
/// before it checks it, so a 256th element raises `system_limit`, also when
/// it is not a character. ERTS checks the tail after the 255th element, so
/// an improper list of 300 characters raises `system_limit`. An element
/// that is not an integer in `0..=0x10FFFF` outside `0xD800..=0xDFFF`, a
/// tail that is not `[]`, and a term that is not a list, raise `badarg`.
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
    match list_name(term) {
        Ok(name) => Ok(Value(Term::Atom(atoms.put(&name)?))),
        Err(NameError::TooLong) => Ok(Raise {
            class: ExceptionClass::Error,
            reason: Term::static_atom("system_limit"),
        }),
        Err(NameError::Badarg) => Ok(BifResult::badarg()),
    }
}

/// Run `erlang:list_to_existing_atom/1`: return the atom whose name has the
/// code points of a list, when the run holds that name, as ERTS
/// `list_to_existing_atom_1` (`bif.c:3239-3258`) does with
/// `erts_atom_get` (`atom.c:381-433`). ERTS gives `badarg` for each result
/// below zero of the name conversion, so a list above the atom limit, a
/// term that is not a proper list of characters, and a name with no atom
/// raise `badarg`.
///
/// # Examples
///
/// ```rust,ignore
/// let atoms = AtomTable::new();
/// let empty = list_to_existing_atom(&atoms, &Term::nil());
/// assert!(matches!(empty, BifResult::Value(Term::Atom(atom)) if atoms.name_of(&atom).is_empty()));
/// let unknown = Term::list_from(vec![Term::Integer(122), Term::Integer(122)]);
/// assert!(matches!(list_to_existing_atom(&atoms, &unknown), BifResult::Raise { .. }));
/// ```
pub(super) fn list_to_existing_atom(atoms: &AtomTable, term: &Term) -> BifResult {
    match list_name(term).ok().and_then(|name| atoms.get(&name)) {
        Some(atom) => Value(Term::Atom(atom)),
        None => BifResult::badarg(),
    }
}

/// Run `erlang:binary_to_existing_atom/2`: return the atom whose name a
/// binary holds, when the run holds that name, as ERTS `binary_to_atom`
/// with `must_exist` one (`erl_unicode.c:1962-1971`, `1991-1993`,
/// `2009-2011`, `2025-2028`) does with `erts_atom_get` (`atom.c:381-433`).
/// With `latin1`, one byte is one character, and the name holds the UTF-8
/// bytes of the code points, so `<<255>>` gives the atom `'ÿ'`. With
/// `utf8` or `unicode`, the binary holds the UTF-8 bytes of the name.
///
/// An argument that is not a byte-aligned binary, an encoding that is not
/// `latin1`, `utf8`, or `unicode`, a name with no atom, and a byte string
/// that `utf8` does not accept, raise `badarg`. ERTS reads no encoding of a
/// name of a binary above the atom limits, because the lookup of such a
/// name fails before it is read (`atom.c:392-394`, `412-415`), and it gives
/// `badarg`, not `system_limit`, for a `latin1` binary above 255 bytes and
/// a `utf8` binary above `MAX_ATOM_SZ_LIMIT` bytes (`atom.c:409-411`).
///
/// # Examples
///
/// ```rust,ignore
/// let atoms = AtomTable::new();
/// let ok = binary_to_existing_atom(&atoms, &Term::binary(b"ok".to_vec()), &Term::static_atom("utf8"));
/// assert!(matches!(ok, BifResult::Value(atom) if atom == Term::static_atom("ok")));
/// let unknown = Term::binary(b"pbeam_zulu".to_vec());
/// assert!(matches!(binary_to_existing_atom(&atoms, &unknown, &Term::static_atom("utf8")), BifResult::Raise { .. }));
/// ```
pub(super) fn binary_to_existing_atom(
    atoms: &AtomTable,
    binary: &Term,
    encoding: &Term,
) -> BifResult {
    let (Term::Binary(bytes), Term::Atom(encoding)) = (binary, encoding) else {
        return BifResult::badarg();
    };
    if !bytes.bit_size().is_multiple_of(8) {
        return BifResult::badarg();
    }
    let name = if encoding.is_static("latin1") {
        if bytes.len() > MAX_ATOM_CHARACTERS {
            return BifResult::badarg();
        }
        bytes
            .iter()
            .map(|&byte| char::from(byte))
            .collect::<String>()
    } else if encoding.is_one_of(&["utf8", "unicode"]) {
        if bytes.len() > MAX_ATOM_SZ_LIMIT {
            return BifResult::badarg();
        }
        let Ok(name) = std::str::from_utf8(bytes) else {
            return BifResult::badarg();
        };
        name.to_owned()
    } else {
        return BifResult::badarg();
    };
    match atoms.get(&name) {
        Some(atom) => Value(Term::Atom(atom)),
        None => BifResult::badarg(),
    }
}

/// Run `erlang:binary_to_atom/2`: return the atom whose name a binary
/// holds, and put a new atom into the atom table of the run, as ERTS
/// `binary_to_atom` (`erl_unicode.c:1962-2033`) does with
/// `erts_atom_put_index` (`atom.c:230-334`). With `latin1`, one byte is one
/// character, and the name holds the UTF-8 bytes of the code points, so
/// `<<255>>` gives the atom `'ÿ'`. With `utf8` or `unicode`, the binary
/// holds the UTF-8 bytes of the name.
///
/// An argument that is not a byte-aligned binary, an encoding that is not
/// `latin1`, `utf8`, or `unicode`, and a byte string that the encoding does
/// not accept — an invalid UTF-8 sequence, an overlong form, or a UTF-8
/// encoded surrogate — raise `badarg`. More than 255 bytes with `latin1`,
/// more than `MAX_ATOM_SZ_LIMIT` bytes with `utf8` or `unicode`
/// (`atom.c:281-285`), and a name of more than 255 characters raise
/// `system_limit`. ERTS stops the UTF-8 analysis at the 255th character
/// (`erl_unicode.c:1260-1263`), so 255 accepted characters and more bytes
/// raise `system_limit`, also when the bytes after them are not valid
/// UTF-8, and the byte limit raises `system_limit` also when the encoding
/// does not accept the binary.
///
/// # Examples
///
/// ```rust,ignore
/// let mut atoms = AtomTable::new();
/// let ok = binary_to_atom(&mut atoms, &Term::binary(b"ok".to_vec()), &Term::static_atom("utf8"))?;
/// assert!(matches!(ok, BifResult::Value(atom) if atom == Term::static_atom("ok")));
/// let y = binary_to_atom(&mut atoms, &Term::binary(vec![255]), &Term::static_atom("latin1"))?;
/// assert!(matches!(y, BifResult::Value(Term::Atom(atom)) if atoms.name_of(&atom) == "ÿ"));
/// assert!(matches!(binary_to_atom(&mut atoms, &Term::binary(vec![255]), &Term::static_atom("utf8"))?, BifResult::Raise { .. }));
/// assert!(matches!(binary_to_atom(&mut atoms, &Term::binary(b"ok".to_vec()), &Term::static_atom("foo"))?, BifResult::Raise { .. }));
/// ```
///
/// # Errors
///
/// A new atom in a full atom table gives the error of `AtomTable::put`,
/// where ERTS stops the emulator.
pub(super) fn binary_to_atom(
    atoms: &mut AtomTable,
    binary: &Term,
    encoding: &Term,
) -> Result<BifResult, BeamError> {
    let (Term::Binary(bytes), Term::Atom(encoding)) = (binary, encoding) else {
        return Ok(BifResult::badarg());
    };
    if bytes.bit_size() % 8 != 0 {
        return Ok(BifResult::badarg());
    }
    let system_limit = || Raise {
        class: ExceptionClass::Error,
        reason: Term::static_atom("system_limit"),
    };
    let name = if encoding.is_static("latin1") {
        if bytes.len() > MAX_ATOM_CHARACTERS {
            return Ok(system_limit());
        }
        bytes
            .iter()
            .map(|&byte| char::from(byte))
            .collect::<String>()
    } else if encoding.is_one_of(&["utf8", "unicode"]) {
        if bytes.len() > MAX_ATOM_SZ_LIMIT {
            return Ok(system_limit());
        }
        let valid = std::str::from_utf8(bytes).unwrap_or_else(|invalid| {
            std::str::from_utf8(&bytes[..invalid.valid_up_to()]).expect("a valid UTF-8 prefix")
        });
        let characters = valid.chars().count();
        if characters > MAX_ATOM_CHARACTERS
            || (characters == MAX_ATOM_CHARACTERS && valid.len() < bytes.len())
        {
            return Ok(system_limit());
        }
        if valid.len() < bytes.len() {
            return Ok(BifResult::badarg());
        }
        valid.to_owned()
    } else {
        return Ok(BifResult::badarg());
    };
    Ok(Value(Term::Atom(atoms.put(&name)?)))
}

/// The byte order of the target, the result of `erlang:system_info(endian)`.
#[cfg(target_endian = "big")]
const ENDIAN: &str = "big";
/// The byte order of the target, the result of `erlang:system_info(endian)`.
#[cfg(target_endian = "little")]
const ENDIAN: &str = "little";

/// Run `erlang:system_info/1`. The item `os_type` returns `{unix, Flavor}`,
/// as ERTS `system_info_1` (`erl_bif_info.c:3026-3027`) does with `os_type`
/// `"unix"` (`sys/unix/sys.c:835`) and the OS flavor of the host. ERTS makes
/// both atoms at emulator start (`os_info_init` in
/// `erl_bif_info.c:6418-6440`); PBeam puts them into the atom table of the
/// run at this call. The item `otp_release` returns `ERLANG_OTP_RELEASE` as
/// a string (`erl_bif_info.c:3231-3234`); `build.rs` takes it from the
/// `erlang` pin in `mise.toml`. The item `schedulers` returns
/// `erts_no_schedulers` (`erl_bif_info.c:3257-3260`), which is 1: PBeam has
/// one scheduler, as ERTS with `erl +S 1`. The item `build_type` returns
/// `opt`, the result of an ERTS build without a debug or other special
/// build flag (`erl_bif_info.c:2799-2824`). The item `endian` returns
/// `big` on a big-endian build and `little` on each other build
/// (`erl_bif_info.c:3101-3106`); PBeam takes the byte order of its own
/// target, as ERTS takes `WORDS_BIGENDIAN` of its build.
///
/// # Examples
///
/// ```rust,ignore
/// let mut atoms = AtomTable::new();
/// let os_type = system_info(&mut scheduler, &Term::static_atom("os_type"))?;
/// // macOS: {unix, darwin}.
/// assert!(matches!(os_type, BifResult::Value(Term::Tuple(_))));
/// assert!(system_info(&mut scheduler, &Term::static_atom("wordsize")).is_err());
/// ```
///
/// # Errors
///
/// Each other item gives a runtime error, also an item that ERTS rejects
/// with `badarg`. A new atom in a full atom table gives the error of
/// `AtomTable::put`.
pub(super) fn system_info(scheduler: &mut Scheduler, item: &Term) -> Result<BifResult, BeamError> {
    let schedulers = scheduler.scheduler_count();
    let atoms = &mut scheduler.atoms;
    match item {
        Term::Atom(atom) if atom.is_static("os_type") => Ok(Value(Term::tuple(vec![
            Term::Atom(Atom::UNIX),
            Term::Atom(atoms.put(&host::os_flavor())?),
        ]))),
        Term::Atom(atom) if atoms.name_of(atom) == "otp_release" => Ok(Value(Term::list_from(
            env!("PBEAM_OTP_RELEASE")
                .bytes()
                .map(|byte| Term::Integer(i64::from(byte))),
        ))),
        Term::Atom(atom) if atoms.name_of(atom) == "schedulers" => {
            Ok(Value(Term::Integer(schedulers)))
        }
        // `erlang:system_flag/2` cannot change the count of online
        // schedulers, so each scheduler of the run is online.
        Term::Atom(atom) if atoms.name_of(atom) == "schedulers_online" => {
            Ok(Value(Term::Integer(schedulers)))
        }
        Term::Atom(atom) if atoms.name_of(atom) == "build_type" => {
            Ok(Value(Term::static_atom("opt")))
        }
        Term::Atom(atom) if atoms.name_of(atom) == "endian" => Ok(Value(Term::static_atom(ENDIAN))),
        // `2 * erts_no_schedulers` (`erl_init.c:1354`). `erlang:system_flag/2`
        // cannot change it in PBeam.
        Term::Atom(atom) if atom.is_static("outstanding_system_requests_limit") => {
            Ok(Value(Term::Integer(2 * schedulers)))
        }
        _ => Err(error(format!(
            "unsupported erlang:system_info/1 argument {}",
            debug_term(item, atoms)
        ))),
    }
}

/// Run a float operation on two numbers, at least one a float, or on the
/// two integers of `erlang:'/'/2`: convert each integer to the nearest
/// float, as the C cast and `big_to_double` do, and run `operation`. A
/// result that is not finite raises `badarith` (`ERTS_FP_ERROR`,
/// `erl_arith.c:487`, `erl_arith.c:1125`), as does an operand that is not a
/// number.
///
/// # Examples
///
/// ```rust,ignore
/// assert!(matches!(float_arith(&Term::Integer(7), &Term::Integer(2), |l, r| l / r), BifResult::Value(Term::Float(3.5))));
/// assert!(matches!(float_arith(&Term::Integer(1), &Term::Integer(0), |l, r| l / r), BifResult::Raise { .. }));
/// ```
pub(super) fn float_arith(left: &Term, right: &Term, operation: fn(f64, f64) -> f64) -> BifResult {
    let (Some(left), Some(right)) = (left.number_float(), right.number_float()) else {
        return BifResult::badarith();
    };
    let result = operation(left, right);
    if result.is_finite() {
        Value(Term::Float(result))
    } else {
        BifResult::badarith()
    }
}

/// Run `erlang:float/1`: return an integer as the nearest float and a float
/// as it is, as ERTS `float_1` (`erl_bif_guard.c:107-133`) does. Each other
/// term raises `badarg`.
///
/// # Examples
///
/// ```rust,ignore
/// assert!(matches!(to_float(&Term::Integer(-3)), BifResult::Value(Term::Float(-3.0))));
/// assert!(matches!(to_float(&Term::static_atom("a")), BifResult::Raise { .. }));
/// ```
pub(super) fn to_float(term: &Term) -> BifResult {
    match term.number_float() {
        Some(value) => Value(Term::Float(value)),
        None => BifResult::badarg(),
    }
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

/// Run `erlang:timestamp()`: return the system time of the run as the tuple
/// `{MegaSecs, Secs, MicroSecs}`, as `timestamp_0` (`erl_time_sup.c:2555-2587`)
/// does with `make_timestamp_value` (`erl_time_sup.c:1906-1925`). The
/// system time is the internal monotonic time plus the time offset of the
/// run, as `system_time_0` adds them (`erl_time_sup.c:2517-2525`).
///
/// # Examples
///
/// ```rust,ignore
/// // {1792,192300,76209}
/// assert!(matches!(timestamp(1_792_192_300_076_209_000), BifResult::Value(Term::Tuple(parts)) if parts.len() == 3));
/// ```
pub(super) fn timestamp(system_time: i64) -> BifResult {
    let (mega, seconds, micros) = time::make_timestamp_value(system_time);
    Value(Term::tuple(vec![
        Term::Integer(mega),
        Term::Integer(seconds),
        Term::Integer(micros),
    ]))
}

/// Run `erlang:localtime()`: return the local date and time of the host
/// time zone as `{{Year, Month, Day}, {Hour, Minute, Second}}`, as
/// `localtime_0` (`bif.c:3880-3901`) does with `get_localtime`
/// (`erl_time_sup.c:1543-1557`). The clock is the host `CLOCK_REALTIME`
/// truncated to a second, as the C library `time` gives it.
///
/// A failed `localtime_r` raises `badarg`; ERTS does not check the result
/// of `sys_localtime_r` there.
///
/// # Examples
///
/// ```rust,ignore
/// // TZ=EST5EDT,M3.2.0,M11.1.0 in January
/// assert!(matches!(localtime(), BifResult::Value(Term::Tuple(parts)) if parts.len() == 2));
/// ```
pub(super) fn localtime() -> BifResult {
    local_clock().map_or_else(BifResult::badarg, |local| Value(date_time(local)))
}

/// Run `erlang:date()`: return the local date of the host time zone as the
/// tuple `{Year, Month, Day}`, as `date_0` (`bif.c:3842-3854`) does with
/// `get_date` (`erl_time_sup.c:1527-1541`). A failed `localtime_r` raises
/// `badarg`; ERTS does not check the result of `sys_localtime_r` there.
///
/// # Examples
///
/// ```rust,ignore
/// // TZ=EST5EDT,M3.2.0,M11.1.0 in January
/// assert!(matches!(date(), BifResult::Value(Term::Tuple(parts)) if parts.len() == 3));
/// ```
pub(super) fn date() -> BifResult {
    local_clock().map_or_else(BifResult::badarg, |(year, month, day, ..)| {
        Value(Term::tuple(vec![
            Term::Integer(year),
            Term::Integer(month),
            Term::Integer(day),
        ]))
    })
}

/// Run `erlang:time()`: return the local time of the host time zone as the
/// tuple `{Hour, Minute, Second}`, as `time_0` (`bif.c:3828-3840`) does
/// with `get_time` (`erl_time_sup.c:1516-1525`). A failed `localtime_r`
/// raises `badarg`; ERTS does not check the result of `sys_localtime_r`
/// there.
///
/// # Examples
///
/// ```rust,ignore
/// // TZ=EST5EDT,M3.2.0,M11.1.0 in January
/// assert!(matches!(time_of_day(), BifResult::Value(Term::Tuple(parts)) if parts.len() == 3));
/// ```
pub(super) fn time_of_day() -> BifResult {
    local_clock().map_or_else(BifResult::badarg, |(_, _, _, hour, minute, second)| {
        Value(Term::tuple(vec![
            Term::Integer(hour),
            Term::Integer(minute),
            Term::Integer(second),
        ]))
    })
}

/// Run `erlang:universaltime()`: return the UTC date and time of the host
/// clock as `{{Year, Month, Day}, {Hour, Minute, Second}}`, as
/// `universaltime_0` (`bif.c:3856-3878`) does with `get_universaltime`
/// (`erl_time_sup.c:1559-1583`). PBeam reads the UTC date of the POSIX time
/// with [`time::seconds_to_univ`], the algorithm of the other UTC BIFs
/// (`erl_time_sup.c:1669-1702`), where ERTS calls `gmtime_r`; the two agree
/// for each second that the host clock can give (`gmtime_r` and the
/// proleptic Gregorian calendar differ before the year 1).
///
/// # Examples
///
/// ```rust,ignore
/// assert!(matches!(universaltime(), BifResult::Value(Term::Tuple(parts)) if parts.len() == 2));
/// ```
pub(super) fn universaltime() -> BifResult {
    Value(date_time(time::seconds_to_univ(local_clock_seconds())))
}

/// Return the local date and time of the host clock, or `None` when
/// `localtime_r` fails. The local time comes from `sys_localtime_r`, which
/// adds no leap second of the time zone (`erl_time_sup.c:1516-1557`).
///
/// # Examples
///
/// ```rust,ignore
/// // TZ=EST5EDT,M3.2.0,M11.1.0 in January
/// assert!(local_clock().is_some());
/// ```
fn local_clock() -> Option<time::DateTime> {
    host::clock::local_zone_time(local_clock_seconds())
}

/// Return the host `CLOCK_REALTIME` truncated to a second, as the C library
/// `time` gives it (`get_time`, `erl_time_sup.c:1516-1525`).
///
/// # Examples
///
/// ```rust,ignore
/// // 2023-11-14 and later.
/// assert!(local_clock_seconds() > 1_700_000_000);
/// ```
fn local_clock_seconds() -> i64 {
    host::clock::system_time() / 1_000_000_000
}

/// Run `erlang:monotonic_time/0`: return the Erlang monotonic time in the
/// native time unit, as ERTS `monotonic_time_0` (`erl_time_sup.c:2500-2507`)
/// does. ERTS gives `get_time(esdp) + ERTS_MONOTONIC_OFFSET_NATIVE`, which
/// is the value of `erlang:monotonic_time(native)` that [`convert_time`]
/// gives for the internal monotonic time of the run. The call charges no
/// reduction and raises nothing.
///
/// # Examples
///
/// ```rust,ignore
/// // The internal monotonic time plus the offset of the Erlang monotonic
/// // time, about -576460752000000000 at the run start.
/// assert!(matches!(monotonic_time(scheduler), BifResult::Value(Term::Integer(value)) if value < 0));
/// ```
pub(super) fn monotonic_time(scheduler: &Scheduler) -> BifResult {
    convert_time(scheduler.timers.now(), 1, &Term::static_atom("native"))
}

/// Run `erlang:system_time/0`: return the Erlang system time in the native
/// time unit, as ERTS `system_time_0` (`erl_time_sup.c:2517-2525`) does.
/// ERTS gives `get_time(esdp) + get_time_offset()`, which is the value of
/// `erlang:system_time(native)` that [`convert_time`] gives for the system
/// time of the run. The call charges no reduction and raises nothing.
///
/// # Examples
///
/// ```rust,ignore
/// // The time of the host clock, about 1.7e18 nanoseconds after 1970.
/// assert!(matches!(system_time(scheduler), BifResult::Value(Term::Integer(value)) if value > 1_700_000_000_000_000_000));
/// ```
pub(super) fn system_time(scheduler: &Scheduler) -> BifResult {
    Value(Term::Integer(scheduler.timers.system_time()))
}

/// Run `erlang:trunc/1`, `erlang:floor/1`, `erlang:ceil/1`, and
/// `erlang:round/1`: return an integer as it is, and a float as the integer
/// of the rounded value, as ERTS `trunc_1`, `floor_1`, `ceil_1`, and
/// `round_1` (`erl_bif_guard.c:134-204`) do. Each other term raises
/// `badarg`. `round` gives the C `round` (`erl_bif_guard.c:202`): the
/// integer nearest to the value, and away from zero at a halfway case.
/// [`double_to_integer`] makes the integer from the rounded value.
///
/// # Examples
///
/// ```rust,ignore
/// assert!(matches!(round_to_integer(&Term::Integer(-7), f64::floor), BifResult::Value(Term::Integer(-7))));
/// assert!(matches!(round_to_integer(&Term::Float(-1.5), f64::floor), BifResult::Value(Term::Integer(-2))));
/// assert!(matches!(round_to_integer(&Term::Float(0.5), f64::round), BifResult::Value(Term::Integer(1))));
/// assert!(matches!(round_to_integer(&Term::static_atom("a"), f64::ceil), BifResult::Raise { .. }));
/// ```
pub(super) fn round_to_integer(term: &Term, round: fn(f64) -> f64) -> BifResult {
    match term {
        Term::Integer(_) | Term::BigInt(_) => Value(term.clone()),
        Term::Float(value) => Value(double_to_integer(round(*value))),
        _ => BifResult::badarg(),
    }
}

/// Run `erlang:trunc/1`: return an integer as it is, and a float as the
/// integer of the value with the fraction removed, as ERTS `trunc_1`
/// (`erl_bif_guard.c:134-151`) does. ERTS rounds the value toward zero with
/// `(f.fd >= 0.0) ? floor(f.fd) : ceil(f.fd)` and then calls
/// [`double_to_integer`]. Each other term raises `badarg`.
///
/// # Examples
///
/// ```rust,ignore
/// assert!(matches!(trunc(&Term::Float(1.5)), BifResult::Value(Term::Integer(1))));
/// assert!(matches!(trunc(&Term::Float(-1.5)), BifResult::Value(Term::Integer(-1))));
/// assert!(matches!(trunc(&Term::nil()), BifResult::Raise { .. }));
/// ```
pub(super) fn trunc(term: &Term) -> BifResult {
    round_to_integer(term, |value| {
        if value >= 0.0 {
            value.floor()
        } else {
            value.ceil()
        }
    })
}

/// Return the integer term of a rounded float `value`, as ERTS
/// `double_to_integer` (`erl_bif_guard.c:398-450`) does. A value in the
/// small integer range gives a small integer, and each other value gives a
/// bignum of the exact integral value of the double. [`Term::from_bigint`]
/// keeps each integer in the signed 64-bit range as [`Term::Integer`], as
/// each PBeam operation does, so `2^59` is the value of the ERTS bignum in
/// the PBeam form of an integer above the small integer range:
/// [`Term::small_integer`] gives `None` for it, as ERTS gives no small
/// integer.
///
/// The value is finite: ERTS makes no infinity or NaN term, and PBeam makes
/// none.
///
/// # Examples
///
/// ```rust,ignore
/// assert!(matches!(double_to_integer(1.0), Term::Integer(1)));
/// assert!(matches!(double_to_integer(1.0e300), Term::BigInt(_)));
/// ```
fn double_to_integer(value: f64) -> Term {
    Term::from_bigint(BigInt::from_f64(value).expect("the float of a term is finite"))
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

/// Run `erlang:make_tuple/2`: return a tuple of `size` elements, each the
/// term `value`, as ERTS `make_tuple_2` (`bif.c:3006-3025`) does. `0` gives
/// `{}`. A `size` that is not a small integer, is negative, or is above
/// [`MAX_TUPLE_SIZE`] raises `badarg`. The BIF uses no reduction.
///
/// # Examples
///
/// ```rust,ignore
/// assert!(matches!(make_tuple(&Term::Integer(0), &Term::static_atom("a")), BifResult::Value(Term::Tuple(elements)) if elements.is_empty()));
/// assert!(matches!(make_tuple(&Term::Integer(2), &Term::static_atom("a")), BifResult::Value(Term::Tuple(elements)) if elements.len() == 2));
/// assert!(matches!(make_tuple(&Term::Integer(-1), &Term::static_atom("a")), BifResult::Raise { .. }));
/// assert!(matches!(make_tuple(&Term::Integer(1 << 59), &Term::static_atom("a")), BifResult::Raise { .. }));
/// ```
pub(super) fn make_tuple(size: &Term, value: &Term) -> BifResult {
    let Some(size) = tuple_size(size) else {
        return BifResult::badarg();
    };
    Value(Term::tuple(vec![value.clone(); size]))
}

/// Run `erlang:make_tuple/3`: return a tuple of `size` elements, each the
/// term `value`, with the element at the 1-based index of each
/// `{Index, Value}` pair of the list `updates` replaced, as ERTS
/// `make_tuple_3` (`bif.c:3027-3078`) does. A pair that is not a 2-tuple,
/// and a pair whose index is not a small integer from 1 to `size`, raise
/// `badarg`, also for `updates` that is not a proper list. A later pair
/// with the index of an earlier pair wins. A `size` that is not a small
/// integer, is negative, or is above [`MAX_TUPLE_SIZE`] raises `badarg`.
/// The BIF uses no reduction.
///
/// # Examples
///
/// ```rust,ignore
/// let updates = Term::list_from([Term::tuple(vec![Term::Integer(1), Term::static_atom("a")])]);
/// assert!(matches!(make_tuple_3(&Term::Integer(2), &Term::static_atom("x"), &updates), BifResult::Value(Term::Tuple(elements)) if elements[0] == Term::static_atom("a")));
/// assert!(matches!(make_tuple_3(&Term::Integer(2), &Term::static_atom("x"), &Term::nil()), BifResult::Value(Term::Tuple(elements)) if elements.len() == 2));
/// // The index 1 is outside the tuple of size 0.
/// assert!(matches!(make_tuple_3(&Term::Integer(0), &Term::static_atom("x"), &updates), BifResult::Raise { .. }));
/// ```
pub(super) fn make_tuple_3(size: &Term, value: &Term, updates: &Term) -> BifResult {
    let Some(size) = tuple_size(size) else {
        return BifResult::badarg();
    };
    let mut elements = vec![value.clone(); size];
    let Some(updates) = updates.proper_list() else {
        return BifResult::badarg();
    };
    for update in updates {
        let Term::Tuple(pair) = update else {
            return BifResult::badarg();
        };
        let [index, value] = &pair[..] else {
            return BifResult::badarg();
        };
        let Some(index) = tuple_index(index.small_integer(), size) else {
            return BifResult::badarg();
        };
        elements[index] = value.clone();
    }
    Value(Term::tuple(elements))
}

/// Run `erlang:insert_element/3`: return the tuple with `value` at the
/// 1-based `index`, as ERTS `insert_element_3` (`bif.c:3110-3142`) does. An
/// `index` of `size + 1` appends the value. An `index` that is not a small
/// integer or is outside the tuple, and a `tuple` that is not a tuple,
/// raise `badarg`. The BIF uses no reduction.
///
/// # Examples
///
/// ```rust,ignore
/// let tuple = Term::tuple(vec![Term::static_atom("a"), Term::static_atom("b")]);
/// assert!(matches!(insert_element(&Term::Integer(1), &tuple, &Term::static_atom("c")), BifResult::Value(Term::Tuple(elements)) if elements[0] == Term::static_atom("c")));
/// assert!(matches!(insert_element(&Term::Integer(4), &tuple, &Term::static_atom("c")), BifResult::Raise { .. }));
/// ```
pub(super) fn insert_element(index: &Term, tuple: &Term, value: &Term) -> BifResult {
    let Term::Tuple(elements) = tuple else {
        return BifResult::badarg();
    };
    let Some(index) = tuple_index(index.small_integer(), elements.len() + 1) else {
        return BifResult::badarg();
    };
    let mut elements = elements.to_vec();
    elements.insert(index, value.clone());
    Value(Term::tuple(elements))
}

/// Run `erlang:delete_element/2`: return the tuple without the element at
/// the 1-based `index`, as ERTS `delete_element_2` (`bif.c:3144-3180`)
/// does. An `index` of the only element gives `{}`. An `index` that is not
/// a small integer or is outside the tuple, and a `tuple` that is not a
/// tuple, raise `badarg`. The BIF uses no reduction.
///
/// # Examples
///
/// ```rust,ignore
/// let tuple = Term::tuple(vec![Term::static_atom("a"), Term::static_atom("b")]);
/// assert!(matches!(delete_element(&Term::Integer(1), &tuple), BifResult::Value(Term::Tuple(elements)) if elements[0] == Term::static_atom("b")));
/// assert!(matches!(delete_element(&Term::Integer(1), &Term::tuple(vec![])), BifResult::Raise { .. }));
/// ```
pub(super) fn delete_element(index: &Term, tuple: &Term) -> BifResult {
    let Term::Tuple(elements) = tuple else {
        return BifResult::badarg();
    };
    let Some(index) = tuple_index(index.small_integer(), elements.len()) else {
        return BifResult::badarg();
    };
    let mut elements = elements.to_vec();
    elements.remove(index);
    Value(Term::tuple(elements))
}

/// Return the 0-based array index of a 1-based tuple index from 1 to
/// `size`, or `None` for an index that is not a small integer or is outside
/// that range. ERTS checks a tuple index with `is_not_small` and a range
/// test (`bif.c:2989-2993`, `3065-3072`, `3158-3162`).
///
/// # Examples
///
/// ```rust,ignore
/// assert_eq!(tuple_index(Term::Integer(1).small_integer(), 2), Some(0));
/// assert_eq!(tuple_index(Term::Integer(3).small_integer(), 2), None);
/// assert_eq!(tuple_index(Term::static_atom("a").small_integer(), 2), None);
/// ```
fn tuple_index(index: Option<i64>, size: usize) -> Option<usize> {
    usize::try_from(index?)
        .ok()
        .filter(|index| (1..=size).contains(index))
        .map(|index| index - 1)
}

/// Return the size of a tuple for `erlang:make_tuple/2,3`, or `None` for a
/// term that is not a small integer from 0 to [`MAX_TUPLE_SIZE`], as ERTS
/// checks it (`bif.c:3012`, `3035`).
///
/// # Examples
///
/// ```rust,ignore
/// assert_eq!(tuple_size(&Term::Integer(0)), Some(0));
/// assert_eq!(tuple_size(&Term::Integer(-1)), None);
/// assert_eq!(tuple_size(&Term::Integer(1 << 24)), None);
/// ```
fn tuple_size(size: &Term) -> Option<usize> {
    usize::try_from(size.small_integer()?)
        .ok()
        .filter(|size| *size <= MAX_TUPLE_SIZE)
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

/// Run `erlang:setelement/3`: return a copy of `tuple` with `value` at the
/// 1-based `index`, as ERTS `setelement_3` (`bif.c:2976-3002`) does. The
/// copy leaves `tuple` as it is. An `index` that is not a small integer or
/// is outside the tuple, and a `tuple` that is not a tuple, raise `badarg`.
/// The BIF uses no reduction.
///
/// # Examples
///
/// ```rust,ignore
/// let tuple = Term::tuple(vec![Term::static_atom("a"), Term::static_atom("b")]);
/// assert!(matches!(setelement(&Term::Integer(1), &tuple, &Term::static_atom("c")), BifResult::Value(Term::Tuple(elements)) if elements[0] == Term::static_atom("c")));
/// assert!(matches!(setelement(&Term::Integer(0), &tuple, &Term::static_atom("c")), BifResult::Raise { .. }));
/// assert!(matches!(setelement(&Term::Integer(3), &tuple, &Term::static_atom("c")), BifResult::Raise { .. }));
/// assert!(matches!(setelement(&Term::Integer(1 << 59), &tuple, &Term::static_atom("c")), BifResult::Raise { .. }));
/// ```
pub(super) fn setelement(index: &Term, tuple: &Term, value: &Term) -> BifResult {
    // ERTS checks the index and the tuple first (`bif.c:2984-2993`), and its
    // index must be a small integer, so a bignum index raises `badarg`.
    let Term::Tuple(elements) = tuple else {
        return BifResult::badarg();
    };
    let Some(index) = tuple_index(index.small_integer(), elements.len()) else {
        return BifResult::badarg();
    };
    let mut elements = elements.to_vec();
    elements[index] = value.clone();
    Value(Term::tuple(elements))
}

/// Run `erlang:fun_info(Fun, Item)`, as ERTS `fun_info_2`
/// (`erts/emulator/beam/erl_bif_info.c:3911-4008`) does. A local fun gives
/// `local` for `type`, the defining module for `module`, the function that
/// holds its lambda for `name`, its arity, its free variables for `env`,
/// `1` for `refc`, the `Index` field of its `FunT` entry for `new_index`
/// (`erl_bif_info.c:3954-3957`, which reads the field that the loader puts
/// in `ErlFunEntry.index`, `jit/asm_load.c:1218-1223`), and the old unique
/// value of the entry for `uniq` (`erl_bif_info.c:3968-3971`). An external
/// fun gives `external`, its module, its function, its arity, `[]` for
/// `env`, and `undefined` for `refc`, `index`, `new_index`, `uniq`,
/// `new_uniq`, and `pid` (`erl_bif_info.c:3943-3969`,
/// `erl_bif_info.c:3985-3990`). A term that is not a fun raises `badarg`
/// (`erl_bif_info.c:3924-3926`), and so does an item that the BIF does not
/// know, which includes `native` and `native_address`
/// (`erl_bif_info.c:4003-4004`). A local fun gives a clear runtime error
/// for `index`, `new_uniq`, and `pid`, because the loader of PBeam keeps
/// no such value (row "Fun information" of `docs/erts-differences.md`).
///
/// # Examples
///
/// ```rust,ignore
/// let result = fun_info(&mut context, &fun, &test_atom("type"))?;
/// assert!(matches!(result, BifResult::Value(_)));
/// ```
///
/// # Errors
///
/// A module that the code table does not hold is a runtime invariant error.
/// A full atom table gives the error of `AtomTable::put`.
pub(super) fn fun_info(
    context: &mut BifContext<'_>,
    fun: &Term,
    what: &Term,
) -> Result<BifResult, BeamError> {
    let (Term::Fun(fun), Term::Atom(what)) = (fun, what) else {
        return Ok(BifResult::badarg());
    };
    let item = context.scheduler.atoms.name_of(what);
    let value = match fun.target() {
        Target::Local(lambda) => match item {
            "type" => Term::static_atom("local"),
            "module" => Term::Atom(lambda.module().clone()),
            "name" => {
                let module = context.scheduler.atoms.name_of(lambda.module());
                match context.scheduler.code.function_name_of(module, lambda)? {
                    Some(name) => Term::Atom(context.scheduler.atoms.put(&name)?),
                    None => Term::nil(),
                }
            }
            "arity" => Term::Integer(i64::from(lambda.arity())),
            "env" => Term::list_from(fun.free().to_vec()),
            "refc" => Term::Integer(1),
            "new_index" => Term::Integer(i64::from(lambda.index())),
            "uniq" => Term::Integer(i64::from(lambda.old_unique())),
            "index" | "new_uniq" | "pid" => {
                return Err(error(format!(
                    "unsupported erlang:fun_info/2 item {item} (see \"Fun information\" in docs/erts-differences.md)"
                )));
            }
            _ => return Ok(BifResult::badarg()),
        },
        Target::External {
            module,
            function,
            arity,
        } => match item {
            "type" => Term::static_atom("external"),
            "module" => Term::Atom(module.clone()),
            "name" => Term::Atom(function.clone()),
            "arity" => Term::Integer(i64::from(*arity)),
            "env" => Term::nil(),
            "refc" | "index" | "new_index" | "uniq" | "new_uniq" | "pid" => {
                Term::static_atom("undefined")
            }
            _ => return Ok(BifResult::badarg()),
        },
    };
    Ok(Value(Term::tuple(vec![Term::Atom(what.clone()), value])))
}

/// Run `erlang:is_function/2`: return `true` when `term` is a fun of
/// `arity` arguments, as ERTS `erl_is_function` (`erl_bif_op.c:237-267`)
/// does. An `arity` that is negative or not an integer raises `badarg`. A
/// non-negative arity that no fun has gives `false`, also a positive bignum
/// (`erl_bif_op.c:244-256`). The `is_function2`
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
    let arity = match arity {
        &Term::Integer(arity) => arity,
        Term::BigInt(value) if value.sign() != num_bigint::Sign::Minus => {
            return Value(Term::static_atom("false"));
        }
        _ => return BifResult::badarg(),
    };
    if arity < 0 {
        return BifResult::badarg();
    }
    let matches = matches!(term, Term::Fun(fun) if i64::from(fun.target().arity()) == arity);
    Value(Term::static_atom(if matches { "true" } else { "false" }))
}

/// Run `erlang:is_record/2` (`size` is `None`) and `erlang:is_record/3`:
/// return `true` when `term` is a tuple whose first element is the atom
/// `name`, and whose size is `size` when `size` is given, as ERTS
/// `is_record_2` and `is_record_3` (`erl_bif_op.c:293-350`) do. A `name`
/// that is not an atom raises `badarg`. A `size` that is an atom selects
/// the native-record test, and a `size` that is not an atom and not an ERTS
/// small integer (-(2^59) to 2^59 - 1) raises `badarg`. PBeam has no native
/// record term, so the native-record test gives `false` (row
/// "Unimplemented instructions" of `docs/erts-differences.md`).
///
/// # Examples
///
/// ```rust,ignore
/// // {a, 1}
/// assert!(matches!(is_record(&pair, &Term::static_atom("a"), Some(&Term::Integer(2))), BifResult::Value(value) if value == Term::static_atom("true")));
/// assert!(matches!(is_record(&pair, &Term::Integer(1), None), BifResult::Raise { .. }));
/// ```
pub(in crate::runtime::interpreter) fn is_record(
    term: &Term,
    name: &Term,
    size: Option<&Term>,
) -> BifResult {
    if !matches!(name, Term::Atom(_)) {
        return BifResult::badarg();
    }
    let size = match size {
        None => None,
        Some(Term::Atom(_)) => return Value(Term::static_atom("false")),
        Some(&Term::Integer(size)) if (SMALL_INTEGER_MIN..=SMALL_INTEGER_MAX).contains(&size) => {
            Some(size)
        }
        Some(_) => return BifResult::badarg(),
    };
    let matches = matches!(term, Term::Tuple(tuple)
        if tuple.first() == Some(name)
            && size.is_none_or(|size| i64::try_from(tuple.len()) == Ok(size)));
    Value(Term::static_atom(if matches { "true" } else { "false" }))
}

/// Run `erlang:is_integer/3`: return `true` when `term` is an integer from
/// `low` to `high`, both included, as ERTS `is_integer_3`
/// (`erl_bif_guard.c:337-349`) does. A `low` or `high` that is not an
/// integer raises `badarg`, also when `term` is not an integer. A `term`
/// that is not an integer gives `false`. In a guard, the `bif3` instruction
/// goes to its fail label instead of the raise. Each argument can be a
/// bignum, and the bounds compare by value.
///
/// # Examples
///
/// ```rust,ignore
/// let (zero, ten) = (Term::Integer(0), Term::Integer(10));
/// assert!(matches!(is_integer_3(&Term::Integer(5), &zero, &ten), BifResult::Value(value) if value == Term::static_atom("true")));
/// assert!(matches!(is_integer_3(&Term::static_atom("foo"), &zero, &ten), BifResult::Value(value) if value == Term::static_atom("false")));
/// assert!(matches!(is_integer_3(&Term::Integer(5), &Term::static_atom("a"), &ten), BifResult::Raise { .. }));
/// ```
pub(super) fn is_integer_3(term: &Term, low: &Term, high: &Term) -> BifResult {
    if !low.is_integer() || !high.is_integer() {
        return BifResult::badarg();
    }
    let within = term.integer_order(low).is_some_and(|order| order.is_ge())
        && term.integer_order(high).is_some_and(|order| order.is_le());
    Value(Term::static_atom(if within { "true" } else { "false" }))
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
/// assert!(matches!(send3(scheduler, &Term::Pid(pid), ok.clone(), &noconnect), BifResult::Value(Term::Atom(atom)) if atom.is_static("ok"));
/// let remote = Term::tuple(vec![Term::static_atom("name"), Term::static_atom("a@b")]);
/// assert!(matches!(send3(scheduler, &remote, ok.clone(), &noconnect), BifResult::Value(Term::Atom(atom)) if atom.is_static("noconnect"));
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
            Term::Atom(option) if option.is_static("noconnect") => connect = false,
            Term::Atom(option) if option.is_one_of(&["nosuspend", "priority"]) => {}
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
    /// A registered name atom. A name that no process has raises `badarg`.
    /// The caller reads the name through the owning atom table.
    Name(&'a Atom),
    /// `{Name, Node}` with the local node, as an atom. A name that no
    /// process has drops the message. The caller reads the name through the
    /// owning atom table.
    NodeName(&'a Atom),
    /// `{Name, Node}` with another node, as an atom.
    Remote(&'a Atom),
    /// A port.
    Port,
}

/// Return the destination of `to`, or `None` for a term that `do_send`
/// rejects with `SEND_BADARG` before any lookup: a term that is not a pid,
/// a reference, an atom, a port, or a 2-tuple of atoms
/// (`bif.c:2614-2621`, `2668-2674`). The caller reads the name of a
/// [`Destination::Name`] or a [`Destination::NodeName`] through the owning
/// [`AtomTable`].
///
/// # Examples
///
/// ```rust,ignore
/// assert!(matches!(destination(&Term::Pid(pid)), Some(Destination::Pid(_))));
/// let remote = Term::tuple(vec![Term::static_atom("name"), Term::static_atom("a@b")]);
/// assert!(matches!(destination(&remote), Some(Destination::Remote(_))));
/// assert!(destination(&Term::Integer(1)).is_none());
/// ```
fn destination(to: &Term) -> Option<Destination<'_>> {
    Some(match to {
        Term::Pid(pid) => Destination::Pid(*pid),
        Term::Reference(reference) => Destination::Alias(*reference),
        Term::Atom(name) => Destination::Name(name),
        Term::Port(_) => Destination::Port,
        Term::Tuple(elements) => match elements.as_ref() {
            [Term::Atom(name), node @ Term::Atom(_)] if *node == local_node() => {
                Destination::NodeName(name)
            }
            [Term::Atom(_), Term::Atom(node)] => Destination::Remote(node),
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
///   is not alive (`remote_send`, `bif.c:2397-2400`). The send finds or
///   makes the not connected dist entry of the node first
///   (`bif.c:2656-2666`), which `erlang:nodes/1,2` gives as `known` (see
///   `Scheduler::send_to_remote_node`).
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
        Destination::Name(atom) => {
            let name = scheduler.atoms.name_of(atom);
            match scheduler.whereis(name) {
                Some(target) => Some(target),
                None => return Ok(Err(BifResult::badarg())),
            }
        }
        Destination::NodeName(atom) => scheduler.whereis(scheduler.atoms.name_of(atom)),
        Destination::Remote(node) => {
            scheduler.send_to_remote_node(node);
            return Ok(Ok(Sent::NoConnect));
        }
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
/// assert!(matches!(result, BifResult::Value(Term::Atom(atom)) if atom.is_static("true"));
/// ```
///
/// # Errors
///
/// A new `capability_denied` atom in a full atom table gives the error of
/// `AtomTable::put`.
pub(super) fn unregister(scheduler: &mut Scheduler, name: &Term) -> Result<BifResult, BeamError> {
    if let Term::Atom(atom) = name
        && let Some(owner) = scheduler.whereis(scheduler.atoms.name_of(atom))
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

/// Run `erlang:hibernate/3`, as ERTS `hibernate_3` (`bif.c:1366-1390`)
/// checks its arguments before it drops the stack. A module or a function
/// that is not an atom raises `badarg`. The check walks at most
/// `MAX_ARG + 1` (256) cons cells of the arguments: a proper list of up to
/// 256 elements hibernates, a longer list or a walked tail that is not `[]`
/// after 256 cells raises `system_limit`, and else a tail that is not `[]`
/// raises `badarg`. A call with 256 arguments thus hibernates, and `apply`
/// fails after the wake-up.
///
/// # Examples
///
/// ```rust,ignore
/// let module = Term::static_atom("m");
/// let arguments = Term::list_from((1..=256).map(Term::Integer));
/// assert!(matches!(hibernate(&module, &module, &arguments), BifResult::Hibernate { .. }));
/// let arguments = Term::list_from((1..=257).map(Term::Integer));
/// assert!(matches!(hibernate(&module, &module, &arguments), BifResult::Raise { .. }));
/// assert!(matches!(hibernate(&Term::Integer(1), &module, &Term::nil()), BifResult::Raise { .. }));
/// ```
pub(super) fn hibernate(module: &Term, function: &Term, arguments: &Term) -> BifResult {
    let (Term::Atom(_), Term::Atom(_)) = (module, function) else {
        return BifResult::badarg();
    };
    let limit = MAX_ARG as usize + 1;
    let (count, rest) = match arguments.list_iter() {
        Some(mut cells) => (cells.by_ref().take(limit).count(), cells.rest()),
        None => (0, arguments),
    };
    if !rest.is_nil() {
        let reason = if count == limit {
            "system_limit"
        } else {
            "badarg"
        };
        return Raise {
            class: ExceptionClass::Error,
            reason: Term::static_atom(reason),
        };
    }
    BifResult::Hibernate {
        module: module.clone(),
        function: function.clone(),
        arguments: arguments.clone(),
    }
}

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
/// assert!(is_process_info_item(&AtomTable::new(), &Term::static_atom("registered_name")));
/// let lookup = Term::tuple(vec![Term::static_atom("dictionary"), Term::Integer(1)]);
/// assert!(is_process_info_item(&AtomTable::new(), &lookup));
/// assert!(!is_process_info_item(&AtomTable::new(), &Term::static_atom("ok")));
/// ```
fn is_process_info_item(atoms: &AtomTable, item: &Term) -> bool {
    match item {
        Term::Atom(name) => PROCESS_INFO_ITEMS.contains(&atoms.name_of(name)),
        Term::Tuple(elements) => {
            matches!(elements.as_ref(), [Term::Atom(key), _] if key.is_static("dictionary"))
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
/// assert!(matches!(dead, BifResult::Value(Term::Atom(atom)) if atom.is_static("undefined"));
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
    let atoms = &context.scheduler.atoms;
    let valid = match item.proper_list() {
        Some(items) => items.clone().all(|item| is_process_info_item(atoms, item)),
        None => is_process_info_item(atoms, item),
    };
    let (true, Term::Pid(id)) = (valid, process) else {
        return Ok(BifResult::badarg());
    };
    let scheduler = &mut *context.scheduler;
    if let Some(denied) = other_process_denial(scheduler, *id, Operation::Inspect)? {
        return Ok(denied);
    }
    let caller = scheduler.current_process_mut();
    let caller_id = caller.id;
    // ponytail: lists of items, other items, and the frames of another process need more state; see "BIF arguments" in docs/erts-differences.md.
    let unsupported = |atoms: &AtomTable| {
        error(format!(
            "unsupported erlang:process_info/2 arguments {}, {}",
            debug_term(process, atoms),
            debug_term(item, atoms)
        ))
    };
    if *id == caller_id {
        return caller_read(
            scheduler,
            vec![process.clone(), item.clone()],
            |scheduler| {
                let current = scheduler.current_reductions();
                let registered = scheduler.registered_name_of(caller_id);
                let (caller, atoms) = scheduler.current_process_and_atoms();
                let messages = caller.mailbox.message_count();
                let info = caller
                    .info(atoms, item, current, messages, true, registered.as_ref())?
                    .ok_or_else(|| unsupported(atoms))?;
                caller.bump_reductions(1 + super::links_reductions(item, &info));
                Ok(Value(info))
            },
        );
    }
    // Another scheduler can hold a delivery for this target in the inbox of
    // this scheduler. Apply it before the read, as ERTS fetches the pending
    // deliveries of the target (`erl_bif_info.c:1403-1416`).
    scheduler.drain_inbox();
    let Some(target) = scheduler.process(*id) else {
        // A live process of another scheduler. The request goes to its
        // scheduler, so the answer comes after the signals that the caller
        // sent before (`erl_bif_info.c:1374-1401`). Another item reads the
        // process state of its scheduler and stays unsupported.
        if let Some(index) = scheduler.remote_owner(*id) {
            if let Term::Atom(name) = item
                && scheduler.atoms.name_of(name) == "registered_name"
            {
                let caller = scheduler.current_process_mut().id;
                let reference = scheduler.make_ref()?;
                scheduler.current_process_mut().mailbox.set_save_end();
                let request = Signal::ProcessInfo {
                    requester: caller,
                    reference,
                    item: item.clone(),
                };
                // The pending monitor of the caller goes before the
                // request, and a target that exits before the owner drains
                // the inbox is answered with `undefined` there.
                scheduler.send_signal_to_owner(index, Some(caller), *id, request);
                return Ok(Trap {
                    module: "erts_internal",
                    function: "await_result",
                    arguments: vec![Term::Reference(reference)],
                });
            }
            return Err(unsupported(&scheduler.atoms));
        }
        return Ok(Value(Term::static_atom("undefined")));
    };
    let messages = target.mailbox.message_count();
    let registered = scheduler.registered_name_of(*id);
    let info = target
        .info(
            &scheduler.atoms,
            item,
            0,
            messages,
            false,
            registered.as_ref(),
        )?
        .ok_or_else(|| unsupported(&scheduler.atoms))?;
    // A queued reply is built in the target process, but ERTS discards
    // the cost that `handle_process_info` returns
    // (`erl_proc_sig_queue.c:6796,7405,5425-5428`), so only the direct
    // read charges the item.
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
    scheduler
        .current_process_mut()
        .bump_reductions(1 + super::links_reductions(item, &info));
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
/// assert!(matches!(alive, BifResult::Value(Term::Atom(atom)) if atom.is_static("true"));
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
    // Another scheduler can hold a signal for this target in the inbox of
    // this scheduler. Apply it before the read, so a pending exit signal
    // leads to the routed request (`erl_bif_info.c:4075-4093`).
    scheduler.drain_inbox();
    let alive = match scheduler.process(*id) {
        // The target is on another scheduler. Route the request, so that
        // the answer comes after the signals that the caller sent before,
        // as `erts_internal:is_process_alive/1` does on ERTS.
        None if scheduler.remote_owner(*id).is_some() => {
            return Ok(Trap {
                module: "erts_internal",
                function: "is_process_alive",
                arguments: vec![pid.clone()],
            });
        }
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
    // The target can send a signal to the caller after the first drain and
    // then exit. Apply the inbox again, so the caller handles the signal
    // before the result (`erl_bif_info.c:4098-4105`).
    scheduler.drain_inbox();
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
/// let exported = function_exported(&AtomTable::new(), &scheduler.code, &erlang, &Term::static_atom("self"), &Term::Integer(0));
/// assert!(matches!(exported, BifResult::Value(Term::Atom(atom)) if atom.is_static("true"));
/// let exported = function_exported(&AtomTable::new(), &scheduler.code, &erlang, &Term::static_atom("self"), &Term::Integer(1 << 59));
/// assert!(matches!(exported, BifResult::Raise { .. }));
/// ```
pub(super) fn function_exported(
    atoms: &AtomTable,
    code: &CodeTable,
    module: &Term,
    name: &Term,
    arity: &Term,
) -> BifResult {
    let (Term::Atom(module), Term::Atom(name), Some(arity)) = (module, name, arity.small_integer())
    else {
        return BifResult::badarg();
    };
    let (module, name) = (atoms.name_of(module), atoms.name_of(name));
    let exported = u32::try_from(arity as i32).is_ok_and(|arity| {
        let exported_by_code = code.get(module).is_some_and(|loaded| {
            loaded
                .tables
                .name_id(name)
                .is_some_and(|id| loaded.exports.contains_key(&(id, arity)))
        });
        exported_by_code
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
/// returns `ok` this way. A list of more than one module with an
/// `-on_load` function raises `system_limit`, after the badarg checks
/// (`beam_bif_load.c:395-408`). The one module with an `-on_load` function
/// returns `{on_load, [Module]}` the same way, and its exports stay hidden
/// until [`finish_after_on_load`] (`beam_bif_load.c:484-504`).
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
    if references.len() > 1 && scheduler.code.prepared_on_load(&references) {
        return Ok(BifResult::Raise {
            class: ExceptionClass::Error,
            reason: Term::static_atom("system_limit"),
        });
    }
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
        Finish::OnLoad(name) => Ok(BifResult::YieldReturn(Term::tuple(vec![
            Term::static_atom("on_load"),
            Term::list_from(vec![Term::Atom(scheduler.atoms.put(&name)?)]),
        ]))),
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
        Finish::NotPurged(names) => {
            let mut modules = Vec::with_capacity(names.len());
            for name in names {
                modules.push(Term::Atom(scheduler.atoms.put(&name)?));
            }
            Ok(Value(Term::tuple(vec![
                Term::static_atom("not_purged"),
                Term::list_from(modules),
            ])))
        }
    }
}

/// Run `erlang:finish_after_on_load(Module, Keep)`, which
/// `erts_code_purger` calls after the `-on_load` function of `Module`, as
/// `finish_after_on_load_2` (`beam_bif_load.c:819-941`) does. A `Keep` that
/// is not `true` or `false` raises `badarg`, and so does a `Module` with no
/// code that waits for its `-on_load` function. With `true`, the code
/// becomes the loaded code of the module, and its exports become visible.
/// With `false`, the code stays until a purge, and the module stays not
/// loaded. Both return `true`, and with `true` the caller first switches
/// out, as for `erlang:finish_loading/1`. The `code` capability check of
/// `erlang:finish_loading/1` admitted the module, so this BIF has no
/// capability check.
///
/// # Examples
///
/// ```rust,ignore
/// let kept = finish_after_on_load(&mut scheduler, &test_atom("on_load_ok"), &Term::static_atom("true"))?;
/// assert!(matches!(kept, BifResult::Value(_)));
/// ```
///
/// # Errors
///
/// `true` after `false` gives the error of
/// [`CodeTable::finish_after_on_load`].
pub(super) fn finish_after_on_load(
    scheduler: &mut Scheduler,
    module: &Term,
    keep: &Term,
) -> Result<BifResult, BeamError> {
    let keep = match keep {
        Term::Atom(atom) if atom.is_static("true") => true,
        Term::Atom(atom) if atom.is_static("false") => false,
        _ => return Ok(BifResult::badarg()),
    };
    let Term::Atom(module) = module else {
        return Ok(BifResult::badarg());
    };
    let name = scheduler.atoms.name_of(module).to_owned();
    if !scheduler.code.finish_after_on_load(&name, keep)? {
        Ok(BifResult::badarg())
    } else if keep {
        // ERTS suspends the caller until the code is committed
        // (`beam_bif_load.c:912-916`, `555-566`).
        Ok(BifResult::YieldReturn(Term::static_atom("true")))
    } else {
        Ok(Value(Term::static_atom("true")))
    }
}

/// Run `erlang:module_loaded/1`, as ERTS `module_loaded_1`
/// (`beam_bif_load.c:738-755`) does: return `true` when `module` has current
/// code and no `-on_load` function waits to run, and `false` for an atom
/// that names no loaded module. A `module` that is not an atom raises
/// `badarg`. Then the caller needs a `code` capability that permits
/// `inspect` for the module name (row "Capability checks" of
/// `docs/erts-differences.md`), after the atom check and before the code
/// table lookup. A module that waits for its `-on_load` function is not in
/// the loaded modules of the code table, so it gives `false`.
///
/// # Examples
///
/// ```rust,ignore
/// let loaded = module_loaded(&mut context, &Term::static_atom("erlang"))?;
/// assert!(matches!(loaded, BifResult::Value(Term::Atom(atom)) if atom.is_static("true"));
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
    // The capability check borrows the scheduler mutably, so the name comes
    // out of the table first.
    let name = context.scheduler.atoms.name_of(module).as_bytes().to_vec();
    let check = denial(
        context.scheduler,
        CapabilityKind::Code,
        Operation::Inspect,
        Some(&name),
    );
    if let Some(denied) = check? {
        return Ok(denied);
    }
    let loaded = context
        .scheduler
        .code
        .get(context.scheduler.atoms.name_of(module))
        .is_some();
    Ok(Value(Term::static_atom(if loaded {
        "true"
    } else {
        "false"
    })))
}

/// The key names of `erlang:process_info/1`, in the order of
/// `pi_1_keys_list` (`erl_bif_info.c:1007-1025`). The `registered_name`
/// item is omitted when the process has no name (`erl_bif_info.c:1608-1625`).
const PROCESS_INFO_1_KEYS: [&str; 17] = [
    "registered_name",
    "current_function",
    "initial_call",
    "status",
    "message_queue_len",
    "links",
    "dictionary",
    "trap_exit",
    "error_handler",
    "priority",
    "group_leader",
    "total_heap_size",
    "heap_size",
    "stack_size",
    "reductions",
    "garbage_collection",
    "suspending",
];

/// The names of an MFA item of `erlang:process_info/1`: module and function
/// names that the atom table interns, or atoms that a start MFA holds.
enum MfaNames {
    /// A module and a function name from the tables of a loaded module.
    Names(String, String, usize),
    /// The atoms of an undefined start MFA.
    Atoms(Atom, Atom, usize),
}

/// The values of the `erlang:process_info/1` items of one process, read
/// while the process is borrowed.
struct ProcessInfoValues {
    registered_name: Option<Atom>,
    current_function: Option<MfaNames>,
    initial_call: MfaNames,
    status: &'static str,
    message_queue_len: i64,
    links: Vec<Term>,
    dictionary: Vec<(Term, Term)>,
    trap_exit: bool,
    error_handler: Option<String>,
    priority: &'static str,
    group_leader: ProcessId,
    total_heap_size: i64,
    heap_size: i64,
    stack_size: i64,
    reductions: i64,
}

/// Run `erlang:process_info/1` as `process_info_bif` with `pi_1_keys_list`
/// (`erl_bif_info.c:1560-1563`, `1007-1025`) does. The result is the 17
/// items of a live process, in the ERTS order, and the `registered_name`
/// item is omitted when the process has no name. The item values come from
/// the state of the process: `current_function` and `initial_call` name its
/// frames or start MFA, and the heap and stack sizes are the words of its
/// PBeam containers (`heap_size` and `total_heap_size` are equal, because
/// PBeam has no separate old heap). The garbage collection item gives the
/// ERTS defaults with `minor_gcs` 0, because PBeam has no garbage
/// collection.
///
/// A pid that no process has gives `undefined`. A read of another process
/// needs the `process` capability with `inspect`, as for
/// `erlang:process_info/2`. The caller first handles its own signals, and
/// traps when the read uses up its reductions or a signal can end it, as
/// `erlang:process_info/2` does. A read of another process with signals
/// gives its current state without the ERTS process-info request, because
/// `ProcessState::info` reads one atom item.
///
/// # Examples
///
/// ```rust,ignore
/// let info = process_info_1(&mut context, &Term::Pid(context.scheduler.current_process().id))?;
/// assert!(matches!(info, BifResult::Value(Term::Cons(_))));
/// assert!(matches!(process_info_1(&mut context, &Term::Integer(1))?, BifResult::Raise { .. }));
/// ```
///
/// # Errors
///
/// A name of an MFA item that the atom table does not hold, and the name
/// numbers of an unloaded module, are runtime errors.
pub(super) fn process_info_1(
    context: &mut BifContext<'_>,
    process: &Term,
) -> Result<BifResult, BeamError> {
    let Term::Pid(id) = process else {
        return Ok(BifResult::badarg());
    };
    let scheduler = &mut *context.scheduler;
    if let Some(denied) = other_process_denial(scheduler, *id, Operation::Inspect)? {
        return Ok(denied);
    }
    if *id == scheduler.current_process().id {
        return caller_read(scheduler, vec![process.clone()], |scheduler| {
            let current = scheduler.current_reductions();
            let messages = scheduler.current_process().mailbox.message_count();
            let registered = scheduler.registered_name_of(*id);
            let values = {
                let caller = scheduler.current_process();
                process_info_values(&scheduler.code, caller, current, messages, true, registered)?
            };
            let info = process_info_term(&mut scheduler.atoms, values)?;
            scheduler
                .current_process_mut()
                .bump_reductions(PROCESS_INFO_1_KEYS.len() as i32);
            Ok(Value(info))
        });
    }
    // Another scheduler can hold a delivery for this target in the inbox of
    // this scheduler. Apply it before the read, as ERTS fetches the pending
    // deliveries of the target (`erl_bif_info.c:1403-1416`).
    scheduler.drain_inbox();
    let Some(target) = scheduler.process(*id) else {
        // A live process of another scheduler needs the process state of its
        // scheduler, which this read cannot reach yet.
        if scheduler.remote_owner(*id).is_some() {
            return Err(error(format!(
                "unsupported erlang:process_info/1 arguments {}",
                debug_term(process, &scheduler.atoms)
            )));
        }
        return Ok(Value(Term::static_atom("undefined")));
    };
    let messages = target.mailbox.message_count();
    let registered = scheduler.registered_name_of(*id);
    let values = process_info_values(&scheduler.code, target, 0, messages, false, registered)?;
    let info = process_info_term(&mut scheduler.atoms, values)?;
    scheduler
        .current_process_mut()
        .bump_reductions(PROCESS_INFO_1_KEYS.len() as i32);
    Ok(Value(info))
}

/// Read the values of the `erlang:process_info/1` items of `process`.
/// `current` is the reduction count of the current turn of the caller.
fn process_info_values(
    code: &CodeTable,
    process: &ProcessState,
    current: u64,
    messages: usize,
    caller: bool,
    registered_name: Option<Atom>,
) -> Result<ProcessInfoValues, BeamError> {
    let current_function = if let Some(frame) = process.frames.last() {
        let module = code.module_by_id(frame.module_id)?;
        let index = frame.next_instruction.saturating_sub(1);
        module.instructions[..index]
            .iter()
            .rev()
            .find_map(|instruction| match instruction {
                Instruction::FunctionInfo {
                    module,
                    name,
                    arity,
                } => Some(MfaNames::Names(
                    module.clone(),
                    name.clone(),
                    *arity as usize,
                )),
                _ => None,
            })
            .map(Some)
            .ok_or_else(|| error("invariant error: the instruction has no func_info before it"))?
    } else {
        process.undefined_start.as_ref().map(|(module, function)| {
            MfaNames::Atoms(module.clone(), function.clone(), process.registers.len())
        })
    };
    let initial_call = match &process.initial_call {
        InitialCall::Entry {
            module,
            function,
            arity,
        } => {
            let loaded = code.module_by_id(*module)?;
            MfaNames::Names(
                loaded.name.clone(),
                loaded.tables.name(*function).to_owned(),
                *arity,
            )
        }
        InitialCall::Undefined {
            module,
            function,
            arity,
        } => MfaNames::Atoms(module.clone(), function.clone(), *arity),
    };
    let status = if process.exiting.is_some() {
        "exiting"
    } else if caller {
        "running"
    } else if process.waiting {
        "waiting"
    } else {
        "runnable"
    };
    let reductions = i64::try_from(process.reductions.saturating_add(current))
        .map_err(|_| error("the reduction total exceeds the PBeam integer range"))?;
    let words = std::mem::size_of::<usize>();
    let heap_size = i64::try_from(process.allocated_bytes() / words)
        .map_err(|_| error("the heap size exceeds the PBeam integer range"))?;
    let stack_size = i64::try_from(
        process
            .frames
            .iter()
            .map(|frame| frame.stack.capacity())
            .sum::<usize>()
            + process.registers.capacity(),
    )
    .map_err(|_| error("the stack size exceeds the PBeam integer range"))?;
    Ok(ProcessInfoValues {
        registered_name,
        current_function,
        initial_call,
        status,
        message_queue_len: i64::try_from(messages).unwrap_or(i64::MAX),
        links: {
            // ERTS reports only active links, in the reverse of the tree
            // walk (`collect_one_link`, `erl_bif_info.c:574-585`,
            // `1764-1775`).
            let links: Vec<Term> = process
                .links
                .post_order()
                .filter(|(_, unlinking)| **unlinking == 0)
                .map(|(id, _)| id.term())
                .collect();
            links.into_iter().rev().collect()
        },
        dictionary: process.dictionary.pairs().cloned().collect(),
        trap_exit: process.trap_exit,
        error_handler: process.error_handler.clone(),
        priority: process.priority.name(),
        group_leader: process.group_leader,
        total_heap_size: heap_size,
        heap_size,
        stack_size,
        reductions,
    })
}

/// Build the `erlang:process_info/1` list from the item values.
fn process_info_term(atoms: &mut AtomTable, values: ProcessInfoValues) -> Result<Term, BeamError> {
    let mfa = |atoms: &mut AtomTable, mfa: MfaNames| -> Result<Term, BeamError> {
        let (module, function, arity) = match mfa {
            MfaNames::Names(module, function, arity) => (
                Term::Atom(atoms.put(&module)?),
                Term::Atom(atoms.put(&function)?),
                arity,
            ),
            MfaNames::Atoms(module, function, arity) => {
                (Term::Atom(module), Term::Atom(function), arity)
            }
        };
        let arity =
            i64::try_from(arity).map_err(|_| error("the arity exceeds the PBeam integer range"))?;
        Ok(Term::tuple(vec![module, function, Term::Integer(arity)]))
    };
    let mut items = Vec::with_capacity(PROCESS_INFO_1_KEYS.len());
    if let Some(name) = values.registered_name {
        items.push(Term::tuple(vec![
            Term::static_atom("registered_name"),
            Term::Atom(name),
        ]));
    }
    let current_function = match values.current_function {
        Some(mfa_names) => mfa(atoms, mfa_names)?,
        None => Term::static_atom("undefined"),
    };
    items.push(Term::tuple(vec![
        Term::static_atom("current_function"),
        current_function,
    ]));
    items.push(Term::tuple(vec![
        Term::static_atom("initial_call"),
        mfa(atoms, values.initial_call)?,
    ]));
    items.push(Term::tuple(vec![
        Term::static_atom("status"),
        Term::static_atom(values.status),
    ]));
    items.push(Term::tuple(vec![
        Term::static_atom("message_queue_len"),
        Term::Integer(values.message_queue_len),
    ]));
    items.push(Term::tuple(vec![
        Term::static_atom("links"),
        Term::list_from(values.links),
    ]));
    let dictionary = values
        .dictionary
        .into_iter()
        .map(|(key, value)| Term::tuple(vec![key, value]));
    items.push(Term::tuple(vec![
        Term::static_atom("dictionary"),
        Term::list_from(dictionary),
    ]));
    items.push(Term::tuple(vec![
        Term::static_atom("trap_exit"),
        Term::static_atom(if values.trap_exit { "true" } else { "false" }),
    ]));
    let error_handler = match values.error_handler {
        Some(name) => Term::Atom(atoms.put(&name)?),
        None => Term::static_atom("error_handler"),
    };
    items.push(Term::tuple(vec![
        Term::static_atom("error_handler"),
        error_handler,
    ]));
    items.push(Term::tuple(vec![
        Term::static_atom("priority"),
        Term::static_atom(values.priority),
    ]));
    items.push(Term::tuple(vec![
        Term::static_atom("group_leader"),
        Term::Pid(values.group_leader),
    ]));
    for (key, value) in [
        ("total_heap_size", values.total_heap_size),
        ("heap_size", values.heap_size),
        ("stack_size", values.stack_size),
        ("reductions", values.reductions),
    ] {
        items.push(Term::tuple(vec![
            Term::static_atom(key),
            Term::Integer(value),
        ]));
    }
    let max_heap_size = Map::from_pairs(
        vec![
            (Term::static_atom("error_logger"), Term::static_atom("true")),
            (
                Term::static_atom("include_shared_binaries"),
                Term::static_atom("false"),
            ),
            (Term::static_atom("kill"), Term::static_atom("true")),
            (Term::static_atom("size"), Term::Integer(0)),
        ],
        &*atoms,
    )
    .expect("the max_heap_size map holds atom keys");
    let minor_gcs = atoms.put("minor_gcs")?;
    items.push(Term::tuple(vec![
        Term::static_atom("garbage_collection"),
        Term::list_from(vec![
            Term::tuple(vec![
                Term::static_atom("max_heap_size"),
                Term::Map(max_heap_size),
            ]),
            Term::tuple(vec![
                Term::static_atom("min_bin_vheap_size"),
                Term::Integer(46422),
            ]),
            Term::tuple(vec![Term::static_atom("min_heap_size"), Term::Integer(233)]),
            Term::tuple(vec![
                Term::static_atom("fullsweep_after"),
                Term::Integer(65535),
            ]),
            Term::tuple(vec![Term::Atom(minor_gcs), Term::Integer(0)]),
        ]),
    ]));
    items.push(Term::tuple(vec![
        Term::static_atom("suspending"),
        Term::nil(),
    ]));
    Ok(Term::list_from(items))
}

/// Handle the signals of the caller before a read of its own process state,
/// as `pi_maybe_flush_signals` (`erl_bif_info.c:1198-1268`) does. Without a
/// flush and without self signals, only fetch. Return `Some(result)` when
/// the call ends with a trap of the BIF `arguments` or with the exit of the
/// caller, and `None` when the read can go on.
///
/// # Examples
///
/// ```rust,ignore
/// assert!(prepare_caller_read(&mut scheduler, vec![pid]).unwrap().is_none());
/// ```
fn prepare_caller_read(
    scheduler: &mut Scheduler,
    arguments: Vec<Term>,
) -> Result<Option<BifResult>, BeamError> {
    // A message of another scheduler waits in the inbox of this one. This
    // read fetches the messages of the caller, as ERTS fetches its in-queue
    // buffers (`erl_proc_sig_queue.c:7771-7799`), so apply the inbox first.
    scheduler.drain_inbox();
    let caller = scheduler.current_process_mut();
    if caller.flush == Flush::None && !caller.self_signals {
        if caller.mailbox.has_outer_messages() {
            caller.mailbox.fetch();
        }
        return Ok(None);
    }
    Ok(Some(match scheduler.flush_signals()? {
        Flushed::Done => return Ok(None),
        Flushed::Exited(reason) => BifResult::Exited(reason),
        Flushed::Yield => BifResult::TrapBif {
            module: "erlang",
            function: "process_info",
            arguments,
        },
    }))
}

/// Run the read of the caller of `erlang:process_info/1,2` after it handles
/// its signals, as `process_info_bif` does (`erl_bif_info.c:1355-1363`,
/// `1374-1379`). A trap or an exit of the flush is the result of the BIF;
/// else `read` runs.
///
/// # Examples
///
/// ```rust,ignore
/// let result = caller_read(&mut scheduler, vec![pid], |_| Ok(Value(Term::nil())))?;
/// ```
fn caller_read(
    scheduler: &mut Scheduler,
    arguments: Vec<Term>,
    read: impl FnOnce(&mut Scheduler) -> Result<BifResult, BeamError>,
) -> Result<BifResult, BeamError> {
    if let Some(result) = prepare_caller_read(scheduler, arguments)? {
        return Ok(result);
    }
    read(scheduler)
}

/// Run `erlang:registered/0` as `registered_0` (`register.c:576-604`) does:
/// return the names of the registered processes. ERTS walks its hash table,
/// so the order is not part of the result.
///
/// # Examples
///
/// ```rust,ignore
/// assert!(matches!(registered(&mut scheduler)?, BifResult::Value(Term::Nil)));
/// ```
///
/// # Errors
///
/// A new atom of a name in a full atom table gives the error of
/// `AtomTable::put`.
pub(super) fn registered(scheduler: &mut Scheduler) -> Result<BifResult, BeamError> {
    let mut names = Vec::new();
    for name in scheduler.registered_names() {
        names.push(Term::Atom(scheduler.atoms.put(&name)?));
    }
    Ok(Value(Term::list_from(names)))
}

/// Run `erlang:ports/0` as `ports_0` (`bif.c:4125-4128`) does with
/// `erts_ptab_list` (`erl_ptab.c:738-...`): return each open port. ERTS
/// walks its port table, so the order is not part of the result. A run
/// with scheduler threads fails, because each scheduler keeps its own port
/// table.
///
/// # Examples
///
/// ```rust,ignore
/// assert!(matches!(ports(&mut scheduler)?, BifResult::Value(Term::Nil)));
/// ```
///
/// # Errors
///
/// The unsupported error of a run with scheduler threads.
pub(super) fn ports(scheduler: &mut Scheduler) -> Result<BifResult, BeamError> {
    // The port table stays with each scheduler, so this run cannot list
    // the ports of the node yet.
    if scheduler.shared().is_some() {
        return Err(error(
            "unsupported: erlang:ports/0 in a run of execute_on_schedulers",
        ));
    }
    let ports = scheduler.resources.borrow_mut::<Ports>().ids();
    Ok(Value(Term::list_from(ports.into_iter().map(Term::Port))))
}

/// Run `erlang:alias/1` as `alias_1` (`bif.c:5852-5892`) does. The options
/// `explicit_unalias` and `reply` set the alias mode, and the last one
/// wins. An unknown option, an improper list, and a term that is not a list
/// raise `badarg`. The result is a pid reference, as `erts_make_pid_ref`
/// gives it, with an alias and no monitor in the monitor tree of the
/// caller. PBeam does not support the `priority` option: it raises a
/// runtime error, as each other priority signal path does.
///
/// # Examples
///
/// ```rust,ignore
/// let alias = alias(&mut context, &Term::nil())?;
/// assert!(matches!(alias, BifResult::Value(Term::Reference(_))));
/// assert!(matches!(alias(&mut context, &Term::static_atom("x"))?, BifResult::badarg()));
/// ```
///
/// # Errors
///
/// The `priority` option, and the errors of `Scheduler::monitor`.
pub(super) fn alias(context: &mut BifContext<'_>, options: &Term) -> Result<BifResult, BeamError> {
    // The alias owners stay with each scheduler, so a message to an alias
    // of another scheduler would be dropped.
    if context.scheduler.shared().is_some() {
        return Err(error(
            "unsupported: erlang:alias/0,1 in a run of execute_on_schedulers",
        ));
    }
    let Some(mut elements) = options.list_iter() else {
        return Ok(BifResult::badarg());
    };
    let mut mode = AliasMode::ExplicitUnalias;
    let mut priority = false;
    for element in elements.by_ref() {
        match element {
            Term::Atom(atom) if atom.is_static("explicit_unalias") => {
                mode = AliasMode::ExplicitUnalias;
            }
            Term::Atom(atom) if atom.is_static("reply") => mode = AliasMode::ReplyDemonitor,
            Term::Atom(atom) if atom.is_static("priority") => priority = true,
            _ => return Ok(BifResult::badarg()),
        }
    }
    if !elements.rest().is_nil() {
        return Ok(BifResult::badarg());
    }
    if priority {
        return Err(error("unsupported erlang:alias/1 option priority"));
    }
    let scheduler = &mut *context.scheduler;
    let caller = scheduler.current_process().id;
    let options = MonitorOptions {
        alias: Some(mode),
        tag: Term::nil(),
        priority: false,
    };
    let reference = scheduler.monitor(MonitorTarget::Process(caller), options)?;
    scheduler.current_process_mut().monitors.insert(
        reference,
        Origin {
            monitor: None,
            alias: Some(mode),
        },
    );
    Ok(Value(Term::Reference(reference)))
}

/// Run `erlang:unalias/1` as `unalias_1` (`bif.c:5896-5927`) does. A term
/// that is not a reference raises `badarg`. An ordinary reference, a pid
/// reference of another process, and a reference that is not an active
/// alias of the caller give `false`. An active alias gives `true`, and the
/// alias ends.
///
/// # Examples
///
/// ```rust,ignore
/// let alias = alias(&mut context, &Term::nil())?;
/// let BifResult::Value(Term::Reference(reference)) = alias else { return };
/// assert!(matches!(unalias(&mut context, &Term::Reference(reference))?, BifResult::Value(Term::Atom(atom))) if atom.is_static("true"));
/// ```
pub(super) fn unalias(
    context: &mut BifContext<'_>,
    reference: &Term,
) -> Result<BifResult, BeamError> {
    let Term::Reference(reference) = reference else {
        return Ok(BifResult::badarg());
    };
    let scheduler = &mut *context.scheduler;
    let caller = scheduler.current_process().id;
    if !reference.is_pid_reference() || scheduler.alias_owner(*reference) != Some(caller) {
        return Ok(Value(Term::static_atom("false")));
    }
    let process = scheduler.current_process_mut();
    let Some(origin) = process.monitors.get_mut(reference) else {
        return Ok(Value(Term::static_atom("false")));
    };
    if origin.alias.is_none() {
        return Ok(Value(Term::static_atom("false")));
    }
    // ERTS keeps a monitor that has an alias option and removes only the
    // alias, and removes an alias without a monitor (`bif.c:5910-5921`).
    if origin.monitor.is_some() {
        origin.alias = None;
    } else {
        process.monitors.remove(reference);
    }
    Ok(Value(Term::static_atom("true")))
}

/// Run `erlang:unique_integer/1` as `unique_integer_1`
/// (`erl_bif_unique.c:840-871`) does. The options are `monotonic` and
/// `positive`: `monotonic` gives a value of the monotonic counter, and
/// `positive` gives a positive value. A term that is not a proper list of
/// those options raises `badarg`; ERTS 29.1.1 does not accept `negative`.
///
/// # Examples
///
/// ```rust,ignore
/// let value = unique_integer_1(&mut scheduler, &Term::nil())?;
/// assert!(matches!(value, BifResult::Value(Term::Integer(value)) if value < 0));
/// assert!(matches!(unique_integer_1(&mut scheduler, &Term::static_atom("positive"))?, BifResult::Raise { .. }));
/// ```
///
/// # Errors
///
/// A counter value above the small integer range is a runtime error.
pub(super) fn unique_integer_1(
    scheduler: &mut Scheduler,
    options: &Term,
) -> Result<BifResult, BeamError> {
    let Some(mut elements) = options.list_iter() else {
        return Ok(BifResult::badarg());
    };
    let (mut monotonic, mut positive) = (false, false);
    for element in elements.by_ref() {
        match element {
            Term::Atom(atom) if atom.is_static("monotonic") => monotonic = true,
            Term::Atom(atom) if atom.is_static("positive") => positive = true,
            _ => return Ok(BifResult::badarg()),
        }
    }
    if !elements.rest().is_nil() {
        return Ok(BifResult::badarg());
    }
    let value = if monotonic {
        scheduler.unique_monotonic_integer(positive)?
    } else if positive {
        scheduler.unique_integer_positive()?
    } else {
        scheduler.unique_integer()?
    };
    Ok(Value(Term::Integer(value)))
}

/// Run `erlang:delete_module/1` as `delete_module_1`
/// (`beam_bif_load.c:679-736`) does. A term that is not an atom raises
/// `badarg`. A module with current code moves to old code and gives `true`,
/// a module with old code from an earlier delete raises `badarg`, and a
/// name with no code gives `undefined`. The caller then needs a `code`
/// capability that permits `load` for the module name.
///
/// # Examples
///
/// ```rust,ignore
/// assert!(matches!(delete_module(&mut context, &Term::static_atom("nomod"))?, BifResult::Value(Term::Atom(atom))) if atom.is_static("undefined"));
/// assert!(matches!(delete_module(&mut context, &Term::Integer(1))?, BifResult::Raise { .. }));
/// ```
///
/// # Errors
///
/// A new `capability_denied` atom in a full atom table gives the error of
/// `AtomTable::put`.
pub(super) fn delete_module(
    scheduler: &mut Scheduler,
    module: &Term,
) -> Result<BifResult, BeamError> {
    let Term::Atom(module) = module else {
        return Ok(BifResult::badarg());
    };
    let name = scheduler.atoms.name_of(module).as_bytes().to_vec();
    let kind = CapabilityKind::Code;
    let denied = denial(scheduler, kind, Operation::Load, Some(&name))?;
    if let Some(denied) = denied {
        return Ok(denied);
    }
    Ok(
        match scheduler.code.delete(scheduler.atoms.name_of(module)) {
            Some(true) => Value(Term::static_atom("true")),
            Some(false) => BifResult::badarg(),
            None => Value(Term::static_atom("undefined")),
        },
    )
}

/// Run `erlang:has_prepared_code_on_load/1` as
/// `has_prepared_code_on_load_1` (`beam_bif_load.c:268-283`) and
/// `erts_has_code_on_load` (`beam_load.c:336-349`) do: return `true` when a
/// reference of `erts_internal:prepare_loading/2` holds prepared code with
/// an `-on_load` function, and `false` when the code has none. A term that
/// is not a prepared-code reference raises `badarg`. A reference that
/// `erlang:finish_loading/1` consumed keeps its flag, as the ERTS magic
/// reference stays valid (`beam_bif_load.c:268-283`). PBeam checks no
/// capability: the call only reads the prepared code state, as the ambient
/// `erlang:function_exported/3` does.
///
/// # Examples
///
/// ```rust,ignore
/// assert!(matches!(has_prepared_code_on_load(&mut scheduler, &Term::Reference(reference))?, BifResult::Value(Term::Atom(atom))) if atom.is_static("true"));
/// assert!(matches!(has_prepared_code_on_load(&mut scheduler, &Term::static_atom("x"))?, BifResult::Raise { .. }));
/// ```
///
/// # Errors
///
/// The function has no error.
pub(super) fn has_prepared_code_on_load(
    scheduler: &mut Scheduler,
    reference: &Term,
) -> Result<BifResult, BeamError> {
    let Term::Reference(reference) = reference else {
        return Ok(BifResult::badarg());
    };
    let on_load = if scheduler.code.prepared_module(reference).is_some() {
        scheduler.code.prepared_on_load(&[*reference])
    } else if let Some(on_load) = scheduler.code.finished_prepared_on_load(reference) {
        on_load
    } else {
        return Ok(BifResult::badarg());
    };
    Ok(Value(Term::static_atom(if on_load {
        "true"
    } else {
        "false"
    })))
}

/// Run `erlang:display/1` as `display_1` (`bif.c:4144-4148`) does: write
/// the `%T` text of the term and a newline to the standard output, and
/// return `true`. The text is [`print_term`]. ERTS writes the newline as
/// `\r\n` on Unix, because `erts_printf` adds a carriage return to standard
/// output (`sys/unix/sys.c:342`). A failed write changes nothing and gives
/// no error, as ERTS does not check the `erts_printf` result.
///
/// # Examples
///
/// ```rust,ignore
/// let result = display(&Term::static_atom("ok"), &AtomTable::new());
/// assert!(matches!(result, BifResult::Value(Term::Atom(atom))) if atom.is_static("true"));
/// ```
pub(super) fn display(term: &Term, atoms: &AtomTable) -> BifResult {
    let mut text = print_term(term, atoms);
    text.extend_from_slice(b"\r\n");
    let mut stdout = std::io::stdout();
    let _ = stdout.write_all(&text);
    let _ = stdout.flush();
    Value(Term::static_atom("true"))
}

/// Run `erlang:display_string/2` as `display_string_2`
/// (`bif.c:4192-4285`) does: write the UTF-8 bytes of the string to the
/// standard output or the standard error, and return `true`. The string is
/// a proper list of Unicode code points or a binary of whole bytes. A
/// device that is not `stdout` or `stderr`, a string with another form or a
/// bad code point, an improper list, and a failed write raise `badarg`.
/// PBeam does not support the `stdin` device, because it writes the bytes
/// with the `TIOCSTI` ioctl: it raises a runtime error.
///
/// # Examples
///
/// ```rust,ignore
/// let result = display_string(&mut atoms, &Term::static_atom("stdout"), &Term::nil());
/// assert!(matches!(result, BifResult::Value(Term::Atom(atom))) if atom.is_static("true"));
/// assert!(matches!(display_string(&mut atoms, &Term::static_atom("stdin"), &Term::nil()), Err(_)));
/// ```
///
/// # Errors
///
/// The `stdin` device, and the errors of `AtomTable::put`.
pub(super) fn display_string(
    atoms: &mut AtomTable,
    device: &Term,
    string: &Term,
) -> Result<BifResult, BeamError> {
    let Term::Atom(device) = device else {
        return Ok(BifResult::badarg());
    };
    let bytes = if let Some(mut elements) = string.list_iter() {
        let mut bytes = Vec::new();
        for element in elements.by_ref() {
            let Term::Integer(code) = element else {
                return Ok(BifResult::badarg());
            };
            let Some(character) = u32::try_from(*code).ok().and_then(char::from_u32) else {
                return Ok(BifResult::badarg());
            };
            let mut buffer = [0; 4];
            bytes.extend_from_slice(character.encode_utf8(&mut buffer).as_bytes());
        }
        if !elements.rest().is_nil() {
            return Ok(BifResult::badarg());
        }
        bytes
    } else if let Term::Binary(binary) = string {
        if binary.bit_size() % 8 != 0 {
            return Ok(BifResult::badarg());
        }
        binary.to_vec()
    } else {
        return Ok(BifResult::badarg());
    };
    let name = atoms.name_of(device);
    if name == "stdout" {
        return Ok(write_display(&mut std::io::stdout(), &bytes));
    }
    if name == "stderr" {
        return Ok(write_display(&mut std::io::stderr(), &bytes));
    }
    if name == "stdin" {
        // PBeam does not write to the terminal input with `TIOCSTI`.
        return Err(error("unsupported erlang:display_string/2 device stdin"));
    }
    Ok(BifResult::badarg())
}

/// Write `bytes` to `writer` and give `true`, or raise `badarg` when the
/// write fails, as `display_string_2` does (`bif.c:4274-4297`).
fn write_display(writer: &mut dyn Write, bytes: &[u8]) -> BifResult {
    match writer.write_all(bytes).and_then(|()| writer.flush()) {
        Ok(()) => Value(Term::static_atom("true")),
        Err(_) => BifResult::badarg(),
    }
}

/// Run `erlang:ref_to_list/1` as `ref_to_list_1` (`bif.c:4459-4465`) does:
/// return the characters of the `%T` text of a reference as a proper list.
/// A term that is not a reference raises `badarg`. [`print_term`] gives the
/// text of a PBeam reference, `#Ref<0.N2.N1.N0>`.
///
/// # Examples
///
/// ```rust,ignore
/// let text = ref_to_list(&Term::Reference(Reference(0)), &AtomTable::new());
/// assert!(matches!(text, BifResult::Value(codes) if codes.proper_list().is_some_and(|codes| codes.len() > 0)));
/// assert!(matches!(ref_to_list(&Term::Integer(1), &AtomTable::new()), BifResult::Raise { .. }));
/// ```
pub(super) fn ref_to_list(term: &Term, atoms: &AtomTable) -> BifResult {
    match term {
        Term::Reference(_) => term_text_list(term, atoms),
        _ => BifResult::badarg(),
    }
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
/// assert!(matches!(info, BifResult::Value(Term::Atom(atom)) if atom.is_static("lists"));
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
    get_module_info_key(context, module, what, "get_module_info/2")
}

/// The pair list that `erlang:get_module_info/1` returns, in the order of
/// the list (`erl_bif_info.c:5970-5984`): the `BUILD_INFO` calls run from
/// `md5` to `module` and each prepends its pair, so the first pair of the
/// list is `module`, and the second pair is the one below.
const MODULE_INFO_MISSING_KEY: &str = "exports";

/// Run `erlang:get_module_info(Module)`, which `Module:module_info/0`
/// calls, as ERTS `get_module_info_1` and `module_info_0`
/// (`erl_bif_info.c:5946-5984`, `6010-6019`) do: return the pair list of
/// the keys `module`, `exports`, `attributes`, `compile`, and `md5`, with
/// the value that `erlang:get_module_info/2` gives for each key. A `Module`
/// that is not an atom or that names no loaded module raises `badarg`. The
/// caller needs a `code` capability that permits `inspect` for the module
/// name, as for `erlang:get_module_info/2`.
///
/// PBeam can give the pair of the key `module` only: the key `exports`
/// needs the order of the ERTS export table, and the keys `attributes`,
/// `compile`, and `md5` need the chunks of the beam file, which the loader
/// does not keep (row "Module information" of `docs/erts-differences.md`).
/// Thus the call of a loaded module stops the run with the runtime error
/// `erlang:get_module_info/1 key exports is not implemented`, as
/// `erlang:get_module_info/2` stops at a key that it does not implement.
/// The release `examples/plug_hello` imports this BIF for the
/// `module_info/0` of each of its modules and calls it nowhere.
///
/// # Examples
///
/// ```rust,ignore
/// // The pair list [{module, lists}, {exports, [... ]}, ...] of ERTS.
/// let result = get_module_info_1(&mut context, &Term::static_atom("lists"))?;
/// ```
///
/// # Errors
///
/// A key that `erlang:get_module_info/2` does not implement gives its
/// runtime error. A new `capability_denied` atom in a full atom table gives
/// the error of `AtomTable::put`.
pub(super) fn get_module_info_1(
    context: &mut BifContext<'_>,
    module: &Term,
) -> Result<BifResult, BeamError> {
    // The first key of the list holds the checks that each key shares.
    let key = Term::static_atom("module");
    match get_module_info_key(context, module, &key, "get_module_info/1")? {
        // The pair of the key `module` is the first pair of the list. The
        // rest of the list needs the keys that PBeam cannot give.
        BifResult::Value(_) => Err(error(format!(
            "erlang:get_module_info/1 key {MODULE_INFO_MISSING_KEY} is not implemented"
        ))),
        other => Ok(other),
    }
}

/// Run the key of `erlang:get_module_info/2` that `bif` names, for the
/// runtime error of a key that PBeam does not implement yet.
fn get_module_info_key(
    context: &mut BifContext<'_>,
    module: &Term,
    what: &Term,
    bif: &str,
) -> Result<BifResult, BeamError> {
    let Term::Atom(name) = module else {
        return Ok(BifResult::badarg());
    };
    let Term::Atom(key) = what else {
        return Ok(BifResult::badarg());
    };
    // The capability check borrows the scheduler mutably, so the name comes
    // out of the table first.
    let name_bytes = context.scheduler.atoms.name_of(name).as_bytes().to_vec();
    let check = denial(
        context.scheduler,
        CapabilityKind::Code,
        Operation::Inspect,
        Some(&name_bytes),
    );
    if let Some(denied) = check? {
        return Ok(denied);
    }
    if context
        .scheduler
        .code
        .get(context.scheduler.atoms.name_of(name))
        .is_none()
    {
        return Ok(BifResult::badarg());
    }
    if key.is_static("module") {
        Ok(Value(module.clone()))
    } else if key.is_one_of(&[
        "md5",
        "exports",
        "functions",
        "nifs",
        "attributes",
        "compile",
        "native_addresses",
        "native",
    ]) {
        Err(error(format!(
            "erlang:{bif} key {} is not implemented",
            context.scheduler.atoms.name_of(key)
        )))
    } else {
        Ok(BifResult::badarg())
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
/// When the timer expires, `Dest` gets `{timeout, Reference, Msg}`
/// (`wrap` `!0`, `erl_hl_timer.c:1824`).
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
    bif_timer(context, time, receiver, message, true)
}

/// Run `erlang:send_after(Time, Dest, Msg)`, as ERTS `send_after_3`
/// (`erl_hl_timer.c:2566-2581`) does: the same checks, capability check,
/// reference, and timer as [`start_timer`], but `setup_bif_timer` gets
/// `wrap` 0, so `Dest` gets `Msg` when the timer expires
/// (`erl_hl_timer.c:1824`).
///
/// # Examples
///
/// ```rust,ignore
/// let result = send_after(&mut context, &Term::Integer(100), &Term::Pid(pid), &msg)?;
/// assert!(matches!(result, BifResult::Value(Term::Reference(_))));
/// let result = send_after(&mut context, &Term::Integer(10), &Term::Integer(1), &msg)?;
/// assert!(matches!(result, BifResult::Raise { .. }));
/// ```
///
/// # Errors
///
/// The errors of [`start_timer`].
pub(super) fn send_after(
    context: &mut BifContext<'_>,
    time: &Term,
    receiver: &Term,
    message: &Term,
) -> Result<BifResult, BeamError> {
    bif_timer(context, time, receiver, message, false)
}

/// Start the BIF timer of [`start_timer`] and [`send_after`], as
/// `setup_bif_timer` (`erl_hl_timer.c:1804-1871`) does. `wrap` makes the
/// message `{timeout, Reference, Msg}`, and else the message is `Msg`.
///
/// # Examples
///
/// ```rust,ignore
/// let result = bif_timer(&mut context, &Term::Integer(100), &Term::Pid(pid), &msg, false)?;
/// assert!(matches!(result, BifResult::Value(Term::Reference(_))));
/// ```
///
/// # Errors
///
/// The errors of [`start_timer`].
fn bif_timer(
    context: &mut BifContext<'_>,
    time: &Term,
    receiver: &Term,
    message: &Term,
    wrap: bool,
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
        Receiver::Name(name) => scheduler.whereis(scheduler.atoms.name_of(name)),
    };
    // The timer queue stays with each scheduler, so a timer that fires on
    // another scheduler cannot reach its receiver.
    if target.is_some_and(|target| scheduler.remote_owner(target).is_some()) {
        return Err(error(
            "unsupported: a BIF timer to a process of another scheduler in a run of execute_on_schedulers",
        ));
    }
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
        let message = if wrap {
            Term::tuple(vec![
                Term::static_atom("timeout"),
                Term::Reference(reference),
                message.clone(),
            ])
        } else {
            message.clone()
        };
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
/// assert!(matches!(result, BifResult::Value(Term::Atom(atom)) if atom.is_static("false"));
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
        let value = if value.is_static("true") {
            true
        } else if value.is_static("false") {
            false
        } else {
            return None;
        };
        if name.is_static("async") {
            asynchronous = value;
        } else if name.is_static("info") {
            info = value;
        } else {
            return None;
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
                    if name.is_static("flush") && value.is_one_of(&["true", "false"]) =>
                {
                    flush = value.is_static("true");
                }
                [Term::Atom(name), value] if name.is_static("flush_timeout") => {
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
        Term::Atom(abort) if abort.is_static("abort") => {
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

/// Run `erlang:port_set_data(Port, Data)`, as `port_set_data_2` does
/// (`erl_bif_port.c:687-724`): the open port keeps a copy of `Data`, and
/// the BIF returns `true`. A term that is not an open port raises `badarg`;
/// an atom names no port, because PBeam registers no port. The BIF is not
/// a port signal, so it does not trap. After the port check, a caller
/// without a `port` capability with `command` that permits the name of the
/// port gets `error:capability_denied`, and the data stays.
///
/// # Examples
///
/// ```rust,ignore
/// let result = port_set_data(scheduler, &Term::Port(port), &Term::static_atom("inet_tcp"))?;
/// assert_eq!(result, BifResult::Value(Term::static_atom("true")));
/// ```
///
/// # Errors
///
/// The atom `capability_denied` in a full atom table.
pub(super) fn port_set_data(
    scheduler: &mut Scheduler,
    port: &Term,
    data: &Term,
) -> Result<BifResult, BeamError> {
    let Some(id) = erts_internal::open_port_id(scheduler, port)? else {
        return Ok(BifResult::badarg());
    };
    if let Some(denied) = erts_internal::port_denial(scheduler, id, Operation::Command)? {
        return Ok(denied);
    }
    let ports = scheduler.resources.borrow_mut::<Ports>();
    ports.get_mut(id).expect("the port is open").data = data.clone();
    Ok(BifResult::Value(Term::static_atom("true")))
}

/// Run `erlang:port_get_data(Port)`, as `port_get_data_1` does
/// (`erl_bif_port.c:726-751`): return the data of `port_set_data/2` of the
/// open port, or `undefined` before it. A term that is not an open port
/// raises `badarg`. After the port check, a caller without a `port`
/// capability with `inspect` that permits the name of the port gets
/// `error:capability_denied`.
///
/// # Examples
///
/// ```rust,ignore
/// let result = port_get_data(scheduler, &Term::Port(port))?;
/// assert_eq!(result, BifResult::Value(Term::static_atom("undefined")));
/// ```
///
/// # Errors
///
/// The atom `capability_denied` in a full atom table.
pub(super) fn port_get_data(
    scheduler: &mut Scheduler,
    port: &Term,
) -> Result<BifResult, BeamError> {
    let Some(id) = erts_internal::open_port_id(scheduler, port)? else {
        return Ok(BifResult::badarg());
    };
    if let Some(denied) = erts_internal::port_denial(scheduler, id, Operation::Inspect)? {
        return Ok(denied);
    }
    let ports = scheduler.resources.borrow_mut::<Ports>();
    let data = ports.get(id).expect("the port is open").data.clone();
    Ok(BifResult::Value(data))
}

/// Indices of a chunk of the table that `processes/0` reads in one step
/// (`ERTS_PTAB_LIST_BIF_TAB_CHUNK_SIZE`, `erl_ptab.c:43`).
const PTAB_CHUNK: u64 = 1000;
/// Indices of the table for one reduction
/// (`ERTS_PTAB_LIST_BIF_TAB_INSPECT_INDICES_PER_RED`, `erl_ptab.c:42`).
const PTAB_INDICES_PER_RED: u64 = 25;
/// Reductions that a call needs to read the table before it traps
/// (`ERTS_PTAB_LIST_BIF_MIN_START_REDS`, `erl_ptab.c:44-46`), and the
/// maximum reductions of one step through the deleted processes
/// (`ERTS_PTAB_LIST_INSPECT_DELETED_MAX_REDS`, `erl_ptab.c:52-54`).
const PTAB_CHUNK_REDS: usize = (PTAB_CHUNK / PTAB_INDICES_PER_RED) as usize;
/// Deleted processes for one reduction
/// (`ERTS_PTAB_LIST_BIF_INSPECT_DELETED_PER_RED`, `erl_ptab.c:50`).
const PTAB_DELETED_PER_RED: usize = 10;
/// List cells of the result for one reduction
/// (`ERTS_PTAB_LIST_BIF_BUILD_RESULT_CONSES_PER_RED`, `erl_ptab.c:57`).
const PTAB_CONSES_PER_RED: usize = 75;

/// The state of an `erlang:processes/0` call, which the call keeps across
/// its traps to `erts_internal:ptab_list_continue/5`, as the
/// `ErtsPTabListBifData` of `erl_ptab.c:120-135`. The process table keeps
/// the start of the read and the processes that leave the table during it
/// (see [`Scheduler::ptab_list_start`]).
///
/// Each list is a term that a trap moves to its arguments and a resume
/// moves back, so that no turn copies the state.
pub(super) struct PtabList {
    /// The next table index to read: `-1` before the call starts its read,
    /// [`PTAB_SIZE`] while the call steps through the deleted processes,
    /// and [`PTAB_SIZE`] + 1 while it builds the result.
    pub(super) tix: i64,
    /// The pids that the call found in the table, the last found first.
    pub(super) found: Term,
    /// The pids that exited before the call read their index, the last
    /// first. They come after the found pids in the result.
    pub(super) gone: Term,
    /// The cells of the result that the call did not build yet: the pids of
    /// `found` and `gone`.
    pub(super) pid_ix: i64,
    /// The cells of the result that the call built: the last pids of the
    /// result, in order.
    pub(super) result: Term,
}

impl PtabList {
    /// Return the state as the arguments of the trap export.
    fn into_arguments(self) -> Vec<Term> {
        vec![
            Term::Integer(self.tix),
            self.found,
            self.gone,
            Term::Integer(self.pid_ix),
            self.result,
        ]
    }
}

#[cfg(test)]
thread_local! {
    /// The table entries and the list cells that [`ptab_list`] read or
    /// built since the last reset.
    pub(super) static PTAB_CELLS: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}

/// Run `erlang:processes/0`: return the pid of each process that exists
/// when the call starts its read, also an exiting process, as
/// `erts_ptab_list` (`erl_ptab.c:746-780`, `888-1280`) lists the process
/// table. A call with fewer than 40 reductions traps before the start
/// (`erl_ptab.c:766-768`). See [`ptab_list`] for the steps.
///
/// # Examples
///
/// ```rust,ignore
/// let result = processes(&mut scheduler)?;
/// assert!(matches!(result, BifResult::TrapBif { function: "ptab_list_continue", .. }));
/// ```
///
/// # Errors
///
/// A budget of the current process that is less than 0.
pub(super) fn processes(scheduler: &mut Scheduler) -> Result<BifResult, BeamError> {
    // The process table stays with each scheduler, so this run cannot list
    // the processes of the node yet.
    if scheduler.shared().is_some() {
        return Err(error(
            "unsupported: erlang:processes/0 in a run of execute_on_schedulers",
        ));
    }
    let state = PtabList {
        tix: -1,
        found: Term::Nil,
        gone: Term::Nil,
        pid_ix: 0,
        result: Term::Nil,
    };
    if scheduler.current_process().bif_reductions_left()? < PTAB_CHUNK_REDS {
        return Ok(ptab_list_trap(scheduler, state));
    }
    ptab_list(scheduler, state)
}

/// Return the trap of an `erlang:processes/0` call to its export. The
/// trap uses the rest of the budget as virtual reductions
/// (`ERTS_BIF_PREP_YIELD2`, `bif.h:477-481`).
fn ptab_list_trap(scheduler: &mut Scheduler, state: PtabList) -> BifResult {
    let fcalls = scheduler.current_process().fcalls;
    scheduler.vbump_reductions(fcalls);
    BifResult::TrapBif {
        module: "erts_internal",
        function: "ptab_list_continue",
        arguments: state.into_arguments(),
    }
}

/// Cons `pid` onto `list` and count the new cell of the result.
fn ptab_list_push(list: &mut Term, pid_ix: &mut i64, pid: ProcessId) {
    *list = Term::cons(Term::Pid(pid), std::mem::replace(list, Term::Nil));
    *pid_ix += 1;
}

/// Return the pid of the policy process when the current process is not a
/// boot process, else `None`: a read of the process table by a process
/// that is not a boot process does not give the pid of the policy process
/// (section "Policy module, policy process, and policy table" of
/// `docs/capability-security-migration.md`).
///
/// # Examples
///
/// ```rust,ignore
/// assert_eq!(hidden_policy(&scheduler), scheduler.policy_id());
/// ```
pub(super) fn hidden_policy(scheduler: &Scheduler) -> Option<ProcessId> {
    if scheduler.current_process().security.is_boot() {
        None
    } else {
        scheduler.policy_id()
    }
}

/// Continue an `erlang:processes/0` call with `state`, as
/// `ptab_list_bif_engine` does (`erl_ptab.c:888-1280`), until the call
/// returns its result or uses its reductions and traps.
///
/// - The start of the read marks the processes of the table. A process
///   that enters the table after the start is not in the result
///   (`erl_ptab.c:1000-1003`). A process that is not a boot process does
///   not get the pid of the policy process, so that it cannot send a signal
///   to it (section "Policy module, policy process, and policy table" of
///   `docs/capability-security-migration.md`).
/// - Each turn reads the next chunks of 1000 indices of the table, in the
///   index order of the ERTS table: the pid number modulo [`PTAB_SIZE`].
///   Each chunk costs 1 reduction for each 25 indices. The call reads the
///   next chunk only when it has the reductions for it
///   (`erl_ptab.c:1022-1037`). The first chunk is in the first turn.
/// - Each process that leaves the table during the read costs 1
///   reduction for each 10, and 1 more when no other read steps through
///   it, in a step of at most 40 reductions (`erl_ptab.c:1046-1170`). A marked process that left the
///   table before the call read its index comes after the found pids, in
///   the order of the exits.
/// - The result costs 1 reduction for each 75 cells (`erl_ptab.c:1201-1254`).
///
/// Row "BIF arguments" of `docs/erts-differences.md`
/// gives the differences.
///
/// # Errors
///
/// A budget of the current process that is less than 0.
pub(super) fn ptab_list(
    scheduler: &mut Scheduler,
    mut state: PtabList,
) -> Result<BifResult, BeamError> {
    let hidden = hidden_policy(scheduler);
    if state.tix < 0 {
        scheduler.ptab_list_start();
        state.tix = 0;
    }
    loop {
        let left = scheduler.current_process().bif_reductions_left()?;
        let have = if state.tix < PTAB_SIZE as i64 {
            let end = (state.tix as u64 + PTAB_CHUNK).min(PTAB_SIZE);
            for id in scheduler.ptab_list_chunk(end) {
                #[cfg(test)]
                PTAB_CELLS.set(PTAB_CELLS.get() + 1);
                if Some(id) != hidden {
                    ptab_list_push(&mut state.found, &mut state.pid_ix, id);
                }
            }
            let reds = (end - state.tix as u64) / PTAB_INDICES_PER_RED;
            state.tix = end as i64;
            let have = left.saturating_sub(reds as usize);
            let next = (end + PTAB_CHUNK).min(PTAB_SIZE) - end;
            scheduler.current_process_mut().bump_reductions(reds as i32);
            if (next / PTAB_INDICES_PER_RED) as usize > have {
                0
            } else {
                have
            }
        } else if state.tix == PTAB_SIZE as i64 {
            let max = left.min(PTAB_CHUNK_REDS);
            let (mut reds, mut taken) = (0, 0);
            while reds < max
                && let Some((entry, free)) = scheduler.ptab_list_deleted()
            {
                #[cfg(test)]
                PTAB_CELLS.set(PTAB_CELLS.get() + 1);
                taken += 1;
                reds += usize::from(taken % PTAB_DELETED_PER_RED == 0) + usize::from(free);
                if let Some(id) = entry
                    && Some(id) != hidden
                {
                    ptab_list_push(&mut state.gone, &mut state.pid_ix, id);
                }
            }
            if scheduler.ptab_list_deleted_done() {
                scheduler.ptab_list_end();
                state.tix += 1;
            }
            scheduler.current_process_mut().bump_reductions(reds as i32);
            left.saturating_sub(reds)
        } else {
            // The result cells come from the end of the result to its
            // start: the gone pids, the last first, then the found pids, as
            // `ptab_list_bif_engine` conses its pid array from the end
            // (`erl_ptab.c:1201-1254`).
            let conses = (PTAB_CONSES_PER_RED * left).min(state.pid_ix as usize);
            for _ in 0..conses {
                let list = if state.gone.is_nil() {
                    &mut state.found
                } else {
                    &mut state.gone
                };
                let (pid, tail) = list
                    .cons_cell()
                    .expect("the result has a pid for each cell");
                let pid = pid.clone();
                *list = tail.clone();
                #[cfg(test)]
                PTAB_CELLS.set(PTAB_CELLS.get() + 1);
                state.result = Term::cons(pid, std::mem::replace(&mut state.result, Term::Nil));
            }
            state.pid_ix -= conses as i64;
            let reds = conses / PTAB_CONSES_PER_RED;
            scheduler.current_process_mut().bump_reductions(reds as i32);
            left - reds
        };
        if state.tix > PTAB_SIZE as i64 && state.pid_ix == 0 {
            return Ok(Value(state.result));
        }
        if have == 0 {
            return Ok(ptab_list_trap(scheduler, state));
        }
    }
}

/// Run `erlang:term_to_binary(Term, [local])` for an ordinary reference, as
/// `term_to_binary_2` does with `DFLAG_LOCAL_EXT` (external.c:1499,
/// 1565-1590): the `LOCAL_EXT` bytes of the reference with the hash of the
/// node (see [`external::local_reference`]).
///
/// # Examples
///
/// ```rust,ignore
/// let local = Term::list_from(vec![Term::static_atom("local")]);
/// let result = term_to_binary_2(&mut context, &Term::Reference(reference), &local)?;
/// ```
///
/// # Errors
///
/// Each other term and each other option list, which PBeam does not encode
/// yet.
pub(super) fn term_to_binary_2(
    context: &mut BifContext<'_>,
    term: &Term,
    options: &Term,
) -> Result<BifResult, BeamError> {
    let local = Term::list_from(vec![Term::static_atom("local")]);
    match term {
        Term::Reference(reference) if *options == local && !reference.is_pid_reference() => {
            let seed = context.scheduler.local_node_hash();
            Ok(Value(Term::binary(external::local_reference(
                *reference, seed,
            ))))
        }
        _ => Err(error(
            "unsupported erlang:term_to_binary/2 other than an ordinary reference with [local]: see \"External term format\" in docs/erts-differences.md",
        )),
    }
}

#[cfg(test)]
#[path = "../../../../tests/runtime/interpreter/bif/erlang.rs"]
mod tests;

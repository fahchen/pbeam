//! Bodies of the `erlang` BIFs that do not fit on one table line.

use super::BifResult::{Trap, Value};
use super::*;
use crate::term::{Fun, Reference};

/// Return the node of a pid or of the calling process.
///
/// PBeam is a local node that is not distributed, so every pid is local and
/// its node is `nonode@nohost`, as on ERTS (`dist.c`).
///
/// # Examples
///
/// ```rust,ignore
/// assert_eq!(local_node(), Term::static_atom("nonode@nohost"));
/// ```
pub(super) fn local_node() -> Term {
    Term::static_atom("nonode@nohost")
}

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

/// Run `erlang:process_flag(trap_exit, Value)`: set the `trap_exit` flag of
/// the calling process and return the old flag. The table pattern accepts
/// only `true` and `false` as the value.
///
/// # Examples
///
/// ```rust,ignore
/// let old = trap_exit(&mut context, &true_atom)?;
/// assert!(matches!(old, BifResult::Value(Term::Atom(atom)) if atom.name() == "false"));
/// ```
pub(super) fn trap_exit(
    context: &mut BifContext<'_>,
    value: &Atom,
) -> Result<BifResult, BeamError> {
    let process = context.scheduler.current_process_mut();
    let old = Term::static_atom(&process.trap_exit.to_string());
    process.trap_exit = value.name() == "true";
    Ok(Value(old))
}

/// Run `erlang:demonitor(Reference, Options)` with the options `[]` or
/// `[flush]`. Other options give a runtime error.
///
/// ERTS `demonitor_2` (bif.c) flushes only when no active monitor was
/// removed: the DOWN message can then be queued. The flush traps into
/// `erts_internal:flush_monitor_messages/3`.
///
/// # Examples
///
/// ```rust,ignore
/// let result = demonitor(&mut context, reference, &[])?;
/// assert!(matches!(result, BifResult::Value(Term::Atom(atom)) if atom.name() == "true"));
/// let result = demonitor(&mut context, gone, &[Term::static_atom("flush")])?;
/// assert!(matches!(result, BifResult::Trap { function: "flush_monitor_messages", .. }));
/// ```
pub(super) fn demonitor(
    context: &mut BifContext<'_>,
    reference: Reference,
    options: &[Term],
) -> Result<BifResult, BeamError> {
    let flush = match options {
        [] => false,
        [Term::Atom(option)] if option.name() == "flush" => true,
        _ => return Err(error(format!("unsupported demonitor options {options:?}"))),
    };
    let removed = context.scheduler.demonitor(reference);
    if removed || !flush {
        return Ok(Value(Term::static_atom("true")));
    }
    Ok(Trap {
        module: "erts_internal",
        function: "flush_monitor_messages",
        arguments: vec![
            Term::Reference(reference),
            Term::static_atom("false"),
            Term::static_atom("true"),
        ],
    })
}

/// Run `erlang:spawn/3`: start a process that runs `module:function` with the
/// elements of `arguments`, as ERTS `spawn_3` does with default spawn options.
///
/// # Examples
///
/// ```rust,ignore
/// let result = spawn(&mut context, &module, &function, &Term::List(vec![]))?;
/// assert!(matches!(result, BifResult::Value(Term::Pid(_))));
/// ```
pub(super) fn spawn(
    context: &mut BifContext<'_>,
    module: &Atom,
    function: &Atom,
    arguments: &Term,
) -> Result<BifResult, BeamError> {
    create_process(
        context.program,
        context.scheduler,
        module,
        function,
        arguments,
        SpawnOptions::default(),
    )
    .map(Value)
}

/// Run `erlang:spawn_link/3`: start a linked process, as ERTS `spawn_link_3`
/// does by adding `SPO_LINK` to the default spawn options.
///
/// # Examples
///
/// ```rust,ignore
/// let result = spawn_link(&mut context, &module, &function, &Term::List(vec![]))?;
/// assert!(matches!(result, BifResult::Value(Term::Pid(_))));
/// ```
pub(super) fn spawn_link(
    context: &mut BifContext<'_>,
    module: &Atom,
    function: &Atom,
    arguments: &Term,
) -> Result<BifResult, BeamError> {
    let options = SpawnOptions {
        link: true,
        ..SpawnOptions::default()
    };
    create_process(
        context.program,
        context.scheduler,
        module,
        function,
        arguments,
        options,
    )
    .map(Value)
}

/// Run `erlang:spawn_opt/4`, which `erlang:spawn_monitor/3` in `erlang.beam`
/// calls with the options `[monitor]`: start a process with the spawn
/// options in `options`, as ERTS `spawn_opt_4` (`bif.c`) does. The option
/// `link` links the new process and `monitor` monitors it. With `monitor`
/// the result is `{Pid, Reference}`, else `Pid`. Any other option gives a
/// runtime error.
///
/// # Examples
///
/// ```rust,ignore
/// let result = spawn_opt(&mut context, &module, &function, &Term::List(vec![]), &[monitor])?;
/// assert!(matches!(result, BifResult::Value(Term::Tuple(_))));
/// assert!(spawn_opt(&mut context, &module, &function, &Term::List(vec![]), &[priority]).is_err());
/// ```
pub(super) fn spawn_opt(
    context: &mut BifContext<'_>,
    module: &Atom,
    function: &Atom,
    arguments: &Term,
    options: &[Term],
) -> Result<BifResult, BeamError> {
    let mut spawn_options = SpawnOptions::default();
    for option in options {
        match option {
            Term::Atom(option) if option.name() == "link" => spawn_options.link = true,
            Term::Atom(option) if option.name() == "monitor" => spawn_options.monitor = true,
            // ponytail: ERTS accepts more options; see "Spawn options" in docs/erts-differences.md.
            _ => return Err(error(format!("unsupported spawn option {option:?}"))),
        }
    }
    create_process(
        context.program,
        context.scheduler,
        module,
        function,
        arguments,
        spawn_options,
    )
    .map(Value)
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

/// Run `erlang:'++'/2`: append `right` to the proper list `left`. With an
/// empty `left`, return `right` itself, which can be any term, as ERTS
/// `append` (`erl_bif_lists.c`) does. A `left` that is not a list raises
/// `badarg`. A non-empty `left` with a `right` that is not a list gives a
/// runtime error, because PBeam has no improper lists. ERTS returns the
/// improper list `[1 | x]` for `[1] ++ x`.
///
/// # Examples
///
/// ```rust,ignore
/// let x = Term::static_atom("ok");
/// assert!(matches!(append(&Term::List(vec![]), &x)?, BifResult::Value(term) if term == x));
/// let list = append(&Term::List(vec![Term::Integer(1)]), &Term::List(vec![Term::Integer(2)]))?;
/// assert!(matches!(list, BifResult::Value(Term::List(elements)) if elements.len() == 2));
/// assert!(matches!(append(&x, &Term::List(vec![]))?, BifResult::Raise { .. }));
/// assert!(append(&Term::List(vec![Term::Integer(1)]), &x).is_err());
/// ```
pub(super) fn append(left: &Term, right: &Term) -> Result<BifResult, BeamError> {
    match (left, right) {
        (Term::List(left), right) if left.is_empty() => Ok(Value(right.clone())),
        (Term::List(left), Term::List(right)) => Ok(Value(Term::List(
            left.iter().chain(right).cloned().collect(),
        ))),
        // ponytail: an improper list result needs an improper list term; see "BIF arguments" in docs/erts-differences.md.
        (Term::List(_), right) => Err(error(format!(
            "unsupported erlang:++/2 tail {right:?}: not a list"
        ))),
        _ => Ok(BifResult::badarg()),
    }
}

/// Run `erlang:element/2`: return the element at the 1-based `index` of a
/// tuple. An `index` that is not an integer or is outside the tuple, and a
/// `tuple` that is not a tuple, raise `badarg`, as in ERTS `element_2`
/// (`bif.c`).
///
/// # Examples
///
/// ```rust,ignore
/// let tuple = Term::Tuple(vec![Term::static_atom("ok"), Term::Integer(1)]);
/// assert!(matches!(element(&Term::Integer(2), &tuple), BifResult::Value(Term::Integer(1))));
/// assert!(matches!(element(&Term::Integer(3), &tuple), BifResult::Raise { .. }));
/// assert!(matches!(element(&Term::Integer(1), &Term::List(vec![])), BifResult::Raise { .. }));
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

/// The largest binary, in bytes, that `erlang:iolist_to_iovec/1` copies into
/// the merged binary of its result. ERTS `IOL2V_COPY_THRESHOLD`
/// (`erl_iolist.c`) is `ERL_ONHEAP_BITS_LIMIT * 8` bits, and
/// `ERL_ONHEAP_BITS_LIMIT` (`erl_bits.h`) is `64 * 8` bits, so the limit is
/// 4096 bits, or 512 bytes. A larger binary is an element of the result.
const IOVEC_COPY_LIMIT: usize = 512;

/// Run `erlang:iolist_to_iovec/1` as ERTS `iolist_to_iovec_1` and
/// `iolist_to_iovec_engine` (`erl_iolist.c`) do. Return `[]` for `[]` and
/// `<<>>`, and `[Bin]` for a binary `Bin` that is not empty. For a list, walk
/// the nested lists in order. Copy each byte and each binary of at most
/// [`IOVEC_COPY_LIMIT`] bytes into one merged binary. Each larger binary
/// closes the current merged binary, if it is not empty, and is an element of
/// the result itself. An empty merged binary is not in the result. An integer
/// outside `0..=255` or another term that is not iodata raises `badarg`.
///
/// ERTS also copies a sub-binary at a bit offset that is not a multiple of 8,
/// whatever its size. PBeam binaries always start at a byte, so only the size
/// selects the rule.
///
/// # Examples
///
/// ```rust,ignore
/// let hello = Term::Binary(b"Hello".to_vec());
/// assert!(matches!(iolist_to_iovec(&hello), BifResult::Value(Term::List(iovec)) if iovec == [hello]));
/// assert!(matches!(iolist_to_iovec(&Term::Binary(Vec::new())), BifResult::Value(Term::List(iovec)) if iovec.is_empty()));
/// // [1, [<<2>>]] gives [<<1,2>>].
/// let list = Term::List(vec![Term::Integer(1), Term::List(vec![Term::Binary(vec![2])])]);
/// assert!(matches!(iolist_to_iovec(&list), BifResult::Value(Term::List(iovec)) if iovec == [Term::Binary(vec![1, 2])]));
/// // [1, <<0:513/unit:8>>] gives [<<1>>, <<0:513/unit:8>>].
/// let large = Term::Binary(vec![0; 513]);
/// let list = Term::List(vec![Term::Integer(1), large.clone()]);
/// assert!(matches!(iolist_to_iovec(&list), BifResult::Value(Term::List(iovec)) if iovec == [Term::Binary(vec![1]), large]));
/// assert!(matches!(iolist_to_iovec(&Term::List(vec![Term::Integer(256)])), BifResult::Raise { .. }));
/// assert!(matches!(iolist_to_iovec(&Term::static_atom("foo")), BifResult::Raise { .. }));
/// ```
pub(super) fn iolist_to_iovec(term: &Term) -> BifResult {
    match term {
        Term::Binary(bytes) if bytes.is_empty() => Value(Term::List(Vec::new())),
        Term::Binary(_) => Value(Term::List(vec![term.clone()])),
        Term::List(elements) => {
            iolist_iovec(elements).map_or_else(BifResult::badarg, |iovec| Value(Term::List(iovec)))
        }
        _ => BifResult::badarg(),
    }
}

/// Convert the elements of an iolist to the iovec of
/// `erlang:iolist_to_iovec/1`, or return `None` when an element is not
/// iodata. Use a stack of list iterators, as the ERTS engine does, so that a
/// deep nested list does not use the Rust call stack.
///
/// # Examples
///
/// ```rust,ignore
/// let iolist = [Term::List(vec![]), Term::Binary(vec![]), Term::Integer(7)];
/// assert_eq!(iolist_iovec(&iolist), Some(vec![Term::Binary(vec![7])]));
/// assert_eq!(iolist_iovec(&[Term::Integer(-1)]), None);
/// ```
fn iolist_iovec(elements: &[Term]) -> Option<Vec<Term>> {
    let mut iovec = Vec::new();
    let mut merged = Vec::new();
    let mut lists = vec![elements.iter()];
    while let Some(list) = lists.last_mut() {
        match list.next() {
            None => {
                lists.pop();
            }
            Some(Term::List(inner)) => lists.push(inner.iter()),
            Some(Term::Integer(byte)) => merged.push(u8::try_from(*byte).ok()?),
            Some(Term::Binary(bytes)) if bytes.len() <= IOVEC_COPY_LIMIT => {
                merged.extend_from_slice(bytes);
            }
            Some(binary @ Term::Binary(_)) => {
                if !merged.is_empty() {
                    iovec.push(Term::Binary(std::mem::take(&mut merged)));
                }
                iovec.push(binary.clone());
            }
            Some(_) => return None,
        }
    }
    if !merged.is_empty() {
        iovec.push(Term::Binary(merged));
    }
    Some(iovec)
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
/// let lookup = Term::Tuple(vec![Term::static_atom("dictionary"), Term::Integer(1)]);
/// assert!(is_process_info_item(&lookup));
/// assert!(!is_process_info_item(&Term::static_atom("ok")));
/// ```
fn is_process_info_item(item: &Term) -> bool {
    match item {
        Term::Atom(name) => PROCESS_INFO_ITEMS.contains(&name.name()),
        Term::Tuple(elements) => {
            matches!(elements.as_slice(), [Term::Atom(key), _] if key.name() == "dictionary")
        }
        _ => false,
    }
}

/// Run `erlang:process_info/2`. As `process_info_bif` (`erl_bif_info.c`)
/// does, first raise `badarg` when `item` is not an item or a proper list of
/// items, then raise `badarg` when `process` is not a pid. Return
/// `undefined` for each valid item when the process is not alive
/// (`erl_bif_info.c:1384-1386`). For a live process, return
/// `{registered_name, Name}`, or `[]` when the process has no name, as
/// `process_info_aux` does. For the calling process, return
/// `{current_stacktrace, []}`, because PBeam records no stack frames. The
/// `current_stacktrace` of another process, a list of items, and any other
/// item give a runtime error.
///
/// # Examples
///
/// ```rust,ignore
/// let process = Term::Pid(context.scheduler.current_process_mut().id);
/// let item = Term::static_atom("current_stacktrace");
/// let info = process_info(&mut context, &process, &item)?;
/// assert!(matches!(info, BifResult::Value(Term::Tuple(elements)) if elements[1] == Term::List(vec![])));
/// let name = process_info(&mut context, &process, &Term::static_atom("registered_name"))?;
/// assert!(matches!(name, BifResult::Value(Term::List(elements)) if elements.is_empty()));
/// let dead = process_info(&mut context, &Term::Pid(ProcessId(99)), &item)?;
/// assert!(matches!(dead, BifResult::Value(Term::Atom(atom)) if atom.name() == "undefined"));
/// let bogus = process_info(&mut context, &process, &Term::static_atom("bogus_item"))?;
/// assert!(matches!(bogus, BifResult::Raise { .. }));
/// assert!(process_info(&mut context, &process, &Term::static_atom("status")).is_err());
/// ```
pub(super) fn process_info(
    context: &mut BifContext<'_>,
    process: &Term,
    item: &Term,
) -> Result<BifResult, BeamError> {
    let valid = match item {
        Term::List(items) => items.iter().all(is_process_info_item),
        item => is_process_info_item(item),
    };
    let (true, Term::Pid(id)) = (valid, process) else {
        return Ok(BifResult::badarg());
    };
    let current = context.scheduler.current_process_mut().id;
    let Some(target) = context.scheduler.process(*id) else {
        return Ok(Value(Term::static_atom("undefined")));
    };
    let info = match item {
        Term::Atom(name) => match (name.name(), &target.registered_name) {
            // ponytail: PBeam records no stack frames; see "Exception stack trace" in docs/erts-differences.md.
            ("current_stacktrace", _) if *id == current => {
                Some(Term::Tuple(vec![item.clone(), Term::List(Vec::new())]))
            }
            ("registered_name", Some(registered)) => Some(Term::Tuple(vec![
                item.clone(),
                Term::Atom(registered.clone()),
            ])),
            ("registered_name", None) => Some(Term::List(Vec::new())),
            _ => None,
        },
        _ => None,
    };
    // ponytail: lists of items, other items, and the frames of another process need more state; see "BIF arguments" in docs/erts-differences.md.
    info.map(Value).ok_or_else(|| {
        error(format!(
            "unsupported erlang:process_info/2 arguments {process:?}, {item:?}"
        ))
    })
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
/// let exported = function_exported(&program, &erlang, &Term::static_atom("self"), &Term::Integer(0));
/// assert!(matches!(exported, BifResult::Value(Term::Atom(atom)) if atom.name() == "true"));
/// let exported = function_exported(&program, &erlang, &Term::static_atom("self"), &Term::Integer(1 << 59));
/// assert!(matches!(exported, BifResult::Raise { .. }));
/// ```
pub(super) fn function_exported(
    program: &BeamProgram,
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
        program
            .modules
            .get(module)
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

//! Bodies of the `erlang` BIFs that do not fit on one table line.

use super::BifResult::{Trap, Value};
use super::*;
use crate::term::Reference;

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
/// empty `left`, return `right` itself, which can be any term, as ERTS does.
/// A non-empty `left` with a `right` that is not a list gives a runtime
/// error, because PBeam has no improper lists. ERTS returns the improper
/// list `[1 | x]` for `[1] ++ x`.
///
/// # Examples
///
/// ```rust,ignore
/// let x = Term::static_atom("ok");
/// assert!(matches!(append(&[], &x)?, BifResult::Value(term) if term == x));
/// let list = append(&[Term::Integer(1)], &Term::List(vec![Term::Integer(2)]))?;
/// assert!(matches!(list, BifResult::Value(Term::List(elements)) if elements.len() == 2));
/// assert!(append(&[Term::Integer(1)], &x).is_err());
/// ```
pub(super) fn append(left: &[Term], right: &Term) -> Result<BifResult, BeamError> {
    match (left, right) {
        ([], right) => Ok(Value(right.clone())),
        (left, Term::List(right)) => Ok(Value(Term::List(
            left.iter().chain(right).cloned().collect(),
        ))),
        // ponytail: an improper list result needs an improper list term; see "BIF arguments" in docs/erts-differences.md.
        _ => Err(error(format!(
            "unsupported erlang:++/2 tail {right:?}: not a list"
        ))),
    }
}

/// Run `erlang:element/2`: return the element at the 1-based `index` of a
/// tuple. An `index` outside the tuple gives a runtime error.
///
/// # Examples
///
/// ```rust,ignore
/// let tuple = [Term::static_atom("ok"), Term::Integer(1)];
/// assert!(matches!(element(2, &tuple)?, BifResult::Value(Term::Integer(1))));
/// assert!(element(0, &tuple).is_err());
/// assert!(element(3, &tuple).is_err());
/// ```
pub(super) fn element(index: i64, elements: &[Term]) -> Result<BifResult, BeamError> {
    // ponytail: ERTS raises badarg; see "BIF arguments" in docs/erts-differences.md.
    let element = usize::try_from(index)
        .ok()
        .and_then(|index| index.checked_sub(1))
        .and_then(|index| elements.get(index))
        .ok_or_else(|| {
            error(format!(
                "unsupported erlang:element/2 index {index}: outside the tuple"
            ))
        })?;
    Ok(Value(element.clone()))
}

/// Run `erlang:process_info/2` for the calling process and the item
/// `current_stacktrace`. Return `{current_stacktrace, []}`, because PBeam
/// records no stack frames. Any other process or item gives a runtime error.
///
/// # Examples
///
/// ```rust,ignore
/// let process = Term::Pid(context.scheduler.current_process_mut().id);
/// let item = Term::static_atom("current_stacktrace");
/// let info = process_info(&mut context, &process, &item)?;
/// assert!(matches!(info, BifResult::Value(Term::Tuple(elements)) if elements[1] == Term::List(vec![])));
/// assert!(process_info(&mut context, &process, &Term::static_atom("registered_name")).is_err());
/// ```
pub(super) fn process_info(
    context: &mut BifContext<'_>,
    process: &Term,
    item: &Term,
) -> Result<BifResult, BeamError> {
    let current = Term::Pid(context.scheduler.current_process_mut().id);
    let stacktrace = Term::static_atom("current_stacktrace");
    // ponytail: other processes and items need process lookup and more state; see "BIF arguments" in docs/erts-differences.md.
    if *process != current || *item != stacktrace {
        return Err(error(format!(
            "unsupported erlang:process_info/2 arguments {process:?}, {item:?}"
        )));
    }
    // ponytail: PBeam records no stack frames; see "Exception stack trace" in docs/erts-differences.md.
    Ok(Value(Term::Tuple(vec![stacktrace, Term::List(Vec::new())])))
}

/// Run `erlang:function_exported/3`: return `true` when a loaded module
/// exports the function or the function is a BIF, as ERTS
/// `function_exported_3` (`bif.c`) does with `erts_find_function` and
/// `erts_is_builtin`. ERTS reads the arity into a C `int`, so only its low
/// 32 bits count, and a negative arity finds no function.
///
/// # Examples
///
/// ```rust,ignore
/// assert!(matches!(function_exported(&program, "erlang", "self", 0), BifResult::Value(Term::Atom(atom)) if atom.name() == "true"));
/// assert!(matches!(function_exported(&program, "erlang", "apply", 3), BifResult::Value(Term::Atom(atom)) if atom.name() == "true"));
/// assert!(matches!(function_exported(&program, "lists", "nomatch", 2), BifResult::Value(Term::Atom(atom)) if atom.name() == "false"));
/// ```
pub(super) fn function_exported(
    program: &BeamProgram,
    module: &str,
    name: &str,
    arity: i64,
) -> BifResult {
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

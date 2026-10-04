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

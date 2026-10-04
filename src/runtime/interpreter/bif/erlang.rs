//! Bodies of the `erlang` BIFs that do not fit on one table line.

use super::BifResult::{Trap, Value};
use super::*;
use crate::term::{ProcessId, Reference};

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

/// Run `erlang:group_leader(Leader, Target)`: make `leader` the group leader
/// of `target` and return `true`.
///
/// # Examples
///
/// ```rust,ignore
/// let result = set_group_leader(&mut context, target, leader)?;
/// assert!(matches!(result, BifResult::Value(Term::Atom(atom)) if atom.name() == "true"));
/// ```
pub(super) fn set_group_leader(
    context: &mut BifContext<'_>,
    target: ProcessId,
    leader: ProcessId,
) -> Result<BifResult, BeamError> {
    context.scheduler.set_group_leader(target, leader)?;
    Ok(Value(Term::static_atom("true")))
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

/// Run `erlang:spawn/3`, `erlang:spawn_link/3`, or `erlang:spawn_monitor/3`:
/// start a process that runs `module:function` with the elements of
/// `arguments`. Return the pid, or `{Pid, Reference}` with `monitor`.
///
/// # Examples
///
/// ```rust,ignore
/// let arguments = Term::List(vec![]);
/// let result = spawn(&mut context, &module, &function, &arguments, false, true)?;
/// assert!(matches!(result, BifResult::Value(Term::Pid(_))));
/// assert!(spawn(&mut context, &module, &function, &Term::Integer(1), false, false).is_err());
/// ```
pub(super) fn spawn(
    context: &mut BifContext<'_>,
    module: &Atom,
    function: &Atom,
    arguments: &Term,
    monitor: bool,
    link: bool,
) -> Result<BifResult, BeamError> {
    let Term::List(arguments) = arguments else {
        return Err(error("spawn arguments are not a proper list"));
    };
    let arguments = arguments.clone();
    let arity = u32::try_from(arguments.len()).map_err(|_| error("arity exceeds BEAM limit"))?;
    let frame = context
        .program
        .mfa_frame(module.name(), function.name(), arity)?;
    let (pid, reference) = context.scheduler.spawn(frame, arguments, monitor, link)?;
    Ok(Value(match reference {
        Some(reference) => Term::Tuple(vec![Term::Pid(pid), Term::Reference(reference)]),
        None => Term::Pid(pid),
    }))
}

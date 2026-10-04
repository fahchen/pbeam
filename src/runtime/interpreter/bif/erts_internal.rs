//! Bodies of the `erts_internal` BIFs that do not fit on one table line.

use super::BifResult::Value;
use super::*;

/// Run `erts_internal:group_leader(Leader, Target)`, which
/// `erlang:group_leader/2` in `erlang.beam` calls: set the group leader of
/// the local process `Target` to `Leader` and return `true`. Return the atom
/// `badarg` without an exception when `Leader` is not a pid, or when `Target`
/// is not a pid or not a live process, as ERTS
/// `erts_internal_group_leader_2` (`bif.c`) does.
///
/// ERTS returns `false` when another target has signals in its queue that it
/// has not handled yet. `erlang:group_leader/2` then sends the change as a
/// signal through `erts_internal:group_leader/3`. PBeam puts each message
/// and signal into its target when it is sent, so it never has such signals
/// and always sets the group leader directly.
///
/// # Examples
///
/// ```rust,ignore
/// let result = group_leader(&mut context, &Term::Pid(leader), &Term::Pid(target))?;
/// assert!(matches!(result, BifResult::Value(Term::Atom(atom)) if atom.name() == "true"));
/// let result = group_leader(&mut context, &Term::Integer(1), &Term::Pid(target))?;
/// assert!(matches!(result, BifResult::Value(Term::Atom(atom)) if atom.name() == "badarg"));
/// ```
pub(super) fn group_leader(
    context: &mut BifContext<'_>,
    leader: &Term,
    target: &Term,
) -> Result<BifResult, BeamError> {
    let set = match (leader, target) {
        (Term::Pid(leader), Term::Pid(target)) => {
            context.scheduler.set_group_leader(*target, *leader)
        }
        _ => false,
    };
    Ok(Value(Term::static_atom(if set {
        "true"
    } else {
        "badarg"
    })))
}

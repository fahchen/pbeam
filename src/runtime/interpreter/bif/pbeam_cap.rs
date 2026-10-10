//! Bodies of the BIFs of `pbeam_cap`, the PBeam module of capability
//! requests. ERTS has no such module, so a call raises `error:undef` there
//! (row "Capability checks" of `docs/erts-differences.md`).

use super::BifResult::Value;
use super::*;
use crate::runtime::capability::{CapabilityKind, Denied};

/// Run `pbeam_cap:revoke(Pid, Kind)`: revoke the copies of the capability
/// kind `Kind` that the caller gave to the process `Pid` at spawn, and
/// return `ok`. Each copy gets a revocation signal, and its holder sends
/// the revocation on to the copies that it gave, when it handles the
/// signal. The revocation also reaches the copies below a holder that
/// exited (see `Scheduler::revoke_given`). The call does not wait for the
/// signals and does not end a holder.
///
/// A `Pid` that is not a pid, and a `Kind` that is not the atom of a
/// capability kind, raise `error:badarg`. When the caller gave `Pid` no
/// copy of `Kind` that still has a delegation record, the call raises
/// `error:capability_denied`: a process can revoke only a capability that
/// it gave.
///
/// # Examples
///
/// ```rust,ignore
/// // The caller spawned `child`.
/// let result = revoke(&mut scheduler, &Term::Pid(child), &Term::static_atom("process"))?;
/// assert!(matches!(result, BifResult::Value(_)));
/// ```
///
/// # Errors
///
/// The errors of `AtomTable::put` for the atom `capability_denied` in a
/// full atom table.
pub(super) fn revoke(
    scheduler: &mut Scheduler,
    holder: &Term,
    kind: &Term,
) -> Result<BifResult, BeamError> {
    let (Term::Pid(holder), Term::Atom(kind)) = (holder, kind) else {
        return Ok(BifResult::badarg());
    };
    let Some(kind) = CapabilityKind::named(kind.name()) else {
        return Ok(BifResult::badarg());
    };
    match scheduler.revoke_given(*holder, kind) {
        Ok(()) => Ok(Value(Term::static_atom("ok"))),
        Err(Denied) => BifResult::capability_denied(&mut scheduler.atoms),
    }
}

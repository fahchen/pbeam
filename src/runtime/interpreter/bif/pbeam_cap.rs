//! Bodies of the BIFs of `pbeam_cap`, the PBeam module of capability
//! requests. ERTS has no such module, so a call raises `error:undef` there
//! (rows "Capability checks" and "Policy module" of
//! `docs/erts-differences.md`).

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
/// it gave. In a run that skips the checks, the call returns `ok` after the
/// argument checks and has no effect.
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

/// Run `pbeam_cap:request(Kind, Request)`: ask the policy process for a
/// capability of the kind `Kind`. The policy process calls
/// `grant(Kind, Request, Requester)` of the policy module, and the caller
/// waits for the grant signal (see [`BifResult::AwaitGrant`]). A grant adds
/// the capability to the caller, and the call returns `ok`; a denial raises
/// `error:capability_denied` (see [`await_grant`]).
///
/// A `Kind` that is not the atom of a capability kind raises `error:badarg`.
/// `Request` can be any term. A run with a Rust policy has no policy
/// process, so the call raises `error:capability_denied` at once, and so
/// does a call after the policy process exited and a call of the policy
/// process itself. In a run that skips the checks, the call returns `ok`
/// after the check of `Kind`: it calls no policy code and adds no
/// capability.
///
/// # Examples
///
/// ```rust,ignore
/// let result = request(&mut scheduler, &Term::static_atom("process"), &Term::List(vec![]))?;
/// assert!(matches!(result, BifResult::AwaitGrant));
/// ```
///
/// # Errors
///
/// A full atom table.
pub(super) fn request(
    scheduler: &mut Scheduler,
    kind: &Term,
    request: &Term,
) -> Result<BifResult, BeamError> {
    if !matches!(kind, Term::Atom(kind) if CapabilityKind::named(kind.name()).is_some()) {
        return Ok(BifResult::badarg());
    }
    if scheduler.skips_checks() {
        return Ok(Value(Term::static_atom("ok")));
    }
    match scheduler.request_grant(kind.clone(), request.clone())? {
        Ok(()) => Ok(BifResult::AwaitGrant),
        Err(Denied) => BifResult::capability_denied(&mut scheduler.atoms),
    }
}

/// Run the export `pbeam_cap:await_grant/0` of a waiting
/// `pbeam_cap:request/2` call: return `ok` after a grant, raise
/// `error:capability_denied` after a denial, or wait again when the grant
/// signal did not come yet, for example when a message woke the caller.
///
/// # Examples
///
/// ```rust,ignore
/// scheduler.current_process_mut().policy_reply = Some(true);
/// assert!(matches!(await_grant(&mut scheduler)?, BifResult::Value(_)));
/// ```
///
/// # Errors
///
/// The errors of `AtomTable::put` for the atom `capability_denied` in a
/// full atom table.
pub(super) fn await_grant(scheduler: &mut Scheduler) -> Result<BifResult, BeamError> {
    match scheduler.current_process_mut().policy_reply.take() {
        Some(true) => Ok(Value(Term::static_atom("ok"))),
        Some(false) => BifResult::capability_denied(&mut scheduler.atoms),
        None => Ok(BifResult::AwaitGrant),
    }
}

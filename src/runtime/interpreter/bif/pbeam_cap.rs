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
    let Some(kind) = CapabilityKind::named(scheduler.atoms.name_of(kind)) else {
        return Ok(BifResult::badarg());
    };
    match scheduler.revoke_given(*holder, kind) {
        Ok(()) => Ok(Value(Term::static_atom("ok"))),
        Err(Denied) => BifResult::capability_denied(&mut scheduler.atoms),
    }
}

/// Run `pbeam_cap:policy_revoke(Pid, Kind)`: revoke the copies of the
/// capability kind `Kind` of the process `Pid`, whatever their source, and
/// return `ok`. Only the policy process can call it, in a call of the
/// policy code. Each copy gets a revocation signal from the policy process,
/// and its holder sends the revocation on to the copies that it gave, when
/// it handles the signal. After the exit of `Pid`, the revocation goes
/// through its released records to the copies below them (see
/// `Scheduler::policy_revoke`). The call does not wait for the signals and
/// does not end a holder.
///
/// A `Pid` that is not a pid, and a `Kind` that is not the atom of a
/// capability kind, raise `error:badarg`. Another caller than the policy
/// process raises `error:capability_denied`. A `Pid` with no copy of `Kind`
/// returns `ok` with no effect. In a run that skips the checks, the call
/// returns `ok` after the argument checks and has no effect.
///
/// # Examples
///
/// ```rust,ignore
/// // The current process is the policy process.
/// let result = policy_revoke(&mut scheduler, &Term::Pid(worker), &Term::static_atom("env"))?;
/// assert!(matches!(result, BifResult::Value(_)));
/// ```
///
/// # Errors
///
/// The errors of `AtomTable::put` for the atom `capability_denied` in a
/// full atom table.
pub(super) fn policy_revoke(
    scheduler: &mut Scheduler,
    holder: &Term,
    kind: &Term,
) -> Result<BifResult, BeamError> {
    let (Term::Pid(holder), Term::Atom(kind)) = (holder, kind) else {
        return Ok(BifResult::badarg());
    };
    let Some(kind) = CapabilityKind::named(scheduler.atoms.name_of(kind)) else {
        return Ok(BifResult::badarg());
    };
    match scheduler.policy_revoke(*holder, kind) {
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
/// process itself. In a run that skips the checks, and in a boot process,
/// the call returns `ok` after the check of `Kind`: it calls no policy code
/// and adds no capability. A boot process has each authority.
///
/// # Examples
///
/// ```rust,ignore
/// let result = request(&mut scheduler, &Term::static_atom("process"), &Term::nil())?;
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
    if scheduler.skips_checks() || scheduler.current_process().security.is_boot() {
        return Ok(Value(Term::static_atom("ok")));
    }
    match scheduler.request_grant(kind.clone(), request.clone())? {
        Ok(()) => Ok(BifResult::AwaitGrant),
        Err(Denied) => BifResult::capability_denied(&mut scheduler.atoms),
    }
}

/// Run `pbeam_cap:enter_runtime(Module, Function)`: in the root process of
/// a run in boot mode, call `initial/1` of the policy module with the
/// requester map `#{pid => <0.0.0>, parent => none, initial_call =>
/// {Module, Function, 0}}`, and wait (see `Scheduler::request_runtime`).
/// The enter runtime signal then replaces the boot authority of the root
/// process with the capabilities of the value, one way, and the call
/// returns `ok` through [`await_grant`]. A denial of `initial/1` stops the
/// run with the error `the policy denied the root process: <cause>`. ERTS
/// has no such BIF (row "Boot authority" of `docs/erts-differences.md`).
///
/// A `Module` or a `Function` that is not an atom raises `error:badarg`. A
/// process that is not a boot process, also each process of a run with no
/// policy, gets `ok` with no effect: it keeps its capabilities and gets no
/// new one. `pbeam_boot` thus calls it in each run.
///
/// # Examples
///
/// ```rust,ignore
/// // The current process is the boot root process.
/// let result = enter_runtime(&mut scheduler, &Term::static_atom("hello"), &Term::static_atom("main"))?;
/// assert!(matches!(result, BifResult::AwaitGrant));
/// ```
///
/// # Errors
///
/// A boot process that is not the root process gives a runtime error. A
/// full atom table.
pub(super) fn enter_runtime(
    scheduler: &mut Scheduler,
    module: &Term,
    function: &Term,
) -> Result<BifResult, BeamError> {
    let (Term::Atom(module), Term::Atom(function)) = (module, function) else {
        return Ok(BifResult::badarg());
    };
    if !scheduler.current_process().security.is_boot() {
        return Ok(Value(Term::static_atom("ok")));
    }
    scheduler.request_runtime(module, function)?;
    Ok(BifResult::AwaitGrant)
}

/// Run `pbeam_cap:serve(Pid, io)`: in a boot process, register the live
/// boot process `Pid` as an IO service, by its pid, and return `ok`. The
/// runtime to boot gate then permits a message from a runtime process to
/// `Pid` when it is an IO request that the sender may make (see
/// `io_service::send_denial`). ERTS has no such BIF (row "Boot authority"
/// of `docs/erts-differences.md`).
///
/// A `Pid` that is not a pid, and a second argument that is not the atom
/// `io`, raise `error:badarg`. A process that is not a boot process, also
/// each process of a run with no policy, gets `ok` with no effect, so
/// `pbeam_boot` calls it in each run and a runtime process gets no entry
/// from it. In a boot process, a `Pid` that is not a live boot process
/// raises `error:badarg`.
///
/// # Examples
///
/// ```rust,ignore
/// // The current process is the boot root process.
/// let result = serve(&mut scheduler, &Term::Pid(user), &Term::static_atom("io"))?;
/// assert!(matches!(result, BifResult::Value(_)));
/// ```
///
/// # Errors
///
/// None: the result is always `Ok`.
pub(super) fn serve(
    scheduler: &mut Scheduler,
    pid: &Term,
    kind: &Term,
) -> Result<BifResult, BeamError> {
    let (Term::Pid(pid), Term::Atom(kind)) = (pid, kind) else {
        return Ok(BifResult::badarg());
    };
    if kind.name() != "io" {
        return Ok(BifResult::badarg());
    }
    if scheduler.current_process().security.is_boot() && !scheduler.serve_io(*pid) {
        return Ok(BifResult::badarg());
    }
    Ok(Value(Term::static_atom("ok")))
}

/// Run the export `pbeam_cap:await_grant/0` of a waiting
/// `pbeam_cap:request/2` or `pbeam_cap:enter_runtime/2` call: return `ok`
/// after a grant, raise `error:capability_denied` after a denial, or wait
/// again when the signal did not come yet, for example when a message woke
/// the caller.
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

//! Bodies of the `erts_internal` BIFs that do not fit on one table line.

use super::BifResult::Value;
use super::erlang::signals_return;
use super::*;
use crate::runtime::signal::Signal;

/// Run `erts_internal:group_leader(Leader, Target)`, which
/// `erlang:group_leader/2` in `erlang.beam` calls, as
/// `erts_internal_group_leader_2` (`bif.c:4914-4937`) does: set the group
/// leader of the local process `Target` to `Leader` at once and return
/// `true`. Return `false` when `Target` is another process with signals or
/// messages that it did not handle (`erts_try_lock_sig_free_proc`,
/// `erl_process.c:14835-14883`); `erlang:group_leader/2` then sends the
/// change as a signal through `erts_internal:group_leader/3`. Return the
/// atom `badarg` without an exception when `Leader` is not a pid, when
/// `Target` is not a pid or no process has it, and when `Target` exits
/// (`erts_set_group_leader`, `bif.c:4860-4870`).
///
/// # Examples
///
/// ```rust,ignore
/// let result = group_leader(&mut context, &Term::Pid(leader), &Term::Pid(target));
/// assert!(matches!(result, BifResult::Value(Term::Atom(atom)) if atom.name() == "true"));
/// let result = group_leader(&mut context, &Term::Integer(1), &Term::Pid(target));
/// assert!(matches!(result, BifResult::Value(Term::Atom(atom)) if atom.name() == "badarg"));
/// ```
pub(super) fn group_leader(
    context: &mut BifContext<'_>,
    leader: &Term,
    target: &Term,
) -> BifResult {
    let (Term::Pid(leader), Term::Pid(target)) = (leader, target) else {
        return Value(Term::static_atom("badarg"));
    };
    let scheduler = &mut *context.scheduler;
    let caller = scheduler.current_process_mut().id;
    let result = match scheduler.process_mut(*target) {
        None => "badarg",
        Some(process) if *target != caller && process.mailbox.has_incoming() => "false",
        Some(process) if process.exiting.is_some() => "badarg",
        Some(process) => {
            process.group_leader = *leader;
            "true"
        }
    };
    Value(Term::static_atom(result))
}

/// Run `erts_internal:group_leader(Leader, Target, Ref)` as
/// `erts_internal_group_leader_3` (`bif.c:4898-4912`) does: send a
/// group-leader signal to the local process `Target` and return `ok`. The
/// target replies `{Ref, true}` when it handles the signal, or
/// `{Ref, badarg}` when it exits first. A target that no process has
/// gives the caller `{Ref, badarg}` at once
/// (`erts_proc_sig_send_group_leader`, `erl_proc_sig_queue.c:2970-3025`).
/// `Leader` that is not a pid, `Target` that is not a pid, and `Ref` that is
/// not a reference raise `badarg`.
///
/// # Examples
///
/// ```rust,ignore
/// let result = group_leader_request(&mut context, &Term::Pid(leader), &Term::Pid(target), &Term::Reference(reference));
/// assert!(matches!(result, BifResult::Value(Term::Atom(atom)) if atom.name() == "ok"));
/// ```
pub(super) fn group_leader_request(
    context: &mut BifContext<'_>,
    leader: &Term,
    target: &Term,
    reference: &Term,
) -> BifResult {
    let (Term::Pid(leader), Term::Pid(target), Term::Reference(reference)) =
        (leader, target, reference)
    else {
        return BifResult::badarg();
    };
    let scheduler = &mut *context.scheduler;
    let caller = scheduler.current_process_mut().id;
    let request = Signal::GroupLeader {
        leader: *leader,
        requester: caller,
        reference: *reference,
    };
    if scheduler
        .send_signal(Some(caller), *target, request)
        .is_err()
    {
        let reply = Term::Tuple(vec![
            Term::Reference(*reference),
            Term::static_atom("badarg"),
        ]);
        scheduler.deliver(caller, reply);
    }
    Value(Term::static_atom("ok"))
}

/// Run `erts_internal:is_process_alive(Pid, Ref)`, which
/// `erts_internal:is_process_alive/1` calls, as
/// `erts_internal_is_process_alive_2` (`erl_bif_info.c:4049-4058`) does:
/// send an is-alive request to `Pid` and return `ok`. The process replies
/// `{Ref, true}` when it handles the request, or `{Ref, false}` when it
/// exits first. A pid that no process has gives the caller `{Ref, false}`
/// at once (`erts_proc_sig_send_is_alive_request`,
/// `erl_proc_sig_queue.c:3027-3073`), and then the result traps to
/// `erlang:bif_handle_signals_return/2` when the caller has incoming
/// signals. A term that is not a pid, and a term that is not a reference
/// or is an alias, raise `badarg`.
///
/// # Examples
///
/// ```rust,ignore
/// let result = is_process_alive(&mut context, &Term::Pid(child), &Term::Reference(reference));
/// assert!(matches!(result, BifResult::Value(Term::Atom(atom)) if atom.name() == "ok"));
/// ```
pub(super) fn is_process_alive(
    context: &mut BifContext<'_>,
    pid: &Term,
    reference: &Term,
) -> BifResult {
    let (Term::Pid(target), Term::Reference(reference)) = (pid, reference) else {
        return BifResult::badarg();
    };
    let scheduler = &mut *context.scheduler;
    if scheduler.alias_owner(*reference).is_some() {
        return BifResult::badarg();
    }
    let caller = scheduler.current_process_mut().id;
    let request = Signal::IsAlive {
        requester: caller,
        reference: *reference,
    };
    if scheduler
        .send_signal(Some(caller), *target, request)
        .is_ok()
    {
        return Value(Term::static_atom("ok"));
    }
    let reply = Term::Tuple(vec![
        Term::Reference(*reference),
        Term::static_atom("false"),
    ]);
    scheduler.deliver(caller, reply);
    signals_return(scheduler, pid, Term::static_atom("ok"))
}

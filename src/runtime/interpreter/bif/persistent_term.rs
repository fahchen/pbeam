//! Bodies of the `persistent_term` BIFs that do not fit on one table line.
//!
//! Each BIF entry first checks the `persistent_term` capability of the
//! caller: `read` for `get/0,1,2` and `write` for `put/2`. ERTS checks no
//! argument of these BIFs, so the check comes before the table access, and
//! a denied call raises `error:capability_denied` and has no effect. The
//! trap exports of [`resume`] continue a call that passed its check, so
//! they do not check again.
//!
//! The node keeps one table, and any scheduler reads it. An update, and a
//! `get/0` that traps, hold the update permission of the node
//! (`try_seize_update_permission`, `erl_bif_persistent.c:1311-1335`). A
//! call that finds the permission with another process suspends until the
//! release, as ERTS does. A `put/2` or an `erase/1` that changes the table
//! suspends the caller until thread progress, and ERTS releases the
//! permission at thread progress, independently of the caller
//! (`erl_bif_persistent.c:774-779`, `1172-1230`). PBeam releases the
//! permission when it changes the table, and switches the caller out
//! twice before the call returns.

use super::BifResult::Value;
use super::*;
use crate::runtime::capability::{CapabilityKind, Operation};

/// Continue a call at a trap export of this module. `get_all_wait/0`,
/// `put_wait/2`, and `erase_wait/1` try the call again after a wait for
/// the update permission, as `ERTS_BIF_YIELD0`, `ERTS_BIF_YIELD3`, and
/// `ERTS_BIF_YIELD1` trap to the BIF (`erl_bif_persistent.c:333-335`,
/// `467-469`, `692-694`). `get_all_trap/2`, `put_common_trap/3`, and
/// `erase_trap/3` continue a chunk or a table copy. `thread_progress/1`
/// returns its argument after one more switch, as `table_updater` resumes
/// the caller (`erl_bif_persistent.c:1172-1230`). Return `None` for another
/// name.
///
/// # Examples
///
/// ```rust,ignore
/// let result = resume(&mut scheduler, "thread_progress", &[Term::static_atom("ok")]);
/// assert!(matches!(result, Some(BifResult::YieldReturn(_))));
/// ```
pub(super) fn resume(
    scheduler: &mut Scheduler,
    function: &str,
    arguments: &[Term],
) -> Option<BifResult> {
    Some(match (function, arguments) {
        ("get_all_wait", []) => get_all_start(scheduler),
        ("get_all_trap", [all, Term::Integer(entries)]) => {
            get_all_trap(scheduler, all.clone(), *entries as usize)
        }
        ("put_wait", [key, value]) => put_start(scheduler, key, value),
        ("put_common_trap", [key, value, Term::Integer(copied)]) => {
            put_trap(scheduler, key, value, *copied as usize)
        }
        ("erase_wait", [key]) => erase_start(scheduler, key),
        ("erase_trap", [key, Term::Integer(first), Term::Integer(second)]) => {
            erase_trap(scheduler, key, *first as usize, *second as usize)
        }
        ("thread_progress", [value]) => BifResult::YieldReturn(value.clone()),
        _ => return None,
    })
}

/// Give the update permission to the current process and return `None`.
/// When another live process holds it, use all reductions and return a
/// trap to the wait export `function` with `arguments`, as the
/// `ERTS_BIF_YIELD` after a failed `try_seize_update_permission` does.
fn seize(
    scheduler: &mut Scheduler,
    function: &'static str,
    arguments: Vec<Term>,
) -> Option<BifResult> {
    let id = scheduler.current_process().id;
    if scheduler.node().persistent_terms().seize(id) {
        return None;
    }
    scheduler.current_process_mut().bump_all_reductions();
    Some(BifResult::Suspend {
        module: "persistent_term",
        function,
        arguments,
    })
}

/// Release the update permission of the current process.
fn release(scheduler: &mut Scheduler) {
    let id = scheduler.current_process().id;
    scheduler.release_persistent_terms(id);
}

/// Run `persistent_term:get/0`: return each key and value of the node, as
/// ERTS `persistent_term_get_0` does (`erl_bif_persistent.c:324-356`). The
/// call holds the update permission until its last chunk, so the result is
/// the table at the call. See [`get_all_trap`] for the chunks.
///
/// The caller needs a `persistent_term` capability that permits `read`,
/// else the call raises `error:capability_denied` and uses no reduction.
///
/// # Examples
///
/// ```rust,ignore
/// let all = get_all(&mut context)?;
/// assert!(matches!(all, BifResult::Value(pairs) if pairs.is_nil()));
/// ```
///
/// # Errors
///
/// A new `capability_denied` atom in a full atom table gives the error of
/// `AtomTable::put`.
pub(super) fn get_all(context: &mut BifContext<'_>) -> Result<BifResult, BeamError> {
    let scheduler = &mut *context.scheduler;
    let check = denial(
        scheduler,
        CapabilityKind::PersistentTerm,
        Operation::Read,
        None,
    );
    if let Some(denied) = check? {
        return Ok(denied);
    }
    Ok(get_all_start(scheduler))
}

/// Seize the update permission and read the table for `get/0`.
fn get_all_start(scheduler: &mut Scheduler) -> BifResult {
    if let Some(wait) = seize(scheduler, "get_all_wait", Vec::new()) {
        return wait;
    }
    let pairs = scheduler.node().persistent_terms().terms.get_all();
    let entries = pairs.len();
    get_all_trap(scheduler, Term::list_from(pairs), entries)
}

/// Run `persistent_term:get/1` with no `default` and `persistent_term:get/2`
/// with its `default`: return the value of `key`. A key with no value gives
/// `default`, or raises `badarg` with no `default`, as
/// `persistent_term_get_1` and `persistent_term_get_2` do with their shared
/// `persistent_term_get` (`erl_bif_persistent.c:358-398`). A read takes no
/// update permission.
///
/// The caller needs a `persistent_term` capability that permits `read`,
/// else the call raises `error:capability_denied`, also for a key with no
/// value.
///
/// # Examples
///
/// ```rust,ignore
/// let missing = get(&mut context, &Term::static_atom("key"), None)?;
/// assert!(matches!(missing, BifResult::Raise { .. }));
/// let default = get(&mut context, &Term::static_atom("key"), Some(&Term::Integer(1)))?;
/// assert!(matches!(default, BifResult::Value(Term::Integer(1))));
/// ```
///
/// # Errors
///
/// A new `capability_denied` atom in a full atom table gives the error of
/// `AtomTable::put`.
pub(super) fn get(
    context: &mut BifContext<'_>,
    key: &Term,
    default: Option<&Term>,
) -> Result<BifResult, BeamError> {
    let scheduler = &mut *context.scheduler;
    let check = denial(
        scheduler,
        CapabilityKind::PersistentTerm,
        Operation::Read,
        None,
    );
    if let Some(denied) = check? {
        return Ok(denied);
    }
    let value = scheduler.node().persistent_terms().terms.get(key).cloned();
    Ok(value
        .or_else(|| default.cloned())
        .map_or_else(BifResult::badarg, Value))
}

/// Use the reductions of the `entries` pairs that remain of the result
/// `all` of `persistent_term:get/0`, and return `all`, as `do_get_all`
/// does (`erl_bif_persistent.c:830-900`). A call takes at most one pair for
/// each reduction of the budget. ERTS reads the budget as an unsigned
/// number, so a negative budget takes all pairs. When the pairs end, the
/// call bumps one reduction for each pair that remained at the call and
/// releases the update permission (`erl_bif_persistent.c:347-351`,
/// `816-826`). Else it uses all reductions and traps to the export
/// `persistent_term:get_all_trap/2` (`erl_bif_persistent.c:263-265`,
/// `352-355`, `819-821`), which continues with no cost in the next turn.
/// ERTS keeps the table and the result so far in the trap state. PBeam
/// keeps the full result and the number of pairs that remain.
///
/// # Examples
///
/// ```rust,ignore
/// scheduler.current_process_mut().fcalls = 10;
/// assert!(matches!(get_all_trap(&mut scheduler, Term::nil(), 4), BifResult::Value(_)));
/// ```
fn get_all_trap(scheduler: &mut Scheduler, all: Term, entries: usize) -> BifResult {
    let process = scheduler.current_process_mut();
    let max_iter = usize::try_from(process.fcalls).unwrap_or(usize::MAX);
    if entries > max_iter {
        process.bump_all_reductions();
        return BifResult::TrapBif {
            module: "persistent_term",
            function: "get_all_trap",
            arguments: vec![all, Term::Integer((entries - max_iter) as i64)],
        };
    }
    process.bump_reductions(entries as i32);
    release(scheduler);
    Value(all)
}

/// Run `persistent_term:erase/1`: erase the value of `key` and return
/// `true`, or return `false` for a key with no value, as
/// `persistent_term_erase_1` does (`erl_bif_persistent.c:417-549`). The
/// call first seizes the update permission. A key with no value releases
/// it and returns `false` with no switch. Else the copy of a rehashed table
/// goes through its slots ([`erase_trap`]).
///
/// The caller needs a `persistent_term` capability that permits `write`,
/// else the call raises `error:capability_denied` and erases nothing,
/// also for a key that has a value.
///
/// # Examples
///
/// ```rust,ignore
/// let missing = erase(&mut context, &Term::static_atom("key"))?;
/// assert!(matches!(missing, BifResult::Value(Term::Atom(atom)) if atom.is_static("false")));
/// ```
///
/// # Errors
///
/// A new `capability_denied` atom in a full atom table gives the error of
/// `AtomTable::put`.
pub(super) fn erase(context: &mut BifContext<'_>, key: &Term) -> Result<BifResult, BeamError> {
    let scheduler = &mut *context.scheduler;
    let check = denial(
        scheduler,
        CapabilityKind::PersistentTerm,
        Operation::Write,
        None,
    );
    if let Some(denied) = check? {
        return Ok(denied);
    }
    Ok(erase_start(scheduler, key))
}

/// Seize the update permission and start `erase/1` of `key`.
fn erase_start(scheduler: &mut Scheduler, key: &Term) -> BifResult {
    if let Some(wait) = seize(scheduler, "erase_wait", vec![key.clone()]) {
        return wait;
    }
    let cost = scheduler.node().persistent_terms().terms.erase_cost(key);
    match cost {
        None => {
            release(scheduler);
            Value(Term::static_atom("false"))
        }
        Some((first, second)) => erase_trap(scheduler, key, first, second),
    }
}

/// Use the reductions of the table copies of `persistent_term:erase/1` of
/// `key`, of which `first` slots of the temporary copy and `second` slots
/// of the rehashed copy remain, as `TRAPPING_COPY_TABLE` does
/// (`erl_bif_persistent.c:275-293`, `1034-1139`). A call goes through at
/// most 32 slots for each reduction of the budget in each copy, but at
/// least 1. When the copies end, the BIF erases the key, bumps one
/// reduction for each 32 slots of the call, and waits for thread progress
/// before it returns `true` (`erl_bif_persistent.c:535-539`). Else it uses
/// all reductions and traps to the export `persistent_term:erase_trap/3`,
/// which continues with the slots that remain. ERTS traps to `erase/1`
/// with a magic reference to the copy state. The caller holds the update
/// permission, so no other update changes the table during the traps.
///
/// # Examples
///
/// ```rust,ignore
/// scheduler.current_process_mut().fcalls = 2;
/// let trap = erase_trap(&mut scheduler, &key, 0, 65);
/// assert!(matches!(trap, BifResult::TrapBif { function: "erase_trap", .. }));
/// ```
fn erase_trap(
    scheduler: &mut Scheduler,
    key: &Term,
    mut first: usize,
    mut second: usize,
) -> BifResult {
    /// `ITERATIONS_PER_RED` of `persistent_term_erase_1`
    /// (`erl_bif_persistent.c:419`).
    const ITERATIONS_PER_RED: usize = 32;
    let process = scheduler.current_process_mut();
    // `MAX(1, max_iterations)` of a `long` that a negative budget makes
    // negative (`erl_bif_persistent.c:284`, `429-430`).
    let limit = (ITERATIONS_PER_RED * process.bif_reductions_or_zero()).max(1);
    // Use at most `limit` slots of one copy in this call, and return true
    // when the copy does not end.
    fn step(remaining: &mut usize, limit: usize, copied: &mut usize) -> bool {
        if *remaining == 0 {
            return false;
        }
        let done = (*remaining).min(limit);
        *remaining -= done;
        *copied += done;
        *remaining > 0
    }
    let mut copied = 0;
    let trapped = step(&mut first, limit, &mut copied) || step(&mut second, limit, &mut copied);
    if trapped {
        process.bump_all_reductions();
        return BifResult::TrapBif {
            module: "persistent_term",
            function: "erase_trap",
            arguments: vec![
                key.clone(),
                Term::Integer(first as i64),
                Term::Integer(second as i64),
            ],
        };
    }
    process.bump_reductions((copied / ITERATIONS_PER_RED) as i32);
    scheduler.node().persistent_terms().terms.erase(key);
    release(scheduler);
    thread_progress(Term::static_atom("true"))
}

/// Switch out the caller after its change, and continue at
/// `thread_progress/1` with `value`. ERTS suspends
/// the caller until thread progress (`suspend_updater`,
/// `erl_bif_persistent.c:774-779`).
fn thread_progress(value: Term) -> BifResult {
    BifResult::TrapBif {
        module: "persistent_term",
        function: "thread_progress",
        arguments: vec![value],
    }
}

/// Run `persistent_term:put/2`: store `value` for `key`, as ERTS
/// `put_common` does (`erl_bif_persistent.c:642-784`). The call first
/// seizes the update permission. A key that has exactly this value already
/// releases it and returns `ok` with no cost (`erl_bif_persistent.c:726-729`).
/// Else the copy of a doubled table goes through its slots ([`put_trap`]).
///
/// The caller needs a `persistent_term` capability that permits `write`,
/// else the call raises `error:capability_denied` and stores nothing, also
/// for a key that has exactly this value.
///
/// # Examples
///
/// ```rust,ignore
/// let result = put(&mut context, &Term::static_atom("key"), &Term::Integer(1))?;
/// assert!(matches!(result, BifResult::TrapBif { function: "thread_progress", .. }));
/// ```
///
/// # Errors
///
/// A new `capability_denied` atom in a full atom table gives the error of
/// `AtomTable::put`.
pub(super) fn put(
    context: &mut BifContext<'_>,
    key: &Term,
    value: &Term,
) -> Result<BifResult, BeamError> {
    let scheduler = &mut *context.scheduler;
    let check = denial(
        scheduler,
        CapabilityKind::PersistentTerm,
        Operation::Write,
        None,
    );
    if let Some(denied) = check? {
        return Ok(denied);
    }
    Ok(put_start(scheduler, key, value))
}

/// Seize the update permission and start `put/2` of `key` and `value`.
fn put_start(scheduler: &mut Scheduler, key: &Term, value: &Term) -> BifResult {
    if let Some(wait) = seize(scheduler, "put_wait", vec![key.clone(), value.clone()]) {
        return wait;
    }
    let cost = scheduler
        .node()
        .persistent_terms()
        .terms
        .put_cost(key, value);
    match cost {
        None => {
            release(scheduler);
            Value(Term::static_atom("ok"))
        }
        Some(copied) => put_trap(scheduler, key, value, copied),
    }
}

/// Use the reductions of the table copy of `persistent_term:put/2` of `key`
/// and `value`, of which `copied` slots remain, as `TRAPPING_COPY_TABLE`
/// does (`erl_bif_persistent.c:275-293`, `1034-1139`). A call goes through
/// at most 32 slots for each reduction of the budget, but at least 1. When
/// the copy ends, the BIF stores the pair, bumps one reduction for each 32
/// slots of the call, and waits for thread progress before it returns `ok`
/// (`erl_bif_persistent.c:763-783`). Else it uses all reductions and traps
/// to the export `persistent_term:put_common_trap/3`, which continues in
/// the next turn. ERTS traps to `put_common_trap/3` with a magic reference
/// to the copy state. The caller holds the update permission, so no other
/// update changes the table during the traps.
///
/// # Examples
///
/// ```rust,ignore
/// scheduler.current_process_mut().fcalls = 2;
/// let trap = put_trap(&mut scheduler, &key, &value, 65);
/// assert!(matches!(trap, BifResult::TrapBif { function: "put_common_trap", .. }));
/// ```
fn put_trap(scheduler: &mut Scheduler, key: &Term, value: &Term, copied: usize) -> BifResult {
    /// `ITERATIONS_PER_RED` of `put_common` (`erl_bif_persistent.c:644`).
    const ITERATIONS_PER_RED: usize = 32;
    let process = scheduler.current_process_mut();
    // `MAX(1, max_iterations)` of a `long` that a negative budget makes
    // negative (`erl_bif_persistent.c:284`, `661-662`).
    let budget = (ITERATIONS_PER_RED * process.bif_reductions_or_zero()).max(1);
    if copied > budget {
        process.bump_all_reductions();
        return BifResult::TrapBif {
            module: "persistent_term",
            function: "put_common_trap",
            arguments: vec![
                key.clone(),
                value.clone(),
                Term::Integer((copied - budget) as i64),
            ],
        };
    }
    process.bump_reductions((copied / ITERATIONS_PER_RED) as i32);
    scheduler.node().persistent_terms().terms.put(key, value);
    release(scheduler);
    thread_progress(Term::static_atom("ok"))
}

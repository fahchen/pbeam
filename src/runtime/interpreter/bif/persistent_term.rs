//! Bodies of the `persistent_term` BIFs that do not fit on one table line.

use super::BifResult::Value;
use super::*;

/// Run `persistent_term:get/0`: return each key and value of the node, as
/// ERTS `persistent_term_get_0` does (`erl_bif_persistent.c:324-356`). ERTS
/// takes the table of the call and keeps it until the last chunk, so the
/// result is the table at the call. See [`get_all_trap`] for the chunks.
///
/// # Examples
///
/// ```rust,ignore
/// process.fcalls = 10;
/// let all = get_all(&mut context);
/// assert!(matches!(all, BifResult::Value(Term::List(pairs)) if pairs.is_empty()));
/// ```
pub(super) fn get_all(context: &mut BifContext<'_>) -> BifResult {
    let pairs = context.scheduler.persistent_terms.get_all();
    let entries = pairs.len();
    get_all_trap(
        context.scheduler.current_process_mut(),
        Term::List(pairs),
        entries,
    )
}

/// Use the reductions of the `entries` pairs that remain of the result
/// `all` of `persistent_term:get/0`, and return `all`, as `do_get_all`
/// does (`erl_bif_persistent.c:830-900`). A call takes at most one pair for
/// each reduction of the budget. ERTS reads the budget as an unsigned
/// number, so a negative budget takes all pairs. When the pairs end, the
/// call bumps one reduction for each pair that remained at the call
/// (`erl_bif_persistent.c:347-351`, `816-826`). Else it uses all reductions
/// and traps to the export `persistent_term:get_all_trap/2`
/// (`erl_bif_persistent.c:263-265`, `352-355`, `819-821`), which continues
/// with no cost in the next turn. ERTS keeps the table and the result so
/// far in the trap state. PBeam keeps the full result and the number of
/// pairs that remain.
///
/// # Examples
///
/// ```rust,ignore
/// process.fcalls = 10;
/// assert!(matches!(get_all_trap(&mut process, Term::List(vec![]), 4), BifResult::Value(_)));
/// assert_eq!(process.fcalls, 6);
/// let trap = get_all_trap(&mut process, Term::List(vec![]), 7);
/// assert!(matches!(trap, BifResult::TrapBif { function: "get_all_trap", arguments, .. } if arguments[1] == Term::Integer(1)));
/// assert_eq!(process.fcalls, 0);
/// ```
pub(super) fn get_all_trap(process: &mut ProcessState, all: Term, entries: usize) -> BifResult {
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
    Value(all)
}

/// Run `persistent_term:put/2`: store `value` for `key`, as ERTS
/// `put_common` does (`erl_bif_persistent.c:642-784`). A key that has
/// exactly this value already returns `ok` with no cost. Else the copy of a
/// doubled table goes through its slots ([`put_trap`]), and the BIF
/// returns `ok` after the process switches out (`ERTS_BIF_YIELD_RETURN`).
///
/// ERTS suspends the caller until thread progress makes the new table
/// visible (`suspend_updater`, `erl_bif_persistent.c:774-779`), so other
/// processes can run more than one turn first. PBeam switches the process
/// out, and the new value is visible at once (row "Persistent terms" in
/// `docs/erts-differences.md`).
///
/// # Examples
///
/// ```rust,ignore
/// let result = put(&mut context, &Term::static_atom("key"), &Term::Integer(1))?;
/// assert!(matches!(result, BifResult::YieldReturn(_)));
/// let same = put(&mut context, &Term::static_atom("key"), &Term::Integer(1))?;
/// assert!(matches!(same, BifResult::Value(_)));
/// ```
///
/// # Errors
///
/// Return the runtime error of [`ProcessState::bif_reductions_left`] for a
/// negative budget.
pub(super) fn put(
    context: &mut BifContext<'_>,
    key: &Term,
    value: &Term,
) -> Result<BifResult, BeamError> {
    match context.scheduler.persistent_terms.put(key, value) {
        None => Ok(Value(Term::static_atom("ok"))),
        Some(copied) => put_trap(context.scheduler.current_process_mut(), copied),
    }
}

/// Use the reductions of the table copy of `persistent_term:put/2`, of
/// which `copied` slots remain, as `TRAPPING_COPY_TABLE` does
/// (`erl_bif_persistent.c:275-293`, `1034-1139`). A call goes through at
/// most 32 slots for each reduction of the budget, but at least 1. When the
/// copy ends, the BIF bumps one reduction for each 32 slots of the call,
/// and returns `ok` after the process switches out
/// (`erl_bif_persistent.c:782-783`). Else it uses all reductions and traps
/// to the export `persistent_term:put_common_trap/1`, which continues with
/// no cost in the next turn. ERTS traps to `put_common_trap/3` with a magic
/// reference to the copy state. PBeam puts the pair in the first call, so
/// the trap keeps the number of slots that remain.
///
/// # Examples
///
/// ```rust,ignore
/// process.fcalls = 2;
/// assert!(matches!(put_trap(&mut process, 64)?, BifResult::YieldReturn(_)));
/// assert_eq!(process.fcalls, 0);
/// process.fcalls = 2;
/// let trap = put_trap(&mut process, 65)?;
/// assert!(matches!(trap, BifResult::TrapBif { function: "put_common_trap", arguments, .. } if arguments[0] == Term::Integer(1)));
/// ```
///
/// # Errors
///
/// Return the runtime error of [`ProcessState::bif_reductions_left`] for a
/// negative budget.
pub(super) fn put_trap(process: &mut ProcessState, copied: usize) -> Result<BifResult, BeamError> {
    /// `ITERATIONS_PER_RED` of `put_common` (`erl_bif_persistent.c:644`).
    const ITERATIONS_PER_RED: usize = 32;
    let budget = (ITERATIONS_PER_RED * process.bif_reductions_left()?).max(1);
    if copied > budget {
        process.bump_all_reductions();
        return Ok(BifResult::TrapBif {
            module: "persistent_term",
            function: "put_common_trap",
            arguments: vec![Term::Integer((copied - budget) as i64)],
        });
    }
    process.bump_reductions((copied / ITERATIONS_PER_RED) as i32);
    Ok(BifResult::YieldReturn(Term::static_atom("ok")))
}

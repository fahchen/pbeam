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

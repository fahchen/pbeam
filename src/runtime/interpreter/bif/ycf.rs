//! The reduction counter of the BIFs that ERTS runs as Yielding C Fun
//! (YCF) code, as `erts_ycf_trap_driver` drives them (`utils.c:3733-3819`).

use super::{ProcessState, to_i64};

/// The end of one call of a YCF BIF.
#[derive(Debug, PartialEq)]
pub(super) enum YcfTurn {
    /// The work ended. The BIF bumps these reductions when it returns a
    /// value.
    Done(i32),
    /// The counter reached 0 or less after this number of units. The BIF
    /// traps, and the next call continues after these units.
    Yield(usize),
}

/// Use the `units` of a YCF BIF from unit `done` with the budget of the
/// calling process, as `erts_ycf_trap_driver` does (`utils.c:3733-3819`).
/// The counter starts at `per_reduction` times the reductions left. Each
/// unit subtracts its amount, and the work yields when the counter is then
/// 0 or less (`mk_consume_reds_code`, `ycf_yield_fun.c:592-610`), also after
/// the last unit. At a yield, the driver bumps the used units divided by
/// `per_reduction`, which is all reductions left, and traps
/// (`utils.c:3797-3817`).
///
/// At the end, the driver bumps the used units divided by `per_reduction`.
/// [`YcfTurn::Done`] gives this number, and the BIF bumps it. A BIF that
/// ends with `BIF_ERROR` bumps nothing in its last call: the macro returns
/// before YCF writes the counter back (`ycf_yield_fun.c:545-576` changes only
/// a `return` statement that it can see before the C preprocessor runs).
///
/// # Examples
///
/// ```rust,ignore
/// process.fcalls = 2;
/// // 15 units for each reduction: a counter of 30.
/// assert_eq!(run_units(&mut process, 15, &[1; 20], 0), YcfTurn::Done(1));
/// assert_eq!(run_units(&mut process, 15, &[1; 40], 0), YcfTurn::Yield(30));
/// assert_eq!(process.fcalls, 0);
/// ```
pub(super) fn run_units(
    process: &mut ProcessState,
    per_reduction: usize,
    units: &[u32],
    done: usize,
) -> YcfTurn {
    // A `long` count (`utils.c:3743`): a negative budget yields after the
    // first unit, as a budget of 0 does.
    let start = to_i64(per_reduction) * process.bif_reductions_signed();
    let mut counter = start;
    let per_reduction = to_i64(per_reduction);
    for (index, &unit) in units.iter().enumerate().skip(done) {
        counter -= i64::from(unit);
        if counter <= 0 {
            process.bump_all_reductions();
            return YcfTurn::Yield(index + 1);
        }
    }
    let used = (start - counter) / per_reduction;
    YcfTurn::Done(i32::try_from(used).expect("the used units are below the budget"))
}

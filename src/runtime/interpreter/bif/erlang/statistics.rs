//! `erlang:statistics/1`.

use num_bigint::BigInt;

use super::{BeamError, BifResult, Scheduler, Term, error};
use crate::runtime::host::clock::user_cpu_milliseconds;
use crate::runtime::time::{MONOTONIC_BEGIN, TIME_UNIT};
use crate::term::debug_term;

/// The native time units of one millisecond, as `ERTS_MONOTONIC_TO_MSEC`
/// divides by them (`erl_time.h`).
const UNITS_PER_MILLISECOND: i64 = TIME_UNIT / 1000;

/// The items of ERTS `statistics_1` that read the run queues of each
/// scheduler, the microstate accounting, or the I/O bytes of each port,
/// which PBeam does not count (row "Statistics" of
/// `docs/erts-differences.md`).
const UNSUPPORTED_ITEMS: &[&str] = &[
    "total_active_tasks",
    "total_run_queue_lengths",
    "total_active_tasks_all",
    "total_run_queue_lengths_all",
    "active_tasks",
    "run_queue_lengths",
    "active_tasks_all",
    "run_queue_lengths_all",
    "microstate_accounting",
    "run_queue",
    "io",
    "run_queues",
];

/// Run `erlang:statistics(Item)` as ERTS `statistics_1`
/// (`erl_bif_info.c:4160-4325`) does:
///
/// - `reductions` and `exact_reductions` give `{Total, SinceLastCall}` of
///   the actual reductions of each turn that ended
///   (`erl_bif_info.c:4240-4267`, `erl_process.c:12103-12137`).
/// - `runtime` gives `{Total, SinceLastCall}` of the user CPU time of the
///   OS process in milliseconds (`erl_bif_info.c:4268-4280`,
///   `erl_time_sup.c:1424-1485`).
/// - `wall_clock` gives `{Total, SinceLastCall}` of the monotonic time since
///   the run start in milliseconds (`erl_bif_info.c:4284-4296`,
///   `erl_time_sup.c:1490-1513`).
/// - `context_switches` gives `{Count, 0}` (`erl_bif_info.c:4230-4234`).
/// - `garbage_collection` gives `{0, 0, 0}`: PBeam frees a term when its
///   last reference goes and runs no garbage collection (row
///   "Statistics" of `docs/erts-differences.md`).
/// - `scheduler_wall_time` and `scheduler_wall_time_all` give `undefined`,
///   as ERTS does while the measurement is off
///   (`erl_bif_info.c:4165-4174`); PBeam cannot turn it on.
///
/// # Examples
///
/// ```rust,ignore
/// let BifResult::Value(Term::Tuple(pair)) = statistics(&mut scheduler, &Term::static_atom("reductions"))? else { panic!() };
/// assert_eq!(pair.len(), 2);
/// assert!(matches!(statistics(&mut scheduler, &Term::Integer(1))?, BifResult::Raise { .. }));
/// ```
///
/// # Errors
///
/// An item of [`UNSUPPORTED_ITEMS`] gives a runtime error. Each other atom
/// and each term that is not an atom raise `badarg`
/// (`erl_bif_info.c:4324`).
pub(in crate::runtime::interpreter::bif) fn statistics(
    scheduler: &mut Scheduler,
    item: &Term,
) -> Result<BifResult, BeamError> {
    let Term::Atom(atom) = item else {
        return Ok(BifResult::badarg());
    };
    let name = scheduler.atoms.name_of(atom);
    let pair = |(total, diff): (u64, u64)| {
        Term::tuple(vec![
            Term::from_bigint(BigInt::from(total)),
            Term::from_bigint(BigInt::from(diff)),
        ])
    };
    let value = match name {
        "reductions" => pair(scheduler.node().statistics().reductions()),
        "exact_reductions" => pair(scheduler.node().statistics().exact_reductions()),
        "runtime" => {
            let (total, diff) = scheduler
                .node()
                .statistics()
                .runtime(user_cpu_milliseconds());
            Term::tuple(vec![Term::Integer(total), Term::Integer(diff)])
        }
        "wall_clock" => {
            let elapsed = scheduler.timers.now() / UNITS_PER_MILLISECOND
                - MONOTONIC_BEGIN / UNITS_PER_MILLISECOND;
            let (total, diff) = scheduler.node().statistics().wall_clock(elapsed);
            Term::tuple(vec![Term::Integer(total), Term::Integer(diff)])
        }
        "context_switches" => Term::tuple(vec![
            Term::from_bigint(BigInt::from(
                scheduler.node().statistics().context_switches(),
            )),
            Term::Integer(0),
        ]),
        "garbage_collection" => {
            Term::tuple(vec![Term::Integer(0), Term::Integer(0), Term::Integer(0)])
        }
        "scheduler_wall_time" | "scheduler_wall_time_all" => Term::static_atom("undefined"),
        _ if UNSUPPORTED_ITEMS.contains(&name) => {
            return Err(error(format!(
                "unsupported erlang:statistics/1 argument {}: see \"Statistics\" in docs/erts-differences.md",
                debug_term(item, &scheduler.atoms)
            )));
        }
        _ => return Ok(BifResult::badarg()),
    };
    Ok(BifResult::Value(value))
}

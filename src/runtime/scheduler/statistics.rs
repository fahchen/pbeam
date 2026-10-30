//! The counters of `erlang:statistics/1` that the schedulers of a node
//! share.

use std::sync::atomic::{AtomicI64, AtomicU64, Ordering};

/// The node counters that `erlang:statistics/1` reads, and the values of
/// the last call of each item that gives a difference.
///
/// ERTS keeps the reductions and the context switches in each run queue
/// (`rq->procs`, `erl_process.c:12094-12137`). Each PBeam scheduler adds to
/// one node counter instead, which gives the same sum. The counters are
/// `Uint` in ERTS, so they and their differences wrap at 64 bits.
///
/// # Examples
///
/// ```rust,ignore
/// let statistics = Statistics::default();
/// statistics.add_reductions(10);
/// assert_eq!(statistics.reductions(), (10, 10));
/// assert_eq!(statistics.reductions(), (10, 0));
/// ```
#[derive(Debug, Default)]
pub(in crate::runtime) struct Statistics {
    /// The actual reductions of each turn that ended, the sum of
    /// `rq->procs.reductions` (`ERTS_PROC_REDUCTIONS_EXECUTED`,
    /// `erl_process.h:529-536`).
    reductions: AtomicU64,
    /// The total at the last `statistics(reductions)`, `last_reductions`
    /// (`erl_process.c:12103-12115`).
    last_reductions: AtomicU64,
    /// The total at the last `statistics(exact_reductions)`,
    /// `last_exact_reductions` (`erl_process.c:12118-12137`).
    last_exact_reductions: AtomicU64,
    /// The processes that a scheduler took to run, the sum of
    /// `rq->procs.context_switches` (`erl_process.c:10178`,
    /// `erl_process.c:12094-12100`).
    context_switches: AtomicU64,
    /// The user CPU time at the last `statistics(runtime)`,
    /// `runtime_prev.data.user` (`erl_time_sup.c:1469-1484`).
    last_runtime: AtomicI64,
    /// The wall clock time at the last `statistics(wall_clock)`,
    /// `wall_clock_prev.time` (`erl_time_sup.c:1504-1512`).
    last_wall_clock: AtomicI64,
}

impl Statistics {
    /// Add the actual reductions `used` of a turn that ended.
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// statistics.add_reductions(4000);
    /// ```
    pub(in crate::runtime) fn add_reductions(&self, used: u64) {
        self.reductions.fetch_add(used, Ordering::Relaxed);
    }

    /// Count one process that a scheduler takes to run.
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// statistics.count_context_switch();
    /// assert_eq!(statistics.context_switches(), 1);
    /// ```
    pub(in crate::runtime) fn count_context_switch(&self) {
        self.context_switches.fetch_add(1, Ordering::Relaxed);
    }

    /// Return the count of context switches.
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// assert_eq!(Statistics::default().context_switches(), 0);
    /// ```
    pub(in crate::runtime) fn context_switches(&self) -> u64 {
        self.context_switches.load(Ordering::Relaxed)
    }

    /// Return the total reductions and the reductions since the last call,
    /// as `erts_get_total_reductions` (`erl_process.c:12103-12115`).
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// statistics.add_reductions(3);
    /// assert_eq!(statistics.reductions(), (3, 3));
    /// ```
    pub(in crate::runtime) fn reductions(&self) -> (u64, u64) {
        let total = self.reductions.load(Ordering::Relaxed);
        let last = self.last_reductions.swap(total, Ordering::Relaxed);
        (total, total.wrapping_sub(last))
    }

    /// Return the total reductions and the reductions since the last call
    /// of this item, as `erts_get_exact_total_reductions`
    /// (`erl_process.c:12118-12137`).
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// statistics.add_reductions(3);
    /// assert_eq!(statistics.exact_reductions(), (3, 3));
    /// ```
    pub(in crate::runtime) fn exact_reductions(&self) -> (u64, u64) {
        let total = self.reductions.load(Ordering::Relaxed);
        let last = self.last_exact_reductions.swap(total, Ordering::Relaxed);
        (total, total.wrapping_sub(last))
    }

    /// Return the user CPU time `user` in milliseconds and the time since
    /// the last call, as `erts_runtime_elapsed_both`
    /// (`erl_time_sup.c:1424-1485`).
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// assert_eq!(statistics.runtime(5), (5, 5));
    /// assert_eq!(statistics.runtime(7), (7, 2));
    /// ```
    pub(in crate::runtime) fn runtime(&self, user: i64) -> (i64, i64) {
        let last = self.last_runtime.swap(user, Ordering::Relaxed);
        (user, user.wrapping_sub(last))
    }

    /// Return the wall clock time `elapsed` in milliseconds since the run
    /// start and the time since the last call, as
    /// `erts_wall_clock_elapsed_both` (`erl_time_sup.c:1490-1513`).
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// assert_eq!(statistics.wall_clock(5), (5, 5));
    /// assert_eq!(statistics.wall_clock(7), (7, 2));
    /// ```
    pub(in crate::runtime) fn wall_clock(&self, elapsed: i64) -> (i64, i64) {
        let last = self.last_wall_clock.swap(elapsed, Ordering::Relaxed);
        (elapsed, elapsed.wrapping_sub(last))
    }
}

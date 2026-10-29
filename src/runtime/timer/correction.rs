//! The time correction of ERTS (`erl_time_sup.c`): the Erlang monotonic time
//! from the OS monotonic time, and the periodic check that moves the time
//! offset and corrects the Erlang monotonic time to the OS system time. The
//! module follows the defaults of the pinned ERTS: time correction on
//! (measured: `erlang:system_info(time_correction)` gives `true`) and the
//! `multi_time_warp` mode (`erl_init.c:226`). All times are nanoseconds, the
//! native time unit. The OS monotonic time is the time since the run start.

use super::super::time::{MONOTONIC_BEGIN, TIME_UNIT};

/// The resolution of the clocks in Hz: the lower of the OS system time and
/// the OS monotonic time resolutions (`erl_time_sup.c:1156-1160`). Measured
/// on the pinned macOS host: 1000000 for `os_system_time_source`
/// (`CLOCK_REALTIME`) and 23809523 for `os_monotonic_time_source`
/// (`CLOCK_UPTIME_RAW`). On Linux both clocks have a resolution of 1 ns;
/// `LARGE_DIFF` then takes its floor of 500 µs, the same value as with this
/// resolution, and the Linux check uses no drift constant. A Linux host with
/// a clock resolution below 100 kHz would get a larger `SMALL_DIFF` in ERTS.
const RESOLUTION: i64 = 1_000_000;

/// `large_diff`: `50 * monotonic_time_unit / resolution`, at least 500
/// microseconds (`erl_time_sup.c:1162-1166`). Each resolution of 100000 Hz
/// or more gives the floor, so the value is the same on a Linux host.
const LARGE_DIFF: i64 = {
    let diff = 50 * TIME_UNIT / RESOLUTION;
    if diff < 500_000 { 500_000 } else { diff }
};

/// `small_diff`: `large_diff / 10` (`erl_time_sup.c:1167`).
const SMALL_DIFF: i64 = LARGE_DIFF / 10;

/// `ERTS_TCORR_ERR_UNIT` and `ERTS_TCORR_ERR_SMALL_ADJ`: the error
/// correction is in parts of 2048, and a small one is about 0.05%
/// (`erl_time_sup.c:312-314`).
const ERR_UNIT: i64 = 2048;
const ERR_SMALL_ADJ: i64 = 1;

// In the `multi_time_warp` mode, a difference of more than `2 * small_diff`
// moves the time offset, so no difference is more than `large_diff`. Thus
// the large error correction `ERTS_TCORR_ERR_LARGE_ADJ`
// (`erl_time_sup.c:523-575`) and the timeout limit of a large difference
// (`erl_time_sup.c:707-713`) are out of reach, and PBeam omits them.
const _: () = assert!(2 * SMALL_DIFF < LARGE_DIFF);

/// `ERTS_INIT_SHORT_INTERVAL_COUNTER` (`erl_time_sup.c:316`).
const INIT_SHORT_INTERVAL_COUNTER: i64 = 10;

/// `ERTS_LONG_TIME_CORRECTION_CHECK` and `ERTS_SHORT_TIME_CORRECTION_CHECK`
/// (`erl_time_sup.c:317-318`).
const LONG_CHECK: i64 = 60 * TIME_UNIT;
const SHORT_CHECK: i64 = 15 * TIME_UNIT;

/// `ERTS_TIME_DRIFT_MAX_ADJ_DIFF` and `ERTS_TIME_DRIFT_MIN_ADJ_DIFF`: 50 and
/// 5 microseconds (`erl_time_sup.c:320-321`).
const DRIFT_MAX_ADJ_DIFF: i64 = 50_000;
const DRIFT_MIN_ADJ_DIFF: i64 = 5_000;

/// `ERTS_MAX_MONOTONIC_DRIFT`: 1 millisecond per second
/// (`erl_time_sup.c:332`).
const MAX_MONOTONIC_DRIFT: i64 = 1_000_000;

/// `ERTS_MIN_MONOTONIC_DRIFT_MEASUREMENT` (`erl_time_sup.c:341-342`).
const MIN_DRIFT_MEASUREMENT: i64 = SHORT_CHECK - 2 * MAX_MONOTONIC_DRIFT;

/// The short check interval in seconds, `short_isecs`
/// (`erl_time_sup.c:1185`).
const SHORT_ISECS: i64 = SHORT_CHECK / TIME_UNIT;

/// `ilength`: `ERTS_ASSUMED_PRECISION_DROP` (32) seconds per microsecond of
/// resolution, 32 for the pinned host (`erl_time_sup.c:339`, `1186-1187`).
const ILENGTH: i64 = 32 * TIME_UNIT / (RESOLUTION * 1000);

/// `drift_adj.intervals`: `ilength / short_isecs`, from 5 to 50
/// (`erl_time_sup.c:1190-1199`). The value is 5 for the pinned host.
const DRIFT_INTERVALS: usize = {
    let intervals = ILENGTH / SHORT_ISECS;
    if intervals > 50 {
        50
    } else if intervals < 5 {
        5
    } else {
        intervals as usize
    }
};

/// `drift_adj.error`: the largest assumed error over a short interval,
/// `32 * monotonic_time_unit / (resolution * short_isecs)`, 2133 nanoseconds
/// for the pinned host (`erl_time_sup.c:1209-1211`).
const DRIFT_ERROR: i64 = 32 * TIME_UNIT / (RESOLUTION * SHORT_ISECS);

// The pinned host uses the average of all drift intervals, `drift_adj.use_avg`
// (`erl_time_sup.c:1188`). Only that branch is implemented.
const _: () = assert!(ILENGTH > SHORT_ISECS);
// The pinned host does not take `quick_init_drift_adj`, because
// `drift_adj.error` is 1 microsecond or more (`erl_time_sup.c:919-920`).
// Only that branch is implemented.
const _: () = assert!(DRIFT_ERROR / 1000 != 0);

/// The check of the host, which ERTS selects at build time. On Linux,
/// `CLOCK_MONOTONIC` with `CLOCK_MONOTONIC_RAW` present is an OS-corrected
/// monotonic clock (`make/autoconf/otp.m4:945-958`), so ERTS uses
/// `check_time_offset` (`erl_time_sup.c:1270-1274`, `929-931`). On each other
/// Unix host, for example `CLOCK_UPTIME_RAW` on macOS, the clock is not
/// corrected, so ERTS uses `check_time_correction` (`erl_time_sup.c:1271-1272`,
/// `927-928`).
#[cfg(target_os = "linux")]
pub(super) const HOST_CHECK: Check = Check::Offset;
#[cfg(not(target_os = "linux"))]
pub(super) const HOST_CHECK: Check = Check::Correction;

/// The periodic check of the time support.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(super) enum Check {
    /// `check_time_offset` (`erl_time_sup.c:789-831`): the OS corrects the
    /// monotonic clock, and the check moves only the time offset.
    #[cfg_attr(
        not(target_os = "linux"),
        allow(dead_code, reason = "the host check of Linux; the unit tests use it")
    )]
    Offset,
    /// `check_time_correction` (`erl_time_sup.c:450-777`): the check moves
    /// the time offset and corrects the Erlang monotonic time for the drift
    /// of the OS monotonic clock and the error to the OS system time.
    #[cfg_attr(
        target_os = "linux",
        allow(dead_code, reason = "the host check of macOS; the unit tests use it")
    )]
    Correction,
}

/// A correction instance, `ErtsMonotonicCorrectionInstance`
/// (`erl_time_sup.c:117-126`): from the OS monotonic time `os_mtime`, the
/// Erlang monotonic time is `erl_mtime` plus the corrected difference.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(super) struct Instance {
    pub(super) erl_mtime: i64,
    pub(super) os_mtime: i64,
    /// The correction for the drift of the OS monotonic clock, in
    /// nanoseconds per second.
    pub(super) drift: i64,
    /// The correction for the error to the OS system time, in parts of
    /// [`ERR_UNIT`].
    pub(super) error: i64,
}

/// One drift interval of `ErtsMonotonicDriftData`
/// (`erl_time_sup.c:128-147`): the clock differences over the interval and
/// the clock readings at its end.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
struct Interval {
    diff_mon: i64,
    diff_sys: i64,
    time_mon: i64,
    time_sys: i64,
}

/// The state of the time support: the time offset and the correction data,
/// `ErtsMonotonicCorrectionData` (`erl_time_sup.c:154-159`).
#[derive(Debug)]
pub(super) struct Correction {
    check: Check,
    /// The time offset: the Erlang system time minus the internal monotonic
    /// time.
    offset: i64,
    pub(super) curr: Instance,
    prev: Instance,
    intervals: [Interval; DRIFT_INTERVALS],
    acc_mon: i64,
    acc_sys: i64,
    ix: usize,
    dirty_counter: i64,
    last_check: i64,
    short_check_interval: i64,
    /// The first check of [`Check::Correction`] is
    /// `init_check_time_correction` (`erl_time_sup.c:835-885`).
    initialized: bool,
}

impl Correction {
    /// Return the state at the run start, as `erts_init_time_sup` sets it
    /// (`erl_time_sup.c:1240-1270`): the OS monotonic time is 0 and the OS
    /// system time is `sinit`.
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// let correction = Correction::new(Check::Offset, 5_000_000_000);
    /// assert_eq!(correction.time_offset(), 4_000_000_000);
    /// ```
    pub(super) fn new(check: Check, sinit: i64) -> Self {
        let curr = Instance {
            erl_mtime: MONOTONIC_BEGIN,
            os_mtime: 0,
            drift: 0,
            error: 0,
        };
        let mut intervals = [Interval::default(); DRIFT_INTERVALS];
        intervals[0].time_sys = sinit;
        Self {
            check,
            offset: sinit - MONOTONIC_BEGIN,
            curr,
            prev: curr,
            intervals,
            acc_mon: 0,
            acc_sys: 0,
            ix: 0,
            dirty_counter: 0,
            last_check: 0,
            short_check_interval: INIT_SHORT_INTERVAL_COUNTER,
            initialized: false,
        }
    }

    /// Return the milliseconds from the run start to the first check, as
    /// `late_init_time_correction` (`erl_time_sup.c:913-934`). For
    /// [`Check::Offset`], `drift_adj.error` is 0, so the first check is
    /// after `ERTS_SHORT_TIME_CORRECTION_CHECK / 10`, 1.5 seconds. For
    /// [`Check::Correction`] on the pinned host, it is after 15 seconds.
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// assert_eq!(Correction::new(Check::Offset, 0).first_timeout(), 1500);
    /// ```
    pub(super) fn first_timeout(&self) -> i64 {
        match self.check {
            Check::Offset => SHORT_CHECK / 10 / 1_000_000,
            Check::Correction => SHORT_CHECK / 1_000_000,
        }
    }

    /// Return the time offset, as `get_time_offset` reads it.
    pub(super) fn time_offset(&self) -> i64 {
        self.offset
    }

    /// Return the internal monotonic time at the OS monotonic time
    /// `os_mtime`, as `read_corrected_time` (`erl_time_sup.c:385-412`) reads
    /// the current or the previous correction instance. For
    /// [`Check::Offset`], the instance never changes and has no correction,
    /// so the value is `os_mtime + moffset`, as `get_os_corrected_time`
    /// (`erl_time_sup.c:780-787`).
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// let correction = Correction::new(Check::Offset, 0);
    /// assert_eq!(correction.monotonic_time(7), 1_000_000_007);
    /// ```
    pub(super) fn monotonic_time(&self, os_mtime: i64) -> i64 {
        let instance = if os_mtime >= self.curr.os_mtime {
            &self.curr
        } else {
            &self.prev
        };
        corrected(os_mtime, instance)
    }

    /// Do one check with the OS clocks from `os_times`, which gives the OS
    /// monotonic time and the OS system time. Return the internal monotonic
    /// time of the check and the milliseconds to the next check.
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// let mut correction = Correction::new(Check::Offset, 0);
    /// let (now, timeout) = correction.check(|| (1_500_000_000, 1_500_000_000));
    /// assert_eq!((now, timeout), (2_500_000_000, 60_000));
    /// ```
    pub(super) fn check(&mut self, mut os_times: impl FnMut() -> (i64, i64)) -> (i64, i64) {
        match self.check {
            Check::Offset => self.check_time_offset(os_times()),
            Check::Correction => {
                if !self.initialized {
                    self.initialized = true;
                    self.init_drift(os_times());
                }
                self.check_time_correction(os_times)
            }
        }
    }

    /// `check_time_offset` (`erl_time_sup.c:789-831`): when the Erlang system
    /// time differs from the OS system time by more than `2 * small_diff`,
    /// the time offset becomes the OS system time minus the internal
    /// monotonic time. The next check is after 60 seconds.
    #[expect(
        clippy::similar_names,
        reason = "the names `mtime`, `stime`, and `smtime_diff` are the names of `check_time_offset` in `erl_time_sup.c`"
    )]
    fn check_time_offset(&mut self, (os_mtime, os_stime): (i64, i64)) -> (i64, i64) {
        let erl_mtime = self.monotonic_time(os_mtime);
        let sdiff = erl_mtime + self.offset - os_stime;
        if !(-2 * SMALL_DIFF..=2 * SMALL_DIFF).contains(&sdiff) {
            self.offset -= sdiff;
        }
        (erl_mtime, LONG_CHECK / 1_000_000)
    }

    /// The drift setup of `init_check_time_correction`
    /// (`erl_time_sup.c:835-882`): each drift interval gets the interval
    /// from the run start to now. A difference between the clocks of more
    /// than `drift_adj.error` plus 1 millisecond per second is a step of the
    /// OS system time, and the setup assumes no drift.
    #[expect(
        clippy::similar_names,
        reason = "the names `mtime`, `stime`, and `smtime_diff` are the names of `init_check_time_correction` in `erl_time_sup.c`"
    )]
    fn init_drift(&mut self, (mtime, stime): (i64, i64)) {
        let old = self.intervals[0];
        let mtime_diff = mtime - old.time_mon;
        let mut stime_diff = stime - old.time_sys;
        let smtime_diff = stime_diff - mtime_diff;
        let max_drift = MAX_MONOTONIC_DRIFT * (mtime_diff / TIME_UNIT);
        if smtime_diff > DRIFT_ERROR + max_drift || smtime_diff < -DRIFT_ERROR - max_drift {
            stime_diff = mtime_diff;
        }
        self.intervals = [Interval {
            diff_mon: mtime_diff,
            diff_sys: stime_diff,
            time_mon: old.time_mon,
            time_sys: old.time_sys,
        }; DRIFT_INTERVALS];
        self.acc_sys = stime_diff * DRIFT_INTERVALS as i64;
        self.acc_mon = mtime_diff * DRIFT_INTERVALS as i64;
        self.ix = 0;
        self.dirty_counter = DRIFT_INTERVALS as i64;
    }

    /// `check_time_correction` (`erl_time_sup.c:450-777`) for the
    /// `multi_time_warp` mode and an OS monotonic clock that the OS does not
    /// correct.
    #[expect(
        clippy::similar_names,
        reason = "the names `mtime`, `stime`, and `smtime_diff` are the names of `check_time_correction` in `erl_time_sup.c`"
    )]
    #[expect(
        clippy::too_many_lines,
        reason = "mirrors `check_time_correction` of `erl_time_sup.c` (497-683) step by step: the error check and the drift measurement share the locals of the original"
    )]
    fn check_time_correction(&mut self, mut os_times: impl FnMut() -> (i64, i64)) -> (i64, i64) {
        let (os_mtime, os_stime) = os_times();
        let ci = self.curr;
        let erl_mtime = corrected(os_mtime, &ci);
        let mut sdiff = erl_mtime + self.offset - os_stime;
        let mut new = ci;
        let mut set_new = false;
        let mut begin_short = false;
        // `erl_time_sup.c:497-582`, without the `single_time_warp` mode.
        if !(-2 * SMALL_DIFF..=2 * SMALL_DIFF).contains(&sdiff) {
            self.offset -= sdiff;
            sdiff = 0;
            begin_short = true;
            if ci.error != 0 {
                set_new = true;
                new.error = 0;
            }
        } else if ci.error == 0 {
            if sdiff < -SMALL_DIFF {
                set_new = true;
                new.error = ERR_SMALL_ADJ;
            } else if sdiff > SMALL_DIFF {
                set_new = true;
                new.error = -ERR_SMALL_ADJ;
            }
        } else if ci.error > 0 {
            if sdiff >= 0 {
                set_new = true;
                new.error = if sdiff > SMALL_DIFF {
                    -ERR_SMALL_ADJ
                } else {
                    0
                };
            }
        } else if sdiff <= 0 {
            set_new = true;
            new.error = if sdiff < -SMALL_DIFF {
                ERR_SMALL_ADJ
            } else {
                0
            };
        }
        // `erl_time_sup.c:584-683`.
        let old = self.intervals[self.ix];
        let mtime_diff = os_mtime - old.time_mon;
        if mtime_diff >= MIN_DRIFT_MEASUREMENT {
            let avg_drift_adj = average_drift(self.acc_sys, self.acc_mon);
            let stime_diff = os_stime - old.time_sys;
            let smtime_diff = stime_diff - mtime_diff;
            self.ix = (self.ix + 1) % DRIFT_INTERVALS;
            let slot = &mut self.intervals[self.ix];
            self.acc_mon = self.acc_mon - slot.diff_mon + mtime_diff;
            self.acc_sys = self.acc_sys - slot.diff_sys + stime_diff;
            *slot = Interval {
                diff_mon: mtime_diff,
                diff_sys: stime_diff,
                time_mon: os_mtime,
                time_sys: os_stime,
            };
            let max_drift = MAX_MONOTONIC_DRIFT * (mtime_diff / TIME_UNIT);
            let dirty = if smtime_diff > DRIFT_ERROR + max_drift
                || smtime_diff < -DRIFT_ERROR - max_drift
            {
                true
            } else if self.dirty_counter > 0 {
                self.dirty_counter -= 1;
                false
            } else {
                if self.dirty_counter == 0 {
                    set_new = true;
                    self.dirty_counter -= 1;
                }
                let drift_adj = average_drift(self.acc_sys, self.acc_mon);
                if (-DRIFT_MAX_ADJ_DIFF..=DRIFT_MAX_ADJ_DIFF).contains(&(avg_drift_adj - drift_adj))
                {
                    let diff =
                        (drift_adj - new.drift).clamp(-DRIFT_MAX_ADJ_DIFF, DRIFT_MAX_ADJ_DIFF);
                    new.drift += diff;
                    set_new |= diff.abs() > DRIFT_MIN_ADJ_DIFF;
                    false
                } else {
                    true
                }
            };
            if dirty {
                // A step of the OS system time: rotate the intervals out
                // before the next drift adjustment.
                self.dirty_counter = DRIFT_INTERVALS as i64;
                begin_short = true;
            }
        }
        // `erl_time_sup.c:685-728`.
        if begin_short || set_new {
            self.short_check_interval = INIT_SHORT_INTERVAL_COUNTER;
        } else if os_mtime - self.last_check >= SHORT_CHECK - TIME_UNIT
            && self.short_check_interval > 0
        {
            self.short_check_interval -= 1;
        }
        self.last_check = os_mtime;
        let mut timeout = if new.error == 0 {
            LONG_CHECK / 1_000_000
        } else {
            (ERR_UNIT * sdiff.abs() / new.error.abs() / 1_000_000).max(10)
        };
        if timeout > SHORT_CHECK / 1_000_000
            && (self.short_check_interval != 0 || self.dirty_counter >= 0)
        {
            timeout = SHORT_CHECK / 1_000_000;
        }
        // `erl_time_sup.c:741-770`: the new instance begins when the OS
        // monotonic time has increased two units.
        if set_new {
            let cstart_os_mtime = os_times().0 + 2;
            self.prev = ci;
            self.curr = Instance {
                erl_mtime: corrected(cstart_os_mtime, &ci),
                os_mtime: cstart_os_mtime,
                ..new
            };
        }
        (erl_mtime, timeout)
    }
}

/// `calc_corrected_erl_mtime` for an OS monotonic clock that the OS does not
/// correct (`erl_time_sup.c:362-380`). The products wrap as in C.
fn corrected(os_mtime: i64, instance: &Instance) -> i64 {
    let mut diff = os_mtime - instance.os_mtime;
    diff += instance.drift.wrapping_mul(diff) / TIME_UNIT;
    instance.erl_mtime + diff + instance.error.wrapping_mul(diff) / ERR_UNIT
}

/// The average drift over all drift intervals in nanoseconds per second,
/// `((stime_acc - mtime_acc) * ERTS_MONOTONIC_TIME_UNIT) / mtime_acc`
/// (`erl_time_sup.c:604-606`, `653-656`). The product wraps as in C.
fn average_drift(acc_sys: i64, acc_mon: i64) -> i64 {
    (acc_sys - acc_mon).wrapping_mul(TIME_UNIT) / acc_mon
}

#[cfg(test)]
#[path = "../../../tests/runtime/timer/correction.rs"]
mod tests;

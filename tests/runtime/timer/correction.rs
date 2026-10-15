use super::{Check, Correction, Instance};

/// An OS system time at the run start.
const SINIT: i64 = 1_700_000_000_000_000_000;

/// The OS system time at the OS monotonic time `m` for a system clock that
/// runs 500 nanoseconds per second faster than the monotonic clock.
fn drifting(m: i64) -> i64 {
    SINIT + m + m / 2_000_000
}

#[test]
fn moves_only_the_time_offset_every_60_seconds_as_check_time_offset() {
    let mut correction = Correction::new(Check::Offset, SINIT);
    let offset = SINIT - 1_000_000_000;
    assert_eq!(correction.first_timeout(), 1500);
    assert_eq!(correction.time_offset(), offset);
    assert_eq!(correction.monotonic_time(7), 1_000_000_007);
    // A difference of 2 * small_diff (100 us) keeps the offset.
    let m = 1_500_000_000;
    assert_eq!(
        correction.check(|| (m, SINIT + m - 100_000)),
        (2_500_000_000, 60_000)
    );
    assert_eq!(correction.time_offset(), offset);
    // A step of one hour forward moves the offset, and the next check is
    // still 60 s later.
    let m = 61_500_000_000;
    let hour = 3_600_000_000_000;
    assert_eq!(
        correction.check(|| (m, SINIT + m + hour)),
        (62_500_000_000, 60_000)
    );
    assert_eq!(correction.time_offset(), offset + hour);
    // A step of 100.001 us backward moves the offset.
    let m = 121_500_000_000;
    correction.check(|| (m, SINIT + m + hour - 100_001));
    assert_eq!(correction.time_offset(), offset + hour - 100_001);
    // The Erlang monotonic time has no correction.
    assert_eq!(correction.monotonic_time(m), m + 1_000_000_000);
}

#[test]
fn corrects_the_erlang_monotonic_time_for_a_known_drift_as_check_time_correction() {
    let mut correction = Correction::new(Check::Correction, SINIT);
    let offset = SINIT - 1_000_000_000;
    assert_eq!(correction.first_timeout(), 15_000);
    // Checks 1 to 5 fill the drift intervals. The error stays inside
    // small_diff, and the short-interval counter keeps 15 s.
    for k in 1..=5 {
        let m = k * 15_000_000_000;
        assert_eq!(
            correction.check(|| (m, drifting(m))),
            (m + 1_000_000_000, 15_000)
        );
        assert_eq!(correction.curr.drift, 0);
    }
    // Check 6 sets the drift correction of 500 ns per second. The new
    // instance begins 2 ns after the second clock reading.
    let m = 90_000_000_000;
    assert_eq!(
        correction.check(|| (m, drifting(m))),
        (91_000_000_000, 15_000)
    );
    assert_eq!(
        correction.curr,
        Instance {
            erl_mtime: 91_000_000_002,
            os_mtime: 90_000_000_002,
            drift: 500,
            error: 0,
        }
    );
    assert_eq!(correction.time_offset(), offset);
    // Before the new instance, the previous instance applies.
    assert_eq!(correction.monotonic_time(m + 1), 91_000_000_001);
    assert_eq!(
        correction.monotonic_time(m + 15_000_000_002),
        106_000_007_502
    );
    // A new correction begins 10 short intervals: the next 9 checks have
    // no change and keep 15 s; the tenth check goes to 60 s.
    for k in 7..=15 {
        let m = k * 15_000_000_000;
        assert_eq!(correction.check(|| (m, drifting(m))).1, 15_000);
    }
    let m = 16 * 15_000_000_000;
    assert_eq!(correction.check(|| (m, drifting(m))).1, 60_000);
    let m = m + 60_000_000_000;
    assert_eq!(correction.check(|| (m, drifting(m))).1, 60_000);
    assert_eq!(correction.curr.drift, 500);
    assert_eq!(correction.time_offset(), offset);
}

#[test]
fn moves_the_time_offset_at_a_step_and_restarts_short_intervals_as_check_time_correction() {
    let mut correction = Correction::new(Check::Correction, SINIT);
    let offset = SINIT - 1_000_000_000;
    let hour = 3_600_000_000_000;
    // A step before the first check: the drift setup assumes no drift.
    let m = 15_000_000_000;
    assert_eq!(
        correction.check(|| (m, SINIT + m + hour)),
        (16_000_000_000, 15_000)
    );
    assert_eq!(correction.time_offset(), offset + hour);
    // Between checks, the offset stays.
    assert_eq!(correction.time_offset(), offset + hour);
    // A step at the next check moves the offset and marks the drift
    // intervals dirty.
    let m = 30_000_000_000;
    correction.check(|| (m, SINIT + m + 2 * hour));
    assert_eq!(correction.time_offset(), offset + 2 * hour);
    assert_eq!(correction.curr.drift, 0);
    // Each next check sees no difference and keeps 15 s. The checks 3 to
    // 7 rotate the dirty drift intervals out, with no new correction.
    for k in 3..=7 {
        let m = k * 15_000_000_000;
        assert_eq!(correction.check(|| (m, SINIT + m + 2 * hour)).1, 15_000);
        assert_eq!(correction.curr.os_mtime, 0);
    }
    // The drift intervals are clean again, so check 8 sets the drift
    // correction 0 as a new instance.
    let m = 8 * 15_000_000_000;
    assert_eq!(correction.check(|| (m, SINIT + m + 2 * hour)).1, 15_000);
    assert_eq!(correction.curr.os_mtime, m + 2);
    assert_eq!(correction.curr.drift, 0);
}

/// Do one check of [`Check::Correction`] at 1 s, before the first drift
/// measurement, with the error correction `error` and the difference
/// `sdiff` of the Erlang system time to the OS system time. Return the new
/// error correction and the timeout.
fn error_check(error: i64, sdiff: i64) -> (i64, i64) {
    let mut correction = Correction::new(Check::Correction, SINIT);
    correction.initialized = true;
    correction.curr.error = error;
    let m = 1_000_000_000;
    let stime = correction.monotonic_time(m) + correction.time_offset() - sdiff;
    let (_, timeout) = correction.check(|| (m, stime));
    (correction.curr.error, timeout)
}

#[test]
fn corrects_an_error_to_the_os_system_time_as_check_time_correction() {
    // The OS system time is 60 us ahead: a small correction of 1/2048
    // until the error is gone, (2048 * 60000 / 1) ns = 122 ms.
    assert_eq!(error_check(0, -60_000), (1, 122));
    assert_eq!(error_check(0, 60_000), (-1, 122));
    assert_eq!(error_check(0, 50_000), (0, 15_000));
    // A correction stays until the difference changes sign, and a timeout
    // is at least 10 ms.
    assert_eq!(error_check(1, -10), (1, 10));
    assert_eq!(error_check(1, 10), (0, 15_000));
    assert_eq!(error_check(1, 60_000), (-1, 122));
    assert_eq!(error_check(-1, 10), (-1, 10));
    assert_eq!(error_check(-1, -10), (0, 15_000));
    assert_eq!(error_check(-1, -60_000), (1, 122));
    // A step past 2 * small_diff moves the offset and ends the correction.
    assert_eq!(error_check(1, 200_000), (0, 15_000));
}

#[test]
fn marks_the_drift_intervals_dirty_at_a_large_change_of_the_average_drift() {
    let mut correction = Correction::new(Check::Correction, SINIT);
    for k in 1..=6 {
        let m = k * 15_000_000_000;
        correction.check(|| (m, SINIT + m));
    }
    assert_eq!(correction.dirty_counter, -1);
    // 5 ms over 15 s is inside the limit of a step, but it moves the average
    // drift by 66666 ns per second, more than 50 us per second.
    let m = 7 * 15_000_000_000;
    correction.check(|| (m, SINIT + m + 5_000_000));
    assert_eq!(correction.dirty_counter, 5);
    assert_eq!(correction.curr.drift, 0);
}

//! The time support of ERTS (`erl_time_sup.c`): the time units, the unit
//! conversion of `erlang:monotonic_time/1` and `os:system_time/1`, the
//! date of a POSIX time, and the POSIX time of a UTC date.

use crate::term::{SMALL_INTEGER_MIN, Term};

/// The native time unit, `ERTS_MONOTONIC_TIME_UNIT`: 1000000000 parts of a
/// second (`erl_time.h:282-283`, `sys/unix/erl_unix_sys.h:192-204`).
pub(super) const TIME_UNIT: i64 = 1_000_000_000;

/// The internal monotonic time at the run start, `ERTS_MONOTONIC_BEGIN`:
/// one second in the native time unit (`erl_time.h:235-236`).
pub(super) const MONOTONIC_BEGIN: i64 = TIME_UNIT;

/// The Erlang monotonic time at the run start,
/// `ERTS_MONOTONIC_TIME_START_EXTERNAL`: the smallest small integer,
/// rounded toward zero to a multiple of one second (`erl_time.h:316-321`).
/// The value is -576460752000000000, as `erlang:system_info(start_time)` on
/// the pinned ERTS.
const MONOTONIC_TIME_START_EXTERNAL: i64 = SMALL_INTEGER_MIN / TIME_UNIT * TIME_UNIT;

/// The offset from the internal monotonic time to the Erlang monotonic time
/// in the native time unit, `ERTS_MONOTONIC_OFFSET_NATIVE`
/// (`erl_time.h:331-332`): [`MONOTONIC_TIME_START_EXTERNAL`] minus
/// [`MONOTONIC_BEGIN`], -576460753000000000. The offset is a multiple of one
/// second.
pub(super) const MONOTONIC_OFFSET_NATIVE: i64 = MONOTONIC_TIME_START_EXTERNAL - MONOTONIC_BEGIN;

/// The unit of `perf_counter`, `ERTS_HRTIME_UNIT` (`sys_time.c:1032-1041`,
/// `sys/unix/erl_unix_sys.h:219`). ERTS uses the TSC frequency only when
/// `have_rdtscp` finds bits 8 and 23 of the `cpuid` leaf `0x80000007` `edx`
/// (`sys_time.c:989-1007`, `ethr_aux.c:167-168`). Intel and AMD define no
/// bit 23 there: the bit numbers come from the Linux software flags
/// `constant_tsc` and `tsc_reliable`. Thus ERTS gives this unit also on an
/// x86_64 host.
pub(super) const PERF_COUNTER_UNIT: i64 = 1_000_000_000;

/// The result of [`time_unit_conversion`].
#[derive(Debug, PartialEq)]
pub(super) enum TimeValue {
    /// The time in the given unit.
    Value(i64),
    /// The time in the native unit, which the BIF passes to
    /// `erlang:convert_time_unit/3` (`erts_convert_time_unit_trap`,
    /// `bif.c:5557`).
    Trap(i64),
    /// The unit is not valid.
    Badarg,
}

/// Convert `value`, a time in the native unit, to `unit`, and add
/// `offset_multiplier` times [`MONOTONIC_OFFSET_NATIVE`], as ERTS
/// `time_unit_conversion` (`erl_time_sup.c:2268-2350`) does.
/// `erlang:monotonic_time/1` gives the internal monotonic time and the
/// multiplier 1, and `os:system_time/1` gives the OS system time and the
/// multiplier 0.
///
/// A negative `value` traps for each `unit`. `native` gives the value with
/// the offset. `second`, `millisecond`, `microsecond`, `nanosecond`, their
/// deprecated names `seconds`, `milli_seconds`, `micro_seconds`, and
/// `nano_seconds`, and the integers 1, 1000, 1000000, and 1000000000 give
/// the value divided by the unit, with the offset in the unit. Because
/// `value` is not negative and the offset is a multiple of one second, the
/// result is the floor. `perf_counter` and each other positive integer
/// trap. Zero, a negative integer, and each other term give
/// [`TimeValue::Badarg`].
///
/// # Examples
///
/// ```rust,ignore
/// let second = Term::static_atom("second");
/// assert_eq!(time_unit_conversion(1_500_000_000, 1, &second), TimeValue::Value(-576_460_752));
/// assert_eq!(time_unit_conversion(1_500_000_000, 0, &Term::Integer(1000)), TimeValue::Value(1500));
/// assert_eq!(time_unit_conversion(7, 0, &Term::Integer(3)), TimeValue::Trap(7));
/// assert_eq!(time_unit_conversion(-7, 0, &Term::Integer(0)), TimeValue::Trap(-7));
/// assert_eq!(time_unit_conversion(7, 0, &Term::Integer(0)), TimeValue::Badarg);
/// ```
pub(super) fn time_unit_conversion(value: i64, offset_multiplier: i64, unit: &Term) -> TimeValue {
    let native = value + offset_multiplier * MONOTONIC_OFFSET_NATIVE;
    if value < 0 {
        return TimeValue::Trap(native);
    }
    let per_second = match unit {
        Term::Atom(unit) => match unit.name() {
            "native" => return TimeValue::Value(native),
            "second" | "seconds" => 1,
            "millisecond" | "milli_seconds" => 1_000,
            "microsecond" | "micro_seconds" => 1_000_000,
            "nanosecond" | "nano_seconds" => 1_000_000_000,
            "perf_counter" => return TimeValue::Trap(native),
            _ => return TimeValue::Badarg,
        },
        Term::Integer(unit @ (1 | 1_000 | 1_000_000 | 1_000_000_000)) => *unit,
        Term::Integer(unit) if *unit > 0 => return TimeValue::Trap(native),
        _ => return TimeValue::Badarg,
    };
    let divisor = TIME_UNIT / per_second;
    TimeValue::Value(value / divisor + offset_multiplier * (MONOTONIC_OFFSET_NATIVE / divisor))
}

/// A date and time `(year, month, day, hour, minute, second)`, as the
/// tuple `{{Year, Month, Day}, {Hour, Minute, Second}}` of the calendar
/// BIFs.
pub(crate) type DateTime = (i64, i64, i64, i64, i64, i64);

/// The seconds in a day, `SECONDS_PER_DAY` (`erl_time_sup.c:1664-1666`).
const SECONDS_PER_DAY: i64 = 24 * SECONDS_PER_HOUR;

/// The seconds in an hour, `SECONDS_PER_HOUR` (`erl_time_sup.c:1664-1666`).
const SECONDS_PER_HOUR: i64 = 60 * SECONDS_PER_MINUTE;

/// The seconds in a minute, `SECONDS_PER_MINUTE` (`erl_time_sup.c:1664-1666`).
const SECONDS_PER_MINUTE: i64 = 60;

/// Return the UTC date and time `(year, month, day, hour, minute, second)`
/// of `time`, the seconds since 1970-01-01 00:00:00 UTC, as ERTS
/// `seconds_to_univ` (`erl_time_sup.c:1669-1702`) does. The arithmetic is
/// the C integer arithmetic: `/` and `%` truncate toward zero, as in Rust.
/// Each 64-bit `time` gives a date. Before the year 1, the truncating
/// division of the leap-year terms gives the ERTS dates, which can differ
/// from the proleptic Gregorian calendar: -62167219200, the start of the
/// year 0, gives -1-12-31.
///
/// # Examples
///
/// ```rust,ignore
/// assert_eq!(seconds_to_univ(0), (1970, 1, 1, 0, 0, 0));
/// assert_eq!(seconds_to_univ(-1), (1969, 12, 31, 23, 59, 59));
/// assert_eq!(seconds_to_univ(951_782_400), (2000, 2, 29, 0, 0, 0));
/// assert_eq!(seconds_to_univ(-62_167_219_201), (-1, 12, 30, 23, 59, 59));
/// ```
pub(super) fn seconds_to_univ(time: i64) -> DateTime {
    let mut days = time / SECONDS_PER_DAY;
    let mut secs = time % SECONDS_PER_DAY;
    if secs < 0 {
        days -= 1;
        secs += SECONDS_PER_DAY;
    }
    let tmp = secs % SECONDS_PER_HOUR;
    let (hour, minute, second) = (
        secs / SECONDS_PER_HOUR,
        tmp / SECONDS_PER_MINUTE,
        tmp % SECONDS_PER_MINUTE,
    );
    // 719468 is the day of 1970-01-01 counted from 0000-03-01.
    days += 719_468;
    let mut year = (10_000 * days + 14_780) / 3_652_425;
    let mut day_of_year = days - (365 * year + year / 4 - year / 100 + year / 400);
    if day_of_year < 0 {
        year -= 1;
        day_of_year = days - (365 * year + year / 4 - year / 100 + year / 400);
    }
    // `mi` counts the months from March.
    let mi = (100 * day_of_year + 52) / 3_060;
    (
        year + (mi + 2) / 12,
        (mi + 2) % 12 + 1,
        day_of_year - (mi * 306 + 5) / 10 + 1,
        hour,
        minute,
        second,
    )
}

/// The first year that `univ_to_local` accepts, `BASEYEAR`
/// (`erl_time_sup.c:1598-1600`).
const BASEYEAR: i64 = 1902;

/// The last year that `univ_to_local` accepts, `INT_MAX - 1`
/// (`erl_time_sup.c:1808`).
const LAST_YEAR: i64 = i32::MAX as i64 - 1;

/// The days in each month of a year that is not a leap year, `mdays`
/// (`erl_time_sup.c:1590-1591`). Index 0 is not a month.
const MDAYS: [i64; 13] = [0, 31, 28, 31, 30, 31, 30, 31, 31, 30, 31, 30, 31];

/// Return the date and time of a term `{{Year, Month, Day}, {Hour, Minute,
/// Second}}`, as ERTS `time_to_parts` (`bif.c:3903-3931`) does. Return
/// `None` for another term, and for an element that is not a small integer
/// (-(2^59) to 2^59 - 1). The values are not checked.
///
/// # Examples
///
/// ```rust,ignore
/// let triple = |a, b, c| Term::Tuple(vec![Term::Integer(a), Term::Integer(b), Term::Integer(c)]);
/// let date = Term::Tuple(vec![triple(2026, 2, 30), triple(24, 0, 0)]);
/// assert_eq!(time_to_parts(&date), Some((2026, 2, 30, 24, 0, 0)));
/// let date = Term::Tuple(vec![triple(1 << 59, 1, 1), triple(0, 0, 0)]);
/// assert_eq!(time_to_parts(&date), None);
/// assert_eq!(time_to_parts(&triple(2026, 1, 1)), None);
/// ```
pub(super) fn time_to_parts(date: &Term) -> Option<DateTime> {
    let Term::Tuple(parts) = date else {
        return None;
    };
    let [Term::Tuple(date), Term::Tuple(time)] = parts.as_slice() else {
        return None;
    };
    let ([year, month, day], [hour, minute, second]) = (small_triple(date)?, small_triple(time)?);
    Some((year, month, day, hour, minute, second))
}

/// Return the three small integers of a tuple with three elements, or
/// `None` (`bif.c:3918-3929`).
///
/// # Examples
///
/// ```rust,ignore
/// assert_eq!(small_triple(&[Term::Integer(1), Term::Integer(2), Term::Integer(3)]), Some([1, 2, 3]));
/// assert_eq!(small_triple(&[Term::Integer(1), Term::Integer(2)]), None);
/// ```
fn small_triple(elements: &[Term]) -> Option<[i64; 3]> {
    let [first, second, third] = elements else {
        return None;
    };
    Some([
        first.small_integer()?,
        second.small_integer()?,
        third.small_integer()?,
    ])
}

/// Return the POSIX time of a UTC date and time, as the first part of ERTS
/// `univ_to_local` (`erl_time_sup.c:1802-1821`) does before it calls
/// `localtime_r`. Return `None` when a value is out of range: the year
/// must be 1902 to 2147483646, the month 1 to 12, the day in the month
/// (29 days in February of a leap year), the hour 0 to 23, and the minute
/// and the second 0 to 59.
///
/// # Examples
///
/// ```rust,ignore
/// assert_eq!(univ_to_clock((1970, 1, 1, 0, 0, 0)), Some(0));
/// assert_eq!(univ_to_clock((2000, 2, 29, 23, 59, 59)), Some(951_868_799));
/// assert_eq!(univ_to_clock((1901, 12, 31, 23, 59, 59)), None);
/// assert_eq!(univ_to_clock((2100, 2, 29, 0, 0, 0)), None);
/// ```
pub(super) fn univ_to_clock((year, month, day, hour, minute, second): DateTime) -> Option<i64> {
    let valid = (BASEYEAR..=LAST_YEAR).contains(&year)
        && (1..=12).contains(&month)
        && (1..=MDAYS[month as usize] + i64::from(month == 2 && is_leap_year(year))).contains(&day)
        && (0..=23).contains(&hour)
        && (0..=59).contains(&minute)
        && (0..=59).contains(&second);
    valid.then(|| second + 60 * (minute + 60 * (hour + 24 * gregday(year, month, day))))
}

/// Return the days from 1970-01-01 to a date with a year from 1600, as
/// ERTS `gregday` (`erl_time_sup.c:1644-1662`) does.
///
/// # Examples
///
/// ```rust,ignore
/// assert_eq!(gregday(1970, 1, 1), 0);
/// assert_eq!(gregday(2000, 3, 1), 11_017);
/// assert_eq!(gregday(1902, 1, 1), -24_837);
/// ```
fn gregday(year: i64, month: i64, day: i64) -> i64 {
    let mut ndays = 0;
    // The days in the years from 1600 to the year before `year`.
    let gyear = year - 1600;
    if gyear > 0 {
        let pyear = gyear - 1;
        ndays = pyear / 4 - pyear / 100 + pyear / 400 + pyear * 365 + 366;
    }
    ndays += MDAYS[1..month as usize].iter().sum::<i64>();
    if is_leap_year(year) && month > 2 {
        ndays += 1;
    }
    ndays += day - 1;
    // 135140 is the day of 1970-01-01 counted from 1600-01-01.
    ndays - 135_140
}

/// Return whether `year` is a leap year in the Gregorian calendar, as the
/// ERTS macro `is_leap_year` (`erl_time_sup.c:1594-1596`).
///
/// # Examples
///
/// ```rust,ignore
/// assert!(is_leap_year(2000));
/// assert!(!is_leap_year(2100));
/// ```
fn is_leap_year(year: i64) -> bool {
    (year % 4 == 0 && year % 100 != 0) || year % 400 == 0
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn derives_the_monotonic_offset_as_erts() {
        // `erlang:system_info(start_time)` on the pinned ERTS.
        assert_eq!(MONOTONIC_TIME_START_EXTERNAL, -576_460_752_000_000_000);
        assert_eq!(MONOTONIC_BEGIN, 1_000_000_000);
        assert_eq!(MONOTONIC_OFFSET_NATIVE, -576_460_753_000_000_000);
    }

    #[test]
    fn converts_each_unit_with_the_offset_multiplier() {
        let atom = Term::static_atom;
        let value = 1_234_567_891_234;
        for (units, expected) in [
            (
                [atom("second"), atom("seconds"), Term::Integer(1)],
                [-576_459_519, 1_234],
            ),
            (
                [
                    atom("millisecond"),
                    atom("milli_seconds"),
                    Term::Integer(1_000),
                ],
                [-576_459_518_433, 1_234_567],
            ),
            (
                [
                    atom("microsecond"),
                    atom("micro_seconds"),
                    Term::Integer(1_000_000),
                ],
                [-576_459_518_432_109, 1_234_567_891],
            ),
            (
                [
                    atom("nanosecond"),
                    atom("nano_seconds"),
                    Term::Integer(1_000_000_000),
                ],
                [-576_459_518_432_108_766, 1_234_567_891_234],
            ),
        ] {
            for unit in &units {
                assert_eq!(
                    time_unit_conversion(value, 1, unit),
                    TimeValue::Value(expected[0])
                );
                assert_eq!(
                    time_unit_conversion(value, 0, unit),
                    TimeValue::Value(expected[1])
                );
            }
        }
        let native = atom("native");
        assert_eq!(
            time_unit_conversion(value, 1, &native),
            TimeValue::Value(-576_459_518_432_108_766)
        );
        assert_eq!(
            time_unit_conversion(value, 0, &native),
            TimeValue::Value(value)
        );
    }

    #[test]
    fn traps_for_perf_counter_another_positive_unit_and_a_negative_value() {
        let atom = Term::static_atom;
        for unit in [
            atom("perf_counter"),
            Term::Integer(3),
            Term::Integer(i64::MAX),
        ] {
            assert_eq!(
                time_unit_conversion(1_000_000_000, 1, &unit),
                TimeValue::Trap(-576_460_752_000_000_000)
            );
            assert_eq!(time_unit_conversion(5, 0, &unit), TimeValue::Trap(5));
        }
        // ERTS traps a negative value before it checks the unit
        // (`erl_time_sup.c:2273-2274`), so a unit that is not valid traps too.
        for unit in [atom("second"), atom("native"), Term::Integer(0), atom("ok")] {
            assert_eq!(time_unit_conversion(-5, 0, &unit), TimeValue::Trap(-5));
            assert_eq!(
                time_unit_conversion(-5, 1, &unit),
                TimeValue::Trap(-576_460_753_000_000_005)
            );
        }
    }

    #[test]
    fn rejects_zero_a_negative_integer_and_each_other_term() {
        for unit in [
            Term::Integer(0),
            Term::Integer(-1),
            Term::Integer(i64::MIN),
            Term::static_atom("ok"),
            Term::Tuple(vec![]),
            Term::List(vec![]),
        ] {
            assert_eq!(time_unit_conversion(5, 1, &unit), TimeValue::Badarg);
            assert_eq!(time_unit_conversion(5, 0, &unit), TimeValue::Badarg);
        }
    }

    #[test]
    fn gives_the_date_of_each_64_bit_posix_time() {
        // The values of `erlang:posixtime_to_universaltime/1` on the pinned
        // ERTS.
        for (time, date) in [
            (0, (1970, 1, 1, 0, 0, 0)),
            (-1, (1969, 12, 31, 23, 59, 59)),
            (86_400, (1970, 1, 2, 0, 0, 0)),
            (951_868_800, (2000, 3, 1, 0, 0, 0)),
            (-62_167_219_200, (-1, 12, 31, 0, 0, 0)),
            (i64::MAX, (292_277_026_596, 12, 4, 15, 30, 7)),
            (i64::MIN, (-292_277_022_657, 1, 26, 8, 29, 52)),
        ] {
            assert_eq!(seconds_to_univ(time), date, "{time}");
        }
    }

    #[test]
    fn reads_only_a_date_tuple_of_small_integers() {
        let triple =
            |a, b, c| Term::Tuple(vec![Term::Integer(a), Term::Integer(b), Term::Integer(c)]);
        let date = |date, time| Term::Tuple(vec![date, time]);
        // The values are not checked here.
        assert_eq!(
            time_to_parts(&date(
                triple(-(1 << 59), 13, 0),
                triple(24, 60, (1 << 59) - 1)
            )),
            Some((-(1 << 59), 13, 0, 24, 60, (1 << 59) - 1))
        );
        for term in [
            triple(2026, 1, 1),
            Term::Tuple(vec![triple(2026, 1, 1)]),
            Term::Tuple(vec![triple(2026, 1, 1), triple(0, 0, 0), triple(0, 0, 0)]),
            date(Term::List(vec![]), triple(0, 0, 0)),
            date(triple(2026, 1, 1), Term::Integer(0)),
            date(
                Term::Tuple(vec![Term::Integer(2026), Term::Integer(1)]),
                triple(0, 0, 0),
            ),
            date(triple(2026, 1, 1), Term::Tuple(vec![Term::Integer(0); 4])),
            date(triple(1 << 59, 1, 1), triple(0, 0, 0)),
            date(triple(2026, 1, 1), triple(0, 0, -(1 << 59) - 1)),
            date(
                triple(2026, 1, 1),
                Term::Tuple(vec![
                    Term::Integer(0),
                    Term::Integer(0),
                    Term::static_atom("ok"),
                ]),
            ),
            Term::static_atom("ok"),
        ] {
            assert_eq!(time_to_parts(&term), None, "{term:?}");
        }
    }

    #[test]
    fn gives_the_posix_time_of_a_date_in_range() {
        for (date, clock) in [
            ((1970, 1, 1, 0, 0, 0), 0),
            ((1969, 12, 31, 23, 59, 59), -1),
            ((1902, 1, 1, 0, 0, 0), -2_145_916_800),
            ((2000, 2, 29, 23, 59, 59), 951_868_799),
            ((2024, 3, 1, 0, 0, 0), 1_709_251_200),
            ((2100, 3, 1, 0, 0, 0), 4_107_542_400),
            ((2_147_483_646, 12, 31, 23, 59, 59), 67_767_976_201_996_799),
        ] {
            assert_eq!(univ_to_clock(date), Some(clock), "{date:?}");
        }
        for date in [
            (1901, 12, 31, 23, 59, 59),
            (2_147_483_647, 1, 1, 0, 0, 0),
            (2100, 2, 29, 0, 0, 0),
            (2026, 2, 29, 0, 0, 0),
            (2026, 0, 1, 0, 0, 0),
            (2026, 13, 1, 0, 0, 0),
            (2026, 4, 31, 0, 0, 0),
            (2026, 1, 0, 0, 0, 0),
            (2026, 1, 1, -1, 0, 0),
            (2026, 1, 1, 24, 0, 0),
            (2026, 1, 1, 0, -1, 0),
            (2026, 1, 1, 0, 60, 0),
            (2026, 1, 1, 0, 0, -1),
            (2026, 1, 1, 0, 0, 60),
        ] {
            assert_eq!(univ_to_clock(date), None, "{date:?}");
        }
    }
}

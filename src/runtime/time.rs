//! The time support of ERTS (`erl_time_sup.c`): the time units, the unit
//! conversion of `erlang:monotonic_time/1` and `os:system_time/1`, and the
//! date of a POSIX time.

use crate::term::Term;

/// The native time unit, `ERTS_MONOTONIC_TIME_UNIT`: 1000000000 parts of a
/// second (`erl_time.h:282-283`, `sys/unix/erl_unix_sys.h:192-204`).
pub(super) const TIME_UNIT: i64 = 1_000_000_000;

/// The offset from the internal monotonic time to the Erlang monotonic time
/// in the native time unit, `ERTS_MONOTONIC_OFFSET_NATIVE`
/// (`erl_time.h:318-321`, `331-340`): `ERTS_MONOTONIC_TIME_START_EXTERNAL`,
/// -576460752000000000, minus `ERTS_MONOTONIC_BEGIN`, 1000000000. The
/// offset is a multiple of one second.
pub(super) const MONOTONIC_OFFSET_NATIVE: i64 = -576_460_753_000_000_000;

/// The unit of `perf_counter`, `ERTS_HRTIME_UNIT`, on a host without
/// `rdtscp` (`sys_time.c:1032-1041`, `sys/unix/erl_unix_sys.h:219`).
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
pub(super) fn seconds_to_univ(time: i64) -> (i64, i64, i64, i64, i64, i64) {
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

#[cfg(test)]
mod tests {
    use super::*;

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
}

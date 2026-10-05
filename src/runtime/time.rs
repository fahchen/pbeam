//! The time support of ERTS (`erl_time_sup.c`): the time units and the unit
//! conversion of `erlang:monotonic_time/1` and `os:system_time/1`.

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
}

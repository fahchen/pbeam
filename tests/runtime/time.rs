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
        Term::from_bigint(num_bigint::BigInt::from(1_u8) << 70_u32),
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
        Term::from_bigint(-(num_bigint::BigInt::from(1_u8) << 70_u32)),
        Term::static_atom("ok"),
        Term::tuple(vec![]),
        Term::nil(),
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
    let triple = |a, b, c| Term::tuple(vec![Term::Integer(a), Term::Integer(b), Term::Integer(c)]);
    let date = |date, time| Term::tuple(vec![date, time]);
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
        Term::tuple(vec![triple(2026, 1, 1)]),
        Term::tuple(vec![triple(2026, 1, 1), triple(0, 0, 0), triple(0, 0, 0)]),
        date(Term::nil(), triple(0, 0, 0)),
        date(triple(2026, 1, 1), Term::Integer(0)),
        date(
            Term::tuple(vec![Term::Integer(2026), Term::Integer(1)]),
            triple(0, 0, 0),
        ),
        date(triple(2026, 1, 1), Term::tuple(vec![Term::Integer(0); 4])),
        date(triple(1 << 59, 1, 1), triple(0, 0, 0)),
        date(triple(2026, 1, 1), triple(0, 0, -(1 << 59) - 1)),
        date(
            triple(2026, 1, 1),
            Term::tuple(vec![
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

#[test]
fn splits_a_time_in_the_native_unit_into_timestamp_parts() {
    // `make_timestamp_value` (`erl_time_sup.c:1906-1925`) gives the parts of
    // `erlang:timestamp/0`. The measurements are `erlang:timestamp()` and
    // `os:timestamp()` on the pinned ERTS, and the parts of their values:
    // 1792192300076209000 ns give {1792,192300,76209}.
    assert_eq!(
        make_timestamp_value(1_792_192_300_076_209_000),
        (1792, 192_300, 76_209)
    );
    assert_eq!(
        make_timestamp_value(1_792_192_300_000_000_000),
        (1792, 192_300, 0)
    );
    assert_eq!(make_timestamp_value(999_999_999), (0, 0, 999_999));
    assert_eq!(make_timestamp_value(1_000_000_000), (0, 1, 0));
    assert_eq!(make_timestamp_value(1_000_000_000_000), (0, 1000, 0));
    assert_eq!(make_timestamp_value(0), (0, 0, 0));
}

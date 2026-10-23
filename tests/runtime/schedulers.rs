//! Unit tests of the `+S` flag parse. The expected values come from
//! `erl +S <Value>` of the pinned ERTS on a host with 10 CPUs
//! (`erl_init.c:1103-1149`); a unit test fixes the host counts, which a
//! fixture cannot do.

use super::Schedulers;

/// The counts of a host with 10 CPUs.
fn ten() -> Schedulers {
    Schedulers {
        host_total: 10,
        host_online: 10,
        total: 10,
        online: 10,
    }
}

/// Apply `value` to the counts of [`ten`] and return the total and the
/// online count, or the error text.
fn apply(value: &str) -> Result<(usize, usize), String> {
    let mut schedulers = ten();
    schedulers.apply_flag(value.as_bytes())?;
    Ok((schedulers.total(), schedulers.online()))
}

#[test]
fn applies_the_s_flag_as_pinned_erts() {
    for (value, expected) in [
        ("2", (2, 2)),
        ("4:2", (4, 2)),
        (":1", (10, 1)),
        ("-1:1", (9, 1)),
        ("+2:-8", (2, 2)),
        ("0:0", (10, 10)),
        // `strtol` saturates and the `int` store truncates: `LONG_MAX`
        // gives -1, and `LONG_MIN` gives 0.
        ("99999999999999999999", (9, 9)),
        ("-99999999999999999999", (10, 10)),
    ] {
        assert_eq!(apply(value), Ok(expected), "{value}");
    }
}

#[test]
fn rejects_a_bad_s_flag_as_pinned_erts() {
    for (value, expected) in [
        ("x", "bad amount of schedulers x"),
        ("2000", "bad amount of schedulers 2000"),
        (
            "3:5",
            "bad amount of schedulers online 5 (total amount of schedulers 3)",
        ),
    ] {
        assert_eq!(apply(value), Err(expected.to_owned()), "{value}");
    }
}

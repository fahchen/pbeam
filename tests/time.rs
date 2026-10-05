mod common;

use std::process::Command;

use common::pinned_beam;
use pbeam_runtime::{BeamProgram, Term};

const TIME_PROBE: &[u8] = include_bytes!("../fixtures/time/time_probe.beam");

/// Run `time_probe:Function()` on the pinned ERTS and return its `~w` output.
fn reference(function: &str) -> Vec<u8> {
    let reference = Command::new("erl")
        .args([
            "-noshell",
            "-pa",
            "fixtures/time",
            "-eval",
            &format!("io:format(\"~w~n\", [time_probe:{function}()]), halt()."),
        ])
        .output()
        .unwrap();
    assert!(reference.status.success());
    assert!(reference.stderr.is_empty());
    reference.stdout
}

/// Load the pinned `erlang.beam`, which runs the `erlang:convert_time_unit/3`
/// trap, and the probe.
fn program() -> BeamProgram {
    let erlang = pinned_beam("erlang");
    BeamProgram::load(&[&erlang, TIME_PROBE]).unwrap()
}

#[test]
fn reads_the_monotonic_and_the_system_clock_as_pinned_erts() {
    // `monotonic/0` checks that `erlang:monotonic_time(native)` starts at
    // -576460752000000000 (`erl_time.h:318-321`, `331-340`), and that each
    // unit gives the floor of a `native` value read before and after it
    // (`erl_time_sup.c:2268-2350`). The unit 3 traps to
    // `erlang:convert_time_unit/3`. `system/0` checks the same relations
    // for `os:system_time/1` on the host `CLOCK_REALTIME`
    // (`erl_time_sup.c:2595-2599`, `sys_time.c:469-497`).
    let program = program();
    let truths = Term::List(vec![Term::Atom(program.atom("true").unwrap()); 16]);
    for function in ["monotonic", "system"] {
        assert_eq!(
            program.execute("time_probe", function, &[]).unwrap(),
            truths
        );
        assert_eq!(
            reference(function),
            b"[true,true,true,true,true,true,true,true,true,true,true,true,true,true,true,true]\n"
        );
    }
}

#[test]
fn raises_badarg_for_a_unit_that_is_not_valid_as_pinned_erts() {
    // Zero, a negative integer, and each term that is not a time unit or a
    // positive integer raise `badarg` (`erl_time_sup.c:2328-2341`).
    let program = program();
    let atom = |name| Term::Atom(program.atom(name).unwrap());
    let badargs = Term::List(vec![Term::Tuple(vec![atom("error"), atom("badarg")]); 10]);
    assert_eq!(
        program.execute("time_probe", "units", &[]).unwrap(),
        Term::List(vec![badargs.clone(), badargs])
    );
    let badargs = ["{error,badarg}"; 10].join(",");
    assert_eq!(
        reference("units"),
        format!("[[{badargs}],[{badargs}]]\n").into_bytes()
    );
}

#[test]
fn converts_the_time_unit_in_the_pinned_erlang_beam() {
    // `erts_internal:perf_counter_unit/0` gives `ERTS_HRTIME_UNIT`
    // (`erl_time_sup.c:2607-2610`), and `erlang:convert_time_unit/3` runs
    // `gc_bif` `erlang:'*'/2`, `erlang:'-'/2`, and `erlang:'div'/2`
    // (`erlang.erl:5299-5340`). It rounds a negative time down.
    let program = program();
    assert_eq!(
        program.execute("time_probe", "convert", &[]).unwrap(),
        Term::List(
            [
                1_000_000_000,
                -1_729_382_256,
                -1_729_382_256,
                5_373_665_196,
                1,
                -1,
                7,
                -8,
                -9,
            ]
            .map(Term::Integer)
            .into()
        )
    );
    assert_eq!(
        reference("convert"),
        b"[1000000000,-1729382256,-1729382256,5373665196,1,-1,7,-8,-9]\n"
    );
}

#[test]
fn reports_the_perf_counter_product_outside_the_integer_range() {
    // `erlang:monotonic_time(perf_counter)` traps to
    // `erlang:convert_time_unit(Time, 1000000000, perf_counter)`, which
    // multiplies 1000000000 and the time (`erlang.erl:5333-5335`). The
    // product is a bignum on ERTS. PBeam has no bignum term and returns a
    // runtime error (row "Integer range" in `docs/erts-differences.md`).
    let program = program();
    let error = program
        .execute("time_probe", "perf_counter", &[])
        .unwrap_err();
    let message = error.to_string();
    let time = message
        .split("unsupported erlang:*/2 result of 1000000000 and ")
        .nth(1)
        .and_then(|rest| rest.strip_suffix(": needs a bignum"))
        .unwrap_or_else(|| panic!("{message}"));
    // The time is the `native` monotonic time, at most 10 s after the start.
    let first_seconds = -576_460_752_000_000_000..-576_460_742_000_000_000;
    let time: i64 = time.parse().unwrap();
    assert!(first_seconds.contains(&time), "{time}");
    // ERTS returns the time in the `perf_counter` unit, 1000000000.
    let erts = reference("perf_counter");
    let erts: i64 = std::str::from_utf8(&erts)
        .unwrap()
        .trim_end()
        .parse()
        .unwrap();
    assert!(first_seconds.contains(&erts), "{erts}");
}

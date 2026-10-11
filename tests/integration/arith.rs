use std::process::Command;

use pbeam_runtime::{BeamProgram, Term};

const ARITH_PROBE: &[u8] = include_bytes!("../../fixtures/arith/arith_probe.beam");

/// Run `arith_probe:Function()` on the pinned ERTS and return its `~w` output.
fn reference(function: &str) -> Vec<u8> {
    let reference = Command::new("erl")
        .args([
            "-noshell",
            "-pa",
            "fixtures/arith",
            "-eval",
            &format!("io:format(\"~w~n\", [arith_probe:{function}()]), halt()."),
        ])
        .output()
        .unwrap();
    assert!(reference.status.success());
    assert!(reference.stderr.is_empty());
    reference.stdout
}

#[test]
fn adds_and_subtracts_integers_as_pinned_erts() {
    // `arith_probe:arith/0` runs `gc_bif2` `erlang:'+'/2` and `erlang:'-'/2`
    // with fail label 0, as `io_lib_format:build_bin/2` and
    // `user_drv:gr_add_cur/3` do. 2^59 and the values up to 2^63 - 1 are
    // ERTS bignums, with the same values. An operand that is not a number
    // raises `error:badarith` (`erl_arith.c:386-389`, `erl_arith.c:599-602`).
    let program = BeamProgram::load(&[ARITH_PROBE]).unwrap();
    let badarith = Term::Atom(program.atom("badarith").unwrap());
    let mut expected: Vec<Term> = [
        3,
        -2,
        -7,
        576_460_752_303_423_488,
        -576_460_752_303_423_489,
        576_460_752_303_423_487,
        i64::MAX,
        i64::MIN,
    ]
    .map(Term::Integer)
    .into();
    expected.extend(std::iter::repeat_n(badarith, 4));
    assert_eq!(
        program.execute("arith_probe", "arith", &[]).unwrap(),
        Term::list_from(expected)
    );
    assert_eq!(
        reference("arith"),
        b"[3,-2,-7,576460752303423488,-576460752303423489,576460752303423487,\
          9223372036854775807,-9223372036854775808,badarith,badarith,badarith,badarith]\n"
    );
}

#[test]
fn jumps_to_the_guard_fail_label_on_badarith_as_pinned_erts() {
    // `arith_probe:guards/0` runs `gc_bif2` `erlang:'+'/2` and
    // `erlang:'-'/2` in a guard. A `badarith` jumps to the fail label, and
    // the next clause runs (`jit/arm/instr_arith.cpp:114-143`).
    let program = BeamProgram::load(&[ARITH_PROBE]).unwrap();
    let atom = |name| Term::Atom(program.atom(name).unwrap());
    let pair = |tag, value| Term::tuple(vec![atom(tag), Term::Integer(value)]);
    let expected = vec![
        pair("up", 2),
        atom("other"),
        atom("other"),
        atom("other"),
        pair("down", 2),
        atom("other"),
        atom("other"),
        atom("other"),
    ];
    assert_eq!(
        program.execute("arith_probe", "guards", &[]).unwrap(),
        Term::list_from(expected)
    );
    assert_eq!(
        reference("guards"),
        b"[{up,2},other,other,other,{down,2},other,other,other]\n"
    );
}

#[test]
fn calls_the_arithmetic_bifs_externally_as_pinned_erts() {
    // `arith_probe:externals/0` calls `erlang:'+'/2` and `erlang:'-'/2`
    // through the `apply` instruction and through an external fun, which go
    // to the BIF as `call_ext` does.
    let program = BeamProgram::load(&[ARITH_PROBE]).unwrap();
    assert_eq!(
        program.execute("arith_probe", "externals", &[]).unwrap(),
        Term::list_from(vec![
            Term::Integer(3),
            Term::Integer(-2),
            Term::Atom(program.atom("badarith").unwrap()),
        ])
    );
    assert_eq!(reference("externals"), b"[3,-2,badarith]\n");
}

#[test]
fn reports_a_result_outside_the_integer_range() {
    // ERTS makes a bignum for a result outside the 64-bit signed range
    // (`erl_arith.c:378-384`, `erl_arith.c:134-142`). PBeam has no bignum term and returns a
    // runtime error (row "Integer range" in `docs/erts-differences.md`).
    let program = BeamProgram::load(&[ARITH_PROBE]).unwrap();
    for (function, message, erts) in [
        (
            "plus_overflow",
            "unsupported erlang:+/2 result of 9223372036854775807 and 1: needs a bignum",
            &b"9223372036854775808\n"[..],
        ),
        (
            "minus_overflow",
            "unsupported erlang:-/2 result of -9223372036854775808 and 1: needs a bignum",
            b"-9223372036854775809\n",
        ),
        (
            "times_overflow",
            "unsupported erlang:*/2 result of 9223372036854775807 and 2: needs a bignum",
            b"18446744073709551614\n",
        ),
        (
            "div_overflow",
            "unsupported erlang:div/2 result of -9223372036854775808 and -1: needs a bignum",
            b"9223372036854775808\n",
        ),
    ] {
        let error = program.execute("arith_probe", function, &[]).unwrap_err();
        assert!(error.to_string().contains(message), "{error}");
        assert_eq!(reference(function), erts);
    }
}

#[test]
fn loads_integer_operands_outside_the_integer_range() {
    // `arith_probe:big_operand/0` has `gc_bif2` `erlang:'+'/2`, and
    // `arith_probe:below_big/1` has `is_lt`, each with the operand
    // `{integer,18446744073709551616}`, as `bsl` in
    // `net:shortest_matching_prefix/3` (net.erl:865) has. ERTS loads the
    // operand as a bignum literal. PBeam has no bignum term: the module
    // loads, and only an instruction that reads the operand returns a
    // runtime error (row "Integer range" in `docs/erts-differences.md`).
    let program = BeamProgram::load(&[ARITH_PROBE]).unwrap();
    for (function, erts) in [
        ("big_operand", &b"18446744073709551617\n"[..]),
        ("big_guard", b"below\n"),
    ] {
        let error = program.execute("arith_probe", function, &[]).unwrap_err();
        assert!(
            error.to_string().contains(
                "unsupported BEAM integer operand outside the 64-bit range: needs a bignum"
            ),
            "{error}"
        );
        assert_eq!(reference(function), erts);
    }
}

#[test]
fn multiplies_and_divides_integers_as_pinned_erts() {
    // `arith_probe:times/0` and `arith_probe:int_div/0` run `gc_bif2`
    // `erlang:'*'/2` and `erlang:'div'/2` with fail label 0, as
    // `erlang:convert_time_unit/3` does (erlang.erl:5333-5336). An operand
    // that is not a number raises `error:badarith` (`stimes_2`,
    // `erl_arith.c:117-120`). `div` truncates toward zero, and a divisor of
    // 0 raises `badarith` before the operand types are checked (`intdiv_2`,
    // `erl_arith.c:127-143`).
    let program = BeamProgram::load(&[ARITH_PROBE]).unwrap();
    let badarith = Term::Atom(program.atom("badarith").unwrap());
    let mut times: Vec<Term> = [
        42,
        -12,
        12,
        0,
        4_611_686_018_427_387_904,
        i64::MIN,
        9_223_372_030_926_249_001,
    ]
    .map(Term::Integer)
    .into();
    times.extend(std::iter::repeat_n(badarith.clone(), 3));
    assert_eq!(
        program.execute("arith_probe", "times", &[]).unwrap(),
        Term::list_from(times)
    );
    assert_eq!(
        reference("times"),
        b"[42,-12,12,0,4611686018427387904,-9223372036854775808,9223372030926249001,\
          badarith,badarith,badarith]\n"
    );
    let mut quotients: Vec<Term> = [
        3,
        -3,
        -3,
        3,
        0,
        i64::MIN,
        -i64::MAX,
        -4_611_686_018_427_387_904,
    ]
    .map(Term::Integer)
    .into();
    quotients.extend(std::iter::repeat_n(badarith, 4));
    assert_eq!(
        program.execute("arith_probe", "int_div", &[]).unwrap(),
        Term::list_from(quotients)
    );
    assert_eq!(
        reference("int_div"),
        b"[3,-3,-3,3,0,-9223372036854775808,-9223372036854775807,-4611686018427387904,\
          badarith,badarith,badarith,badarith]\n"
    );
}

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
    assert_eq!(reference.stderr, [] as [u8; 0]);
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
        program.execute("arith_probe", "arith", &[]).unwrap().term,
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
        program.execute("arith_probe", "guards", &[]).unwrap().term,
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
        program
            .execute("arith_probe", "externals", &[])
            .unwrap()
            .term,
        Term::list_from(vec![
            Term::Integer(3),
            Term::Integer(-2),
            Term::Atom(program.atom("badarith").unwrap()),
        ])
    );
    assert_eq!(reference("externals"), b"[3,-2,badarith]\n");
}

/// Return the integer of decimal `text` in its canonical form.
fn integer(text: &str) -> Term {
    match text.parse::<i64>() {
        Ok(value) => Term::Integer(value),
        Err(_) => Term::BigInt(std::sync::Arc::new(text.parse().unwrap())),
    }
}

#[test]
fn makes_a_bignum_for_a_result_outside_the_integer_range() {
    // ERTS makes a bignum for a result outside the 64-bit signed range
    // (`erl_arith.c:378-384`, `erl_arith.c:134-142`), and so does PBeam.
    let program = BeamProgram::load(&[ARITH_PROBE]).unwrap();
    for (function, erts) in [
        ("plus_overflow", "9223372036854775808"),
        ("minus_overflow", "-9223372036854775809"),
        ("times_overflow", "18446744073709551614"),
        ("div_overflow", "9223372036854775808"),
    ] {
        let term = program.execute("arith_probe", function, &[]).unwrap().term;
        assert_eq!(term, integer(erts), "{function}");
        assert_eq!(reference(function), format!("{erts}\n").into_bytes());
    }
}

#[test]
fn loads_integer_operands_outside_the_integer_range() {
    // `arith_probe:big_operand/0` has `gc_bif2` `erlang:'+'/2`, and
    // `arith_probe:below_big/1` has `is_lt`, each with the operand
    // `{integer,18446744073709551616}`, as `bsl` in
    // `net:shortest_matching_prefix/3` (net.erl:865) has. ERTS loads the
    // operand as a bignum literal (`marshal_integer`,
    // `beam_file.c:2134-2215`), and so does PBeam.
    let program = BeamProgram::load(&[ARITH_PROBE]).unwrap();
    assert_eq!(
        program
            .execute("arith_probe", "big_operand", &[])
            .unwrap()
            .term,
        integer("18446744073709551617")
    );
    assert_eq!(reference("big_operand"), b"18446744073709551617\n");
    assert_eq!(
        program
            .execute("arith_probe", "big_guard", &[])
            .unwrap()
            .term,
        Term::Atom(program.atom("below").unwrap())
    );
    assert_eq!(reference("big_guard"), b"below\n");
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
        program.execute("arith_probe", "times", &[]).unwrap().term,
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
        program.execute("arith_probe", "int_div", &[]).unwrap().term,
        Term::list_from(quotients)
    );
    assert_eq!(
        reference("int_div"),
        b"[3,-3,-3,3,0,-9223372036854775808,-9223372036854775807,-4611686018427387904,\
          badarith,badarith,badarith,badarith]\n"
    );
}

#[test]
fn adds_and_subtracts_at_each_bound_of_the_small_integer_as_pinned_erts() {
    // `arith_probe:small_bounds/0` runs `gc_bif2` `erlang:'+'/2` and
    // `erlang:'-'/2` at -(2^59) and 2^59 - 1. ERTS adds two small integers
    // with a small result inline and calls the BIF for a bignum result or a
    // bignum operand (`emit_i_plus` and `emit_i_minus`,
    // `jit/arm/instr_arith.cpp:145-205` and `:334-392`). The `Bif` arm of
    // PBeam adds each pair of 64-bit integers with a 64-bit result inline,
    // and the values are the same on each side of each bound.
    let program = BeamProgram::load(&[ARITH_PROBE]).unwrap();
    let expected: Vec<Term> = [
        576_460_752_303_423_487,
        576_460_752_303_423_488,
        576_460_752_303_423_488,
        -576_460_752_303_423_488,
        -576_460_752_303_423_489,
        -576_460_752_303_423_489,
        576_460_752_303_423_487,
        -576_460_752_303_423_488,
    ]
    .map(Term::Integer)
    .into();
    assert_eq!(
        program
            .execute("arith_probe", "small_bounds", &[])
            .unwrap()
            .term,
        Term::list_from(expected)
    );
    assert_eq!(
        reference("small_bounds"),
        b"[576460752303423487,576460752303423488,576460752303423488,-576460752303423488,\
          -576460752303423489,-576460752303423489,576460752303423487,-576460752303423488]\n"
    );
}

#[test]
fn stops_at_the_float_argument_of_an_addition() {
    // ERTS adds a float argument in the BIF that `emit_i_plus` calls
    // (`jit/arm/instr_arith.cpp:190-201`), as `erts_mixed_plus` does
    // (`erl_arith.c:358-508`).
    let program = BeamProgram::load(&[ARITH_PROBE]).unwrap();
    assert_eq!(
        program
            .execute("arith_probe", "float_plus", &[])
            .unwrap()
            .term,
        Term::Float(3.5)
    );
    assert_eq!(reference("float_plus"), b"3.5\n");
}

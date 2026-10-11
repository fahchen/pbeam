use std::process::Command;

use pbeam_runtime::{BeamProgram, Term};

const BODY_BIF_PROBE: &[u8] = include_bytes!("../../fixtures/body_bif/body_bif_probe.beam");

/// Run `body_bif_probe:Call` on the pinned ERTS and return its `~w` output.
fn reference(call: &str) -> Vec<u8> {
    let reference = Command::new("erl")
        .args([
            "-noshell",
            "-pa",
            "fixtures/body_bif",
            "-eval",
            &format!("io:format(\"~w~n\", [body_bif_probe:{call}]), halt()."),
        ])
        .output()
        .unwrap();
    assert!(reference.status.success());
    assert!(reference.stderr.is_empty());
    reference.stdout
}

/// Return the list of the atoms `true` and `false` for `values`.
fn booleans(program: &BeamProgram, values: &[bool]) -> Term {
    Term::list_from(
        values
            .iter()
            .map(|&value| Term::Atom(program.atom(if value { "true" } else { "false" }).unwrap())),
    )
}

#[test]
fn runs_max_less_and_equal_in_body_position_as_pinned_erts() {
    // `body_bif_probe:max/0`, `less/0`, and `equal/0` run the `bif` forms of
    // `erlang:max/2` (`max_2`, `erl_bif_guard.c:481-485`), `erlang:'<'/2`
    // (`slt_2`, `erl_bif_op.c:103-106`), and `erlang:'=='/2` (`seqeq_2`,
    // `erl_bif_op.c:118-121`) with fail label 0 on pairs of integers,
    // bignums, and terms of different types.
    let program = BeamProgram::load(&[BODY_BIF_PROBE]).unwrap();
    let atom = |name| Term::Atom(program.atom(name).unwrap());
    let binary = |bytes: &[u8]| Term::Binary(bytes.to_vec());
    assert_eq!(
        program.execute("body_bif_probe", "max", &[]).unwrap(),
        Term::list_from(vec![
            Term::Integer(2),
            Term::Integer(2),
            Term::Integer(-3),
            Term::Integer(576_460_752_303_423_488),
            Term::Integer(1),
            Term::Integer(i64::MAX),
            atom("a"),
            atom("a"),
            atom("b"),
            atom("b"),
            Term::list_from(vec![Term::Integer(1)]),
            Term::list_from(vec![Term::Integer(1), Term::Integer(2)]),
            binary(b"a"),
            Term::tuple(vec![Term::Integer(1), Term::Integer(2)]),
            Term::nil(),
            Term::tuple(Vec::new()),
            Term::tuple(vec![Term::Integer(1), atom("a")]),
            binary(b"ab"),
        ])
    );
    assert_eq!(
        reference("max()"),
        b"[2,2,-3,576460752303423488,1,9223372036854775807,a,a,b,b,[1],[1,2],<<97>>,{1,2},[],{},\
          {1,a},<<97,98>>]\n"
    );
    assert_eq!(
        program.execute("body_bif_probe", "less", &[]).unwrap(),
        booleans(
            &program,
            &[
                true, false, false, false, true, false, true, false, true, false, true, false,
                false, false, true, true, false, false,
            ]
        )
    );
    assert_eq!(
        reference("less()"),
        b"[true,false,false,false,true,false,true,false,true,false,true,false,false,false,true,\
          true,false,false]\n"
    );
    assert_eq!(
        program.execute("body_bif_probe", "equal", &[]).unwrap(),
        booleans(
            &program,
            &[
                false, false, true, false, false, false, false, false, false, false, false, false,
                false, false, false, false, true, true,
            ]
        )
    );
    assert_eq!(
        reference("equal()"),
        b"[false,false,true,false,false,false,false,false,false,false,false,false,false,false,\
          false,false,true,true]\n"
    );
}

#[test]
fn negates_integers_in_body_position_as_pinned_erts() {
    // `body_bif_probe:neg/0` runs `gc_bif` `erlang:'-'/1` with fail label 0,
    // as `io_lib_format:field_width/2` (io_lib_format.erl:254) does
    // (`erts_unary_minus`, `erl_arith.c:509-567`). An operand that is not a
    // number raises `error:badarith`.
    let program = BeamProgram::load(&[BODY_BIF_PROBE]).unwrap();
    let badarith = Term::Atom(program.atom("badarith").unwrap());
    let mut expected: Vec<Term> = [
        0,
        -5,
        5,
        -576_460_752_303_423_488,
        576_460_752_303_423_488,
        -i64::MAX,
    ]
    .map(Term::Integer)
    .into();
    expected.extend(std::iter::repeat_n(badarith, 3));
    assert_eq!(
        program.execute("body_bif_probe", "neg", &[]).unwrap(),
        Term::list_from(expected)
    );
    assert_eq!(
        reference("neg()"),
        b"[0,-5,5,-576460752303423488,576460752303423488,-9223372036854775807,\
          badarith,badarith,badarith]\n"
    );
}

#[test]
fn reports_a_negation_outside_the_integer_range() {
    // ERTS makes the bignum 2^63 for `-(-(2^63))` (`erl_arith.c:524-531`).
    // PBeam has no bignum term and returns a runtime error (row "Integer
    // range" in `docs/erts-differences.md`).
    let program = BeamProgram::load(&[BODY_BIF_PROBE]).unwrap();
    let error = program
        .execute("body_bif_probe", "neg_overflow", &[])
        .unwrap_err();
    assert!(
        error
            .to_string()
            .contains("unsupported erlang:-/1 result of -9223372036854775808: needs a bignum"),
        "{error}"
    );
    assert_eq!(reference("neg_overflow()"), b"9223372036854775808\n");
}

#[test]
fn ands_integers_bitwise_as_pinned_erts() {
    // `body_bif_probe:bit_and/0` runs `gc_bif` `erlang:'band'/2` with fail
    // label 0 (`band_2` and `erts_band`, `erl_arith.c:164-170`,
    // `erl_arith.c:1252-1278`), and `executable/1` runs it in a guard, as
    // `os:verify_executable/3` (os.erl:455-456) does. An operand that is not
    // an integer raises `error:badarith`, and in a guard it jumps to the
    // fail label.
    let program = BeamProgram::load(&[BODY_BIF_PROBE]).unwrap();
    let atom = |name| Term::Atom(program.atom(name).unwrap());
    let mut expected: Vec<Term> = [
        8,
        255,
        -8,
        576_460_752_303_423_488,
        i64::MIN,
        -576_460_752_303_423_489,
        0,
    ]
    .map(Term::Integer)
    .into();
    expected.extend(std::iter::repeat_n(atom("badarith"), 3));
    assert_eq!(
        program.execute("body_bif_probe", "bit_and", &[]).unwrap(),
        Term::list_from(expected)
    );
    assert_eq!(
        reference("bit_and()"),
        b"[8,255,-8,576460752303423488,-9223372036854775808,-576460752303423489,0,\
          badarith,badarith,badarith]\n"
    );
    for (mode, erts, expected) in [
        (Term::Integer(0o755), "8#755", "yes"),
        (Term::Integer(0o644), "8#644", "no"),
        (Term::Integer(0o100), "8#100", "yes"),
        (atom("a"), "a", "no"),
    ] {
        assert_eq!(
            program
                .execute("body_bif_probe", "executable", &[mode])
                .unwrap(),
            atom(expected)
        );
        assert_eq!(
            reference(&format!("executable({erts})")),
            format!("{expected}\n").as_bytes()
        );
    }
}

#[test]
fn tests_binaries_in_body_position_as_pinned_erts() {
    // `body_bif_probe:binary/0` runs `bif` `erlang:is_binary/1` with fail
    // label 0, as `unicode:no_conversion_needed/3` (unicode.erl:359-360)
    // does (`is_binary_1`, `erl_bif_op.c:206-213`).
    let program = BeamProgram::load(&[BODY_BIF_PROBE]).unwrap();
    assert_eq!(
        program.execute("body_bif_probe", "binary", &[]).unwrap(),
        booleans(
            &program,
            &[true, true, false, false, false, false, false, false, false]
        )
    );
    assert_eq!(
        reference("binary()"),
        b"[true,true,false,false,false,false,false,false,false]\n"
    );
}

#[test]
fn calls_the_bifs_externally_as_pinned_erts() {
    // `body_bif_probe:externals/0` calls each BIF through the `apply`
    // instruction or through an external fun, which go to the BIF as
    // `call_ext` does.
    let program = BeamProgram::load(&[BODY_BIF_PROBE]).unwrap();
    let atom = |name| Term::Atom(program.atom(name).unwrap());
    assert_eq!(
        program.execute("body_bif_probe", "externals", &[]).unwrap(),
        Term::list_from(vec![
            atom("a"),
            atom("false"),
            atom("true"),
            Term::Integer(-7),
            Term::Integer(2),
            atom("true"),
            atom("badarith"),
            atom("badarith"),
        ])
    );
    assert_eq!(
        reference("externals()"),
        b"[a,false,true,-7,2,true,badarith,badarith]\n"
    );
}

#[test]
fn charges_a_bignum_operand_as_pinned_erts() {
    // `body_bif_probe:costs/0` measures the reductions of `band` and unary
    // `-`. A bignum operand costs 1 more reduction (`bump_reds_linear`,
    // `erl_arith.c:56-72`): the negation of the small integer -(2^59) gives
    // the bignum 2^59 and costs nothing more, and the negation of the
    // bignum 2^59 costs 1 more.
    let program = BeamProgram::load(&[BODY_BIF_PROBE]).unwrap();
    let pair =
        |value, reductions| Term::tuple(vec![Term::Integer(value), Term::Integer(reductions)]);
    assert_eq!(
        program.execute("body_bif_probe", "costs", &[]).unwrap(),
        Term::list_from(vec![
            pair(8, 1),
            pair(576_460_752_303_423_488, 2),
            pair(i64::MAX, 2),
            pair(-5, 1),
            pair(576_460_752_303_423_488, 1),
            pair(-576_460_752_303_423_488, 2),
        ])
    );
    assert_eq!(
        reference("costs()"),
        b"[{8,1},{576460752303423488,2},{9223372036854775807,2},{-5,1},\
          {576460752303423488,1},{-576460752303423488,2}]\n"
    );
}

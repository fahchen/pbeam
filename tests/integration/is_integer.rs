use std::process::Command;

use pbeam_runtime::{BeamProgram, Term};

const IS_INTEGER_PROBE: &[u8] = include_bytes!("../../fixtures/is_integer/is_integer_probe.beam");

/// Evaluate an expression on the pinned ERTS with `is_integer_probe` in the
/// code path and return its `~w` output.
fn erl_reference(expression: &str) -> String {
    let reference = Command::new("erl")
        .args([
            "-noshell",
            "-pa",
            "fixtures/is_integer",
            "-eval",
            &format!("io:format(\"~w~n\", [{expression}]), halt()."),
        ])
        .output()
        .unwrap();
    assert!(
        reference.status.success() && reference.stderr.is_empty(),
        "{reference:?}"
    );
    String::from_utf8(reference.stdout).unwrap()
}

#[test]
fn tells_whether_a_term_is_an_integer_in_a_range_like_erts() {
    // The pinned compiler emits each `is_integer/3` call of `body/3` as
    // `bif3` with the fail label 0, and of the guard of `guard/3` as `bif3`
    // with a fail label. A bound that is not an integer raises `badarg` in a
    // body (`erl_bif_guard.c:339-342`) and goes to the fail label in a guard.
    // The fun and `apply/3` run the BIF through its export. PBeam gives the
    // stack trace `[]`, so the probe catches only the class and the reason.
    let program = BeamProgram::load(&[IS_INTEGER_PROBE]).unwrap();
    let atom = |name: &str| Term::Atom(program.atom(name).unwrap());
    let badarg = Term::tuple(vec![atom("error"), atom("badarg")]);
    let mut expected = [
        "true", "true", "true", "false", "false", "false", "false", "true", "true", "false",
    ]
    .map(atom)
    .to_vec();
    expected.extend(std::iter::repeat_n(badarg.clone(), 4));
    expected.extend(["in", "out", "out", "out", "out", "true", "false"].map(atom));
    expected.push(badarg);
    assert_eq!(
        program
            .execute("is_integer_probe", "main", &[])
            .unwrap()
            .term,
        Term::list_from(expected)
    );

    assert_eq!(
        erl_reference("is_integer_probe:main()"),
        "[true,true,true,false,false,false,false,true,true,false,{error,badarg},\
         {error,badarg},{error,badarg},{error,badarg},in,out,out,out,out,true,false,\
         {error,badarg}]\n"
    );
}

#[test]
fn stops_at_a_bignum_bound() {
    // ERTS compares a bignum bound (`erl_bif_guard.c:347-348`). PBeam has no
    // bignum: the run stops when `move` reads the operand 2^70 (row
    // "Integer range" of `docs/erts-differences.md`).
    let program = BeamProgram::load(&[IS_INTEGER_PROBE]).unwrap();
    let error = program
        .execute("is_integer_probe", "big", &[])
        .unwrap_err()
        .to_string();
    assert_eq!(
        error,
        "unsupported BEAM integer operand outside the 64-bit range: needs a bignum"
    );

    assert_eq!(erl_reference("is_integer_probe:big()"), "true\n");
}

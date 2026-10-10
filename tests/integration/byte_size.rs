use std::process::Command;

use pbeam_runtime::{BeamProgram, Term};

const BYTE_SIZE_PROBE: &[u8] = include_bytes!("../../fixtures/byte_size/byte_size_probe.beam");

/// Evaluate an expression on the pinned ERTS with `byte_size_probe` in the
/// code path and return its `~w` output.
fn erl_reference(expression: &str) -> String {
    let reference = Command::new("erl")
        .args([
            "-noshell",
            "-pa",
            "fixtures/byte_size",
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
fn gives_the_byte_size_like_erts() {
    // The pinned compiler emits each `byte_size/1` call of `main/0` and
    // `body/1` as `gc_bif` with the fail label 0, and of the guard of
    // `guard/1` as `gc_bif` with a fail label. The fun and `apply/3` run the
    // BIF through its export. PBeam gives the stack trace `[]`, so the probe
    // catches only the class and the reason.
    let program = BeamProgram::load(&[BYTE_SIZE_PROBE]).unwrap();
    let atom = |name: &str| Term::Atom(program.atom(name).unwrap());
    let int = Term::Integer;
    let badarg = Term::Tuple(vec![atom("error"), atom("badarg")]);
    let mut expected = vec![
        int(0),
        int(1),
        int(6),
        int(43),
        int(3),
        int(2),
        atom("small"),
        atom("big"),
        atom("other"),
        atom("other"),
    ];
    expected.extend(std::iter::repeat_n(badarg, 5));
    assert_eq!(
        program.execute("byte_size_probe", "main", &[]).unwrap(),
        Term::List(expected)
    );

    assert_eq!(
        erl_reference("byte_size_probe:main()"),
        "[0,1,6,43,3,2,small,big,other,other,{error,badarg},{error,badarg},\
         {error,badarg},{error,badarg},{error,badarg}]\n"
    );
}

#[test]
fn stops_at_a_bitstring_literal() {
    // ERTS rounds the size of a bitstring up to the next byte. PBeam has no
    // bitstring that is not a binary: the loader keeps a `BIT_BINARY_EXT`
    // literal as an unsupported literal, so the run stops when `move` reads
    // it (row "Bitstring terms" of `docs/erts-differences.md`).
    let program = BeamProgram::load(&[BYTE_SIZE_PROBE]).unwrap();
    let error = program
        .execute("byte_size_probe", "bits", &[])
        .unwrap_err()
        .to_string();
    assert!(
        error.starts_with("unsupported BEAM literal index "),
        "{error}"
    );

    assert_eq!(erl_reference("byte_size_probe:bits()"), "[1,4]\n");
}

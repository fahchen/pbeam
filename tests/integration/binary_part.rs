use std::process::Command;

use pbeam_runtime::{BeamProgram, Term};

const BINARY_PART_PROBE: &[u8] =
    include_bytes!("../../fixtures/binary_part/binary_part_probe.beam");

/// Evaluate an expression on the pinned ERTS with `binary_part_probe` in the
/// code path and return its `~w` output.
fn erl_reference(expression: &str) -> String {
    let reference = Command::new("erl")
        .args([
            "-noshell",
            "-pa",
            "fixtures/binary_part",
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
fn gives_the_binary_part_like_erts() {
    // The pinned compiler emits each `binary_part/3` call of `main/0` and
    // `body/3` as `gc_bif3` with the fail label 0, and of the guards of
    // `guard/3` as `gc_bif3` with a fail label. The fun and `apply/3` run the
    // BIF through its export. PBeam gives the stack trace `[]`, so the probe
    // catches only the class and the reason.
    let program = BeamProgram::load(&[BINARY_PART_PROBE]).unwrap();
    let atom = |name: &str| Term::Atom(program.atom(name).unwrap());
    let binary = |bytes: &[u8]| Term::binary(bytes.to_vec());
    let badarg = Term::tuple(vec![atom("error"), atom("badarg")]);
    let mut expected = vec![
        binary(b"hello"),
        binary(b"ell"),
        binary(b""),
        binary(b"lo"),
        binary(b"he"),
        binary(b""),
        binary(b"bc"),
        binary(b"ab"),
        atom("ell"),
        atom("part"),
        atom("other"),
        atom("other"),
    ];
    expected.extend(std::iter::repeat_n(badarg, 12));
    assert_eq!(
        program.execute("binary_part_probe", "main", &[]).unwrap(),
        Term::list_from(expected)
    );

    assert_eq!(
        erl_reference("binary_part_probe:main()"),
        "[<<104,101,108,108,111>>,<<101,108,108>>,<<>>,<<108,111>>,<<104,101>>,<<>>,\
         <<98,99>>,<<97,98>>,ell,part,other,other,{error,badarg},{error,badarg},\
         {error,badarg},{error,badarg},{error,badarg},{error,badarg},{error,badarg},\
         {error,badarg},{error,badarg},{error,badarg},{error,badarg},{error,badarg}]\n"
    );
}

#[test]
fn stops_at_a_bitstring_literal() {
    // ERTS raises `badarg` for a bitstring whose size is not a multiple of 8.
    // PBeam has no bitstring that is not a binary: the loader keeps a
    // `BIT_BINARY_EXT` literal as an unsupported literal, so the run stops
    // when `move` reads it (row "Bitstring terms" of
    // `docs/erts-differences.md`).
    let program = BeamProgram::load(&[BINARY_PART_PROBE]).unwrap();
    let error = program
        .execute("binary_part_probe", "bits", &[])
        .unwrap_err()
        .to_string();
    assert!(
        error.starts_with("unsupported BEAM literal index "),
        "{error}"
    );

    assert_eq!(
        erl_reference("binary_part_probe:bits()"),
        "{error,badarg}\n"
    );
}

#[test]
fn gives_a_part_that_shares_the_bytes_of_its_source() {
    // `erts_binary_part` makes a sub-binary that refers to the source
    // (`binary_part_3` in `erl_bif_binary.c`; `binary_referenced_byte_size_1`,
    // `erl_bif_binary.c:2508-2530`), so a part keeps the offset of its bytes
    // in the array of the source. The source and the part are one array when
    // the part points into it, and a copy would point outside it.
    let program = BeamProgram::load(&[BINARY_PART_PROBE]).unwrap();
    let result = program.execute("binary_part_probe", "main", &[]).unwrap();
    let parts: Vec<&Term> = result.proper_list().expect("a list").collect();
    // `main/0` gives `binary_part(<<"hello">>, 0, 5)` first and
    // `binary_part(<<"hello">>, 1, 3)` second.
    let (Term::Binary(source), Term::Binary(part)) = (parts[0], parts[1]) else {
        panic!("the results are binaries");
    };
    assert_eq!((&source[..], &part[..]), (&b"hello"[..], &b"ell"[..]));
    let (source, part) = (source.as_ptr() as usize, part.as_ptr() as usize);
    assert!(
        (source..source + 5).contains(&part),
        "the part {part:#x} is not in the array of the source {source:#x}"
    );
    assert_eq!(part - source, 1);
}

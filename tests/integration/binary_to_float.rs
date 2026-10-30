//! Runs `binary_to_float` on PBeam and compares the results with the pinned ERTS.

use std::process::Command;

use pbeam_runtime::{BeamProgram, Map, Term};

const BINARY_TO_FLOAT_PROBE: &[u8] =
    include_bytes!("../../fixtures/binary_to_float/binary_to_float_probe.beam");

/// The `~w` output of `binary_to_float_probe:main()` on the pinned ERTS.
const EXPECTED: &[u8] =
    include_bytes!("../../fixtures/binary_to_float/binary_to_float_probe.expected");

/// The `~w` output of `binary_to_float_probe:bits()` on the pinned ERTS.
const BITS_EXPECTED: &[u8] = b"[{error,badarg},{error,badarg}]\n";

#[test]
fn reads_floats_from_binaries_as_pinned_erts() {
    // `erlang:binary_to_float/1` follows `binary_to_float_1` (bif.c:3727-3756)
    // with `do_charbuf_to_float` (bif.c:3684-3697): the binary must hold at
    // least one byte and a whole number of bytes, and its text must read as
    // a float. ERTS copies the bytes to a buffer and gives them a NUL
    // terminator (bif.c:3742-3746), so a zero byte ends the text. A text
    // that overflows an infinity raises `badarg`, and a text that underflows
    // to zero gives `0.0` (sys_float.c:111-113, 130-132). Each exception
    // carries the frame of the BIF with its arguments and the `error_info`
    // map (beam_common.c:939-991).
    let program = BeamProgram::load(&[BINARY_TO_FLOAT_PROBE]).unwrap();
    let result = program
        .execute("binary_to_float_probe", "main", &[])
        .unwrap();
    let atom = |name: &str| Term::Atom(result.atom(name).unwrap());
    let binary = |bytes: &[u8]| Term::binary(bytes.to_vec());
    let ok = |value: f64| Term::tuple(vec![atom("ok"), Term::Float(value)]);
    let error_info = Map::from_pairs(
        vec![(atom("module"), atom("erl_erts_errors"))],
        program.atoms(),
    )
    .unwrap();
    let failed = |binary: Term| {
        let frame = Term::tuple(vec![
            atom("erlang"),
            atom("binary_to_float"),
            Term::list_from(vec![binary]),
            Term::list_from(vec![Term::tuple(vec![
                atom("error_info"),
                Term::Map(error_info.clone()),
            ])]),
        ]);
        let caller = |function: &str, arity: i64| {
            Term::tuple(vec![
                atom("binary_to_float_probe"),
                atom(function),
                Term::Integer(arity),
                Term::nil(),
            ])
        };
        Term::tuple(vec![
            atom("error"),
            atom("badarg"),
            Term::list_from(vec![frame, caller("result", 1), caller("main", 0)]),
        ])
    };
    let mut expected = vec![
        ok(1.0),
        ok(17.0),
        ok(22.017_764),
        ok(1.5),
        ok(-0.0),
        ok(0.1),
        ok(0.0),
        ok(5.0e-324),
        ok(2.225_073_858_507_201e-308),
    ];
    expected.extend([
        failed(binary(b"1.0e309")),
        ok(1.0),
        failed(binary(&[49, 0, 46, 48])),
        failed(binary(&[0])),
        failed(binary(b"")),
        failed(binary(b"1")),
        failed(binary(b"1.0 ")),
        failed(binary(b" 1.0")),
        failed(binary(b"0x1.0")),
        failed(Term::Integer(1)),
        failed(Term::nil()),
        failed(atom("ok")),
    ]);
    assert_eq!(result.term, Term::list_from(expected));
    // `binary_to_float_probe:bits()` calls the BIF with a bit string of one
    // bit, which ERTS rejects, because it is not a whole number of bytes
    // (`bif.c:3735`). Only the reference run of this test reads that case:
    // PBeam has no bitstring term of a size that is not a multiple of 8 bits
    // (row "Bitstring terms" of `docs/erts-differences.md`), so the program
    // of this test cannot build the argument. `binary_to_float` tests the
    // size of its argument (`bif/float.rs`), and the case `<<1:1>>` waits
    // for a bitstring term, as task #761 records.

    for (function, expected) in [("main", EXPECTED), ("bits", BITS_EXPECTED)] {
        let reference = Command::new("erl")
            .args([
                "-noshell",
                "-pa",
                "fixtures/binary_to_float",
                "-eval",
                &format!("io:format(\"~w~n\", [binary_to_float_probe:{function}()]), halt()."),
            ])
            .output()
            .unwrap();
        assert!(
            reference.status.success() && reference.stderr.is_empty(),
            "{reference:?}"
        );
        assert_eq!(reference.stdout, expected, "{function}");
    }
}

use std::process::Command;

use pbeam_runtime::{BeamProgram, Map, Term};

const EXISTING_ATOM_PROBE: &[u8] =
    include_bytes!("../../fixtures/existing_atom/existing_atom_probe.beam");

/// The `~w` output of `existing_atom_probe:main()` on the pinned ERTS. The
/// atom `'ÿ'` prints as one latin-1 byte, so the file is not UTF-8.
const EXPECTED: &[u8] = include_bytes!("../../fixtures/existing_atom/existing_atom_probe.expected");

/// The `~w` output of `existing_atom_probe:bits()` on the pinned ERTS.
const BITS_EXPECTED: &[u8] = b"{error,badarg}\n";

#[test]
fn reads_atoms_that_exist_as_pinned_erts() {
    // `erlang:list_to_existing_atom/1` follows `list_to_existing_atom_1`
    // (bif.c:3239-3258), which reads the name with `erts_unicode_list_to_buf`
    // (utils.c:2862-2925) and gives `badarg` for each result below zero of
    // that call, also for the `system_limit` of `list_to_atom/1`.
    // `erlang:binary_to_existing_atom/2` follows `binary_to_atom` with
    // `must_exist` one (erl_unicode.c:1962-1971, 1991-1993, 2009-2011,
    // 2025-2028), which reads `latin1` as one character for each byte and
    // `utf8` and `unicode` as the UTF-8 bytes of the name, and gives
    // `badarg` when `erts_atom_get` (atom.c:381-433) finds no atom. The two
    // BIFs read the atom table of the run and add no atom to it, so the
    // probe makes the atoms of its accepted cases first. Each exception
    // carries the frame of the BIF with its arguments and the `error_info`
    // map (beam_common.c:939-991).
    let program = BeamProgram::load(&[EXISTING_ATOM_PROBE]).unwrap();
    let result = program.execute("existing_atom_probe", "main", &[]).unwrap();
    let atom = |name: &str| Term::Atom(result.atom(name).unwrap());
    let binary = |bytes: &[u8]| Term::binary(bytes.to_vec());
    let list = |codes: &[i64]| {
        Term::list_from(codes.iter().copied().map(Term::Integer).collect::<Vec<_>>())
    };
    let repeated = |code: i64, count: usize| Term::list_from(vec![Term::Integer(code); count]);
    let ok = |value: Term| Term::tuple(vec![atom("ok"), value]);
    let error_info = Map::from_pairs(
        vec![(atom("module"), atom("erl_erts_errors"))],
        program.atoms(),
    )
    .unwrap();
    let caller = |function: &str, arity: i64| {
        Term::tuple(vec![
            atom("existing_atom_probe"),
            atom(function),
            Term::Integer(arity),
            Term::nil(),
        ])
    };
    let failed = |name: &str, arguments: Vec<Term>, helper: &str, arity: i64| {
        let frame = Term::tuple(vec![
            atom("erlang"),
            atom(name),
            Term::list_from(arguments),
            Term::list_from(vec![Term::tuple(vec![
                atom("error_info"),
                Term::Map(error_info.clone()),
            ])]),
        ]);
        Term::tuple(vec![
            atom("error"),
            atom("badarg"),
            Term::list_from(vec![frame, caller(helper, arity), caller("main", 0)]),
        ])
    };
    let failed_list = |list: Term| failed("list_to_existing_atom", vec![list], "list_result", 1);
    let failed_binary = |binary: Term, encoding: Term| {
        failed(
            "binary_to_existing_atom",
            vec![binary, encoding],
            "binary_result",
            2,
        )
    };
    let long = "a".repeat(255);
    let mut expected = vec![
        ok(binary(b"ok")),
        failed_list(list(&[111, 107, 111, 107])),
        ok(binary(b"")),
        ok(binary(&[195, 191])),
        ok(binary(long.as_bytes())),
        failed_list(atom("bad")),
        failed_list(Term::list_with_tail(
            vec![Term::Integer(111), Term::Integer(107)],
            atom("bad"),
        )),
        failed_list(list(&[111, 0x110000])),
        failed_list(list(&[111, 0xD800])),
        failed_list(Term::list_from(vec![Term::Integer(111), Term::Float(1.0)])),
        failed_list(list(&[111, -1])),
        failed_list(repeated(97, 256)),
        ok(binary(b"ok")),
        ok(binary(b"ok")),
        ok(binary(b"ok")),
        ok(binary(b"")),
        ok(binary(b"")),
        ok(binary(&[195, 191])),
        ok(binary(&[195, 191])),
        failed_binary(binary(&[255]), atom("utf8")),
        ok(binary(long.as_bytes())),
    ];
    expected.extend([
        failed_binary(binary(&[97; 256]), atom("latin1")),
        failed_binary(binary(&[97; 256]), atom("utf8")),
        failed_binary(binary(&[255; 256]), atom("latin1")),
        failed_binary(binary(&[97; 1021]), atom("utf8")),
        failed_binary(binary(b"ok"), atom("foo")),
        failed_binary(binary(b"ok"), binary(b"utf8")),
        failed_binary(atom("ok"), atom("utf8")),
        failed_binary(list(&[111, 107]), atom("utf8")),
    ]);
    assert_eq!(result.term, Term::list_from(expected));
    // `existing_atom_probe:bits()` calls the BIF with a bit string of one
    // bit, which ERTS rejects, because it is not a whole number of bytes
    // (`erl_unicode.c:1969-1971`). Only the reference run of this test reads
    // that case: PBeam has no bitstring term of a size that is not a
    // multiple of 8 bits (row "Bitstring terms" of
    // `docs/erts-differences.md`), so the program of this test cannot build
    // the argument. `binary_to_existing_atom` tests the size of its argument
    // (`bif/erlang.rs`), and the case `<<1:1>>` waits for a bitstring term,
    // as task #761 records.

    for (function, expected) in [("main", EXPECTED), ("bits", BITS_EXPECTED)] {
        let reference = Command::new("erl")
            .args([
                "-noshell",
                "-pa",
                "fixtures/existing_atom",
                "-eval",
                &format!("io:format(\"~w~n\", [existing_atom_probe:{function}()]), halt()."),
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

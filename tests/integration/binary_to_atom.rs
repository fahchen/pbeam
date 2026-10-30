//! Runs `binary_to_atom` on PBeam and compares the results with the pinned ERTS.

use std::process::Command;

use pbeam_runtime::{BeamProgram, Map, Term};

const BINARY_TO_ATOM_PROBE: &[u8] =
    include_bytes!("../../fixtures/binary_to_atom/binary_to_atom_probe.beam");

/// The `~w` output of `binary_to_atom_probe:main()` on the pinned ERTS. The
/// fixture cuts the stack trace after the frame of `main/0`, because the
/// `erl -eval` caller adds its own frames and a PBeam run has none.
const EXPECTED: &str = include_str!("../../fixtures/binary_to_atom/binary_to_atom_probe.expected");

#[test]
fn converts_a_binary_to_an_atom_as_pinned_erts() {
    // `erlang:binary_to_atom/2` follows `binary_to_atom`
    // (erl_unicode.c:1962-2033) and `erts_atom_put_index` (atom.c:230-334):
    // `latin1` reads one byte for each character and gives the atom of the
    // code points, `utf8` and `unicode` read the UTF-8 bytes of the name,
    // and a binary that the encoding does not accept, an encoding that is
    // not `latin1`, `utf8`, or `unicode`, and an argument that is not a
    // byte-aligned binary raise `badarg`. More than 255 bytes with `latin1`,
    // a name of more than 255 characters, and a binary above
    // `MAX_ATOM_SZ_LIMIT` bytes with `utf8` or `unicode` raise
    // `system_limit`; ERTS checks the byte limit before it verifies the
    // encoding and stops the UTF-8 analysis at the 255th character
    // (erl_unicode.c:1260-1263, atom.c:281-285). Each exception carries the
    // frame of the BIF with its arguments and the `error_info` map
    // (`beam_common.c:939-991`).
    let program = BeamProgram::load(&[BINARY_TO_ATOM_PROBE]).unwrap();
    let atom = |name: &str| Term::Atom(program.atom(name).unwrap());
    let binary = |bytes: &[u8]| Term::binary(bytes.to_vec());
    let ok = |name: &[u8]| Term::tuple(vec![atom("ok"), binary(name)]);
    let error_info = Map::from_pairs(
        vec![(atom("module"), atom("erl_erts_errors"))],
        program.atoms(),
    )
    .unwrap();
    let failed = |arguments: Vec<Term>, reason: &str| {
        let frame = Term::tuple(vec![
            atom("erlang"),
            atom("binary_to_atom"),
            Term::list_from(arguments),
            Term::list_from(vec![Term::tuple(vec![
                atom("error_info"),
                Term::Map(error_info.clone()),
            ])]),
        ]);
        let caller = |function: &str, arity: i64| {
            Term::tuple(vec![
                atom("binary_to_atom_probe"),
                atom(function),
                Term::Integer(arity),
                Term::nil(),
            ])
        };
        Term::tuple(vec![
            atom("error"),
            atom(reason),
            Term::list_from(vec![frame, caller("result", 2), caller("main", 0)]),
        ])
    };
    let utf8 = atom("utf8");
    let latin1 = || atom("latin1");
    let mut expected = vec![
        ok(&[111, 107]),
        ok(&[111, 107]),
        ok(&[111, 107]),
        ok(&[]),
        ok(&[0]),
        ok(&[195, 191]),
        ok(&[195, 133, 194, 128]),
        ok(&[207, 128]),
        ok(&[240, 159, 152, 128]),
        Term::tuple(vec![atom("ok"), Term::Integer(510)]),
        Term::tuple(vec![atom("ok"), Term::Integer(255)]),
    ];
    expected.extend([
        failed(vec![binary(&[255]), utf8.clone()], "badarg"),
        failed(vec![binary(&[237, 160, 128]), utf8.clone()], "badarg"),
        failed(vec![binary(&[192, 128]), utf8.clone()], "badarg"),
        failed(vec![binary(&[226, 130]), utf8.clone()], "badarg"),
        failed(vec![binary(&vec![255; 256]), latin1()], "system_limit"),
        failed(vec![binary(&vec![97; 256]), utf8.clone()], "system_limit"),
        failed(vec![binary(&vec![255; 1021]), utf8], "system_limit"),
        failed(vec![binary(&[255]), atom("foo")], "badarg"),
        failed(vec![binary(&[255]), binary(b"utf8")], "badarg"),
        failed(vec![atom("ok"), latin1()], "badarg"),
        failed(
            vec![Term::list_from(vec![Term::Integer(255)]), latin1()],
            "badarg",
        ),
        failed(vec![Term::Integer(255), latin1()], "badarg"),
    ]);
    assert_eq!(
        program
            .execute("binary_to_atom_probe", "main", &[])
            .unwrap()
            .term,
        Term::list_from(expected)
    );

    let reference = Command::new("erl")
        .args([
            "-noshell",
            "-pa",
            "fixtures/binary_to_atom",
            "-eval",
            "io:format(\"~w~n\", [binary_to_atom_probe:main()]), halt().",
        ])
        .output()
        .unwrap();
    assert!(
        reference.status.success() && reference.stderr.is_empty(),
        "{reference:?}"
    );
    assert_eq!(String::from_utf8(reference.stdout).unwrap(), EXPECTED);
}

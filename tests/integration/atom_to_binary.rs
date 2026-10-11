use std::process::Command;

use pbeam_runtime::{BeamProgram, Term};

const ATOM_TO_BINARY_PROBE: &[u8] =
    include_bytes!("../../fixtures/atom_to_binary/atom_to_binary_probe.beam");

/// The `~w` text of `atom_to_binary_probe:main()` on the pinned ERTS.
const ERTS_RESULT: &[u8] = b"[[<<111,107>>,<<111,107>>,<<111,107>>,{error,badarg}],\
[<<>>,<<>>,<<>>,{error,badarg}],\
[<<104,233,108,108,111>>,<<104,195,169,108,108,111>>,<<104,195,169,108,108,111>>,{error,badarg}],\
[{error,badarg},<<207,128>>,<<207,128>>,{error,badarg}],\
[{error,badarg},<<195,191,196,128>>,<<195,191,196,128>>,{error,badarg}],\
{error,badarg},{error,badarg},{error,badarg}]\n";

#[test]
fn converts_atoms_to_binaries_as_pinned_erts() {
    // `erlang:atom_to_binary/2` follows `atom_to_binary_2`
    // (erl_unicode.c:1928-1960): `utf8` and `unicode` give the UTF-8 bytes
    // of the name, and `latin1` gives one byte for each character. A
    // character above 255 with `latin1`, an encoding that is not valid,
    // and a term that is not an atom raise `badarg`. PBeam gives the stack
    // trace `[]`, not the `error_info` frame of ERTS; this is the
    // "Exception stack trace" row in `docs/erts-differences.md`.
    let program = BeamProgram::load(&[ATOM_TO_BINARY_PROBE]).unwrap();
    let error = Term::Atom(program.atom("error").unwrap());
    let badarg = Term::Atom(program.atom("badarg").unwrap());
    let raised = Term::Tuple(vec![error, badarg]);
    let binary = |bytes: &[u8]| Term::Binary(bytes.to_vec());
    let ok = binary(b"ok");
    let utf8 = binary("h\u{e9}llo".as_bytes());
    let pi = binary(&[207, 128]);
    let two = binary(&[195, 191, 196, 128]);
    let expected = Term::list_from(vec![
        Term::list_from(vec![ok.clone(), ok.clone(), ok, raised.clone()]),
        Term::list_from(vec![binary(b""), binary(b""), binary(b""), raised.clone()]),
        Term::list_from(vec![
            binary(&[104, 233, 108, 108, 111]),
            utf8.clone(),
            utf8,
            raised.clone(),
        ]),
        Term::list_from(vec![raised.clone(), pi.clone(), pi, raised.clone()]),
        Term::list_from(vec![raised.clone(), two.clone(), two, raised.clone()]),
        raised.clone(),
        raised.clone(),
        raised,
    ]);
    assert_eq!(
        program
            .execute("atom_to_binary_probe", "main", &[])
            .unwrap(),
        expected
    );

    let reference = Command::new("erl")
        .args([
            "-noshell",
            "-pa",
            "fixtures/atom_to_binary",
            "-eval",
            "io:format(\"~w~n\", [atom_to_binary_probe:main()]), halt().",
        ])
        .output()
        .unwrap();
    assert!(
        reference.status.success() && reference.stderr.is_empty(),
        "{reference:?}"
    );
    assert_eq!(
        String::from_utf8_lossy(&reference.stdout),
        String::from_utf8_lossy(ERTS_RESULT)
    );
}

use std::process::Command;

use pbeam_runtime::{BeamProgram, Term};

const INSPECT_BIF_PROBE: &[u8] =
    include_bytes!("../../fixtures/inspect_bif/inspect_bif_probe.beam");

/// The binaries of the `integer_to_binary/2` calls of
/// `inspect_bif_probe:main/0`, in the order of the calls.
const DIGITS: [&str; 16] = [
    "0",
    "1",
    "-15",
    "FF",
    "-FF",
    "Z",
    "10",
    "101",
    "4DO2A5OXWE0V",
    "-40000000000000000000",
    "100000000000000000000000000000000000000000000000000000000000",
    "-800000000000000",
    "111111111111111111111111111111111111111111111111111111111111111",
    "1Y2P0IJ32E8E7",
    "-8000000000000000",
    "-9223372036854775808",
];

#[test]
fn runs_the_bifs_of_io_inspect_of_a_tuple_as_pinned_erts() {
    // `inspect_bif_probe:main/0` calls the BIFs that
    // `IO.inspect({:ok, 1})` needs. `erlang:tuple_to_list/1`
    // (`tuple_to_list_1`, `bif.c:3762-3783`) gives the elements of a tuple
    // and raises `badarg` for each other term. `erlang:integer_to_binary/2`
    // (`integer_to_binary_2`, `binary.c:147-182`) writes small integers and
    // bignums in bases 2 to 36 with upper-case letters (`Sint_to_buf`,
    // `utils.c:2994-3030`), and raises `badarg` for a term that is not an
    // integer and for a base that is not a small integer from 2 to 36.
    // PBeam gives the stack trace `[]`, so the probe catches only the class
    // and the reason.
    let program = BeamProgram::load(&[INSPECT_BIF_PROBE]).unwrap();
    let atom = |name: &str| Term::Atom(program.atom(name).unwrap());
    let badarg = Term::tuple(vec![atom("error"), atom("badarg")]);
    let expected = Term::list_from(vec![
        Term::list_from(vec![
            Term::nil(),
            Term::list_from(vec![atom("ok"), Term::Integer(1)]),
            Term::list_from(vec![
                atom("a"),
                Term::tuple(vec![atom("b")]),
                Term::list_from(vec![atom("c")]),
                Term::Binary(b"d".to_vec()),
            ]),
        ]),
        Term::list_from(vec![badarg.clone(); 5]),
        Term::list_from(DIGITS.map(|digits| Term::Binary(digits.as_bytes().to_vec()))),
        Term::list_from(vec![badarg; 9]),
    ]);
    assert_eq!(
        program.execute("inspect_bif_probe", "main", &[]).unwrap(),
        expected
    );

    let reference = Command::new("erl")
        .args([
            "-noshell",
            "-pa",
            "fixtures/inspect_bif",
            "-eval",
            "io:format(\"~w~n\", [inspect_bif_probe:main()]), halt().",
        ])
        .output()
        .unwrap();
    assert!(
        reference.status.success() && reference.stderr.is_empty(),
        "{reference:?}"
    );
    assert_eq!(
        String::from_utf8(reference.stdout).unwrap(),
        format!("{}\n", text(&expected))
    );
}

/// Return the `~w` text of a term of the probe result: an atom, an
/// integer, a binary, a list, or a tuple. `~w` prints each byte of a
/// binary as an integer.
fn text(term: &Term) -> String {
    let join = |terms: &[Term]| terms.iter().map(text).collect::<Vec<_>>().join(",");
    match term {
        Term::Atom(atom) => atom.name().to_owned(),
        Term::Integer(value) => value.to_string(),
        Term::Binary(bytes) => {
            let bytes: Vec<String> = bytes.iter().map(u8::to_string).collect();
            format!("<<{}>>", bytes.join(","))
        }
        _ if let Some(elements) = term.proper_list() => {
            format!("[{}]", join(&elements.cloned().collect::<Vec<_>>()))
        }
        Term::Tuple(elements) => format!("{{{}}}", join(elements)),
        other => panic!("the probe gives no such term: {other:?}"),
    }
}

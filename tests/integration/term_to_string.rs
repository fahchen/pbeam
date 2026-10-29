//! Runs term printing with a limit on PBeam and compares the text with the pinned ERTS.

use std::process::Command;

use pbeam_runtime::{BeamProgram, Term};

const TERM_TO_STRING_PROBE: &[u8] =
    include_bytes!("../../fixtures/term_to_string/term_to_string_probe.beam");

/// The text of each case of `term_to_string_probe:cases/0`, or `None` for
/// `badarg`, as the pinned ERTS gives them.
const EXPECTED: [Option<&str>; 18] = [
    Some("ab... "),
    Some("{a,\"x... "),
    Some("{a,\"xyz\",<<1... "),
    Some("[1,2,... "),
    Some("[a|b]"),
    Some("<<\"ab... "),
    Some("<<1,2,3:4>>... "),
    Some("<<1,2,3:4>>"),
    Some("fun lists:map/2... "),
    Some("fun lists:map/2"),
    Some("\"ab... "),
    Some("#{a... "),
    Some("#{1=>a... "),
    Some("#{1=>a,2=>bb... "),
    None,
    None,
    None,
    Some("x"),
];

#[test]
fn prints_the_term_with_the_limit_as_pinned_erts() {
    // `erts_internal:term_to_string/2` prints with `%.*T` and the limit as
    // the precision (bif.c:4156-4190, erl_printf_term.c:352-824). `init`
    // uses it for its halt text (init.erl:665-679).
    let program = BeamProgram::load(&[TERM_TO_STRING_PROBE]).unwrap();
    let result = program
        .execute("term_to_string_probe", "cases", &[])
        .unwrap();
    let texts: Vec<Option<String>> = result
        .term
        .proper_list()
        .unwrap()
        .map(|case| {
            case.proper_list().map(|bytes| {
                let bytes: Vec<u8> = bytes
                    .map(|byte| match byte {
                        Term::Integer(byte) => u8::try_from(*byte).unwrap(),
                        other => panic!("{other:?}"),
                    })
                    .collect();
                String::from_utf8(bytes).unwrap()
            })
        })
        .collect();
    let expected: Vec<Option<String>> = EXPECTED
        .iter()
        .map(|text| text.map(str::to_owned))
        .collect();
    assert_eq!(texts, expected);
    // The same cases on the pinned ERTS, written with `~p` as strings.
    let output = Command::new("erl")
        .args([
            "-noshell",
            "-pa",
            "fixtures/term_to_string",
            "-eval",
            "io:format(\"~0p~n\", [term_to_string_probe:cases()]), halt().",
        ])
        .output()
        .unwrap();
    let erts_expected = format!(
        "[{}]\n",
        EXPECTED
            .iter()
            .map(|text| match text {
                Some(text) => format!("{text:?}"),
                None => "badarg".to_owned(),
            })
            .collect::<Vec<_>>()
            .join(",")
    );
    assert_eq!(String::from_utf8_lossy(&output.stdout), erts_expected);
}

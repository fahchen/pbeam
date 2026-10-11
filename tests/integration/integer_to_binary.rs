use std::process::Command;

use pbeam_runtime::{BeamProgram, Term};

const INTEGER_TO_BINARY_PROBE: &[u8] =
    include_bytes!("../../fixtures/integer_to_binary/integer_to_binary_probe.beam");

/// The decimal strings of the integers of `integer_to_binary_probe:main/0`.
const DIGITS: [&str; 10] = [
    "0",
    "7",
    "-15",
    "1000000",
    "576460752303423487",
    "-576460752303423488",
    "576460752303423488",
    "-576460752303423489",
    "9223372036854775807",
    "-9223372036854775808",
];

#[test]
fn converts_integers_to_binaries_as_pinned_erts() {
    // `integer_to_binary_probe:main/0` calls `erlang:integer_to_binary/1`
    // with `call_ext`. It converts small integers and bignums, and catches
    // the `badarg` of five terms that are not integers
    // (`integer_to_binary_1`, `binary.c:119-145`). PBeam gives the stack
    // trace `[]`, so the probe catches only the class and the reason.
    let program = BeamProgram::load(&[INTEGER_TO_BINARY_PROBE]).unwrap();
    let error = Term::Tuple(vec![
        Term::Atom(program.atom("error").unwrap()),
        Term::Atom(program.atom("badarg").unwrap()),
    ]);
    let mut expected: Vec<Term> = DIGITS
        .map(|digits| Term::Binary(digits.as_bytes().to_vec()))
        .into();
    expected.extend(std::iter::repeat_n(error, 5));
    assert_eq!(
        program
            .execute("integer_to_binary_probe", "main", &[])
            .unwrap(),
        Term::list_from(expected)
    );

    let reference = Command::new("erl")
        .args([
            "-noshell",
            "-pa",
            "fixtures/integer_to_binary",
            "-eval",
            "io:format(\"~w~n\", [integer_to_binary_probe:main()]), halt().",
        ])
        .output()
        .unwrap();
    assert!(
        reference.status.success() && reference.stderr.is_empty(),
        "{reference:?}"
    );
    // `~w` prints each byte of a binary as an integer.
    let binaries: Vec<String> = DIGITS
        .iter()
        .map(|digits| {
            let bytes: Vec<String> = digits.bytes().map(|byte| byte.to_string()).collect();
            format!("<<{}>>", bytes.join(","))
        })
        .collect();
    assert_eq!(
        String::from_utf8(reference.stdout).unwrap(),
        format!(
            "[{},{}]\n",
            binaries.join(","),
            ["{error,badarg}"; 5].join(",")
        )
    );
}

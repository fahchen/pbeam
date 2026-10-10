use std::process::Command;

use pbeam_runtime::{BeamProgram, Term};

const IOLIST_SPLIT_PROBE: &[u8] =
    include_bytes!("../../fixtures/iolist_split/iolist_split_probe.beam");

/// `iolist_split_probe:main/0` on the pinned ERTS, as `io:format("~w")`
/// prints it: the results of `erlang:iolist_to_binary/1` for each input
/// shape and error, and the checks of the long inputs whose walk traps.
const EXPECTED: &str = "[{iolist_to_binary,[<<1,2,3>>,<<>>,<<>>,<<>>,<<97,98,99>>,<<1,2,3>>,<<1,2,3,4,5>>,<<1,\
    2>>,<<>>,<<97,98,99,100,101,102,103>>,<<0,255>>,{error,badarg},{error,badarg},{error,\
    badarg},{error,badarg},{error,badarg},{error,badarg},{error,badarg},{error,badarg},\
    {error,badarg}]},{iolist_to_binary_long,[true,true,true,true]}]";

/// Return a term as `io:format("~w")` prints it, for the atoms, integers,
/// binaries, lists, and tuples of the probe result.
///
/// # Examples
///
/// ```rust,ignore
/// assert_eq!(text(&Term::Binary(vec![1, 2])), "<<1,2>>");
/// ```
fn text(term: &Term) -> String {
    let join = |terms: &[Term]| terms.iter().map(text).collect::<Vec<_>>().join(",");
    match term {
        Term::Atom(atom) => atom.name().to_owned(),
        Term::Integer(value) => value.to_string(),
        Term::Binary(bytes) => {
            let bytes: Vec<String> = bytes.iter().map(u8::to_string).collect();
            format!("<<{}>>", bytes.join(","))
        }
        Term::List(terms) => format!("[{}]", join(terms)),
        Term::Tuple(terms) => format!("{{{}}}", join(terms)),
        other => panic!("unexpected term {other:?}"),
    }
}

#[test]
fn converts_iodata_as_pinned_erts() {
    // Each call is a body `call_ext`. The probe catches only the class and
    // the reason of an error, because PBeam gives a BIF error the stack
    // trace `[]` (`erl_iolist.c:857-928`).
    let program = BeamProgram::load(&[IOLIST_SPLIT_PROBE]).unwrap();
    let result = program.execute("iolist_split_probe", "main", &[]).unwrap();
    assert_eq!(text(&result), EXPECTED);

    let reference = Command::new("erl")
        .args([
            "-noshell",
            "-pa",
            "fixtures/iolist_split",
            "-eval",
            "io:format(\"~w~n\", [iolist_split_probe:main()]), halt().",
        ])
        .output()
        .unwrap();
    assert!(
        reference.status.success() && reference.stderr.is_empty(),
        "{reference:?}"
    );
    assert_eq!(
        String::from_utf8(reference.stdout).unwrap(),
        format!("{EXPECTED}\n")
    );
}

use std::process::Command;

use pbeam_runtime::{BeamProgram, Term};

const IOLIST_SPLIT_PROBE: &[u8] =
    include_bytes!("../../fixtures/iolist_split/iolist_split_probe.beam");

/// `iolist_split_probe:main/0` on the pinned ERTS, as `io:format("~w")`
/// prints it: the results of `erlang:iolist_to_binary/1` and
/// `binary:split/2,3` for each input shape, option, and error, and the
/// checks of the long inputs whose walk, search, or result build traps.
const EXPECTED: &str = "[{iolist_to_binary,[<<1,2,3>>,<<>>,<<>>,<<>>,<<97,98,99>>,<<1,2,3>>,<<1,2,3,4,5>>,<<1,\
    2>>,<<>>,<<97,98,99,100,101,102,103>>,<<0,255>>,{error,badarg},{error,badarg},{error,\
    badarg},{error,badarg},{error,badarg},{error,badarg},{error,badarg},{error,badarg},\
    {error,badarg}]},{iolist_to_binary_long,[true,true,true,true]},{split2,[[<<97>>,<<98,44,\
    99>>],[<<97,98,99>>],[<<>>],[<<>>,<<>>],[<<97,98>>,<<>>],[<<>>,<<100,97,98,99,97,98>>],\
    [<<98,98>>,<<>>],[<<120,120>>,<<120,121,97,98>>],[<<98>>,<<97,110,97>>],[<<>>,<<>>],\
    [<<97>>,<<>>],{error,badarg},{error,badarg},{error,badarg},{error,badarg},{error,badarg},\
    {error,badarg},{error,badarg},{error,badarg},{error,badarg},{error,badarg}]},{split3,\
    [[<<>>,<<97,44,98,44,44,99,44,44>>],[<<>>,<<97>>,<<98>>,<<>>,<<99>>,<<>>,<<>>],[<<>>,\
    <<97,44,98,44,44,99,44,44>>],[<<>>,<<97>>,<<98>>,<<>>,<<99>>],[<<97,44,98,44,44,99,44,\
    44>>],[<<97>>,<<98>>,<<99>>],[<<97>>,<<98>>,<<99>>],[<<97>>,<<98>>],[<<97,98>>],[],[<<97,\
    98>>],[],[],[],[<<>>],[<<98,97,110>>,<<110,97>>],[<<98,97,110>>,<<110,97>>],[<<97>>,\
    <<99>>],[<<97,98,99>>],[<<97,98,99>>],[<<>>,<<100>>,<<>>],[<<>>,<<>>,<<>>],[<<98,98>>,\
    <<98>>,<<>>],[<<120,121,122>>],[<<120,120>>,<<>>,<<>>,<<>>,<<>>],[<<98>>,<<>>,<<>>,<<>>],\
    [],[<<97>>,<<98>>,<<>>],{error,badarg},{error,badarg},{error,badarg},{error,badarg},\
    {error,badarg},{error,badarg},{error,badarg},{error,badarg},{error,badarg},{error,\
    badarg},{error,badarg},{error,badarg},{error,badarg}]},{split_long,[true,true,true,true,\
    true,true,true,true]}]";

/// Return a term as `io:format("~w")` prints it, for the atoms, integers,
/// binaries, lists, and tuples of the probe result.
///
/// # Examples
///
/// ```rust,ignore
/// assert_eq!(text(&Term::binary(vec![1, 2])), "<<1,2>>");
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
        _ if let Some(terms) = term.proper_list() => {
            format!("[{}]", join(&terms.cloned().collect::<Vec<_>>()))
        }
        Term::Tuple(terms) => format!("{{{}}}", join(terms)),
        other => panic!("unexpected term {other:?}"),
    }
}

#[test]
fn converts_iodata_and_splits_binaries_as_pinned_erts() {
    // Each call is a body `call_ext`. The probe catches only the class and
    // the reason of an error, because PBeam gives a BIF error the stack
    // trace `[]` (`erl_iolist.c:857-928`, `erl_bif_binary.c:1346-1645`).
    let program = BeamProgram::load(&[IOLIST_SPLIT_PROBE]).unwrap();
    let result = program.execute("iolist_split_probe", "main", &[]).unwrap();
    assert_eq!(text(result.term()), EXPECTED);

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

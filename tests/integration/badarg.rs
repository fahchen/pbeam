use std::process::Command;

use pbeam_runtime::{BeamProgram, Term, debug_term};

const BADARG_PROBE: &[u8] = include_bytes!("../../fixtures/badarg/badarg_probe.beam");

#[test]
fn raises_badarg_that_catch_and_try_catch_as_pinned_erts() {
    // Each `catch` of the probe gets `{'EXIT', {badarg, Stack}}` with a list
    // as `Stack`. The probe runs `element/2` through a `bif` instruction
    // with fail label 0, and the other BIFs through `call_ext`.
    // `process_info/2` raises badarg for a first argument that is not a pid
    // and for an item that is not valid. PBeam gives the stack trace `[]`;
    // this is the "Exception stack trace" row in `docs/erts-differences.md`.
    let program = BeamProgram::load(&[BADARG_PROBE]).unwrap();
    let badarg = Term::Atom(program.atom("badarg").unwrap());
    let error = Term::Atom(program.atom("error").unwrap());
    let caught = Term::tuple(vec![error, badarg.clone()]);
    let mut expected = vec![badarg; 11];
    expected.extend([caught.clone(), caught]);
    assert_eq!(
        program.execute("badarg_probe", "main", &[]).unwrap().term,
        Term::tuple(expected)
    );

    let reference = Command::new("erl")
        .args([
            "-noshell",
            "-pa",
            "fixtures/badarg",
            "-eval",
            "io:format(\"~w~n\", [badarg_probe:main()]), halt().",
        ])
        .output()
        .unwrap();
    assert!(reference.status.success());
    assert!(reference.stderr.is_empty());
    assert_eq!(
        reference.stdout,
        b"{badarg,badarg,badarg,badarg,badarg,badarg,badarg,badarg,badarg,badarg,badarg,{error,badarg},{error,badarg}}\n"
    );
}

const NODE_PROBE: &[u8] = include_bytes!("../../fixtures/badarg/node_probe.beam");

#[test]
fn raises_badarg_for_the_node_of_a_term_that_is_not_a_node_container_as_pinned_erts() {
    // ERTS `node_1` raises `badarg` for a term that is not a pid, a port, or
    // a reference (`dist.c:6219-6225`); in a guard, the guard fails.
    let program = BeamProgram::load(&[NODE_PROBE]).unwrap();
    let result = program.execute("node_probe", "main", &[]).unwrap();
    assert_eq!(
        debug_term(&result.term, program.atoms()),
        "List([Tuple([Atom(\"badarg\"), Tuple([Atom(\"erlang\"), Atom(\"node\"), List([Atom(\"foo\")]), List([Tuple([Atom(\"error_info\"), Map([(Atom(\"module\"), Atom(\"erl_erts_errors\"))])])])])]), Atom(\"nonode@nohost\"), Atom(\"nonode@nohost\"), Atom(\"other\"), Atom(\"local\")])"
    );
    let reference = Command::new("erl")
        .args([
            "-noshell",
            "-pa",
            "fixtures/badarg",
            "-eval",
            "io:format(\"~w~n\", [node_probe:main()]), halt().",
        ])
        .output()
        .unwrap();
    assert_eq!(
        reference.stdout,
        b"[{badarg,{erlang,node,[foo],[{error_info,#{module => erl_erts_errors}}]}},nonode@nohost,nonode@nohost,other,local]\n"
    );
}

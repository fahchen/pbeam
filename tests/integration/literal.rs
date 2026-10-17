use std::process::Command;

use pbeam_runtime::{BeamProgram, Term};

const LITERAL_PROBE: &[u8] = include_bytes!("../../fixtures/literal/literal_probe.beam");

#[test]
fn reads_a_large_tuple_literal_as_pinned_erts() {
    // The pinned compiler writes a tuple of more than 255 elements as
    // `LARGE_TUPLE_EXT` (tag 105) in the literal table, and ERTS `dec_term`
    // reads its four-byte element count (`external.c:4542-4549`).
    let program = BeamProgram::load(&[LITERAL_PROBE]).unwrap();
    let result = program
        .execute("literal_probe", "large_facts", &[])
        .unwrap();
    assert_eq!(
        result.term,
        Term::tuple(vec![
            Term::Integer(300),
            Term::Integer(1),
            Term::Integer(300),
        ])
    );
    let run = program.execute("literal_probe", "large", &[]).unwrap();
    let Term::Tuple(elements) = &run.term else {
        panic!("large/0 gives a tuple");
    };
    assert_eq!(elements.len(), 300);
    assert_eq!(elements[0], Term::Integer(1));
    assert_eq!(elements[299], Term::Integer(300));

    let reference = Command::new("erl")
        .args([
            "-noshell",
            "-pa",
            "fixtures/literal",
            "-eval",
            "io:format(\"~p~n\", [literal_probe:large_facts()]), halt().",
        ])
        .output()
        .unwrap();
    assert!(reference.status.success());
    assert!(reference.stderr.is_empty());
    assert_eq!(reference.stdout, b"{300,1,300}\n");
}

#[test]
fn decodes_tuple_and_list_literals_and_builds_lists_on_the_empty_list() {
    let program = BeamProgram::load(&[LITERAL_PROBE]).unwrap();
    let atom = |name: &str| Term::Atom(program.atom(name).unwrap());
    assert_eq!(
        program.execute("literal_probe", "main", &[]).unwrap().term,
        Term::tuple(vec![
            Term::list_from(vec![atom("flush")]),
            Term::tuple(vec![atom("error"), atom("enotsup")]),
            atom("same"),
        ])
    );

    let reference = Command::new("erl")
        .args([
            "-noshell",
            "-pa",
            "fixtures/literal",
            "-eval",
            "io:format(\"~p~n\", [literal_probe:main()]), halt().",
        ])
        .output()
        .unwrap();
    assert!(reference.status.success());
    assert!(reference.stderr.is_empty());
    assert_eq!(reference.stdout, b"{[flush],{error,enotsup},same}\n");
}

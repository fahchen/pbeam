use std::process::Command;

use pbeam_runtime::{BeamProgram, Term};

const LITERAL_PROBE: &[u8] = include_bytes!("../../fixtures/literal/literal_probe.beam");

#[test]
fn decodes_tuple_and_list_literals_and_builds_lists_on_the_empty_list() {
    let program = BeamProgram::load(&[LITERAL_PROBE]).unwrap();
    let atom = |name: &str| Term::Atom(program.atom(name).unwrap());
    assert_eq!(
        program.execute("literal_probe", "main", &[]).unwrap(),
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

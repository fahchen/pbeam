use std::process::Command;

use pbeam_runtime::{BeamProgram, Term};

const PERSISTENT_TERM_PROBE: &[u8] =
    include_bytes!("../fixtures/persistent_term/persistent_term_probe.beam");

#[test]
fn keeps_persistent_terms_for_all_processes_as_pinned_erts() {
    // `main/0` reads a missing key, puts a value, puts the same value again,
    // and lets a spawned process replace the value. The keys start with the
    // probe module name, so the persistent terms of the ERTS logger do not
    // change the reference result.
    let reference = Command::new("erl")
        .args([
            "-noshell",
            "-pa",
            "fixtures/persistent_term",
            "-eval",
            "io:format(\"~p~n\", [persistent_term_probe:main()]), halt().",
        ])
        .output()
        .unwrap();
    assert!(reference.status.success());
    assert!(reference.stderr.is_empty());
    assert_eq!(
        reference.stdout,
        b"{default,ok,1,ok,ok,{value,[a,<<\"b\">>]},undefined}\n"
    );

    let program = BeamProgram::load(&[PERSISTENT_TERM_PROBE]).unwrap();
    let atom = |name: &str| Term::Atom(program.atom(name).unwrap());
    assert_eq!(
        program
            .execute("persistent_term_probe", "main", &[])
            .unwrap(),
        Term::Tuple(vec![
            atom("default"),
            atom("ok"),
            Term::Integer(1),
            atom("ok"),
            atom("ok"),
            Term::Tuple(vec![
                atom("value"),
                Term::List(vec![atom("a"), Term::Binary(b"b".to_vec())]),
            ]),
            atom("undefined"),
        ])
    );
}

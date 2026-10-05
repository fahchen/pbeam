use std::process::Command;

use pbeam_runtime::{BeamProgram, Term};

const REGISTRY_PROBE: &[u8] = include_bytes!("../fixtures/registry/registry_probe.beam");

#[test]
fn looks_up_registered_names_as_pinned_erts() {
    // `registry_probe:names/0` registers the root process and finds it with
    // `whereis/1`. A name that no process has and the atom `undefined` give
    // `undefined`. A term that is not an atom raises `error:badarg`, as
    // ERTS `whereis_1` (`bif.c:2342-2351`) does. A name that an exit freed
    // gives `undefined`.
    let program = BeamProgram::load(&[REGISTRY_PROBE]).unwrap();
    let atom = |name| Term::Atom(program.atom(name).unwrap());
    assert_eq!(
        program.execute("registry_probe", "names", &[]).unwrap(),
        Term::List(vec![
            atom("named"),
            atom("undefined"),
            atom("undefined"),
            atom("badarg"),
            atom("badarg"),
            atom("undefined"),
        ])
    );

    let reference = Command::new("erl")
        .args([
            "-noshell",
            "-pa",
            "fixtures/registry",
            "-eval",
            "io:format(\"~w~n\", [registry_probe:names()]), halt().",
        ])
        .output()
        .unwrap();
    assert!(reference.status.success());
    assert!(reference.stderr.is_empty());
    assert_eq!(
        reference.stdout,
        b"[named,undefined,undefined,badarg,badarg,undefined]\n"
    );
}

//! The host-term boundary of `BeamProgram::execute`: an argument carries the
//! atom table that owns its atoms, and the run imports each atom of the
//! argument into the atom table of the run.
//!
//! The test loads two programs that share the module name `host_term_probe`
//! and hold different code. A local fun holds the entry label of its function,
//! so a local fun of the first program must not enter a run of the second
//! program. An external fun holds no label and resolves through an export at
//! call time, so it imports across programs.

use pbeam_runtime::{BeamProgram, Term};

const FIRST: &[u8] = include_bytes!("../../fixtures/host_term_first/host_term_probe.beam");
const SECOND: &[u8] = include_bytes!("../../fixtures/host_term_second/host_term_probe.beam");

#[test]
fn rejects_a_local_fun_of_another_program() {
    let first = BeamProgram::load(&[FIRST]).unwrap();
    let second = BeamProgram::load(&[SECOND]).unwrap();

    // The two programs share one module name and hold different code.
    let name = |program: &BeamProgram, function: &str| {
        let result = program.execute("host_term_probe", function, &[]).unwrap();
        match &result {
            Term::Atom(atom) => atom.name().to_owned(),
            other => panic!("expected an atom, got {other:?}"),
        }
    };
    assert_eq!(name(&first, "value"), "first");
    assert_eq!(name(&second, "value"), "second");

    // A local fun holds the entry label of `value/0` in the first program.
    // The second program can hold different code under the same name, so the
    // import rejects the local fun before the run starts.
    let local = first.execute("host_term_probe", "local_fun", &[]).unwrap();
    let error = second
        .execute("host_term_probe", "accept", &[first.host_term(local)])
        .unwrap_err();
    assert_eq!(
        error.to_string(),
        "local fun of module host_term_probe belongs to another program"
    );

    // An external fun holds no label, so the import accepts it.
    let external = first
        .execute("host_term_probe", "external_fun", &[])
        .unwrap();
    let applied = second
        .execute("host_term_probe", "accept", &[first.host_term(external)])
        .unwrap();
    assert!(matches!(applied, Term::Fun(_)));
}

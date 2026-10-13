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
        match result.term() {
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
        .execute("host_term_probe", "accept", &[local])
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
        .execute("host_term_probe", "accept", &[external])
        .unwrap();
    assert!(matches!(applied.term(), Term::Fun(_)));
}

#[test]
fn imports_an_atom_that_the_run_created() {
    let first = BeamProgram::load(&[FIRST]).unwrap();
    let second = BeamProgram::load(&[SECOND]).unwrap();

    // `created_atom/0` calls `erlang:list_to_atom/1`, so the name is in the
    // atom table of the run and not in the table of the program. The host
    // result of the run carries the table of the run, so the import of the
    // second program resolves the name.
    let created = first
        .execute("host_term_probe", "created_atom", &[])
        .unwrap();
    let applied = second
        .execute("host_term_probe", "accept", &[created])
        .unwrap();
    let Term::Atom(atom) = applied.term() else {
        panic!("the second program returns the atom");
    };
    assert_eq!(atom.name(), "host_term_created");
}

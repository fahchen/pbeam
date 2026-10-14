//! The host-term boundary of `BeamProgram::execute`: an argument carries the
//! atom table that owns its atoms, and the run imports each atom of the
//! argument into the atom table of the run.
//!
//! The test loads two programs that share the module name `host_term_probe`
//! and hold different code. A local fun holds the entry label of its function,
//! and a run can load different code under one module name on demand, so the
//! import rejects every local fun. An external fun holds no label and resolves
//! through an export at call time, so it imports across programs.

#[path = "../common/mod.rs"]
mod common;

use common::pinned_beam;
use pbeam_runtime::{BeamProgram, Map, Term, debug_term};

const FIRST: &[u8] = include_bytes!("../../fixtures/host_term_first/host_term_probe.beam");
const LOADER: &[u8] = include_bytes!("../../fixtures/host_term_loader/host_term_loader.beam");
const SECOND: &[u8] = include_bytes!("../../fixtures/host_term_second/host_term_probe.beam");

#[test]
fn rejects_a_local_fun_of_another_program() {
    let first = BeamProgram::load(&[FIRST]).unwrap();
    let second = BeamProgram::load(&[SECOND]).unwrap();

    // The two programs share one module name and hold different code.
    let name = |program: &BeamProgram, function: &str| {
        let result = program.execute("host_term_probe", function, &[]).unwrap();
        match &result.term {
            Term::Atom(atom) => result.atoms().name_of(atom).to_owned(),
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
    assert!(matches!(applied.term, Term::Fun(_)));
}

#[test]
fn imports_an_atom_that_the_run_created() {
    let first = BeamProgram::load(&[FIRST]).unwrap();
    let second = BeamProgram::load(&[SECOND]).unwrap();

    // `created_atom/1` calls `erlang:list_to_atom/1` on the character list
    // of the argument, so the name is made at run time. The program table
    // does not hold the name; only the run creates it.
    let name = "host_term_created";
    assert!(first.atom(name).is_none());
    let characters = Term::list_from(name.bytes().map(|byte| Term::Integer(i64::from(byte))));
    let created = first
        .execute(
            "host_term_probe",
            "created_atom",
            &[first.host_term(characters)],
        )
        .unwrap();
    // The program table of `first` still does not hold the name, and the
    // host result carries the table of the run, which does.
    assert!(first.atom(name).is_none());
    assert_eq!(created.atoms().name_of(&created.atom(name).unwrap()), name);
    let applied = second
        .execute("host_term_probe", "accept", &[created])
        .unwrap();
    let Term::Atom(atom) = &applied.term else {
        panic!("the second program returns the atom");
    };
    assert_eq!(applied.atoms().name_of(atom), name);
}

#[test]
fn builds_a_map_from_an_atom_that_the_run_created() {
    // `created_atom/1` calls `erlang:list_to_atom/1`, so the name is made at
    // run time. The program table holds no name at the index of the created
    // atom, so a host that builds a map from the run atom passes the table of
    // the host result, which is the table of the run. The table of the run
    // owns the created atom and the static atoms. `program.atoms()` is not the
    // owning table, and the order rule then has no name to read for the
    // created atom.
    let program = BeamProgram::load(&[FIRST]).unwrap();
    let name = "host_term_created";
    assert!(program.atom(name).is_none());
    let characters = Term::list_from(name.bytes().map(|byte| Term::Integer(i64::from(byte))));
    let created = program
        .execute(
            "host_term_probe",
            "created_atom",
            &[program.host_term(characters)],
        )
        .unwrap();
    assert!(program.atom(name).is_none());
    // Two nested list keys make the flatmap key order fall back to the term
    // order, which reads the name of each nested atom through the table.
    let key = |name: &str| Term::list_from([Term::Atom(created.atom(name).unwrap())]);
    let map = Map::from_pairs(
        vec![
            (key(name), Term::Atom(created.atom("ok").unwrap())),
            (key("true"), Term::Atom(created.atom("ok").unwrap())),
        ],
        created.atoms(),
    )
    .unwrap();
    // `host_term_created` comes before the static `true` in the term order,
    // so the created key is the first key of the map.
    let term = Term::Map(map);
    assert_eq!(
        debug_term(&term, created.atoms()),
        format!(
            "Map([(List([Atom(\"{name}\")]), Atom(\"ok\")), \
             (List([Atom(\"true\")]), Atom(\"ok\"))])"
        )
    );
}

#[test]
fn rejects_a_local_fun_of_another_run() {
    // `host_term_loader` calls `erlang:load_module/2`, which the `erlang` and
    // `zlib` modules of the pinned OTP provide.
    let erlang = pinned_beam("erlang");
    let zlib = pinned_beam("zlib");
    let program = BeamProgram::load(&[LOADER, &erlang, &zlib]).unwrap();
    // The first run loads one code of `host_term_probe` and returns a local
    // fun of it.
    let first = program
        .execute(
            "host_term_loader",
            "load",
            &[program.host_term(Term::binary(FIRST.to_vec()))],
        )
        .unwrap();
    // The second run of the same program loads different code under the same
    // module name.
    let _second = program
        .execute(
            "host_term_loader",
            "load",
            &[program.host_term(Term::binary(SECOND.to_vec()))],
        )
        .unwrap();
    assert!(matches!(&first.term, Term::Fun(_)));

    // The entry label of the local fun names a function of the code of the
    // first run, and the run of the destination can load other code under
    // that name after the import, so the import rejects the local fun.
    let error = program
        .execute("host_term_loader", "accept", &[first])
        .unwrap_err();
    assert_eq!(
        error.to_string(),
        "local fun of module host_term_probe has no verified code in this program"
    );
}

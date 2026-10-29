use std::sync::Arc;

use super::{ATOM_HANDLE, Env, NON_VALUE};
use crate::{AtomTable, Term};

#[test]
fn gives_a_runtime_error_for_a_new_atom_when_the_atom_table_is_full() {
    // ERTS stops the node with this text (`index.c:86-91`).
    let atoms = Arc::new(AtomTable::new());
    let mut index = 0;
    while atoms.put(&format!("pbeam_full_{index}")).is_ok() {
        index += 1;
    }
    let mut env = Env::new(&atoms, None, &[]);
    assert_eq!(env.make_atom(b"pbeam_new"), NON_VALUE);
    let Err(failure) = env.finish(NON_VALUE) else {
        panic!("the call did not fail");
    };
    assert_eq!(
        failure.to_string(),
        "no more index entries in atom_tab (max=1048576)"
    );
}

/// The handle of an atom is env-independent: an atom that one env makes,
/// which a NIF of a library caches as rustler caches its atoms (rustler
/// 0.38.0, `src/types/atom.rs:21-23`), reads as the same atom in another
/// env, as no env owns an ERTS atom (`erl_term.h`, `make_atom`).
#[test]
fn reads_an_atom_of_another_env() {
    let atoms = Arc::new(AtomTable::new());
    let handle = {
        let mut env = Env::new(&atoms, None, &[]);
        env.make_atom(b"pbeam_shared")
    };
    let other = Env::new(&atoms, None, &[]);
    assert_eq!(handle & ATOM_HANDLE, ATOM_HANDLE);
    assert_eq!(
        other.get(handle),
        Some(Term::Atom(atoms.put("pbeam_shared").unwrap()))
    );
    // An atom handle takes no slot of the env that reads it.
    assert_eq!(other.handles(), Vec::<usize>::new());
}

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

#[test]
fn reads_the_atom_of_an_atom_handle_in_each_env_of_the_table() {
    // An ERTS atom is an immediate term (`make_atom`, `erl_term.h:300`).
    let atoms = Arc::new(AtomTable::new());
    let mut load = Env::new(&atoms, None, &[]);
    let handle = load.make_atom(b"pbeam_kept");
    drop(load);
    let call = Env::new(&atoms, None, &[Term::Integer(1)]);
    assert_eq!(call.get(handle), atoms.get("pbeam_kept").map(Term::Atom));
    assert_eq!(call.get(1), Some(Term::Integer(1)));
}

#[test]
fn reads_no_term_for_an_atom_handle_of_no_atom() {
    // ERTS reads such a handle as a term, which is undefined behavior.
    let atoms = Arc::new(AtomTable::new());
    let env = Env::new(&atoms, None, &[]);
    assert_eq!(env.get(ATOM_HANDLE | 1_048_576), None);
    assert_eq!(env.get(ATOM_HANDLE | (1 << 32)), None);
}

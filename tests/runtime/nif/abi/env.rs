use std::sync::Arc;

use super::{Env, NON_VALUE};
use crate::AtomTable;

#[test]
fn gives_a_runtime_error_for_a_new_atom_when_the_atom_table_is_full() {
    // ERTS stops the node with this text (`index.c:86-91`).
    let atoms = Arc::new(AtomTable::new());
    let mut index = 0;
    while atoms.put(&format!("pbeam_full_{index}")).is_ok() {
        index += 1;
    }
    let mut env = Env::new(&atoms, &[]);
    assert_eq!(env.make_atom(b"pbeam_new"), NON_VALUE);
    let Err(failure) = env.finish(NON_VALUE) else {
        panic!("the call did not fail");
    };
    assert_eq!(
        failure.to_string(),
        "no more index entries in atom_tab (max=1048576)"
    );
}

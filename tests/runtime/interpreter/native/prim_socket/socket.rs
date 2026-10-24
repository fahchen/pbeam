use super::*;

#[test]
fn finds_no_field_in_a_term_that_is_not_a_map() {
    // `prim_socket:open/4` and `prim_socket:bind/2` give the NIFs only maps
    // (prim_socket.erl:1109-1155), so only a direct call reaches this case.
    let atoms = AtomTable::new();
    let key = Term::Atom(atoms.put("family").unwrap());
    assert_eq!(field(&atoms, &key, "family"), None);
}

use super::*;

#[test]
fn finds_no_field_in_a_term_that_is_not_a_map() {
    // `prim_socket:open/4` and `prim_socket:bind/2` give the NIFs only maps
    // (prim_socket.erl:1109-1155), so only a direct call reaches this case.
    let atoms = AtomTable::new();
    let key = Term::Atom(atoms.put("family").unwrap());
    assert_eq!(field(&atoms, &key, "family"), None);
}

#[test]
fn reads_no_unsigned_int_from_a_term_that_is_not_an_integer() {
    // `enif_get_uint` fails for a term that is not an integer. `socket:bind/2`
    // rejects a non-integer `flowinfo` before the NIF runs (the pinned ERTS
    // gives `{error, {invalid, {sockaddr, ...}}}`), so no fixture reaches it.
    assert_eq!(uint(&Term::Nil), None);
    assert_eq!(uint(&Term::Integer(-1)), None);
    assert_eq!(uint(&Term::Integer(7)), Some(7));
}

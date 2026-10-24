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

#[cfg(any(target_os = "linux", target_os = "android"))]
#[test]
fn gives_an_abstract_source_name_with_its_leading_nul() {
    // `esock_encode_sockaddr_un` keeps the reported length of a name whose
    // first byte is NUL on Linux (socket_util.c:1482-1494).
    let address = UnixAddr::new_abstract(b"pbeam").unwrap();
    assert_eq!(unix_path(&address), b"\0pbeam");
}

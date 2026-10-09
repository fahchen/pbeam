use super::{ATOM_LIMIT, Atom, AtomTable, STATIC_ATOM_NAMES};

#[test]
fn starts_with_the_erts_static_atoms() {
    let table = AtomTable::new();
    assert_eq!(table.indices.len(), 1150);
    // ERTS 29.1.1 crash dump: false is 0, ok is 506, get_definition is 1149.
    for (name, index) in [("false", 0), ("true", 1), ("", 5), ("ok", 506)] {
        assert_eq!(table.get(name).unwrap().index, index);
        assert_eq!(Atom::from_static(name), table.get(name));
    }
    assert_eq!(table.get("get_definition").unwrap().index, 1149);
    assert_eq!(
        table.get("get_definition").unwrap().name(),
        "get_definition"
    );
    assert_eq!(table.get("pbeam_zeta"), None);
    assert_eq!(Atom::from_static("pbeam_zeta"), None);
    assert_eq!(STATIC_ATOM_NAMES[1149], "get_definition");
}

#[test]
fn gives_a_new_atom_the_next_index_and_keeps_existing_indices() {
    let mut table = AtomTable::new();
    let zeta = table.put("pbeam_zeta").unwrap();
    assert_eq!(zeta.index, 1150);
    assert_eq!(table.put("pbeam_alpha").unwrap().index, 1151);
    assert_eq!(table.put("pbeam_zeta").unwrap(), zeta);
    assert_eq!(table.get("pbeam_zeta"), Some(zeta));
    assert_eq!(table.put("ok").unwrap().index, 506);
    assert_eq!(table.indices.len(), 1152);
}

#[test]
fn stops_at_the_erts_atom_limit() {
    let mut table = AtomTable::new();
    for index in table.indices.len()..ATOM_LIMIT {
        table.put(&index.to_string()).unwrap();
    }
    assert_eq!(table.put("1048575").unwrap().index, 1048575);
    // An existing atom still resolves at the limit.
    assert_eq!(table.put("ok").unwrap().index, 506);
    assert_eq!(
        table.put("pbeam_zeta").unwrap_err().to_string(),
        "no more index entries in atom_tab (max=1048576)"
    );
    assert_eq!(table.indices.len(), ATOM_LIMIT);
}

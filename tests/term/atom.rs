use std::sync::Arc;

use super::{ATOM_LIMIT, Atom, AtomTable, Fun, Lambda, Map, STATIC_ATOM_NAMES, Target, Term};

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

#[test]
fn shares_the_name_of_a_copied_atom_and_keeps_the_name_out_of_line() {
    // ERTS makes an atom term one tagged word that holds the atom-table
    // index (`make_atom` and `_unchecked_atom_val` in `erl_term.h:300`,
    // `:303`), and the table holds the name (`atom.c:38`). Thus a copy of
    // an atom allocates nothing. PBeam keeps the name with the atom, so a
    // copy must share it. `size_of::<Atom>()` is 24 bytes: a fat
    // `Arc<str>` of 16 bytes and a 4-byte index, where an inline `String`
    // needs 32 bytes.
    let table = AtomTable::new();
    let atom = table.get("ok").unwrap();
    let copy = atom.clone();
    assert_eq!(atom, copy);
    assert!(Arc::ptr_eq(&atom.name, &copy.name));
    assert!(size_of::<Atom>() <= 24);
}

#[test]
fn imports_a_fun_between_two_atom_tables() {
    // The two tables are two copies of one program, so the local fun is of
    // the program of the destination. The source holds three names that the
    // destination does not, so each later name gets a higher index in the
    // source: the module and the function of a fun have different indices
    // in the two tables.
    let mut source = AtomTable::new();
    let mut destination = source.clone();
    for index in 0..3 {
        source.put(&format!("pbeam_host_filler_{index}")).unwrap();
    }
    let module_source = source.put("pbeam_host_module").unwrap();
    let function_source = source.put("pbeam_host_function").unwrap();
    let module_destination = destination.put("pbeam_host_module").unwrap();
    let function_destination = destination.put("pbeam_host_function").unwrap();
    assert_eq!(module_source.index(), 1153);
    assert_eq!(function_source.index(), 1154);
    assert_eq!(module_destination.index(), 1150);
    assert_eq!(function_destination.index(), 1151);

    let local = Term::Fun(Fun::new(
        Lambda::new(module_source.clone(), 0, 1, 53, 1),
        vec![Term::Atom(function_source.clone())],
    ));
    let external = Term::Fun(Fun::external(
        module_source.clone(),
        function_source.clone(),
        1,
    ));

    let imported = destination.import_term(&source, &local).unwrap();
    let Term::Fun(imported_local) = &imported else {
        panic!("the import of a local fun gives a fun");
    };
    let Target::Local(lambda) = imported_local.target() else {
        panic!("the import of a local fun gives a local fun");
    };
    assert_eq!(lambda.module().index(), module_destination.index());
    assert_eq!(
        destination.name(lambda.module().index()),
        Some("pbeam_host_module")
    );
    assert_eq!(imported_local.free().len(), 1);
    let Term::Atom(free) = &imported_local.free()[0] else {
        panic!("the free variable of the local fun is an atom");
    };
    assert_eq!(free.index(), function_destination.index());
    assert_eq!(destination.name(free.index()), Some("pbeam_host_function"));

    let imported = destination.import_term(&source, &external).unwrap();
    let Term::Fun(imported_external) = &imported else {
        panic!("the import of an external fun gives a fun");
    };
    let Target::External {
        module,
        function,
        arity,
    } = imported_external.target()
    else {
        panic!("the import of an external fun gives an external fun");
    };
    assert_eq!(module.index(), module_destination.index());
    assert_eq!(function.index(), function_destination.index());
    assert_eq!(*arity, 1);
    assert_eq!(destination.name(module.index()), Some("pbeam_host_module"));
    assert_eq!(
        destination.name(function.index()),
        Some("pbeam_host_function")
    );

    // The walk remaps each atom of a nested term: a tuple, a list, a map,
    // and a primitive.
    let nested = Term::tuple(vec![
        Term::Atom(module_source.clone()),
        Term::list_from([Term::Atom(function_source.clone())]),
        Term::Map(
            Map::from_pairs(vec![(
                Term::Atom(module_source.clone()),
                Term::Atom(function_source.clone()),
            )])
            .unwrap(),
        ),
        Term::Integer(1),
    ]);
    assert_eq!(
        destination.import_term(&source, &nested).unwrap(),
        Term::tuple(vec![
            Term::Atom(module_destination.clone()),
            Term::list_from([Term::Atom(function_destination.clone())]),
            Term::Map(
                Map::from_pairs(vec![(
                    Term::Atom(module_destination.clone()),
                    Term::Atom(function_destination.clone()),
                )])
                .unwrap(),
            ),
            Term::Integer(1),
        ])
    );

    // An index at or above the source table is not a value of the table, for
    // a term atom and for the module of a local fun.
    let outside_index = u32::try_from(source.names.len()).unwrap();
    let outside = |index| Atom {
        index,
        name: Arc::from("pbeam_host_outside"),
    };
    let outside_error =
        format!("atom index {outside_index} is outside the atom table of the host term");
    assert_eq!(
        destination
            .import_term(&source, &Term::Atom(outside(outside_index)))
            .unwrap_err()
            .to_string(),
        outside_error
    );
    let outside_lambda = Term::Fun(Fun::new(
        Lambda::new(outside(outside_index), 0, 1, 53, 1),
        Vec::new(),
    ));
    assert_eq!(
        destination
            .import_term(&source, &outside_lambda)
            .unwrap_err()
            .to_string(),
        outside_error
    );
}

#[test]
fn fails_to_import_when_the_destination_table_is_full() {
    // The destination table holds `ATOM_LIMIT` atoms and no `pbeam_zeta`.
    // The import reaches `AtomTable::put` with the name of the source atom
    // and gives the error of the full table.
    let mut source = AtomTable::new();
    let mut destination = source.clone();
    for index in destination.indices.len()..ATOM_LIMIT {
        destination.put(&index.to_string()).unwrap();
    }
    let term = Term::Atom(source.put("pbeam_zeta").unwrap());
    assert_eq!(
        destination
            .import_term(&source, &term)
            .unwrap_err()
            .to_string(),
        "no more index entries in atom_tab (max=1048576)"
    );
}

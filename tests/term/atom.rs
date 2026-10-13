use crate::term::Lambda;
use std::sync::Arc;

use super::{ATOM_LIMIT, Atom, AtomTable, Fun, HostTerm, Map, STATIC_ATOM_NAMES, Target, Term};

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

    // A local fun of the same program is rejected, because the import
    // cannot check the code of the module of the fun: a run can load
    // different code under that name on demand.
    assert_eq!(
        destination
            .import_term(&source, &local)
            .unwrap_err()
            .to_string(),
        "local fun of module pbeam_host_module has no verified code in this program"
    );

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

#[test]
fn keeps_the_sharing_of_a_compound_that_the_source_holds_twice() {
    // The source holds one tuple, one cons cell, one map, and one fun twice.
    // The import remembers each compound by the identity of its storage and
    // imports it one time, so each pair of elements of the imported tuple
    // shares one allocation. Without the memory the walk would build one
    // allocation for each occurrence.
    let source = AtomTable::new();
    let mut destination = source.clone();
    let atom = source.get("ok").unwrap();
    let tuple = Term::tuple(vec![Term::Atom(atom.clone()), Term::Integer(1)]);
    let cell = Term::list_from([Term::Integer(2)]);
    let map =
        Term::Map(Map::from_pairs(vec![(Term::Atom(atom.clone()), Term::Integer(3))]).unwrap());
    let fun = Term::Fun(Fun::external(atom.clone(), atom.clone(), 1));
    let term = Term::tuple(vec![
        tuple.clone(),
        tuple.clone(),
        cell.clone(),
        cell.clone(),
        map.clone(),
        map.clone(),
        fun.clone(),
        fun.clone(),
    ]);
    let imported = destination.import_term(&source, &term).unwrap();
    let Term::Tuple(elements) = &imported else {
        panic!("the import of a tuple gives a tuple");
    };
    assert_eq!(elements.len(), 8);
    // `shares_storage_with` covers a tuple, a cons cell, and a map.
    for pair in elements[..6].chunks(2) {
        assert!(pair[0].shares_storage_with(&pair[1]));
    }
    // Two funs share one thing instead.
    let (Term::Fun(left), Term::Fun(right)) = (&elements[6], &elements[7]) else {
        panic!("the import of a fun gives a fun");
    };
    assert!(left.same_thing(right));
}

#[test]
fn compares_and_debug_prints_a_host_term() {
    // The atom table of a host term is storage metadata, so two host terms
    // compare by their terms. `Debug` writes the term and not the table.
    let term = Term::tuple(vec![Term::Integer(1)]);
    let host = HostTerm {
        term: term.clone(),
        atoms: Arc::new(AtomTable::new()),
    };
    let same = HostTerm {
        term: term.clone(),
        atoms: Arc::new(AtomTable::new()),
    };
    let other = HostTerm {
        term: Term::Integer(2),
        atoms: Arc::new(AtomTable::new()),
    };
    assert_eq!(host, same);
    assert_ne!(host, other);
    assert_eq!(&host.term, &term);
    assert_eq!(&term, &host.term);
    assert!(host.atom("ok").is_some());
    assert_eq!(host.atom("pbeam_unknown"), None);
    let printed = format!("{host:?}");
    assert_eq!(printed, format!("{term:?}"));
    assert_eq!(printed, "Tuple([Integer(1)])");
}

#[test]
fn compares_host_terms_of_two_tables_by_their_names() {
    // The two tables give `pbeam_host_a` and `pbeam_host_b` different
    // indices, and the index order of the two keys differs, so the two maps
    // hold their pairs in a different canonical order. The two host terms
    // hold the same value, so they are equal.
    let mut first = AtomTable::new();
    let mut second = first.clone();
    for index in 0..3 {
        second.put(&format!("pbeam_host_filler_{index}")).unwrap();
    }
    let first_a = first.put("pbeam_host_a").unwrap();
    let first_b = first.put("pbeam_host_b").unwrap();
    let second_b = second.put("pbeam_host_b").unwrap();
    let second_a = second.put("pbeam_host_a").unwrap();
    assert_ne!(first_a.index(), second_a.index());
    assert_ne!(first_b.index(), second_b.index());
    let first_map = Map::from_pairs(vec![
        (Term::Atom(first_a), Term::Integer(1)),
        (Term::Atom(first_b), Term::Integer(2)),
    ])
    .unwrap();
    let second_map = Map::from_pairs(vec![
        (Term::Atom(second_b), Term::Integer(2)),
        (Term::Atom(second_a), Term::Integer(1)),
    ])
    .unwrap();
    let first_host = HostTerm {
        term: Term::Map(first_map),
        atoms: Arc::new(first),
    };
    let second_host = HostTerm {
        term: Term::Map(second_map),
        atoms: Arc::new(second),
    };
    assert_eq!(first_host, second_host);
    // Two host terms that share one table compare with no import.
    assert_eq!(first_host, first_host.clone());

    // A term that the other table cannot import is not equal.
    let foreign = HostTerm {
        term: Term::Atom(Atom {
            index: 1 << 30,
            name: Arc::from("pbeam_host_foreign"),
        }),
        atoms: Arc::new(AtomTable::new()),
    };
    assert_ne!(first_host, foreign);
}

#[test]
fn compares_terms_with_a_memory_of_the_shared_compounds() {
    // The memory decides the pair of a shared tuple and its independent
    // equal copy one time.
    let shared = Term::tuple(vec![Term::Integer(1)]);
    let copy = Term::tuple(vec![Term::Integer(1)]);
    let left = Term::tuple(vec![shared.clone(), shared.clone()]);
    let right = Term::tuple(vec![copy.clone(), copy.clone()]);
    assert!(left.eq_shared(&right));
    assert!(shared.eq_shared(&shared.clone()));
    // A tuple pair of a different length.
    assert!(!Term::tuple(vec![Term::Integer(1)]).eq_shared(&Term::tuple(vec![])));

    // A primitive pair and the empty list.
    assert!(Term::Integer(1).eq_shared(&Term::Integer(1)));
    assert!(!Term::Integer(1).eq_shared(&Term::Integer(2)));
    assert!(Term::nil().eq_shared(&Term::nil()));

    // A cons pair: one shared cell, one equal copy, and one longer list.
    let cell = Term::list_from([Term::Integer(1)]);
    assert!(cell.eq_shared(&cell.clone()));
    assert!(cell.eq_shared(&Term::list_from([Term::Integer(1)])));
    assert!(!cell.eq_shared(&Term::list_from([Term::Integer(1), Term::Integer(2)])));

    // A map pair: one shared form, one equal copy, and one shorter map.
    let map = Term::Map(Map::from_pairs(vec![(Term::Integer(1), Term::Integer(2))]).unwrap());
    assert!(map.eq_shared(&map.clone()));
    assert!(map.eq_shared(&Term::Map(
        Map::from_pairs(vec![(Term::Integer(1), Term::Integer(2))]).unwrap()
    )));
    assert!(!map.eq_shared(&Term::Map(Map::from_pairs(vec![]).unwrap())));

    // A fun pair: one shared thing, one equal copy, another free-variable
    // count, and another entry.
    let atom = Atom::from_static("ok").unwrap();
    let fun = Term::Fun(Fun::new(
        Lambda::new(atom.clone(), 0, 1, 53, 1),
        vec![Term::Integer(1)],
    ));
    assert!(fun.eq_shared(&fun.clone()));
    assert!(fun.eq_shared(&Term::Fun(Fun::new(
        Lambda::new(atom.clone(), 0, 1, 53, 1),
        vec![Term::Integer(1)],
    ))));
    assert!(!fun.eq_shared(&Term::Fun(Fun::new(
        Lambda::new(atom.clone(), 0, 1, 53, 1),
        vec![],
    ))));
    assert!(!fun.eq_shared(&Term::Fun(Fun::new(
        Lambda::new(atom, 1, 1, 53, 1),
        vec![Term::Integer(1)],
    ))));

    // A term that holds one cons cell, one map, and one external fun twice
    // takes the memory for each pair of the repeated compound.
    let cell = Term::list_from([Term::Integer(1)]);
    let cell_copy = Term::list_from([Term::Integer(1)]);
    assert!(
        Term::tuple(vec![cell.clone(), cell])
            .eq_shared(&Term::tuple(vec![cell_copy.clone(), cell_copy]))
    );
    let map = Term::Map(Map::from_pairs(vec![(Term::Integer(1), Term::Integer(2))]).unwrap());
    let map_copy = Term::Map(Map::from_pairs(vec![(Term::Integer(1), Term::Integer(2))]).unwrap());
    assert!(
        Term::tuple(vec![map.clone(), map])
            .eq_shared(&Term::tuple(vec![map_copy.clone(), map_copy]))
    );
    let atom = Atom::from_static("ok").unwrap();
    let fun = Term::Fun(Fun::external(atom.clone(), atom.clone(), 1));
    let fun_copy = Term::Fun(Fun::external(atom.clone(), atom.clone(), 1));
    assert!(
        Term::tuple(vec![fun.clone(), fun])
            .eq_shared(&Term::tuple(vec![fun_copy.clone(), fun_copy]))
    );
}

#[test]
fn compares_a_shared_term_with_a_memory() {
    // `T(n) = {T(n-1), T(n-1)}` has storage in `n` and expansion `2^n`. A
    // comparison without the memory would visit 2^30 pairs; the memory
    // visits each pair of the two terms one time.
    let mut term = Term::Integer(1);
    for _ in 0..30 {
        term = Term::tuple(vec![term.clone(), term]);
    }
    let left = HostTerm {
        term: term.clone(),
        atoms: Arc::new(AtomTable::new()),
    };
    let right = HostTerm {
        term,
        atoms: Arc::new(AtomTable::new()),
    };
    assert_eq!(left, right);
}

#[test]
#[should_panic(expected = "atom index 1150 is outside the atom table")]
fn rejects_the_read_of_an_atom_index_outside_the_table() {
    // An atom term holds an index of the table that owns it. An index that
    // the table does not hold is a broken invariant, so the read fails.
    let table = AtomTable::new();
    let atom = Atom {
        index: 1150,
        name: Arc::from("outside"),
    };
    table.name_of(&atom);
}

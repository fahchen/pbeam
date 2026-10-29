use crate::term::Lambda;
use std::sync::Arc;

use super::{
    ATOM_LIMIT, Atom, AtomTable, Fun, HostTerm, Map, PRELOADED_ATOM_NAMES, STATIC_ATOM_NAMES,
    Target, Term,
};

#[test]
fn starts_with_the_erts_static_atoms() {
    let table = AtomTable::new();
    assert_eq!(
        table.read_indices().len(),
        1150 + PRELOADED_ATOM_NAMES.len()
    );
    // ERTS 29.1.1 crash dump: false is 0, ok is 506, get_definition is 1149.
    for (name, index) in [("false", 0), ("true", 1), ("", 5), ("ok", 506)] {
        assert_eq!(table.get(name).unwrap().index, index);
        assert_eq!(Atom::from_static(name), table.get(name));
    }
    assert_eq!(table.get("get_definition").unwrap().index, 1149);
    assert_eq!(
        table.name_of(&table.get("get_definition").unwrap()),
        "get_definition"
    );
    assert_eq!(table.get("pbeam_zeta"), None);
    assert_eq!(Atom::from_static("pbeam_zeta"), None);
    assert_eq!(STATIC_ATOM_NAMES[1149], "get_definition");
    // The default table holds the same static atoms as a new table.
    assert_eq!(AtomTable::default().get("ok"), table.get("ok"));
}

#[test]
fn gives_a_new_atom_the_next_index_and_keeps_existing_indices() {
    let table = AtomTable::new();
    let zeta = table.put("pbeam_zeta").unwrap();
    assert_eq!(zeta.index, 1167);
    assert_eq!(table.put("pbeam_alpha").unwrap().index, 1168);
    assert_eq!(table.put("pbeam_zeta").unwrap(), zeta);
    assert_eq!(table.get("pbeam_zeta"), Some(zeta));
    assert_eq!(table.put("ok").unwrap().index, 506);
    assert_eq!(table.read_indices().len(), 1169);
}

#[test]
fn gives_one_index_to_a_name_that_threads_put_at_once() {
    // ERTS creates each atom once under the atom table lock
    // (`erts_atom_put_index`, `atom.c:231`), so each thread reads one index
    // for each name.
    let table = AtomTable::new();
    let names: Vec<String> = (0..4000)
        .map(|index| format!("pbeam_race_{index}"))
        .collect();
    let indices: Vec<Vec<u32>> = std::thread::scope(|scope| {
        let handles: Vec<_> = (0..8)
            .map(|_| {
                scope.spawn(|| {
                    names
                        .iter()
                        .map(|name| table.put(name).unwrap().index)
                        .collect::<Vec<u32>>()
                })
            })
            .collect();
        handles
            .into_iter()
            .map(|handle| handle.join().unwrap())
            .collect()
    });
    for thread in &indices {
        assert_eq!(thread, &indices[0]);
    }
    for (name, &index) in names.iter().zip(&indices[0]) {
        assert_eq!(table.name(index), Some(name.as_str()));
    }
    assert_eq!(table.read_indices().len(), 1167 + names.len());
}

#[test]
fn writes_the_count_and_the_program_of_a_table() {
    let table = AtomTable::new();
    assert!(format!("{table:?}").starts_with("AtomTable { len: 1167, program: ProgramId("));
}

#[test]
fn stops_at_the_erts_atom_limit() {
    let table = AtomTable::new();
    let count = table.read_indices().len();
    for index in count..ATOM_LIMIT {
        table.put(&index.to_string()).unwrap();
    }
    assert_eq!(table.put("1048575").unwrap().index, 1_048_575);
    // An existing atom still resolves at the limit.
    assert_eq!(table.put("ok").unwrap().index, 506);
    assert_eq!(
        table.put("pbeam_zeta").unwrap_err().to_string(),
        "no more index entries in atom_tab (max=1048576)"
    );
    assert_eq!(table.read_indices().len(), ATOM_LIMIT);
}

#[test]
fn stores_only_the_index_in_a_copied_atom() {
    // ERTS makes an atom term one tagged word that holds the atom-table
    // index (`make_atom` and `_unchecked_atom_val` in `erl_term.h:300`,
    // `:303`), and the table holds the name (`atom.c:38`). Thus a copy of
    // an atom allocates nothing. PBeam follows that rule: the atom holds
    // the 4-byte index and no name, and the table keeps each name one time.
    let table = AtomTable::new();
    let atom = table.get("ok").unwrap();
    let copy = atom.clone();
    assert_eq!(atom, copy);
    assert_eq!(atom, Atom { index: 506 });
    assert_eq!(size_of::<Atom>(), 4);
}

#[test]
fn imports_a_fun_between_two_atom_tables() {
    // The two tables are two copies of one program, so the local fun is of
    // the program of the destination. The source holds three names that the
    // destination does not, so each later name gets a higher index in the
    // source: the module and the function of a fun have different indices
    // in the two tables.
    let source = AtomTable::new();
    let destination = source.clone();
    for index in 0..3 {
        source.put(&format!("pbeam_host_filler_{index}")).unwrap();
    }
    let module_source = source.put("pbeam_host_module").unwrap();
    let function_source = source.put("pbeam_host_function").unwrap();
    let module_destination = destination.put("pbeam_host_module").unwrap();
    let function_destination = destination.put("pbeam_host_function").unwrap();
    assert_eq!(module_source.index(), 1170);
    assert_eq!(function_source.index(), 1171);
    assert_eq!(module_destination.index(), 1167);
    assert_eq!(function_destination.index(), 1168);

    let local = Term::Fun(Fun::new(
        Lambda::new(module_source.clone(), 0, 0, 1, 53, 1),
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
            Map::from_pairs(
                vec![(Term::Atom(module_source), Term::Atom(function_source))],
                &source,
            )
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
                Map::from_pairs(
                    vec![(
                        Term::Atom(module_destination),
                        Term::Atom(function_destination),
                    )],
                    &destination,
                )
                .unwrap(),
            ),
            Term::Integer(1),
        ])
    );

    // An index at or above the source table is not a value of the table, for
    // a term atom and for the module of a local fun.
    let outside_index = u32::try_from(source.read_indices().len()).unwrap();
    let outside = |index| Atom { index };
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
        Lambda::new(outside(outside_index), 0, 0, 1, 53, 1),
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
    let source = AtomTable::new();
    let destination = source.clone();
    let count = destination.read_indices().len();
    for index in count..ATOM_LIMIT {
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
    let destination = source.clone();
    let atom = source.get("ok").unwrap();
    let tuple = Term::tuple(vec![Term::Atom(atom.clone()), Term::Integer(1)]);
    let cell = Term::list_from([Term::Integer(2)]);
    let map = Term::Map(
        Map::from_pairs(vec![(Term::Atom(atom.clone()), Term::Integer(3))], &source).unwrap(),
    );
    let fun = Term::Fun(Fun::external(atom.clone(), atom, 1));
    let term = Term::tuple(vec![
        tuple.clone(),
        tuple,
        cell.clone(),
        cell,
        map.clone(),
        map,
        fun.clone(),
        fun,
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
    let first = AtomTable::new();
    let second = first.clone();
    for index in 0..3 {
        second.put(&format!("pbeam_host_filler_{index}")).unwrap();
    }
    let first_a = first.put("pbeam_host_a").unwrap();
    let first_b = first.put("pbeam_host_b").unwrap();
    let second_b = second.put("pbeam_host_b").unwrap();
    let second_a = second.put("pbeam_host_a").unwrap();
    assert_ne!(first_a.index(), second_a.index());
    assert_ne!(first_b.index(), second_b.index());
    let first_map = Map::from_pairs(
        vec![
            (Term::Atom(first_a), Term::Integer(1)),
            (Term::Atom(first_b), Term::Integer(2)),
        ],
        &first,
    )
    .unwrap();
    let second_map = Map::from_pairs(
        vec![
            (Term::Atom(second_b), Term::Integer(2)),
            (Term::Atom(second_a), Term::Integer(1)),
        ],
        &second,
    )
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
        term: Term::Atom(Atom { index: 1 << 30 }),
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
    let right = Term::tuple(vec![copy.clone(), copy]);
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
    let map = Term::Map(
        Map::from_pairs(
            vec![(Term::Integer(1), Term::Integer(2))],
            &AtomTable::new(),
        )
        .unwrap(),
    );
    assert!(map.eq_shared(&map.clone()));
    assert!(
        map.eq_shared(&Term::Map(
            Map::from_pairs(
                vec![(Term::Integer(1), Term::Integer(2))],
                &AtomTable::new()
            )
            .unwrap()
        ))
    );
    assert!(!map.eq_shared(&Term::Map(
        Map::from_pairs(vec![], &AtomTable::new()).unwrap()
    )));

    // A fun pair: one shared thing, one equal copy, another free-variable
    // count, and another entry.
    let atom = Atom::from_static("ok").unwrap();
    let fun = Term::Fun(Fun::new(
        Lambda::new(atom.clone(), 0, 0, 1, 53, 1),
        vec![Term::Integer(1)],
    ));
    assert!(fun.eq_shared(&fun.clone()));
    assert!(fun.eq_shared(&Term::Fun(Fun::new(
        Lambda::new(atom.clone(), 0, 0, 1, 53, 1),
        vec![Term::Integer(1)],
    ))));
    assert!(!fun.eq_shared(&Term::Fun(Fun::new(
        Lambda::new(atom.clone(), 0, 0, 1, 53, 1),
        vec![],
    ))));
    assert!(!fun.eq_shared(&Term::Fun(Fun::new(
        Lambda::new(atom, 1, 1, 1, 53, 1),
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
    let map = Term::Map(
        Map::from_pairs(
            vec![(Term::Integer(1), Term::Integer(2))],
            &AtomTable::new(),
        )
        .unwrap(),
    );
    let map_copy = Term::Map(
        Map::from_pairs(
            vec![(Term::Integer(1), Term::Integer(2))],
            &AtomTable::new(),
        )
        .unwrap(),
    );
    assert!(
        Term::tuple(vec![map.clone(), map])
            .eq_shared(&Term::tuple(vec![map_copy.clone(), map_copy]))
    );
    let atom = Atom::from_static("ok").unwrap();
    let fun = Term::Fun(Fun::external(atom.clone(), atom.clone(), 1));
    let fun_copy = Term::Fun(Fun::external(atom.clone(), atom, 1));
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
fn is_static_compares_the_index_with_one_static_name() {
    // `Atom::is_static` compares the index of the atom with the index of the
    // static name, as `Atom::from_static(name).unwrap().index()` does, for
    // each of the 1150 static names. The comparison is thus a comparison of
    // the atom with the static name, with no table.
    for (index, name) in STATIC_ATOM_NAMES.iter().enumerate() {
        let index = u32::try_from(index).unwrap();
        let atom = Atom::from_static(name).unwrap();
        assert_eq!(atom.index(), index);
        assert!(atom.is_static(name));
        // A static name that is not the name of the atom is false.
        let other = if *name == "ok" { "error" } else { "ok" };
        assert!(!atom.is_static(other));
    }
    let ok = Atom::from_static("ok").unwrap();
    assert!(ok.is_static("ok"));
    assert!(!ok.is_static("error"));
    assert!(ok.is_one_of(&["ok", "error"]));
    assert!(!ok.is_one_of(&["error"]));
}

#[test]
fn is_one_of_compares_the_index_with_each_static_name() {
    let ok = Atom::from_static("ok").unwrap();
    assert!(ok.is_one_of(&["ok"]));
    assert!(ok.is_one_of(&["error", "ok"]));
    assert!(!ok.is_one_of(&["error"]));
    assert!(!ok.is_one_of(&[]));
    // An atom that the run created has an index above the static range, so it
    // is not one of the static names.
    let runtime = AtomTable::new().put("pbeam_zeta").unwrap();
    assert!(!runtime.is_one_of(&["ok", "error"]));
}

#[test]
#[should_panic(expected = "the name is a static atom")]
fn is_static_panics_for_a_name_that_is_not_a_static_atom() {
    Atom::from_static("ok").unwrap().is_static("pbeam_zeta");
}

#[test]
#[should_panic(expected = "atom index 1167 is outside the atom table")]
fn rejects_the_read_of_an_atom_index_outside_the_table() {
    // An atom term holds an index of the table that owns it. An index that
    // the table does not hold is a broken invariant, so the read fails.
    let table = AtomTable::new();
    let atom = Atom { index: 1167 };
    table.name_of(&atom);
}

#[test]
fn preloads_the_fixed_atoms_of_runtime_paths_after_the_static_atoms() {
    // ERTS makes these atoms at emulator start (`os_info_init`,
    // `inet_init`, and the `load` of the `prim_file` and `prim_tty` NIF
    // libraries), so a runtime path uses a constant and puts no name.
    let table = AtomTable::new();
    let constants = [
        (Atom::UNIX, "unix"),
        (Atom::TCP, "tcp"),
        (Atom::INET_ASYNC, "inet_async"),
        (Atom::INET_REPLY, "inet_reply"),
        (Atom::TCP_CLOSED, "tcp_closed"),
        (Atom::TCP_ERROR, "tcp_error"),
        (Atom::EMPTY_OUT_Q, "empty_out_q"),
        (Atom::READ, "read"),
        (Atom::WRITE, "write"),
        (Atom::READ_WRITE, "read_write"),
        (Atom::DEVICE, "device"),
        (Atom::DIRECTORY, "directory"),
        (Atom::REGULAR, "regular"),
        (Atom::SYMLINK, "symlink"),
        (Atom::OTHER, "other"),
        (Atom::FILE_INFO, "file_info"),
        (Atom::EBADF, "ebadf"),
    ];
    assert_eq!(constants.len(), PRELOADED_ATOM_NAMES.len());
    let count = table.read_indices().len();
    for (position, (atom, name)) in (1150..).zip(constants) {
        assert_eq!(atom.index, position);
        assert_eq!(PRELOADED_ATOM_NAMES[(position - 1150) as usize], name);
        assert!(Atom::from_static(name).is_none(), "{name} is static");
        assert_eq!(table.get(name), Some(atom.clone()));
        assert_eq!(table.name_of(&atom), name);
        assert_eq!(table.put(name).unwrap(), atom);
    }
    assert_eq!(table.read_indices().len(), count);
    assert_eq!(table.put("pbeam_zeta").unwrap().index, 1167);
}

#[test]
fn fails_to_import_a_native_record() {
    let source = AtomTable::new();
    let destination = source.clone();
    let z = source.put("z").unwrap();
    let def = crate::term::RecordDef::new(z.clone(), z.clone(), false, vec![(z, None)], &source);
    let record = Term::Record(crate::term::Record::new(
        Arc::new(def),
        vec![Term::Integer(1)],
    ));
    let error = destination.import_term(&source, &record).unwrap_err();
    assert!(
        error
            .to_string()
            .contains("unsupported import of a native record into another atom table"),
        "{error}"
    );
}

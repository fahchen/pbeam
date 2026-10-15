use super::{
    Access, EMPTY, EtsTables, KeyIndex, Table, TableType, atom_hash, key_hash, stored_hash,
};
use crate::term::{Atom, AtomTable, Map, ProcessId, Reference, Term, test_atom_table};

#[test]
fn deletes_only_the_tables_of_the_owner() {
    let mut tables = EtsTables::default();
    let atoms = test_atom_table();
    let table = |owner| Table::new(ProcessId(owner), Access::Protected, 1, TableType::Set);
    tables.create(Reference(0), None, table(1));
    tables.create(Reference(1), Some("ets_probe_owned"), table(1));
    tables.create(Reference(2), Some("ets_probe_named"), table(2));
    tables.create(Reference(3), None, table(2));

    assert_eq!(tables.delete_owned(&atoms, ProcessId(1), 100), Some(36));
    assert!(!tables.name_in_use("ets_probe_owned"));
    assert!(tables.name_in_use("ets_probe_named"));
    let mut live: Vec<_> = tables.tables.keys().copied().collect();
    live.sort_by_key(|table| table.0);
    assert_eq!(live, vec![Reference(2), Reference(3)]);
}

#[test]
fn finds_a_named_table_of_each_access_mode() {
    let mut tables = EtsTables::default();
    let atoms = test_atom_table();
    tables.create(
        Reference(0),
        None,
        Table::new(ProcessId(1), Access::Public, 1, TableType::Set),
    );
    tables.create(
        Reference(1),
        Some("ets_probe_private"),
        Table::new(ProcessId(1), Access::Private, 1, TableType::Set),
    );
    assert_eq!(tables.whereis("ets_probe_private"), Some(Reference(1)));
    assert_eq!(tables.whereis("ets_probe_missing"), None);
    assert_eq!(tables.delete_owned(&atoms, ProcessId(1), 100), Some(36));
    assert_eq!(tables.whereis("ets_probe_private"), None);
}

#[test]
fn deletes_only_the_object_with_an_exactly_equal_key() {
    let mut table = Table::new(ProcessId(0), Access::Public, 2, TableType::Set);
    let atoms = test_atom_table();
    let object = |value, key| vec![Term::Integer(value), key];
    let tuple = |value| Term::tuple(vec![Term::Integer(value)]);
    table.insert(&atoms, object(1, tuple(1)));
    table.insert(&atoms, object(2, tuple(2)));
    table.delete(&atoms, &tuple(3));
    table.delete(&atoms, &tuple(1));
    assert_eq!(table.object(&atoms, &tuple(1)), None);
    assert_eq!(
        table.object(&atoms, &tuple(2)),
        Some(&object(2, tuple(2))[..])
    );
}

#[test]
fn finds_no_key_in_an_empty_table() {
    let table = Table::new(ProcessId(0), Access::Public, 1, TableType::Set);
    let atoms = test_atom_table();
    assert_eq!(table.object(&atoms, &Term::Integer(1)), None);
    assert_eq!(table.object(&atoms, &Term::static_atom("ok")), None);
}

#[test]
fn replaces_an_object_in_its_slot_and_frees_a_deleted_slot() {
    let mut table = Table::new(ProcessId(0), Access::Public, 1, TableType::Set);
    let atoms = test_atom_table();
    for key in 0..10 {
        table.insert(&atoms, vec![Term::Integer(key), Term::Integer(key)]);
    }
    // An equal key replaces the object in its slot, so the slot count holds.
    table.insert(&atoms, vec![Term::Integer(5), Term::Integer(50)]);
    assert_eq!(table.slot_count(), 10);
    assert_eq!(
        table.live_objects()[5],
        vec![Term::Integer(5), Term::Integer(50)]
    );
    // A delete frees one slot and leaves every other object in its slot.
    table.delete(&atoms, &Term::Integer(5));
    assert_eq!(table.slot_count(), 10);
    assert_eq!(table.objects[5], None);
    assert!(
        table
            .objects
            .iter()
            .enumerate()
            .all(|(slot, object)| slot == 5 || object.is_some())
    );
    assert_eq!(table.live_objects().len(), 9);
    assert_eq!(
        table.live_objects()[5],
        vec![Term::Integer(6), Term::Integer(6)]
    );
    assert_eq!(
        table.object(&atoms, &Term::Integer(9)),
        Some(&[Term::Integer(9), Term::Integer(9)][..])
    );
    assert_eq!(table.object(&atoms, &Term::Integer(5)), None);
}

#[test]
fn finds_a_key_after_a_delete_shifts_its_probe_cluster() {
    let mut table = Table::new(ProcessId(0), Access::Public, 1, TableType::Set);
    let atoms = test_atom_table();
    // The stored hashes of 0, 3, 5, and 16 give the index slot 6, so the four
    // entries form one probe cluster at the index slots 6, 7, 0, and 1.
    for key in [0, 3, 5, 16] {
        table.insert(&atoms, vec![Term::Integer(key)]);
    }
    table.delete(&atoms, &Term::Integer(0));
    // The delete shifts the cluster down and wraps at the end of the index.
    // The object slot of each other entry does not change.
    assert_eq!(table.index.slots[6], 1);
    assert_eq!(table.index.slots[7], 2);
    assert_eq!(table.index.slots[0], 3);
    assert_eq!(table.index.slots[1], EMPTY);
    assert_eq!(table.objects[0], None);
    assert_eq!(
        table.live_objects(),
        vec![
            vec![Term::Integer(3)],
            vec![Term::Integer(5)],
            vec![Term::Integer(16)]
        ]
    );
    // The lookup of 16 walks the shifted cluster.
    assert_eq!(
        table.object(&atoms, &Term::Integer(16)),
        Some(&[Term::Integer(16)][..])
    );
    // The absent key 21 hashes to index slot 6 and stops at the free slot 1.
    assert_eq!(table.object(&atoms, &Term::Integer(21)), None);
    // A later insert reuses the free object slot 0, so the new object takes
    // the place of the deleted one in the slot order.
    table.insert(&atoms, vec![Term::Integer(0)]);
    assert_eq!(table.index.slots[1], 0);
    assert_eq!(table.objects[0], Some(vec![Term::Integer(0)]));
    assert_eq!(
        table.live_objects(),
        vec![
            vec![Term::Integer(0)],
            vec![Term::Integer(3)],
            vec![Term::Integer(5)],
            vec![Term::Integer(16)]
        ]
    );
}

#[test]
fn finds_a_key_after_a_delete_does_not_move_a_blocked_entry() {
    let mut table = Table::new(ProcessId(0), Access::Public, 1, TableType::Set);
    let atoms = test_atom_table();
    // The keys 0 and 3 hash to index slot 6 and the key 22 hashes to index
    // slot 7. The cluster is 0 at 6, 22 at 7, and 3 at 0.
    for key in [0, 22, 3] {
        table.insert(&atoms, vec![Term::Integer(key)]);
    }
    table.delete(&atoms, &Term::Integer(0));
    // The entry 22 stays at index slot 7, because its home slot is between
    // the hole and its slot, and the entry 3 moves into the hole.
    assert_eq!(table.index.slots[6], 2);
    assert_eq!(table.index.slots[7], 1);
    assert_eq!(table.index.slots[0], EMPTY);
    assert_eq!(table.objects[0], None);
    assert_eq!(
        table.object(&atoms, &Term::Integer(22)),
        Some(&[Term::Integer(22)][..])
    );
    assert_eq!(
        table.object(&atoms, &Term::Integer(3)),
        Some(&[Term::Integer(3)][..])
    );
    assert_eq!(table.object(&atoms, &Term::Integer(0)), None);
}

#[test]
fn finds_and_keeps_a_key_of_each_shape() {
    let mut table = Table::new(ProcessId(0), Access::Public, 1, TableType::Set);
    let atoms = test_atom_table();
    let keys = vec![
        Term::static_atom("undefined"),
        Term::Integer(7),
        // A bignum hashes as its magnitude, not as a small-integer word.
        Term::Integer(1 << 60),
        Term::tuple(vec![Term::Integer(1), Term::static_atom("true")]),
        Term::list_from([Term::Integer(1), Term::Integer(2)]),
        Term::binary(vec![1, 2, 3]),
        Term::Map(
            Map::from_pairs(
                vec![(Term::static_atom("ok"), Term::Integer(1))],
                &crate::term::test_atom_table(),
            )
            .unwrap(),
        ),
    ];
    for key in &keys {
        table.insert(&atoms, vec![key.clone()]);
    }
    for key in &keys {
        assert_eq!(table.object(&atoms, key), Some(std::slice::from_ref(key)));
    }
    // Equal content in another storage replaces the object of that key.
    let equal = Term::tuple(vec![Term::Integer(1), Term::static_atom("true")]);
    table.insert(&atoms, vec![equal.clone()]);
    assert_eq!(table.live_objects().len(), keys.len());
    assert_eq!(
        table.object(&atoms, &equal),
        Some(std::slice::from_ref(&equal))
    );
    // A term that is not equal to a stored key does not find one.
    let other = Term::tuple(vec![Term::Integer(2), Term::static_atom("true")]);
    assert_eq!(table.object(&atoms, &other), None);
}

#[test]
fn keeps_every_key_through_growth_and_a_delete_wave() {
    let mut table = Table::new(ProcessId(0), Access::Public, 1, TableType::Set);
    let atoms = test_atom_table();
    for key in 0..200 {
        table.insert(&atoms, vec![Term::Integer(key)]);
    }
    for key in 0..200 {
        let key = Term::Integer(key);
        assert_eq!(table.object(&atoms, &key), Some(std::slice::from_ref(&key)));
    }
    for key in 0..100 {
        table.delete(&atoms, &Term::Integer(key));
    }
    for key in 100..200 {
        let key = Term::Integer(key);
        assert_eq!(table.object(&atoms, &key), Some(std::slice::from_ref(&key)));
    }
    for key in 0..100 {
        assert_eq!(table.object(&atoms, &Term::Integer(key)), None);
    }
    // New keys fill the deleted slots and then grow the index again.
    for key in 200..400 {
        table.insert(&atoms, vec![Term::Integer(key)]);
    }
    for key in 100..400 {
        let key = Term::Integer(key);
        assert_eq!(table.object(&atoms, &key), Some(std::slice::from_ref(&key)));
    }
}

#[test]
fn counts_the_objects_of_the_tables() {
    let mut tables = EtsTables::default();
    let mut table = Table::new(ProcessId(0), Access::Public, 1, TableType::Set);
    let mut atoms = test_atom_table();
    let key = atoms.put("k").unwrap();
    table.insert(
        &atoms,
        vec![Term::Atom(key), Term::tuple(vec![Term::Integer(1)])],
    );
    tables.create(Reference(1), None, table);
    let mut live = crate::term::LiveBytes::new();
    tables.account_terms(&mut live);
    assert!(live.bytes() > 0);
}

#[test]
fn counts_the_object_array_of_immediate_elements() {
    let mut tables = EtsTables::default();
    let mut table = Table::new(ProcessId(0), Access::Public, 1, TableType::Set);
    let atoms = test_atom_table();
    table.insert(&atoms, vec![Term::Integer(1), Term::Integer(2)]);
    let object = table.allocated_bytes();
    assert!(object > 0);
    tables.create(Reference(1), None, table);
    let mut live = crate::term::LiveBytes::new();
    tables.account_terms(&mut live);
    assert_eq!(live.bytes(), object);
}

#[test]
fn hashes_an_atom_key_by_the_name_as_pinned_erts() {
    // `atom_hash` (`atom.c:82-105`) gives the `hvalue` of an atom, and
    // `MAKE_HASH` (`erl_db_hash.c:277-282`) shifts it one bit right for the
    // stored hash of an ETS hash table entry. Erlang has no accessor for the
    // name hash of an atom, so the values come from that algorithm.
    let mut atoms = AtomTable::new();
    for (name, hash) in [
        ("ab", 0x672),
        ("\u{e9}", 0xe9),
        ("ok", 0x75b),
        ("alpha", 0x68_36e1),
        ("get_definition", 0x4f5_034e),
    ] {
        let atom = atoms.put(name).unwrap();
        assert_eq!(atom_hash(&atoms, &atom), hash, "{name}");
        assert_eq!(key_hash(&atoms, &Term::Atom(atom.clone())), hash, "{name}");
        assert_eq!(stored_hash(&atoms, &Term::Atom(atom)), hash >> 1, "{name}");
    }
    // A static atom hashes by its name, not by its index.
    let ok = Atom::from_static("ok").unwrap();
    assert_eq!(key_hash(&atoms, &Term::Atom(ok)), 0x75b);
}

#[test]
fn hashes_a_non_atom_key_with_the_internal_hash() {
    // `MAKE_HASH` uses `erts_internal_hash` for each key that is not an atom
    // (`erl_db_hash.c:277-282`). The values are the pinned ERTS results of
    // `erts_debug:get_internal_state({internal_hash, Term})`:
    // `[1, {}, [a], <<>>, {1,2}, ok]` gives
    // `[7938660269479274665, 4510301282025278020, 18304788118332233160,
    // 10873612744977862784, 10043369381193841056, 4036026909208831136]`.
    let atoms = AtomTable::new();
    let integer = Term::Integer(1);
    assert_eq!(key_hash(&atoms, &integer), 7_938_660_269_479_274_665);
    assert_eq!(stored_hash(&atoms, &integer), 3_969_330_134_739_637_332);
    assert_eq!(
        key_hash(&atoms, &Term::tuple(vec![])),
        4_510_301_282_025_278_020
    );
    assert_eq!(
        key_hash(
            &atoms,
            &Term::tuple(vec![Term::Integer(1), Term::Integer(2)])
        ),
        10_043_369_381_193_841_056
    );
    assert_eq!(
        key_hash(&atoms, &Term::binary(vec![])),
        10_873_612_744_977_862_784
    );
}

#[test]
fn keeps_the_objects_of_a_bag_key_in_insertion_order_across_deletes() {
    // A bag key keeps its objects from the oldest to the newest, and a
    // delete frees each slot of the key. The next objects reuse the free
    // slots, last in first out, and still come in insertion order. A key
    // that shares no slot with the deleted key keeps its objects.
    let atoms = AtomTable::new();
    let mut table = Table::new(ProcessId(0), Access::Public, 1, TableType::Bag);
    let object = |key, value| vec![Term::Integer(key), Term::Integer(value)];
    for value in 1..=3 {
        assert_eq!(table.insert(&atoms, object(1, value)), value as usize - 1);
    }
    assert_eq!(table.insert(&atoms, object(2, 1)), 0);
    assert_eq!(table.insert(&atoms, object(1, 2)), 1);
    table.delete(&atoms, &Term::Integer(1));
    assert!(table.objects(&atoms, &Term::Integer(1)).is_empty());
    assert_eq!(table.slot_count(), 4);
    for value in [7, 8, 9] {
        assert_eq!(table.insert(&atoms, object(1, value)), value as usize - 7);
    }
    assert_eq!(table.slot_count(), 4);
    let objects = |key| -> Vec<Vec<Term>> {
        (table.objects(&atoms, &Term::Integer(key)).into_iter())
            .map(<[Term]>::to_vec)
            .collect()
    };
    assert_eq!(objects(1), vec![object(1, 7), object(1, 8), object(1, 9)]);
    assert_eq!(objects(2), vec![object(2, 1)]);
}

#[test]
fn keeps_equal_objects_in_a_duplicate_bag() {
    let atoms = AtomTable::new();
    let mut table = Table::new(ProcessId(0), Access::Public, 1, TableType::DuplicateBag);
    let object = vec![Term::Integer(5)];
    assert_eq!(table.insert(&atoms, object.clone()), 0);
    assert_eq!(table.insert(&atoms, object.clone()), 0);
    assert_eq!(table.objects(&atoms, &object[0]).len(), 2);
    assert_eq!(table.table_type().name(), "duplicate_bag");
    assert_eq!(TableType::Bag.name(), "bag");
    assert_eq!(TableType::Set.name(), "set");
}

#[test]
fn replaces_the_slot_of_an_index_entry_after_a_probe() {
    // The hashes 1 and 9 have one home slot in the first 8 slots, so the
    // entry of 9 is one slot after its home slot.
    let mut index = KeyIndex::default();
    index.insert(1, 0);
    index.insert(9, 1);
    index.replace(9, 1, 5);
    assert_eq!(index.find(9, |slot| slot == 5), Some(5));
    assert_eq!(index.find(1, |slot| slot == 0), Some(0));
}

#[test]
fn counts_the_compares_of_a_bag_list_insert_as_one_insert_after_another() {
    // `insert_all` must give the counts and the objects that one `insert`
    // after another gives: objects that were in the table, objects that the
    // list added, an equal object in each part, and a new key.
    let atoms = AtomTable::new();
    let object = |key, value| vec![Term::Integer(key), Term::Integer(value)];
    let filled = || {
        let mut table = Table::new(ProcessId(0), Access::Public, 1, TableType::Bag);
        for value in 1..=5 {
            table.insert(&atoms, object(1, value));
        }
        table.insert(&atoms, object(2, 1));
        table
    };
    let list = [
        (1, 3),
        (1, 6),
        (1, 6),
        (2, 1),
        (1, 1),
        (1, 7),
        (3, 1),
        (1, 6),
        (1, 4),
    ];
    let mut one_by_one = filled();
    let expected: Vec<usize> = (list.iter())
        .map(|(key, value)| one_by_one.insert(&atoms, object(*key, *value)))
        .collect();
    let mut table = filled();
    let objects = list
        .iter()
        .map(|(key, value)| object(*key, *value))
        .collect();
    assert_eq!(table.insert_all(&atoms, objects), expected);
    assert_eq!(expected, vec![2, 5, 0, 0, 5, 6, 0, 1, 3]);
    assert_eq!(table.live_objects(), one_by_one.live_objects());
}

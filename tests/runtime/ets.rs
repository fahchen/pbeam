use super::{
    Access, Busy, EMPTY, EtsTables, KeyIndex, Table, TableType, atom_hash, key_hash, object_hash,
    segment, stored_hash,
};
use crate::term::{Atom, AtomTable, Map, ProcessId, Reference, Term, test_atom_table};

#[test]
fn bounds_the_work_of_one_turn_of_a_busy_list_insert() {
    // The steps of a list insert put one object into the table for each two
    // units, and a step ends between two objects, so one turn puts at most
    // `units / 2 + 1` objects in (`ets_insert_2_list_insert_db_term_list`,
    // `erl_db.c:1927-1949`). The position stays in the table, so the next
    // turn continues where the one before it stopped, and the units of the
    // last step may go below 0, as the YCF counter does
    // (`erl_db.c:1939`).
    let atoms = AtomTable::new();
    let mut table = Table::new(ProcessId(0), Access::Public, 1, TableType::Set);
    assert_eq!(table.step_busy(&atoms, 7), (0, None));
    let objects = (0..10).map(|value| vec![Term::Integer(value)]).collect();
    table.busy = Some(Busy::new(ProcessId(0), objects, false));
    assert_eq!(table.step_busy(&atoms, 7), (8, None));
    assert_eq!(table.live_objects().len(), 4);
    assert_eq!(table.step_busy(&atoms, 2), (2, None));
    assert_eq!(table.live_objects().len(), 5);
    assert_eq!(
        table.step_busy(&atoms, 100),
        (10, Some(Term::static_atom("true")))
    );
    assert_eq!(table.live_objects().len(), 10);
    assert!(table.busy.is_none());
    assert_eq!(
        table.take_ended(ProcessId(0)),
        Some(Term::static_atom("true"))
    );
    assert_eq!(table.take_ended(ProcessId(0)), None);
}

#[test]
fn deletes_only_the_tables_of_the_owner() {
    let mut tables = EtsTables::default();
    let table = |owner| Table::new(ProcessId(owner), Access::Protected, 1, TableType::Set);
    tables.create(Reference(0), None, table(1));
    tables.create(Reference(1), Some("ets_probe_owned"), table(1));
    tables.create(Reference(2), Some("ets_probe_named"), table(2));
    tables.create(Reference(3), None, table(2));

    assert_eq!(tables.delete_owned(ProcessId(1), 100), Some(36));
    assert!(!tables.name_in_use("ets_probe_owned"));
    assert!(tables.name_in_use("ets_probe_named"));
    let mut live: Vec<_> = tables.tables.keys().copied().collect();
    live.sort_by_key(|table| table.0);
    assert_eq!(live, vec![Reference(2), Reference(3)]);
}

#[test]
fn frees_a_large_deleted_table_in_bounded_turns() {
    let mut tables = EtsTables::default();
    let atoms = test_atom_table();
    let mut table = Table::new(ProcessId(1), Access::Public, 1, TableType::Set);
    let count = 40_000;
    for key in 0..count {
        table.insert(&atoms, vec![Term::Integer(key)]);
    }
    let mut per_segment = vec![0; segment(table.segment_slots() - 1) + 1];
    for key in 0..count {
        per_segment[segment(table.bucket(stored_hash(&atoms, &Term::Integer(key))))] += 1;
    }
    let largest = *per_segment.iter().max().expect("a table has a segment");
    tables.create(Reference(0), Some("ets_probe_large"), table);
    tables.delete_table(Reference(0), ProcessId(1));
    assert!(!tables.name_in_use("ets_probe_large"));
    assert!(tables.tables.is_empty());

    let live = |tables: &EtsTables| {
        tables
            .freeing
            .get(&Reference(0))
            .map_or(0, |freeing| freeing.table.objects.iter().flatten().count())
    };
    let budget = 4000;
    let mut left = live(&tables);
    let mut segments = tables.freeing[&Reference(0)].segments;
    let mut traps = 0;
    assert_eq!(left, per_segment.iter().sum::<usize>());
    loop {
        let rest = tables.free_segments(ProcessId(1), budget);
        let now = live(&tables);
        let freed = left - now;
        assert!(freed > 0);
        assert!(freed <= usize::try_from(budget).expect("the budget is positive") + largest);
        left = now;
        if let Some(rest) = rest {
            assert!((0..budget).contains(&rest));
            break;
        }
        traps += 1;
        let next = tables.freeing[&Reference(0)].segments;
        assert!(next < segments);
        segments = next;
    }
    assert!(traps >= 3, "{traps} traps");
    assert_eq!(left, 0);
    assert!(tables.freeing.is_empty());
}

#[test]
fn finds_a_named_table_of_each_access_mode() {
    let mut tables = EtsTables::default();
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
    assert_eq!(tables.delete_owned(ProcessId(1), 100), Some(36));
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
fn counts_a_deleted_table_until_each_segment_is_free() {
    let mut tables = EtsTables::default();
    let mut table = Table::new(ProcessId(1), Access::Public, 1, TableType::Set);
    let atoms = test_atom_table();
    for key in 0..2000 {
        table.insert(
            &atoms,
            vec![Term::Integer(key), Term::tuple(vec![Term::Integer(key)])],
        );
    }
    tables.create(Reference(0), None, table);
    let bytes = |tables: &EtsTables| {
        let mut live = crate::term::LiveBytes::new();
        tables.account_terms(&mut live);
        live.bytes()
    };
    let before = bytes(&tables);
    tables.delete_table(Reference(0), ProcessId(1));
    assert_eq!(bytes(&tables), before);
    assert_eq!(tables.free_segments(ProcessId(1), 100), None);
    let during = bytes(&tables);
    assert!(0 < during && during < before, "{during} of {before}");
    assert!(tables.free_segments(ProcessId(1), i32::MAX).is_some());
    assert_eq!(bytes(&tables), 0);
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
    let mut insert = |object| table.insert_all(&atoms, vec![object])[0];
    for value in 1..=3 {
        assert_eq!(insert(object(1, value)), value as usize - 1);
    }
    assert_eq!(insert(object(2, 1)), 0);
    assert_eq!(insert(object(1, 2)), 1);
    table.delete(&atoms, &Term::Integer(1));
    assert!(table.objects(&atoms, &Term::Integer(1)).is_empty());
    assert_eq!(table.slot_count(), 4);
    for value in [7, 8, 9] {
        assert_eq!(
            table.insert_all(&atoms, vec![object(1, value)]),
            vec![value as usize - 7]
        );
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
fn hashes_bag_objects_whose_changing_elements_cancelled_under_rotate_and_xor() {
    // The objects `{7, I, 0 x 63, I}` had one hash for each `I` when the
    // object hash was a rotate by 5 and an XOR of the element hashes: the
    // two `I` elements are 64 rotations apart and cancel. A list insert of
    // these objects then compared each object with all earlier objects in
    // one turn. Each object must now have its own hash, and the list insert
    // must give the ERTS counts of one insert after another.
    const OBJECTS: i64 = 10_000;
    let object = |value| {
        let mut object = vec![Term::Integer(7), Term::Integer(value)];
        object.extend(std::iter::repeat_n(Term::Integer(0), 63));
        object.push(Term::Integer(value));
        object
    };
    let objects: Vec<Vec<Term>> = (0..OBJECTS).map(object).collect();
    let hashes: std::collections::HashSet<u64> =
        objects.iter().map(|object| object_hash(object)).collect();
    assert_eq!(hashes.len(), objects.len());

    let atoms = AtomTable::new();
    let mut table = Table::new(ProcessId(0), Access::Public, 1, TableType::Bag);
    let counts = table.insert_all(&atoms, objects);
    assert_eq!(counts, (0..OBJECTS as usize).collect::<Vec<_>>());
}

#[test]
fn keeps_equal_objects_in_a_duplicate_bag() {
    let atoms = AtomTable::new();
    let mut table = Table::new(ProcessId(0), Access::Public, 1, TableType::DuplicateBag);
    let object = vec![Term::Integer(5)];
    assert_eq!(
        table.insert_all(&atoms, vec![object.clone(), object.clone()]),
        vec![0, 0]
    );
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

/// Add `object` to the `bag` table `table` as ERTS adds one object
/// (`erl_db_hash.c:1422-1452`), and return the number of objects that the
/// insert compared with `object` and found not exactly equal. The insert
/// compares `object` with each object of its key, from the newest to the
/// oldest, and an exactly equal object stops it with no change. This is
/// the reference model of `Table::insert_all`.
fn insert_after_compares(table: &mut Table, atoms: &AtomTable, object: Vec<Term>) -> usize {
    let key = table.key(&object);
    let hash = stored_hash(atoms, key);
    let newest = table.position(hash, key).unwrap_or(EMPTY);
    let mut compared = 0;
    let mut slot = newest;
    while slot != EMPTY {
        if *table.object_of(slot) == object {
            return compared;
        }
        compared += 1;
        slot = table.older[slot];
    }
    table.add(atoms, hash, object, newest);
    compared
}

#[test]
fn counts_the_compares_of_a_bag_list_insert_as_one_insert_after_another() {
    // `insert_all` must give the counts and the objects that one insert
    // after another gives: objects that were in the table, objects that the
    // list added, an equal object in each part, and a new key.
    let atoms = AtomTable::new();
    let object = |key, value| vec![Term::Integer(key), Term::Integer(value)];
    let filled = || {
        let mut table = Table::new(ProcessId(0), Access::Public, 1, TableType::Bag);
        for value in 1..=5 {
            insert_after_compares(&mut table, &atoms, object(1, value));
        }
        insert_after_compares(&mut table, &atoms, object(2, 1));
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
        .map(|(key, value)| insert_after_compares(&mut one_by_one, &atoms, object(*key, *value)))
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

/// Return the keys of each bucket chain of `table`, bucket 0 first.
fn chains(table: &Table) -> Vec<Vec<Term>> {
    (0..table.bucket_count())
        .map(|bucket| {
            table
                .chain(bucket)
                .map(|object| object[0].clone())
                .collect()
        })
        .collect()
}

#[test]
fn keeps_the_erts_bucket_chains_through_splits_and_joins() {
    // The reference model is the ERTS chain list: a new key goes to the end
    // of its bucket (erl_db_hash.c:1384-1393, 1447-1452), a split keeps the
    // order of both chains (erl_db_hash.c:3690-3740), and a join puts the
    // chain of the top bucket before the chain of its low bucket
    // (erl_db_hash.c:3743-3818). The table must give the same chains.
    let mut table = Table::new(ProcessId(0), Access::Public, 1, TableType::Set);
    let atoms = test_atom_table();
    let mut model: Vec<Vec<Term>> = vec![Vec::new(); 256];
    let bucket = |buckets: usize, key: &Term| {
        let mask = buckets.next_power_of_two() - 1;
        let slot = stored_hash(&atoms, key) as usize & mask;
        if slot >= buckets {
            slot & (mask >> 1)
        } else {
            slot
        }
    };
    let insert = |table: &mut Table, model: &mut Vec<Vec<Term>>, key: i64| {
        let key = Term::Integer(key);
        table.insert(&atoms, vec![key.clone()]);
        let index = bucket(model.len(), &key);
        model[index].push(key);
        if model.iter().map(Vec::len).sum::<usize>() > model.len() {
            let to = model.len();
            let mask = (to + 1).next_power_of_two() - 1;
            let from = to & (mask >> 1);
            let (high, low) = model[from]
                .drain(..)
                .partition(|key| stored_hash(&atoms, key) as usize & mask == to);
            model[from] = low;
            model.push(high);
        }
    };
    for key in 0..6000 {
        insert(&mut table, &mut model, key);
    }
    assert_eq!(chains(&table), model);
    for key in 0..5000 {
        let key = Term::Integer(key);
        table.delete(&atoms, &key);
        let index = bucket(model.len(), &key);
        model[index].retain(|kept| *kept != key);
        while model.len() > table.bucket_count() {
            let source = model.pop().unwrap();
            let target = model.len() & ((model.len() + 1).next_power_of_two() - 1) >> 1;
            let low = std::mem::replace(&mut model[target], source);
            model[target].extend(low);
        }
    }
    assert!(table.bucket_count() < 6000);
    assert_eq!(chains(&table), model);
    for key in 10_000..12_000 {
        insert(&mut table, &mut model, key);
    }
    assert_eq!(chains(&table), model);
    let key = Term::Integer(10_000);
    assert_eq!(
        table.key_bucket(&atoms, &key),
        Some(bucket(model.len(), &key))
    );
    assert_eq!(table.key_bucket(&atoms, &Term::Integer(0)), None);
}

use super::{Access, EMPTY, EtsTables, Table};
use crate::term::{Map, ProcessId, Reference, Term};

#[test]
fn deletes_only_the_tables_of_the_owner() {
    let mut tables = EtsTables::default();
    let table = |owner| Table::new(ProcessId(owner), Access::Protected, 1);
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
fn finds_a_named_table_of_each_access_mode() {
    let mut tables = EtsTables::default();
    tables.create(
        Reference(0),
        None,
        Table::new(ProcessId(1), Access::Public, 1),
    );
    tables.create(
        Reference(1),
        Some("ets_probe_private"),
        Table::new(ProcessId(1), Access::Private, 1),
    );
    assert_eq!(tables.whereis("ets_probe_private"), Some(Reference(1)));
    assert_eq!(tables.whereis("ets_probe_missing"), None);
    assert_eq!(tables.delete_owned(ProcessId(1), 100), Some(36));
    assert_eq!(tables.whereis("ets_probe_private"), None);
}

#[test]
fn deletes_only_the_object_with_an_exactly_equal_key() {
    let mut table = Table::new(ProcessId(0), Access::Public, 2);
    let object = |value, key| vec![Term::Integer(value), key];
    let tuple = |value| Term::tuple(vec![Term::Integer(value)]);
    table.insert(object(1, tuple(1)));
    table.insert(object(2, tuple(2)));
    table.delete(&tuple(3));
    table.delete(&tuple(1));
    assert_eq!(table.object(&tuple(1)), None);
    assert_eq!(table.object(&tuple(2)), Some(&object(2, tuple(2))[..]));
}

#[test]
fn finds_no_key_in_an_empty_table() {
    let table = Table::new(ProcessId(0), Access::Public, 1);
    assert_eq!(table.object(&Term::Integer(1)), None);
    assert_eq!(table.object(&Term::static_atom("ok")), None);
}

#[test]
fn replaces_an_object_in_its_slot_and_frees_a_deleted_slot() {
    let mut table = Table::new(ProcessId(0), Access::Public, 1);
    for key in 0..10 {
        table.insert(vec![Term::Integer(key), Term::Integer(key)]);
    }
    // An equal key replaces the object in its slot, so the slot count holds.
    table.insert(vec![Term::Integer(5), Term::Integer(50)]);
    assert_eq!(table.slot_count(), 10);
    assert_eq!(
        table.live_objects()[5],
        vec![Term::Integer(5), Term::Integer(50)]
    );
    // A delete frees one slot and leaves every other object in its slot.
    table.delete(&Term::Integer(5));
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
        table.object(&Term::Integer(9)),
        Some(&[Term::Integer(9), Term::Integer(9)][..])
    );
    assert_eq!(table.object(&Term::Integer(5)), None);
}

#[test]
fn finds_a_key_after_a_delete_shifts_its_probe_cluster() {
    let mut table = Table::new(ProcessId(0), Access::Public, 1);
    // The stored hashes of 0, 3, 5, and 16 give the index slot 6, so the four
    // entries form one probe cluster at the index slots 6, 7, 0, and 1.
    for key in [0, 3, 5, 16] {
        table.insert(vec![Term::Integer(key)]);
    }
    table.delete(&Term::Integer(0));
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
        table.object(&Term::Integer(16)),
        Some(&[Term::Integer(16)][..])
    );
    // The absent key 21 hashes to index slot 6 and stops at the free slot 1.
    assert_eq!(table.object(&Term::Integer(21)), None);
    // A later insert reuses the free object slot 0, so the new object takes
    // the place of the deleted one in the slot order.
    table.insert(vec![Term::Integer(0)]);
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
    let mut table = Table::new(ProcessId(0), Access::Public, 1);
    // The keys 0 and 3 hash to index slot 6 and the key 22 hashes to index
    // slot 7. The cluster is 0 at 6, 22 at 7, and 3 at 0.
    for key in [0, 22, 3] {
        table.insert(vec![Term::Integer(key)]);
    }
    table.delete(&Term::Integer(0));
    // The entry 22 stays at index slot 7, because its home slot is between
    // the hole and its slot, and the entry 3 moves into the hole.
    assert_eq!(table.index.slots[6], 2);
    assert_eq!(table.index.slots[7], 1);
    assert_eq!(table.index.slots[0], EMPTY);
    assert_eq!(table.objects[0], None);
    assert_eq!(
        table.object(&Term::Integer(22)),
        Some(&[Term::Integer(22)][..])
    );
    assert_eq!(
        table.object(&Term::Integer(3)),
        Some(&[Term::Integer(3)][..])
    );
    assert_eq!(table.object(&Term::Integer(0)), None);
}

#[test]
fn finds_and_keeps_a_key_of_each_shape() {
    let mut table = Table::new(ProcessId(0), Access::Public, 1);
    let keys = vec![
        Term::static_atom("undefined"),
        Term::Integer(7),
        // A bignum hashes as its magnitude, not as a small-integer word.
        Term::Integer(1 << 60),
        Term::tuple(vec![Term::Integer(1), Term::static_atom("true")]),
        Term::list_from([Term::Integer(1), Term::Integer(2)]),
        Term::binary(vec![1, 2, 3]),
        Term::Map(Map::from_pairs(vec![(Term::static_atom("ok"), Term::Integer(1))]).unwrap()),
    ];
    for key in &keys {
        table.insert(vec![key.clone()]);
    }
    for key in &keys {
        assert_eq!(table.object(key), Some(std::slice::from_ref(key)));
    }
    // Equal content in another storage replaces the object of that key.
    let equal = Term::tuple(vec![Term::Integer(1), Term::static_atom("true")]);
    table.insert(vec![equal.clone()]);
    assert_eq!(table.live_objects().len(), keys.len());
    assert_eq!(table.object(&equal), Some(std::slice::from_ref(&equal)));
    // A term that is not equal to a stored key does not find one.
    let other = Term::tuple(vec![Term::Integer(2), Term::static_atom("true")]);
    assert_eq!(table.object(&other), None);
}

#[test]
fn keeps_every_key_through_growth_and_a_delete_wave() {
    let mut table = Table::new(ProcessId(0), Access::Public, 1);
    for key in 0..200 {
        table.insert(vec![Term::Integer(key)]);
    }
    for key in 0..200 {
        let key = Term::Integer(key);
        assert_eq!(table.object(&key), Some(std::slice::from_ref(&key)));
    }
    for key in 0..100 {
        table.delete(&Term::Integer(key));
    }
    for key in 100..200 {
        let key = Term::Integer(key);
        assert_eq!(table.object(&key), Some(std::slice::from_ref(&key)));
    }
    for key in 0..100 {
        assert_eq!(table.object(&Term::Integer(key)), None);
    }
    // New keys fill the deleted slots and then grow the index again.
    for key in 200..400 {
        table.insert(vec![Term::Integer(key)]);
    }
    for key in 100..400 {
        let key = Term::Integer(key);
        assert_eq!(table.object(&key), Some(std::slice::from_ref(&key)));
    }
}

#[test]
fn counts_the_objects_of_the_tables() {
    let mut tables = EtsTables::default();
    let mut table = Table::new(ProcessId(0), Access::Public, 1);
    table.insert(vec![
        crate::term::test_atom("k"),
        Term::tuple(vec![Term::Integer(1)]),
    ]);
    tables.create(Reference(1), None, table);
    let mut live = crate::term::LiveBytes::new();
    tables.account_terms(&mut live);
    assert!(live.bytes() > 0);
}

#[test]
fn counts_the_object_array_of_immediate_elements() {
    let mut tables = EtsTables::default();
    let mut table = Table::new(ProcessId(0), Access::Public, 1);
    table.insert(vec![Term::Integer(1), Term::Integer(2)]);
    let object = table.allocated_bytes();
    assert!(object > 0);
    tables.create(Reference(1), None, table);
    let mut live = crate::term::LiveBytes::new();
    tables.account_terms(&mut live);
    assert_eq!(live.bytes(), object);
}

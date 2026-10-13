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
fn keeps_the_insertion_order_of_a_replaced_and_a_deleted_object() {
    let mut table = Table::new(ProcessId(0), Access::Public, 1);
    for key in 0..10 {
        table.insert(vec![Term::Integer(key), Term::Integer(key)]);
    }
    // An equal key replaces the object at its position, so the order holds.
    table.insert(vec![Term::Integer(5), Term::Integer(50)]);
    assert_eq!(table.objects.len(), 10);
    assert_eq!(table.objects[5], vec![Term::Integer(5), Term::Integer(50)]);
    // A delete removes the object and keeps the order of the rest.
    table.delete(&Term::Integer(5));
    assert_eq!(table.objects.len(), 9);
    assert_eq!(table.objects[5], vec![Term::Integer(6), Term::Integer(6)]);
    assert_eq!(
        table.object(&Term::Integer(9)),
        Some(&[Term::Integer(9), Term::Integer(9)][..])
    );
    assert_eq!(table.object(&Term::Integer(5)), None);
}

#[test]
fn finds_a_key_after_a_delete_shifts_its_cluster() {
    let mut table = Table::new(ProcessId(0), Access::Public, 1);
    // The stored hashes of 0, 3, 5, and 16 give the index slot 6, so the four
    // keys form one probe cluster at the slots 6, 7, 0, and 1.
    for key in [0, 3, 5, 16] {
        table.insert(vec![Term::Integer(key)]);
    }
    table.delete(&Term::Integer(0));
    // The delete moves the cluster down and wraps at the end of the table.
    assert_eq!(table.index.positions[6], 0);
    assert_eq!(table.index.positions[7], 1);
    assert_eq!(table.index.positions[0], 2);
    assert_eq!(table.index.positions[1], EMPTY);
    assert_eq!(
        table.objects,
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
    // The absent key 21 hashes to slot 6 and stops at the free slot 1.
    assert_eq!(table.object(&Term::Integer(21)), None);
    // A later insert of the deleted key goes to the free slot and to the end
    // of the object order.
    table.insert(vec![Term::Integer(0)]);
    assert_eq!(table.index.positions[1], 3);
    assert_eq!(table.objects.len(), 4);
    assert_eq!(table.objects[3], vec![Term::Integer(0)]);
}

#[test]
fn finds_a_key_after_a_delete_does_not_move_a_blocked_entry() {
    let mut table = Table::new(ProcessId(0), Access::Public, 1);
    // The keys 0 and 3 hash to slot 6 and the key 22 hashes to slot 7. The
    // cluster is 0 at slot 6, 22 at slot 7, and 3 at slot 0.
    for key in [0, 22, 3] {
        table.insert(vec![Term::Integer(key)]);
    }
    table.delete(&Term::Integer(0));
    // The entry 22 stays at slot 7, because its home slot is between the hole
    // and its slot, and the entry 3 moves into the hole.
    assert_eq!(table.index.positions[6], 1);
    assert_eq!(table.index.positions[7], 0);
    assert_eq!(table.index.positions[0], EMPTY);
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
    assert_eq!(table.objects.len(), keys.len());
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

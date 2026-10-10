use super::{Access, EtsTables, Table};
use crate::term::{ProcessId, Reference, Term};

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
    let tuple = |value| Term::Tuple(vec![Term::Integer(value)]);
    table.insert(object(1, tuple(1)));
    table.insert(object(2, tuple(2)));
    table.delete(&tuple(3));
    table.delete(&tuple(1));
    assert_eq!(table.object(&tuple(1)), None);
    assert_eq!(table.object(&tuple(2)), Some(&object(2, tuple(2))[..]));
}

//! Unit tests of the native records of `src/term/record.rs`.

use std::{cmp::Ordering, sync::Arc};

use super::{Record, RecordDef};
use crate::term::{AtomTable, Term};

/// Return a definition of `nr_def:priv` with the fields `keys` and no
/// defaults.
fn def(atoms: &AtomTable, keys: &[&str]) -> Arc<RecordDef> {
    let fields = keys
        .iter()
        .map(|key| (atoms.put(key).unwrap(), None))
        .collect();
    Arc::new(RecordDef::new(
        atoms.put("nr_def").unwrap(),
        atoms.put("priv").unwrap(),
        false,
        fields,
        atoms,
    ))
}

#[test]
fn gives_the_key_terms_in_the_sorted_order() {
    let atoms = AtomTable::new();
    let def = def(&atoms, &["pbeam_rec_b", "pbeam_rec_a"]);
    let names: Vec<_> = def
        .key_terms()
        .iter()
        .map(|key| match key {
            Term::Atom(atom) => atoms.name_of(atom).to_owned(),
            _ => panic!("a key is an atom"),
        })
        .collect();
    assert_eq!(names, ["pbeam_rec_b", "pbeam_rec_a"]);
}

#[test]
fn shares_the_thing_between_copies_only() {
    let atoms = AtomTable::new();
    let def = def(&atoms, &["z"]);
    let record = Record::new(Arc::clone(&def), vec![Term::Integer(1)]);
    let copy = record.clone();
    assert_eq!(Arc::strong_count(record.arc()), 2);
    assert!(record.same_thing(&copy));
    let mut taken = Record::new(def, vec![Term::Integer(1)]);
    assert_eq!(taken.take_last(), Some(vec![Term::Integer(1)]));
    assert!(!record.same_thing(&taken));
}

#[test]
fn compares_two_definitions_by_the_keys_last() {
    let atoms = AtomTable::new();
    let left = Record::new(def(&atoms, &["pbeam_rec_a"]), vec![Term::Integer(1)]);
    let right = Record::new(def(&atoms, &["pbeam_rec_b"]), vec![Term::Integer(1)]);
    assert_eq!(left.compare_defs(&right, &atoms), Ordering::Less);
    assert_eq!(right.compare_defs(&left, &atoms), Ordering::Greater);
    let same = Record::new(def(&atoms, &["pbeam_rec_a"]), vec![Term::Integer(2)]);
    assert_eq!(left.compare_defs(&same, &atoms), Ordering::Equal);
}

#[test]
#[should_panic(expected = "a record definition is a tuple")]
fn rejects_a_definition_that_holds_no_tuple() {
    // `RecordDef::new` always builds the tuple, so only a test can break it.
    let atoms = AtomTable::new();
    let def = def(&atoms, &["pbeam_rec_a"]);
    let broken = RecordDef {
        module: def.module.clone(),
        name: def.name.clone(),
        is_exported: def.is_exported,
        keys: def.keys.clone(),
        order: def.order.clone(),
        defaults: def.defaults.clone(),
        hash: def.hash,
        tuple: Term::Nil,
    };
    let _ = broken.key_terms();
}

#[test]
fn gives_no_record_of_another_term_and_prints_indexes_without_a_table() {
    let atoms = AtomTable::new();
    let def = def(&atoms, &["pbeam_rec_a"]);
    assert!(Term::Integer(1).record().is_none());
    let record = Term::Record(Record::new(Arc::clone(&def), vec![Term::Integer(1)]));
    assert!(record.record().is_some());
    assert_eq!(
        format!("{record:?}"),
        format!(
            "Record({}:{}, [Integer(1)])",
            def.module.index(),
            def.name.index()
        )
    );
    assert_eq!(
        crate::term::debug_term(&record, &atoms),
        "Record(nr_def:priv, [Integer(1)])"
    );
}

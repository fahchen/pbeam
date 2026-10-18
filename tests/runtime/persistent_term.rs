use super::PersistentTerms;
use crate::term::{Term, test_atom as atom};

fn logger_config(name: &str) -> Term {
    Term::tuple(vec![atom("logger_config"), atom(name)])
}

/// Return the slot of each stored key, by slot.
fn slots(terms: &PersistentTerms) -> Vec<(usize, Term)> {
    terms
        .buckets
        .iter()
        .enumerate()
        .filter_map(|(slot, bucket)| bucket.as_ref().map(|(key, _)| (slot, key.clone())))
        .collect()
}

#[test]
fn puts_and_gets_values_by_exactly_equal_keys() {
    let mut terms = PersistentTerms::default();
    assert_eq!(terms.get(&logger_config("default")), None);
    terms.put(&logger_config("default"), &Term::Integer(1));
    terms.put(&logger_config("other"), &Term::Integer(2));
    terms.put(&Term::Integer(1), &Term::Integer(3));
    // A key that is equal to a stored key, but is another term, finds its value.
    assert_eq!(
        terms.get(&logger_config("default")),
        Some(&Term::Integer(1))
    );
    assert_eq!(terms.get(&logger_config("other")), Some(&Term::Integer(2)));
    assert_eq!(terms.get(&Term::Integer(1)), Some(&Term::Integer(3)));
    assert_eq!(terms.get(&Term::tuple(vec![atom("logger_config")])), None);
    // An equal key replaces the value and adds no key. The same value
    // changes nothing.
    assert_eq!(terms.put(&logger_config("other"), &Term::Integer(2)), None);
    assert_eq!(terms.put(&logger_config("default"), &Term::nil()), Some(0));
    assert_eq!(terms.get(&logger_config("default")), Some(&Term::nil()));
    assert_eq!(terms.num_entries, 3);
}

#[test]
fn keeps_the_erts_slots_through_collisions_and_a_resize() {
    let (true_, ok) = (Term::static_atom("true"), Term::static_atom("ok"));
    let mut terms = PersistentTerms::default();
    terms.put(&true_, &Term::Integer(1));
    terms.put(&Term::Integer(39), &Term::Integer(2));
    terms.put(&ok, &Term::Integer(3));
    terms.put(&Term::Integer(7), &Term::Integer(4));
    // true and 39 both hash to slot 7 of 8: 39 goes past the end to slot 0, and ok
    // (slot 0) goes to slot 1.
    assert_eq!(terms.buckets.len(), 8);
    assert_eq!(
        slots(&terms),
        [
            (0, Term::Integer(39)),
            (1, ok.clone()),
            (3, Term::Integer(7)),
            (7, true_.clone())
        ]
    );
    assert_eq!(terms.get(&Term::Integer(39)), Some(&Term::Integer(2)));
    // A missing key that hashes to slot 7 walks past the end to the free slot 2.
    assert_eq!(terms.get(&Term::tuple(vec![ok.clone()])), None);

    // The fifth key finds the table half full and doubles it. The old keys go
    // again in old slot order: 39, ok, 7, true. {ok} and true both hash to slot 7.
    // The copy goes through the 16 new and the 8 old slots.
    let ok_tuple = Term::tuple(vec![ok.clone()]);
    assert_eq!(terms.put(&ok_tuple, &Term::Integer(5)), Some(24));
    assert_eq!(terms.buckets.len(), 16);
    assert_eq!(
        slots(&terms),
        [
            (0, ok.clone()),
            (3, Term::Integer(7)),
            (7, true_.clone()),
            (8, ok_tuple.clone()),
            (15, Term::Integer(39))
        ]
    );
    assert_eq!(terms.get(&ok_tuple), Some(&Term::Integer(5)));

    // A new value for a stored key keeps its slot and adds no key.
    terms.put(&ok, &Term::Integer(30));
    assert_eq!(terms.num_entries, 5);
    assert_eq!(slots(&terms)[0], (0, ok.clone()));
    assert_eq!(terms.get(&ok), Some(&Term::Integer(30)));
}

#[test]
fn erases_a_key_in_place_when_the_next_slot_is_free() {
    let (true_, ok) = (Term::static_atom("true"), Term::static_atom("ok"));
    let mut terms = PersistentTerms::default();
    terms.put(&true_, &Term::Integer(1));
    terms.put(&Term::Integer(39), &Term::Integer(2));
    terms.put(&ok, &Term::Integer(3));
    terms.put(&Term::Integer(7), &Term::Integer(4));
    // 7 is in slot 3, and slot 4 is free, so it is no stepping stone to
    // another key (`is_erasable`).
    assert!(terms.erase(&Term::Integer(7)));
    assert_eq!(
        slots(&terms),
        [(0, Term::Integer(39)), (1, ok.clone()), (7, true_.clone())]
    );
    assert_eq!(terms.num_entries, 3);
    // A key without a value erases nothing.
    assert!(!terms.erase(&Term::Integer(7)));
    assert_eq!(terms.num_entries, 3);
    assert_eq!(terms.get(&ok), Some(&Term::Integer(3)));
}

#[test]
fn rehashes_the_other_pairs_when_the_slot_is_a_stepping_stone() {
    let (true_, ok) = (Term::static_atom("true"), Term::static_atom("ok"));
    let mut terms = PersistentTerms::default();
    terms.put(&true_, &Term::Integer(1));
    terms.put(&Term::Integer(39), &Term::Integer(2));
    terms.put(&ok, &Term::Integer(3));
    terms.put(&Term::Integer(7), &Term::Integer(4));
    // true is in slot 7, and slot 0 holds 39, so the table is copied
    // without true and the other pairs go again in old slot order: 39, ok,
    // and 7.
    assert!(terms.erase(&true_));
    assert_eq!(
        slots(&terms),
        [
            (0, ok.clone()),
            (3, Term::Integer(7)),
            (7, Term::Integer(39))
        ]
    );
    assert_eq!(terms.num_entries, 3);
    assert_eq!(terms.get(&Term::Integer(39)), Some(&Term::Integer(2)));
}

#[test]
fn shrinks_the_table_when_the_erase_leaves_it_mostly_empty() {
    let (true_, ok) = (Term::static_atom("true"), Term::static_atom("ok"));
    let mut terms = PersistentTerms::default();
    terms.put(&true_, &Term::Integer(1));
    terms.put(&Term::Integer(39), &Term::Integer(2));
    terms.put(&ok, &Term::Integer(3));
    terms.put(&Term::Integer(7), &Term::Integer(4));
    let ok_tuple = Term::tuple(vec![ok.clone()]);
    terms.put(&ok_tuple, &Term::Integer(5));
    // Five keys in 16 slots: no shrink yet, and slot 1 is free.
    assert!(terms.erase(&ok));
    assert_eq!(terms.buckets.len(), 16);
    // Four keys in 16 slots meet `MUST_SHRINK` when 7 is erased: the copy
    // goes to 8 slots and the remaining pairs true, {ok}, and 39 go again
    // in old slot order.
    assert!(terms.erase(&Term::Integer(7)));
    assert_eq!(terms.buckets.len(), 8);
    assert_eq!(
        slots(&terms),
        [
            (0, ok_tuple.clone()),
            (1, Term::Integer(39)),
            (7, true_.clone())
        ]
    );
    assert_eq!(terms.get(&ok_tuple), Some(&Term::Integer(5)));
}

#[test]
fn counts_the_keys_and_values_of_the_table() {
    let mut terms = PersistentTerms::default();
    terms.put(&atom("key"), &Term::tuple(vec![Term::Integer(1)]));
    let mut live = crate::term::LiveBytes::new();
    terms.account_terms(&mut live);
    assert!(live.bytes() > 0);
}

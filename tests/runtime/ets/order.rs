use std::cmp::Ordering;
use std::sync::Arc;

use super::{KeyOrder, Locate, OrderKey, Probe};
use crate::term::{AtomTable, Reference, Term};

fn order(keys: &[Term]) -> (Arc<AtomTable>, KeyOrder) {
    let atoms = Arc::new(AtomTable::new());
    let mut order = KeyOrder::new(Arc::clone(&atoms));
    for key in keys {
        order.put(key.clone());
    }
    (atoms, order)
}

#[test]
fn keeps_one_key_for_keys_that_compare_equal() {
    // `1` and `1.0` are one key (`cmp_key_eq`, `erl_db_tree.c:758-761`), and
    // the put of the second key replaces the first.
    let (atoms, mut order) = order(&[Term::Integer(3), Term::Integer(1)]);
    order.put(Term::Float(1.0));
    let keys: Vec<_> = order.iter().cloned().collect();
    assert_eq!(keys, [Term::Float(1.0), Term::Integer(3)]);
    assert_eq!(
        order.find(&atoms, &Term::Integer(1)),
        Ok(Some(&Term::Float(1.0)))
    );
    assert_eq!(order.find(&atoms, &Term::Integer(2)), Ok(None));
}

#[test]
fn steps_and_splits_around_a_key_that_is_not_in_the_order() {
    let (atoms, order) = order(&[Term::Integer(1), Term::Integer(3), Term::Integer(5)]);
    let above: Vec<_> = order
        .beyond(&atoms, &Term::Float(2.0), true)
        .unwrap()
        .collect();
    assert_eq!(above, [&Term::Integer(3), &Term::Integer(5)]);
    let below: Vec<_> = order
        .beyond(&atoms, &Term::Integer(3), false)
        .unwrap()
        .collect();
    assert_eq!(below, [&Term::Integer(1)]);
    let split = order.split(|key| Ok(matches!(key, Term::Integer(number) if *number < 4)));
    assert_eq!(
        split,
        Ok((Some(&Term::Integer(3)), Some(&Term::Integer(5))))
    );
}

#[test]
fn keeps_a_key_for_each_alias_reference() {
    // A PBeam alias reference compares by its three reference numbers
    // (`src/term/order.rs`), so an ordered_set table holds two different
    // aliases as two keys.
    let alias = |number| Term::Reference(Reference(number).pid_reference());
    let (atoms, mut order) = order(&[alias(1)]);
    assert_eq!(order.find(&atoms, &alias(2)), Ok(None));
    order.put(alias(2));
    assert_eq!(order.len(), 2);
    let keys: Vec<_> = order.iter().cloned().collect();
    assert_eq!(keys, [alias(1), alias(2)]);
    order.remove(&atoms, &alias(1));
    assert_eq!(order.len(), 1);
    order.remove(&atoms, &alias(7));
    assert_eq!(order.len(), 1);
    order.clear();
    assert_eq!(order.len(), 0);
}

#[test]
#[should_panic(expected = "a probe is never a key of the order")]
fn never_reads_the_key_of_a_probe() {
    let probe = Probe::new(|_: &Term| Ok(Ordering::Less));
    let _ = Locate::term(&probe);
}

#[test]
fn compares_keys_and_probes_with_equality_and_partial_order() {
    let atoms = Arc::new(AtomTable::new());
    let key = |term| OrderKey {
        term,
        atoms: Arc::clone(&atoms),
    };
    let (one, float) = (key(Term::Integer(1)), key(Term::Float(1.0)));
    assert_eq!(one, float);
    assert_eq!(one.partial_cmp(&float), Some(Ordering::Equal));
    let (left, right): (&dyn Locate, &dyn Locate) = (&one, &float);
    assert!(left == right);
    assert_eq!(left.partial_cmp(right), Some(Ordering::Equal));
}

#[test]
fn gives_the_first_error_of_the_split_test() {
    // `split` gives the error of `below` (`KeyOrder::split`): the probe
    // keeps the first error of the search. Each key fails with its own
    // error, and the error of key 1, which the search compares first,
    // stays when the later keys fail too.
    let (_, order) = order(&[Term::Integer(1), Term::Integer(3), Term::Integer(5)]);
    let split = order.split(|key| Err(crate::error::BeamError::Runtime(format!("split {key:?}"))));
    assert_eq!(
        split,
        Err(crate::error::BeamError::Runtime(
            "split Integer(1)".to_owned()
        ))
    );
}

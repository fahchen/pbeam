use std::cmp::Ordering;
use std::sync::Arc;

use super::{KeyOrder, Locate, Probe};
use crate::term::{AtomTable, Reference, Term};

fn order(keys: &[Term]) -> (Arc<AtomTable>, KeyOrder) {
    let atoms = Arc::new(AtomTable::new());
    let mut order = KeyOrder::new(Arc::clone(&atoms));
    for key in keys {
        order.put(&atoms, key.clone());
    }
    (atoms, order)
}

#[test]
fn keeps_one_key_for_keys_that_compare_equal() {
    // `1` and `1.0` are one key (`cmp_key_eq`, `erl_db_tree.c:758-761`), and
    // the put of the second key replaces the first.
    let (atoms, mut order) = order(&[Term::Integer(3), Term::Integer(1)]);
    order.put(&atoms, Term::Float(1.0));
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
fn makes_the_order_stale_at_a_key_with_no_order() {
    // The order of two different alias references is not supported
    // (`src/term/order.rs`), so the put of the second one makes the order
    // stale, and each later search gives the runtime error.
    let alias = |number| Term::Reference(Reference(number).pid_reference());
    let (atoms, mut order) = order(&[alias(1)]);
    assert!(order.find(&atoms, &alias(2)).is_err());
    assert!(order.check().is_ok());
    order.put(&atoms, alias(2));
    assert!(order.check().is_err());
    order.remove(&atoms, &alias(1));
    assert_eq!(order.len(), 1);
    order.clear();
    assert!(order.check().is_ok());
    // A remove that cannot compare also makes the order stale.
    let (atoms, mut order) = super::tests::order(&[alias(1)]);
    order.remove(&atoms, &alias(2));
    assert!(order.check().is_err());
}

#[test]
#[should_panic(expected = "a probe is never a key of the order")]
fn never_reads_the_key_of_a_probe() {
    let probe = Probe::new(|_: &Term| Ok(Ordering::Less));
    let _ = Locate::term(&probe);
}

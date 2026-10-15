use super::{DELETE_DELAY, KnownNodes};
use crate::term::AtomTable;

#[test]
fn keeps_the_first_send_order_and_deletes_an_entry_after_the_delay() {
    let mut atoms = AtomTable::new();
    let a = atoms.put("a@h").unwrap();
    let b = atoms.put("b@h").unwrap();
    let c = atoms.put("c@h").unwrap();
    let mut known = KnownNodes::default();
    known.send(&a, 0);
    known.send(&b, 10);
    known.send(&a, 20);
    known.send(&c, 30);
    assert_eq!(known.names(30), vec![a.clone(), b.clone(), c.clone()]);
    // The timer of `a` ends at `DELETE_DELAY`. The send at 20 used the
    // entry, so the timer starts again and `a` stays.
    assert_eq!(
        known.names(DELETE_DELAY),
        vec![a.clone(), b.clone(), c.clone()]
    );
    assert_eq!(known.names(DELETE_DELAY + 10), vec![a.clone(), c.clone()]);
    assert_eq!(known.names(DELETE_DELAY + 30), vec![a.clone()]);
    assert_eq!(known.names(2 * DELETE_DELAY - 1), vec![a.clone()]);
    assert!(known.names(2 * DELETE_DELAY).is_empty());
}

#[test]
fn runs_the_timers_that_ended_before_a_send() {
    let mut atoms = AtomTable::new();
    let a = atoms.put("a@h").unwrap();
    let b = atoms.put("b@h").unwrap();
    let mut known = KnownNodes::default();
    known.send(&a, 0);
    known.send(&a, 1);
    known.send(&b, 1);
    // At `3 * DELETE_DELAY` the timer of `a` ended twice: the first end
    // starts it again, and the second end deletes the entry. A new send
    // makes a new entry after `b`.
    known.send(&a, 3 * DELETE_DELAY);
    assert_eq!(known.names(3 * DELETE_DELAY), vec![a.clone()]);
    known.send(&b, 3 * DELETE_DELAY);
    assert_eq!(known.names(3 * DELETE_DELAY), vec![a, b]);
}

use super::{DELETE_DELAY, KnownNodes};
use crate::term::AtomTable;

#[test]
fn keeps_the_first_send_order_and_deletes_an_entry_after_the_delay() {
    let atoms = AtomTable::new();
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
    assert_eq!(known.names(2 * DELETE_DELAY), [] as [crate::term::Atom; 0]);
}

#[test]
fn runs_the_timers_that_ended_before_a_send() {
    let atoms = AtomTable::new();
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

#[test]
fn finds_an_entry_among_many_nodes_without_a_walk_of_all_entries() {
    let atoms = AtomTable::new();
    let names: Vec<_> = (0..1000)
        .map(|n| atoms.put(&format!("n{n}@h")).unwrap())
        .collect();
    let mut known = KnownNodes::default();
    for (time, name) in (0..).zip(&names) {
        known.send(name, time);
    }
    // A send finds the entry of a node by its atom index, and runs no
    // timer that did not end.
    known.send(&names[500], 1000);
    known.send(&names[999], 1000);
    assert_eq!(known.steps, 0);
    assert_eq!(known.names(1000), names);
    // At `DELETE_DELAY + 10`, the timers of the first 11 entries ended, and
    // a send to the first node makes a new entry after all others.
    known.send(&names[0], DELETE_DELAY + 10);
    assert_eq!(known.steps, 11);
    let mut expected = names[11..].to_vec();
    expected.push(names[0].clone());
    assert_eq!(known.names(DELETE_DELAY + 10), expected);
    // The sends used the entries 500 and 999 after their timers started,
    // so only these timers start again when they end.
    assert_eq!(
        known.names(2 * DELETE_DELAY),
        vec![names[500].clone(), names[999].clone(), names[0].clone()]
    );
    assert_eq!(known.steps, 1000);
}

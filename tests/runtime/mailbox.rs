use super::{MARKER_SLOTS, Mailbox, Step};
use crate::runtime::signal::Signal;
use crate::term::test_atom;
use crate::term::{ProcessId, Reference, Term};

fn int(value: i64) -> Term {
    Term::Integer(value)
}

fn mailbox(values: &[i64]) -> Mailbox {
    let mut mailbox = Mailbox::default();
    for value in values {
        mailbox.push(int(*value));
    }
    mailbox.fetch();
    mailbox
}

fn link(from: u64) -> Signal {
    Signal::Link {
        from: ProcessId(from),
    }
}

#[test]
fn walks_and_removes_messages() {
    let mut mailbox = mailbox(&[1, 2, 3]);
    assert!(mailbox.save_next());
    assert_eq!(mailbox.current(), Some(&int(2)));
    assert!(mailbox.remove_current());
    assert_eq!(mailbox.messages(), vec![int(1), int(3)]);
    assert_eq!(mailbox.current(), Some(&int(1)));
    assert!(mailbox.save_next() && mailbox.save_next());
    assert!(!mailbox.save_next() && !mailbox.remove_current());

    // A message that arrives is not current before a fetch.
    mailbox.push(int(4));
    assert_eq!(mailbox.current(), None);
    assert!(!mailbox.save_next() && !mailbox.remove_current());
    mailbox.fetch();
    assert_eq!(mailbox.current(), Some(&int(4)));
    assert!(mailbox.remove_current());
    assert_eq!(mailbox.messages(), vec![int(1), int(3)]);
}

#[test]
fn counts_only_the_messages_of_the_inner_part() {
    // `mq_len` counts the messages of the inner part, and no receive marker
    // (`erl_proc_sig_queue.h:100-110`).
    let mut mailbox = Mailbox::default();
    assert!(!mailbox.has_messages() && !mailbox.has_incoming());
    assert_eq!(mailbox.reserve_marker(), test_atom("undefined"));
    mailbox.push(int(1));
    assert!(mailbox.has_incoming() && mailbox.has_outer_messages());
    assert!(!mailbox.has_messages() && !mailbox.has_control_signals());
    mailbox.reserve_marker();
    assert!(!mailbox.has_incoming() && !mailbox.has_outer_messages());
    mailbox.push(int(2));
    mailbox.fetch();
    assert_eq!(mailbox.message_count(), 2);
    assert!(mailbox.has_messages());
}

#[test]
fn fetches_into_the_middle_part_behind_a_signal() {
    // The outer part goes to the inner part only when the middle part is
    // empty and the outer part has no signal (`erl_proc_sig_queue.c:1378-1386`).
    let mut mailbox = mailbox(&[1]);
    mailbox.push_signal(link(5));
    mailbox.push(int(2));
    assert!(mailbox.has_control_signals() && !mailbox.has_middle());
    mailbox.fetch();
    assert!(mailbox.has_control_signals() && mailbox.has_middle());
    assert_eq!(mailbox.message_count(), 1);
    // A later fetch goes behind the signal too.
    mailbox.push(int(3));
    mailbox.fetch();
    assert_eq!(mailbox.message_count(), 1);
    assert!(mailbox.save_next());
    assert_eq!(mailbox.current(), None);

    // The pass moves the message before the signal into the inner part at
    // its end, and the save pointer at the end of the inner part then
    // points to it.
    let mut walk = mailbox.walk();
    let Some(Step::Signal(node, signal)) = mailbox.next_signal(&mut walk) else {
        panic!("the middle part starts with a signal");
    };
    assert_eq!(signal, link(5));
    assert_eq!(mailbox.messages_before(node), 1);
    mailbox.remove(node);
    assert!(mailbox.can_stretch(&walk));
    assert_eq!(mailbox.next_signal(&mut walk), None);
    mailbox.end_walk(walk);
    assert!(!mailbox.has_control_signals() && !mailbox.has_incoming());
    assert_eq!(mailbox.current(), Some(&int(2)));
    assert_eq!(mailbox.message_count(), 3);
}

#[test]
fn converts_a_signal_in_its_place_and_stops_before_the_next_signal() {
    let mut mailbox = Mailbox::default();
    mailbox.push(int(1));
    mailbox.push_signal(link(5));
    mailbox.push(int(3));
    mailbox.push_signal(Signal::Flush);
    mailbox.push(int(4));
    mailbox.fetch();
    assert_eq!(mailbox.message_count(), 0);

    let mut walk = mailbox.walk();
    let Some(Step::Signal(node, _)) = mailbox.next_signal(&mut walk) else {
        panic!("the middle part has a signal");
    };
    mailbox.convert(&mut walk, node, int(2));
    // The pass stops before the flush signal: the messages before it go to
    // the inner part, and the flush signal stays in the middle part.
    mailbox.end_walk(walk);
    assert_eq!(mailbox.messages(), vec![int(1), int(2), int(3), int(4)]);
    assert_eq!(mailbox.message_count(), 3);
    assert!(mailbox.has_control_signals());
    // A message after the save pointer stops a stretch of the limit
    // (`stretch_limit`, `erl_proc_sig_queue.c:7180-7256`).
    assert!(!mailbox.can_stretch(&mailbox.walk()));
    assert!(mailbox.save_next() && mailbox.save_next() && mailbox.save_next());
    assert!(mailbox.can_stretch(&mailbox.walk()));

    let mut walk = mailbox.walk();
    assert!(matches!(
        mailbox.next_signal(&mut walk),
        Some(Step::Signal(_, Signal::Flush))
    ));
}

#[test]
fn starts_a_receive_after_a_bound_marker() {
    let mut mailbox = Mailbox::default();
    assert_eq!(mailbox.reserve_marker(), test_atom("undefined"));
    mailbox.push(int(1));
    let marker = mailbox.reserve_marker();
    assert_eq!(marker, int(-(1 << 59)));
    let reference = Term::Reference(Reference(7));
    mailbox.bind_marker(&int(5), &reference);
    mailbox.bind_marker(&reference, &reference);
    mailbox.bind_marker(&marker, &reference);
    mailbox.push(int(2));
    mailbox.use_marker(&int(5));
    mailbox.use_marker(&Term::Reference(Reference(8)));
    assert_eq!(mailbox.current(), Some(&int(1)));
    mailbox.use_marker(&reference);
    assert_eq!(mailbox.current(), None);
    mailbox.fetch();
    assert_eq!(mailbox.current(), Some(&int(2)));

    assert!(mailbox.remove_current());
    mailbox.clear_marker(&test_atom("other"));
    assert!(mailbox.markers[0].id.is_some());
    mailbox.clear_marker(&reference);
    assert!(mailbox.save_next());
    mailbox.set_save_first();
    assert_eq!(mailbox.current(), Some(&int(1)));
    assert!(mailbox.remove_current());
    assert_eq!(mailbox.first, None);
    assert!(mailbox.markers.is_empty());
}

#[test]
fn moves_the_save_pointer_past_a_marker_at_the_end() {
    let mut mailbox = mailbox(&[1]);
    assert!(mailbox.save_next());
    let marker = mailbox.reserve_marker();
    mailbox.push(int(2));
    mailbox.fetch();
    assert_eq!(mailbox.current(), Some(&int(2)));
    mailbox.bind_marker(&marker, &test_atom("none"));
    assert!(mailbox.markers[0].id.is_none());

    // A marker at the front of the queue leaves when a receive ends.
    mailbox.set_save_first();
    assert!(mailbox.remove_current());
    assert!(mailbox.markers.is_empty());
    assert_eq!(mailbox.current(), Some(&int(2)));
}

#[test]
fn reserves_a_marker_after_the_messages_that_arrived() {
    // `erts_msgq_recv_marker_insert` fetches before it inserts the
    // marker (`erl_proc_sig_queue.h:2226-2236`).
    let mut mailbox = mailbox(&[1]);
    mailbox.push(int(2));
    let marker = mailbox.reserve_marker();
    let reference = Term::Reference(Reference(3));
    mailbox.bind_marker(&marker, &reference);
    mailbox.push(int(3));
    mailbox.use_marker(&reference);
    assert_eq!(mailbox.current(), None);
    mailbox.fetch();
    assert_eq!(mailbox.current(), Some(&int(3)));

    // A removed node at the fetch boundary moves the boundary back.
    mailbox.set_save_first();
    assert!(mailbox.remove_current() && mailbox.remove_current());
    assert!(mailbox.remove_current());
    assert!(mailbox.markers.is_empty());
    mailbox.push(int(4));
    assert_eq!(mailbox.current(), None);
}

#[test]
fn moves_the_save_pointer_past_a_marker_of_the_middle_part_when_it_enters() {
    // A marker goes to the middle part when the middle part is not empty
    // (`recv_marker_insert`, `erl_proc_sig_queue.c:4290-4348`). A use of the
    // marker then waits until the handling moves it into the inner part
    // (`erl_proc_sig_queue.h:2121-2148`, `erl_proc_sig_queue.c:6918-6941`).
    let mut mailbox = mailbox(&[1]);
    mailbox.push_signal(link(5));
    mailbox.push(int(2));
    let marker = mailbox.reserve_marker();
    let reference = Term::Reference(Reference(3));
    mailbox.bind_marker(&marker, &reference);
    mailbox.push(int(3));
    mailbox.fetch();
    mailbox.use_marker(&reference);
    assert_eq!(mailbox.current(), None);

    let mut walk = mailbox.walk();
    let Some(Step::Signal(node, _)) = mailbox.next_signal(&mut walk) else {
        panic!("the middle part starts with a signal");
    };
    mailbox.remove(node);
    assert_eq!(mailbox.next_signal(&mut walk), Some(Step::Marker));
    assert!(mailbox.can_stretch(&walk));
    assert_eq!(mailbox.next_signal(&mut walk), None);
    mailbox.end_walk(walk);
    assert_eq!(mailbox.current(), Some(&int(3)));
    assert!(!mailbox.has_incoming());

    // When all slots are in use and the least recently allocated marker is
    // in the middle part, no marker is made (`recv_marker_reuse`,
    // `erl_proc_sig_queue.c:4174-4176`).
    let mut full = mailbox_with_signal();
    for _ in 0..MARKER_SLOTS {
        assert!(matches!(full.reserve_marker(), Term::Integer(_)));
    }
    assert_eq!(full.reserve_marker(), test_atom("undefined"));

    // The search for a cleared marker stops at the first marker of the
    // middle part and uses the least recently allocated marker.
    let mut reused = self::mailbox(&[1]);
    let oldest = reused.reserve_marker();
    reused.push_signal(link(5));
    reused.fetch();
    for _ in 1..MARKER_SLOTS {
        reused.reserve_marker();
    }
    reused.reserve_marker();
    assert!(reused.marker(&oldest).is_none());
    assert_eq!(reused.markers.len(), MARKER_SLOTS);
}

fn mailbox_with_signal() -> Mailbox {
    let mut mailbox = mailbox(&[1]);
    mailbox.push_signal(link(5));
    mailbox.fetch();
    mailbox
}

#[test]
fn sets_the_save_pointer_to_the_end_of_the_queue() {
    // `erts_msgq_set_save_end` (`erl_proc_sig_queue.h:2397-2421`) sets the
    // save pointer to the end of the inner part, or passes a marker through
    // the middle part that moves it there.
    let mut mailbox = mailbox(&[1]);
    mailbox.push(int(2));
    mailbox.set_save_end();
    assert_eq!(mailbox.current(), None);
    assert_eq!(mailbox.message_count(), 2);

    let mut mailbox = mailbox_with_signal();
    mailbox.push(int(2));
    mailbox.set_save_end();
    mailbox.push(int(3));
    mailbox.fetch();
    let mut walk = mailbox.walk();
    let Some(Step::Signal(node, _)) = mailbox.next_signal(&mut walk) else {
        panic!("the middle part starts with a signal");
    };
    mailbox.remove(node);
    assert_eq!(mailbox.next_signal(&mut walk), Some(Step::Marker));
    assert!(mailbox.markers.is_empty());
    mailbox.end_walk(walk);
    assert_eq!(mailbox.current(), Some(&int(3)));
    assert_eq!(mailbox.messages(), vec![int(1), int(2), int(3)]);

    // A later use of a marker clears the wait of the set-save marker.
    let mut mailbox = mailbox_with_signal();
    mailbox.set_save_end();
    let marker = mailbox.reserve_marker();
    let reference = Term::Reference(Reference(4));
    mailbox.bind_marker(&marker, &reference);
    mailbox.use_marker(&reference);
    assert!(!mailbox.markers[0].set_save && mailbox.markers[1].set_save);
}

#[test]
fn reuses_marker_slots_and_drops_passed_markers() {
    let mut mailbox = mailbox(&[0]);
    let oldest = mailbox.reserve_marker();
    mailbox.push(int(1));
    let markers: Vec<Term> = (1..MARKER_SLOTS)
        .map(|_| mailbox.reserve_marker())
        .collect();
    let reference = Term::Reference(Reference(0));
    mailbox.bind_marker(&oldest, &reference);
    mailbox.use_marker(&reference);
    assert_eq!(mailbox.current(), Some(&int(1)));

    // The oldest marker holds the save pointer. Its reuse moves the
    // save pointer back to the same message.
    mailbox.reserve_marker();
    assert_eq!(mailbox.current(), Some(&int(1)));
    assert!(
        mailbox
            .markers
            .iter()
            .all(|marker| marker.id != Some(reference.clone()))
    );

    // A cleared marker is used again before the oldest marker.
    mailbox.clear_marker(&markers[3]);
    mailbox.reserve_marker();
    assert_eq!(mailbox.markers.len(), MARKER_SLOTS);
    assert!(mailbox.markers.iter().all(|marker| marker.id.is_some()));
    assert_eq!(mailbox.markers[0].id, Some(markers[0].clone()));

    // A marker leaves the queue after more than four passes.
    for _ in 0..5 {
        mailbox.set_save_first();
        assert!(mailbox.save_next() && mailbox.save_next());
    }
    assert!(mailbox.markers.is_empty());
    assert_eq!(mailbox.messages(), vec![int(0), int(1)]);
}

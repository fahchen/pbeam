//! The shared node of a multi-scheduler run.

use super::{Node, ReceiverSite};
use crate::term::{PortId, ProcessId, Term};

/// The port counter gives the next identifier, and its end is the overflow
/// error.
#[test]
fn a_port_counter_that_ends_gives_the_overflow_error() {
    let (node, _readers) = Node::new(1).unwrap();
    assert_eq!(node.next_port().unwrap(), PortId(0));
    node.next_port
        .store(u64::MAX, std::sync::atomic::Ordering::Relaxed);
    let error = node.next_port().unwrap_err();
    assert_eq!(error.to_string(), "port identifier overflow");
}

/// A name registration requires a live owner.
#[test]
fn a_name_registration_requires_a_live_owner() {
    let (node, _readers) = Node::new(1).unwrap();
    assert!(!node.name_register("pbeam_dead_name", ProcessId(99)));
}

/// An exiting process has its owner, but no live owner, as
/// `erts_proc_lookup` finds no process with the `EXITING` flag.
#[test]
fn an_exiting_process_has_no_live_owner() {
    let (node, _readers) = Node::new(2).unwrap();
    node.register(ProcessId(7), 1);
    assert_eq!(node.live_owner(ProcessId(7)), Some(1));
    node.set_exiting(ProcessId(7));
    assert_eq!(node.owner(ProcessId(7)), Some(1));
    assert_eq!(node.live_owner(ProcessId(7)), None);
}

/// `setnode/2` gives `badarg` for a `net_kernel` that exits, as
/// `setnode_2` checks `ERTS_PROC_IS_EXITING` (`dist.c:5020-5021`). Only a
/// race between the name lookup and the exit reaches this check, so the
/// test marks the process as exiting directly. The exit of the owner gives
/// the node its name back, and the exit of another process does not.
#[test]
fn a_net_kernel_that_exits_gets_no_node_name() {
    let atoms = crate::term::AtomTable::new();
    let name = atoms.put("a@host").unwrap();
    let nonode = (crate::term::Atom::from_static("nonode@nohost").unwrap(), 0);
    let (node, _readers) = Node::new(1).unwrap();
    node.register(ProcessId(7), 0);
    node.set_exiting(ProcessId(7));
    assert!(!node.set_this_node(&atoms, ProcessId(7), &name, 4));
    assert_eq!(atoms.this_node(), nonode);
    node.register(ProcessId(8), 0);
    assert!(node.set_this_node(&atoms, ProcessId(8), &name, 4));
    node.net_exits(ProcessId(7), &atoms);
    assert_eq!(atoms.this_node(), (name, 4));
    node.net_exits(ProcessId(8), &atoms);
    assert_eq!(atoms.this_node(), nonode);
}

/// The node counts its roots: the run ends after the exit of the last root
/// completes, and each result keeps the slot of its root.
#[test]
fn the_run_ends_after_the_exit_of_the_last_root() {
    let (node, _readers) = Node::new(2).unwrap();
    node.add_root(1);
    node.add_root(0);
    node.root_result(1, Ok(Term::Integer(1)));
    assert!(!node.is_shutdown());
    node.root_exit(1);
    assert!(!node.is_shutdown());
    node.root_result(0, Ok(Term::Integer(0)));
    node.root_exit(0);
    assert!(node.is_shutdown());
    assert_eq!(
        node.take_root_results(),
        [Some(Ok(Term::Integer(0))), Some(Ok(Term::Integer(1)))]
    );
}

/// A root that fails ends the run while another root is still alive.
#[test]
fn a_failed_root_ends_the_run_while_another_root_lives() {
    let (node, _readers) = Node::new(2).unwrap();
    node.add_root(0);
    node.add_root(1);
    node.root_result(1, Err(Term::static_atom("badarg")));
    node.root_exit(1);
    assert!(node.is_shutdown());
    assert_eq!(
        node.take_root_results(),
        [None, Some(Err(Term::static_atom("badarg")))]
    );
}

/// A root needs a slot of the node.
#[test]
#[should_panic(expected = "the node has the root slot")]
fn a_root_needs_a_slot_of_the_node() {
    Node::single().0.add_root(1);
}

/// The last scheduler that becomes idle decides the impossible wait, and a
/// scheduler that still runs prevents it.
#[test]
fn the_last_idle_scheduler_decides_the_wait() {
    let (node, _readers) = Node::new(2).unwrap();
    assert!(!node.enter_idle(), "the other scheduler still runs");
    assert!(node.enter_idle(), "each scheduler is idle");
}

/// A sender puts its delivery into the inbox of an idle receiver and then
/// becomes idle. The delivery in flight prevents the decision, and the
/// receiver decides after it took the delivery and became idle again.
#[test]
fn a_delivery_in_flight_prevents_the_decision() {
    let (node, _readers) = Node::new(2).unwrap();
    assert!(!node.enter_idle(), "the receiver waits");
    node.send_message(0, Some(ProcessId(1)), Term::Integer(1), ProcessId(0), None);
    assert!(!node.enter_idle(), "the delivery waits in the inbox");
    node.leave_idle();
    let mut deliveries = Vec::new();
    node.take(0, &mut deliveries);
    assert_eq!(deliveries.len(), 1);
    assert!(node.enter_idle(), "no delivery waits");
}

/// A receiver that woke leaves the idle state before it takes its inbox, so
/// a sender that becomes idle while the receiver handles the delivery does
/// not decide.
#[test]
fn a_scheduler_that_left_the_idle_state_prevents_the_decision() {
    let (node, _readers) = Node::new(2).unwrap();
    assert!(!node.enter_idle());
    node.leave_idle();
    assert!(!node.enter_idle(), "the receiver handles its delivery");
}

/// The last idle scheduler holds the idle count while it reads the inboxes,
/// so no scheduler can leave the idle state and take a delivery during the
/// check.
#[test]
fn the_check_of_the_inboxes_holds_the_idle_count() {
    let (node, _readers) = Node::new(1).unwrap();
    let inbox = node.inboxes[0].queue.lock().unwrap();
    std::thread::scope(|scope| {
        let decider = scope.spawn(|| node.enter_idle());
        // The decider holds the idle count and waits for the inbox.
        while node.idle.try_lock().is_ok() {
            std::thread::yield_now();
        }
        assert!(node.idle.try_lock().is_err(), "a leave waits for the check");
        drop(inbox);
        assert!(decider.join().unwrap());
    });
    node.leave_idle();
    assert_eq!(*node.idle.lock().unwrap(), 0);
}

/// A timer for a receiver of another scheduler requires a live receiver.
/// A cancel after the exit start keeps the timer in the tree of the
/// receiver for its exit.
#[test]
fn a_remote_timer_requires_a_live_receiver() {
    let (node, _readers) = Node::new(2).unwrap();
    let receiver = ProcessId(7);
    let reference = crate::term::Reference::ordinary(1).unwrap();
    node.register(receiver, 1);
    assert!(node.register_timer(reference, 0, 1100, Some(receiver), ReceiverSite::Remote));
    assert_eq!(node.timer_end(reference), Some(1100));
    node.set_exiting(receiver);
    let late = crate::term::Reference::ordinary(2).unwrap();
    assert!(!node.register_timer(late, 0, 1100, Some(receiver), ReceiverSite::Remote));
    assert_eq!(node.timer_end(late), None);
    // A cancel between the exit start and the take leaves the tree entry
    // for the exit to count.
    assert_eq!(node.take_timer(reference), Some((0, 1100)));
    assert_eq!(node.take_timer(reference), None);
    assert!(!node.take_receiver_timers(receiver).is_empty());
    assert!(node.take_receiver_timers(receiver).is_empty());
}

//! The shared node of a multi-scheduler run.

use super::Node;
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
    Node::single().add_root(1);
}

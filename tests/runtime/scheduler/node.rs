//! The shared node of a multi-scheduler run.

use super::Node;
use crate::term::{PortId, ProcessId};

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

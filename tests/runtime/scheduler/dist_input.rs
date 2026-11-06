//! Connection state that a fixture cannot inspect.

use std::sync::Arc;

use super::Scheduler;
use crate::{
    runtime::scheduler::{dist_table::Controller, node::Node},
    term::{AtomTable, ProcessId, Reference},
};

// ERTS decodes the control before it stores a sequence (dist.c:2084-2093).
#[test]
fn rejects_a_control_term_split_between_fragments() {
    let (node, mut readers) = Node::new(1).unwrap();
    let scheduler = Scheduler::with_shared(Arc::new(AtomTable::new()), node, 0, readers.remove(0));
    let name = scheduler.atoms.put("split_control@host").unwrap();
    let connection = {
        let mut dist = scheduler.node.dist();
        dist.insert(&name, Reference(8), 0);
        dist.new_connection(&name);
        dist.connect(&name, Controller::Process(ProcessId(0)), 0)
            .unwrap();
        dist.connected(&name).unwrap().2
    };
    let mut first = vec![131, b'E'];
    first.extend(3_u64.to_be_bytes());
    first.extend(2_u64.to_be_bytes());
    first.extend([0, 97]); // Empty atom table and an incomplete small integer.
    assert!(scheduler.dist_fragment(&name, connection, &first).is_err());
    assert!(
        scheduler
            .node
            .dist()
            .fragments_in(&name, connection)
            .unwrap()
            .is_empty()
    );
}

// The fixture can observe a disconnect, but cannot inspect the sequence map.
#[test]
fn clears_fragment_sequences_when_a_connection_is_rejected() {
    let (node, mut readers) = Node::new(1).unwrap();
    let mut scheduler =
        Scheduler::with_shared(Arc::new(AtomTable::new()), node, 0, readers.remove(0));
    let name = scheduler.atoms.put("rejected_fragments@host").unwrap();
    let connection = {
        let mut dist = scheduler.node.dist();
        dist.insert(&name, Reference(9), 0);
        dist.new_connection(&name);
        dist.connect(&name, Controller::Process(ProcessId(0)), 0)
            .unwrap();
        dist.connected(&name).unwrap().2
    };
    let mut first = vec![131, b'E'];
    first.extend(3_u64.to_be_bytes());
    first.extend(3_u64.to_be_bytes());
    first.extend([0, 97, 1]);
    scheduler
        .net_message(&name, connection, super::FRAGMENTS, &first)
        .unwrap();
    let mut wrong = vec![131, b'F'];
    wrong.extend(3_u64.to_be_bytes());
    wrong.extend(1_u64.to_be_bytes());
    scheduler
        .net_message(&name, connection, super::FRAGMENTS, &wrong)
        .unwrap();
    let mut dist = scheduler.node.dist();
    assert!(dist.connected(&name).is_none());
    dist.new_connection(&name);
    dist.connect(&name, Controller::Process(ProcessId(0)), 0)
        .unwrap();
    let next = dist.connected(&name).unwrap().2;
    assert!(dist.fragments_in(&name, next).unwrap().is_empty());
}

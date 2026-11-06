//! Fragment sequences with atom cache updates interleaved between messages.

use std::sync::Arc;

use super::{Scheduler, decode_dist};
use crate::{
    runtime::scheduler::{dist_table::Controller, node::Node},
    term::{AtomTable, ProcessId, Reference, Term},
};

// A compiled fixture cannot force the fragment order at the input boundary.
#[test]
fn reassembles_interleaved_fragments_with_their_own_atom_tables() {
    let (node, mut readers) = Node::new(1).unwrap();
    let scheduler = Scheduler::with_shared(Arc::new(AtomTable::new()), node, 0, readers.remove(0));
    let name = scheduler.atoms.put("fragment_peer@host").unwrap();
    let connection = {
        let mut dist = scheduler.node.dist();
        dist.insert(&name, Reference(7), 0);
        dist.new_connection(&name);
        dist.connect(&name, Controller::Process(ProcessId(0)), 0)
            .unwrap();
        dist.connected(&name).unwrap().2
    };
    let first = |sequence: u64, atom: &[u8]| {
        let mut packet = vec![131, b'E'];
        packet.extend(sequence.to_be_bytes());
        packet.extend(2_u64.to_be_bytes());
        packet.extend([1, 8, 0, u8::try_from(atom.len()).unwrap()]);
        packet.extend(atom);
        packet.extend([97, 1, 82]);
        packet
    };
    let last = |sequence: u64| {
        let mut packet = vec![131, b'F'];
        packet.extend(sequence.to_be_bytes());
        packet.extend(1_u64.to_be_bytes());
        packet.push(0);
        packet
    };
    for order in [[(1, "alpha"), (2, "beta")], [(2, "beta"), (1, "alpha")]] {
        assert!(
            scheduler
                .dist_fragment(&name, connection, &first(1, b"alpha"))
                .unwrap()
                .is_none()
        );
        assert!(
            scheduler
                .dist_fragment(&name, connection, &first(2, b"beta"))
                .unwrap()
                .is_none()
        );
        for (sequence, expected) in order {
            let (bytes, atoms) = scheduler
                .dist_fragment(&name, connection, &last(sequence))
                .unwrap()
                .unwrap();
            let (control, used) = decode_dist(&scheduler, &bytes, Some(&atoms))
                .unwrap()
                .unwrap();
            assert_eq!(control, Term::Integer(1));
            let (term, _) = decode_dist(&scheduler, &bytes[used..], Some(&atoms))
                .unwrap()
                .unwrap();
            assert_eq!(term, Term::Atom(scheduler.atoms.put(expected).unwrap()));
        }
    }
}

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

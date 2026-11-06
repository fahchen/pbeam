//! Distribution port queue states that cannot be forced by a BEAM fixture.

use std::sync::Arc;

use socket2::{Domain, Socket, Type};

use super::{Controller, Scheduler};
use crate::{
    runtime::{
        port::{
            Driver,
            inet::{Inet, SocketType},
        },
        process::{ProcessState, Start, tests::frame},
        scheduler::node::Node,
    },
    term::{AtomTable, Binary, ProcessId, Reference},
};

fn scheduler() -> Scheduler {
    let (node, mut readers) = Node::new(1).unwrap();
    let mut scheduler =
        Scheduler::with_shared(Arc::new(AtomTable::new()), node, 0, readers.remove(0));
    scheduler.insert(ProcessState::new(
        ProcessId(0),
        Start::Entry(frame()),
        vec![],
    ));
    scheduler.schedule_next().unwrap();
    scheduler
}

#[test]
fn drops_a_queued_drain_after_the_controller_is_detached() {
    // Another scheduler can detach a connection while this scheduler waits
    // for its port lock. A BEAM fixture cannot force that interleaving.
    let mut scheduler = scheduler();
    let port = scheduler
        .open_port(
            b"tcp_inet".to_vec(),
            Driver::Inet(Inet::new(SocketType::Stream)),
        )
        .unwrap();
    let node = scheduler.atoms.put("detached_controller@host").unwrap();
    {
        let mut dist = scheduler.node.dist();
        dist.insert(&node, Reference(8), 0);
        dist.new_connection(&node);
        let controller = Controller::Port(port);
        assert!(dist.connect(&node, controller, 0).is_some());
        assert!(dist.controller_exits(controller).is_none());
        assert!(dist.connected(&node).is_none());
    }
    scheduler.dist_port_send(port).unwrap();
    assert!(scheduler.node.port(port).is_some());
}

#[test]
fn closes_a_distribution_port_when_the_write_fails() {
    // The peer closes after the packet queues but before the port writes it.
    // The BEAM fixture has no way to hold the port at that boundary.
    let mut scheduler = scheduler();
    let (socket, peer) = Socket::pair(Domain::UNIX, Type::STREAM, None).unwrap();
    let mut inet = Inet::new(SocketType::Stream).copy(socket);
    inet.control(0x03f1_a307, &[22, 0, 0, 0, 1], ProcessId(0))
        .unwrap();
    let port = scheduler
        .open_port(b"tcp_inet".to_vec(), Driver::Inet(inet))
        .unwrap();
    let node = scheduler.atoms.put("write_failure@host").unwrap();
    {
        let mut dist = scheduler.node.dist();
        dist.insert(&node, Reference(7), 0);
        dist.new_connection(&node);
        assert!(dist.connect(&node, Controller::Port(port), 0).is_some());
        dist.enqueue(&node, vec![Binary::from(vec![1])]);
    }
    drop(peer);
    scheduler.dist_port_send(port).unwrap();
    assert!(scheduler.node.dist().connected(&node).is_none());
}

#[test]
fn fragments_an_oversized_distribution_packet() {
    let mut packet = vec![131, b'D'];
    packet.extend(vec![42; 65537]);
    let fragments = super::fragment_packet(vec![Binary::from(packet)], ProcessId(10), 2, 0);
    assert_eq!(fragments.len(), 2);
    let first = fragments[0][0].to_bytes();
    let last = fragments[1][0].to_bytes();
    assert_eq!(
        &first[..18],
        &[131, b'E', 0, 0, 0, 0, 0, 0, 5, 3, 0, 0, 0, 0, 0, 0, 0, 2]
    );
    assert_eq!(
        &last[..18],
        &[131, b'F', 0, 0, 0, 0, 0, 0, 5, 3, 0, 0, 0, 0, 0, 0, 0, 1]
    );
    assert_eq!(last[18..], [42]);
}

#[test]
fn keeps_large_distribution_header_and_control_in_the_first_fragment() {
    for (header, control, payload) in [(70_000, 3, 70_000), (3, 70_000, 3)] {
        let mut packet = vec![42; header + control + payload];
        packet[..2].copy_from_slice(&[131, b'D']);
        let fragments = super::fragment_packet(
            vec![Binary::from(packet.clone())],
            ProcessId(10),
            header,
            control,
        );
        assert_eq!(fragments.len(), 2);
        let first = fragments[0][0].to_bytes();
        assert!(first.len() >= 18 + header - 2 + control);
        assert_eq!(
            &first[18..18 + header - 2 + control],
            &packet[2..header + control]
        );
    }
}

#[test]
fn keeps_the_encoded_long_atom_cache_header_in_the_first_fragment() {
    let scheduler = scheduler();
    let atoms = (0..160)
        .map(|i| {
            let name = format!("{}{i}", "\u{800}".repeat(200));
            crate::term::Term::Atom(scheduler.atoms.put(&name).unwrap())
        })
        .collect();
    let control = crate::term::Term::tuple(atoms);
    let message = crate::term::Term::Binary(Binary::from(vec![42; 70_000]));
    let (elements, header, control_len) = super::encode_dist_cached(
        &scheduler,
        &control,
        Some(&message),
        &mut std::collections::HashMap::new(),
    )
    .unwrap();
    assert!(header > 65_536, "header length: {header}");
    let original = elements[0].to_bytes();
    let fragments = super::fragment_packet(elements, ProcessId(10), header, control_len);
    let first = fragments[0][0].to_bytes();
    assert_eq!(&first[18..18 + header - 2], &original[2..header]);
}

//! The steps of a `tcp_inet` port run under the port lock of the BIF of
//! the caller, and their messages go after it, when another scheduler can
//! close the port.

use std::sync::Arc;

use socket2::{Domain, Socket, Type};

use super::super::{Scheduler, node::Node};
use crate::runtime::port::inet::{Inet, Receiver, SocketType};
use crate::runtime::process::{ProcessState, Start, tests::frame};
use crate::term::{AtomTable, PortId, ProcessId};

/// Return a scheduler of a node with one scheduler and one current process.
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
fn receives_on_the_locked_descriptor_without_the_port_table() {
    // The step takes the descriptor that the control locked
    // (`erts_port_control`, io.c:4268-4420): port 7 is not in the table.
    let (local, _peer) = Socket::pair(Domain::UNIX, Type::STREAM, None).unwrap();
    let mut tcp = Inet::new(SocketType::Stream).copy(local);
    let (reply, events) = scheduler().tcp_receive(PortId(7), &mut tcp, u32::MAX, 0);
    let id = u16::from_be_bytes([reply[1], reply[2]]);
    assert_eq!((reply[0], events), (1, Vec::new()));
    assert_eq!(
        tcp.stream.receiver,
        Some(Receiver {
            id,
            caller: ProcessId(0),
            timer: None
        })
    );
}

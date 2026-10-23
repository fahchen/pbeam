//! The driver steps of a `tcp_inet` port that another scheduler closed
//! after the port lock of the BIF of the caller.

use std::sync::Arc;

use super::super::{Scheduler, node::Node};
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
fn activates_a_port_that_another_scheduler_closed_without_a_step() {
    // ERTS runs the control under one port lock (`erts_port_control`,
    // io.c:4268), so the activation comes before the close.
    assert_eq!(scheduler().tcp_activate(PortId(7), true).unwrap(), vec![1]);
}

#[test]
fn drops_the_output_of_a_port_that_another_scheduler_closed() {
    // The close drops the queue of the port, as for an output that came
    // before the close (`erts_port_output`, io.c:1715).
    scheduler().tcp_send(PortId(7), &[1, 2]).unwrap();
}

//! Connection changes that a BEAM fixture cannot schedule at packet delivery.

use std::sync::Arc;

use super::{Controller, Scheduler};
use crate::{
    runtime::scheduler::node::Node,
    term::{AtomTable, ProcessId, Reference},
};

#[test]
fn ignores_fragments_from_a_detached_or_replaced_connection() {
    // A fixture cannot force the connection change during packet delivery.
    // These calls supply the same node and connection ids as that race.
    let (node, mut readers) = Node::new(1).unwrap();
    let mut scheduler =
        Scheduler::with_shared(Arc::new(AtomTable::new()), node, 0, readers.remove(0));
    let name = scheduler.atoms.put("fragment_race@host").unwrap();
    scheduler.reject_fragment_connection(&name, 0);
    let first = [131, b'E', 0, 0, 0, 0, 0, 0, 0, 1, 0, 0, 0, 0, 0, 0, 0, 1, 0];
    let continuation = [131, b'F', 0, 0, 0, 0, 0, 0, 0, 1, 0, 0, 0, 0, 0, 0, 0, 1];
    assert!(scheduler.dist_fragment(&name, 0, &first).unwrap().is_none());
    assert!(
        scheduler
            .dist_fragment(&name, 0, &continuation)
            .unwrap()
            .is_none()
    );
    let connection = {
        let mut dist = scheduler.node.dist();
        dist.insert(&name, Reference(8), 0);
        dist.new_connection(&name);
        dist.connect(&name, Controller::Process(ProcessId(1)), 0)
            .unwrap();
        dist.connected(&name).unwrap().2
    };
    let stale = connection.wrapping_add(1);
    scheduler.reject_fragment_connection(&name, stale);
    assert!(
        scheduler
            .dist_fragment(&name, stale, &first)
            .unwrap()
            .is_none()
    );
    assert!(
        scheduler
            .dist_fragment(&name, stale, &continuation)
            .unwrap()
            .is_none()
    );
    // A fixture cannot force a replacement between decoding and storage.
    // This call supplies the same connection id that the race would leave.
    assert!(
        scheduler
            .store_first_fragment(&name, stale, 1, 2, vec![97, 42], vec![])
            .unwrap()
            .is_none()
    );
    // A guard that only returns `None` would also pass the assertion above, so
    // check that the stale call stored nothing on the active connection.
    assert!(
        scheduler
            .node
            .dist()
            .fragments_in(&name, connection)
            .expect("the active connection still exists")
            .is_empty()
    );
    assert_eq!(
        scheduler.node.dist().connected(&name).unwrap().2,
        connection
    );
}

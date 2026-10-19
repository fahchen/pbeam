//! The shared node of a multi-scheduler run.

use super::SharedNode;
use crate::term::PortId;

/// The port counter gives the next identifier, and its end is the overflow
/// error.
#[test]
fn a_port_counter_that_ends_gives_the_overflow_error() {
    let (node, _readers) = SharedNode::new(1).unwrap();
    assert_eq!(node.next_port().unwrap(), PortId(0));
    node.next_port
        .store(u64::MAX, std::sync::atomic::Ordering::Relaxed);
    let error = node.next_port().unwrap_err();
    assert_eq!(error.to_string(), "port identifier overflow");
}

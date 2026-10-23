use super::{Direction, NodePoll};
use crate::{runtime::scheduler::node::Node, term::PortId};
use std::{
    os::{fd::AsRawFd, unix::net::UnixStream},
    sync::atomic::Ordering,
};

/// Wait until the thread of `poll` starts its loop once more: it clears
/// the change mark at the start of each pass. After the register, nothing
/// writes the wake socket, so only a wait that ends by itself starts the
/// next pass.
fn wait_once(poll: &NodePoll) {
    poll.changed.store(true, Ordering::SeqCst);
    while poll.changed.load(Ordering::SeqCst) {
        std::thread::yield_now();
    }
}

#[test]
fn ends_the_wait_on_an_fd_that_a_port_closed_after_the_collect() {
    // A port can close its fd after the thread collected it and before the
    // wait. The wait then reports the fd as invalid, and the thread waits
    // again without it. The highest of a block of open fds
    // builds that state: a new fd of another test takes the lowest free fd
    // (POSIX), so it does not take the closed fd while the block is open.
    // The thread starts first: its wake socket would take the closed fd.
    let (node, _wake) = Node::single();
    NodePoll::start(&node);
    let mut held: Vec<_> = (0..32).map(|_| UnixStream::pair().expect("pair")).collect();
    let closed = held.pop().expect("a pair").0.as_raw_fd();
    node.poll
        .register(PortId(1), 0, Direction::Read, None, Some(closed));
    // The register can wake the first pass; the second pass comes after a
    // wait on the invalid fd.
    wait_once(&node.poll);
    wait_once(&node.poll);
    node.poll
        .register(PortId(1), 0, Direction::Read, Some(closed), None);
    assert!(node.poll.take_ready(0).is_empty());
    drop(held);
}

#[test]
fn collects_again_when_an_fd_changed_during_the_collect() {
    // A port can register an fd while the thread collects the fds. The
    // thread then collects again before it waits, or it would wait without
    // the new fd. The test holds the fd table, so the thread waits in its
    // collect while the change comes.
    let (node, _wake) = Node::single();
    NodePoll::start(&node);
    let table = node.poll.fds();
    node.poll.changed();
    while node.poll.changed.load(Ordering::SeqCst) {
        std::thread::yield_now();
    }
    node.poll.changed.store(true, Ordering::SeqCst);
    drop(table);
    while node.poll.changed.load(Ordering::SeqCst) {
        std::thread::yield_now();
    }
    assert!(!node.poll.serves(0));
}

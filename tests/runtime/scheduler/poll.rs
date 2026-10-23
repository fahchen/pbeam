use super::{Direction, NodePoll};
use crate::{runtime::scheduler::node::Node, term::PortId};
use std::{
    os::{fd::AsRawFd, unix::net::UnixStream},
    sync::atomic::Ordering,
};

#[test]
fn reports_a_write_fd_from_a_wait_with_no_read_fd() {
    // A wait in which only a write fd is ready gives no read fd
    // (`Readiness::Pending`), and the thread still reports the write fd.
    let (node, _wake) = Node::single();
    NodePoll::start(&node);
    let (end, _other) = UnixStream::pair().expect("pair");
    let fd = end.as_raw_fd();
    node.poll
        .register(PortId(1), 0, Direction::Write, None, Some(fd));
    loop {
        let ready = node.poll.take_ready(0);
        if !ready.is_empty() {
            assert_eq!(ready, vec![(fd, Direction::Write)]);
            break;
        }
        std::thread::yield_now();
    }
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

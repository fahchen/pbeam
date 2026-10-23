//! The poll thread of the node, as the ERTS poll thread
//! (`erts_no_poll_threads = 1`, `erl_check_io.c:226-227`), which waits for
//! I/O while the schedulers run processes (`erl_process.c:3339-3372`).
//!
//! The thread waits on the fds of the ports of the node table, on fd 0 when
//! a scheduler has its read select, and on its own wake socket. When an fd
//! of a scheduler is ready, the thread sets the I/O mark of that scheduler
//! and wakes it through its inbox wake socket. The scheduler handles the
//! I/O at its next scheduling step, as the switch-point check of ERTS does
//! (`erl_process.c:9946-9960`). The thread leaves out the fds of a marked
//! scheduler until the scheduler clears the mark, so a ready fd wakes its
//! scheduler once, as a one-shot event of ERTS (`ERTS_POLL_EV_ONESHOT`).

use std::{
    ffi::c_int,
    io::{Read, Write},
    os::{fd::AsRawFd, unix::net::UnixStream},
    sync::{
        Arc, OnceLock, Weak,
        atomic::{AtomicBool, AtomicUsize, Ordering},
    },
    thread,
};

use super::node::Node;
use crate::runtime::host::input::{self, Readiness};

/// No scheduler has the read select of fd 0.
const NO_SCHEDULER: usize = usize::MAX;

/// The shared state of the poll thread of a node.
#[derive(Debug)]
pub(in crate::runtime) struct NodePoll {
    /// The write end of the wake socket of the thread, after it started.
    wake: OnceLock<UnixStream>,
    /// True while the thread waits on its last fd set.
    sleeping: AtomicBool,
    /// True when an fd set changed after the thread collected its set.
    changed: AtomicBool,
    /// The I/O mark of each scheduler.
    marks: Box<[AtomicBool]>,
    /// The scheduler that has the read select of fd 0, or [`NO_SCHEDULER`].
    stdin: AtomicUsize,
}

impl NodePoll {
    /// Return the poll state of a node of `count` schedulers.
    pub(in crate::runtime) fn new(count: usize) -> Arc<Self> {
        Arc::new(Self {
            wake: OnceLock::new(),
            sleeping: AtomicBool::new(false),
            changed: AtomicBool::new(false),
            marks: (0..count).map(|_| AtomicBool::new(false)).collect(),
            stdin: AtomicUsize::new(NO_SCHEDULER),
        })
    }

    /// Start the thread of `node` when it does not run, and make it wait
    /// again with the new fds.
    pub(in crate::runtime) fn start(node: &Arc<Node>) {
        let poll = &node.poll;
        poll.wake.get_or_init(|| {
            let (reader, writer) = UnixStream::pair().expect("the poll wake socket opens");
            for end in [&reader, &writer] {
                end.set_nonblocking(true)
                    .expect("the poll wake socket is non-blocking");
            }
            let node = Arc::downgrade(node);
            thread::Builder::new()
                .name("pbeam-poll".into())
                .spawn(move || run(&node, reader))
                .expect("the poll thread starts");
            writer
        });
        poll.changed();
    }

    /// Record that a port or fd 0 has a new fd to wait on, and wake the
    /// thread when it waits on its old set.
    pub(in crate::runtime) fn changed(&self) {
        self.changed.store(true, Ordering::SeqCst);
        if self.sleeping.swap(false, Ordering::SeqCst)
            && let Some(mut wake) = self.wake.get()
        {
            let _ = wake.write(&[1]);
        }
    }

    /// Record that the scheduler `index` has the read select of fd 0, or
    /// that no scheduler has it for `None`.
    pub(in crate::runtime) fn select_stdin(&self, index: Option<usize>) {
        self.stdin
            .store(index.unwrap_or(NO_SCHEDULER), Ordering::SeqCst);
        if index.is_some() {
            self.changed();
        }
    }

    /// Record that the scheduler `index` no longer has the read select of
    /// fd 0, when the record names it.
    pub(in crate::runtime) fn clear_stdin(&self, index: usize) {
        let _ =
            self.stdin
                .compare_exchange(index, NO_SCHEDULER, Ordering::SeqCst, Ordering::SeqCst);
    }

    /// Clear the I/O mark of the scheduler `index`, and return true when it
    /// was set.
    pub(in crate::runtime) fn take_mark(&self, index: usize) -> bool {
        self.marks[index].load(Ordering::Relaxed) && self.marks[index].swap(false, Ordering::SeqCst)
    }
}

/// Run the poll thread until its node ends: the node drop closes the write
/// end of the wake socket, which ends the wait.
fn run(node: &Weak<Node>, mut reader: UnixStream) {
    let wake_fd = reader.as_raw_fd();
    loop {
        let Some(strong) = node.upgrade() else { return };
        let poll = &strong.poll;
        poll.changed.store(false, Ordering::SeqCst);
        let (reads, writes) = collect(&strong);
        poll.sleeping.store(true, Ordering::SeqCst);
        if poll.changed.load(Ordering::SeqCst) {
            poll.sleeping.store(false, Ordering::SeqCst);
            continue;
        }
        drop(strong);
        let mut fds: Vec<c_int> = reads.iter().map(|&(fd, _)| fd).collect();
        fds.push(wake_fd);
        let write_fds: Vec<c_int> = writes.iter().map(|&(fd, _)| fd).collect();
        let (readiness, writable) = input::wait(&fds, &write_fds, None);
        let Some(strong) = node.upgrade() else { return };
        strong.poll.sleeping.store(false, Ordering::SeqCst);
        let ready = match readiness {
            Readiness::Ready(ready) => ready,
            // A port closed one of its fds after the collect; the next
            // collect leaves it out.
            Readiness::Pending | Readiness::Invalid => Vec::new(),
        };
        if ready.contains(&wake_fd) {
            drain(&mut reader);
        }
        let owners = ready
            .iter()
            .filter_map(|fd| reads.iter().find(|(read, _)| read == fd))
            .chain(
                writable
                    .iter()
                    .filter_map(|fd| writes.iter().find(|(write, _)| write == fd)),
            )
            .map(|&(_, index)| index);
        for index in owners {
            if !strong.poll.marks[index].swap(true, Ordering::SeqCst) {
                strong.wake_scheduler(index);
            }
        }
    }
}

/// An fd with the scheduler that serves it.
type Owned = (c_int, usize);

/// Return the read fds and the write fds of each scheduler without an I/O
/// mark, each with its scheduler.
fn collect(node: &Node) -> (Vec<Owned>, Vec<Owned>) {
    let mut reads = Vec::new();
    let mut writes = Vec::new();
    let stdin = node.poll.stdin.load(Ordering::SeqCst);
    for index in 0..node.scheduler_count() {
        if node.poll.marks[index].load(Ordering::SeqCst) {
            continue;
        }
        // The table lock is released before the scans lock each port.
        let handles = node.ports().handles(index);
        let with = |fd| (fd, index);
        reads.extend(
            crate::runtime::port::wait_fds(&handles)
                .into_iter()
                .map(with),
        );
        writes.extend(
            crate::runtime::port::write_fds(&handles)
                .into_iter()
                .map(with),
        );
        if stdin == index {
            reads.push((libc::STDIN_FILENO, index));
        }
    }
    (reads, writes)
}

/// Read the wake bytes. The end of file comes only after the node ended,
/// and then the upgrade of the node in [`run`] ends the thread first.
fn drain(reader: &mut UnixStream) {
    let mut bytes = [0; 64];
    // The socket is non-blocking: an error means that no more bytes wait.
    while matches!(reader.read(&mut bytes), Ok(count) if count > 0) {}
}

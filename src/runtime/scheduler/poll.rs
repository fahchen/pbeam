//! The poll thread of the node, as the ERTS poll thread
//! (`erts_no_poll_threads = 1`, `erl_check_io.c:226-227`), which waits for
//! I/O while the schedulers run processes (`erl_process.c:3339-3372`).
//!
//! The thread is the only waiter on the fds of the ports and on fd 0, as
//! ERTS keeps each fd in exactly one pollset (`erl_check_io.c:90-111`). It
//! waits on the fds that the ports registered (see [`NodePoll::register`]),
//! on fd 0 when a scheduler has its read select, and on its own wake
//! socket. When an fd is ready, the thread adds the fd and its direction to
//! the ready list of the scheduler that serves it, and wakes that
//! scheduler, as the ERTS poll thread schedules a port task
//! (`erl_process.c:3370-3373`). The scheduler serves the list at its next
//! scheduling step (`erl_process.c:9946-9960`). The thread leaves out a
//! reported fd until its scheduler served it, as a one-shot event of ERTS
//! (`ERTS_POLL_EV_ONESHOT`, `erl_check_io.c:494-511`).

use std::{
    collections::HashMap,
    ffi::c_int,
    io::{Read, Write},
    os::{fd::AsRawFd, unix::net::UnixStream},
    sync::{
        Arc, Condvar, Mutex, MutexGuard, OnceLock, Weak,
        atomic::{AtomicBool, AtomicUsize, Ordering},
    },
    thread,
    time::Duration,
};

use super::node::Node;
use crate::{
    runtime::host::input::{self, Readiness},
    term::{PortId, ProcessId, Term},
};

/// No scheduler has the read select of fd 0.
const NO_SCHEDULER: usize = usize::MAX;

/// The scheduler index of an fd of a socket select: the thread itself
/// sends its message (see [`NodePoll::select`]).
const SELECT: usize = usize::MAX;

/// The direction of a ready fd.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(in crate::runtime) enum Direction {
    /// A `read` of the fd does not block.
    Read,
    /// A `write` of the fd does not block.
    Write,
}

/// An fd and its direction.
pub(in crate::runtime) type Ready = (c_int, Direction);

/// The I/O state of one scheduler.
#[derive(Debug, Default)]
struct Served {
    /// The fds that the thread found ready and the scheduler did not take.
    ready: Vec<Ready>,
    /// The fds that the thread reported and the scheduler did not serve:
    /// the thread leaves them out of its wait.
    disarmed: Vec<Ready>,
    /// True while the scheduler waits on its own fds in its idle wait.
    owned: bool,
    /// True while the thread may wait on the fds of the scheduler.
    polled: bool,
}

/// The shared state of the poll thread of a node.
#[derive(Debug)]
pub(in crate::runtime) struct NodePoll {
    /// The write end of the wake socket of the thread, after it started.
    wake: OnceLock<UnixStream>,
    /// The node, after the thread started, to wake a scheduler that waits
    /// on its own fds (see [`NodePoll::register`]).
    node: OnceLock<Weak<Node>>,
    /// True while the thread waits on its last fd set.
    sleeping: AtomicBool,
    /// True when an fd set changed after the thread collected its set.
    changed: AtomicBool,
    /// The fds of the ports with their direction, each with the scheduler
    /// that serves it and the port that registered it.
    fds: Mutex<HashMap<(c_int, Direction), (usize, PortId)>>,
    /// True when the ready list of a scheduler is not empty.
    marks: Box<[AtomicBool]>,
    /// The I/O state of each scheduler.
    served: Box<[Mutex<Served>]>,
    /// The signal of each scheduler that the thread stopped its wait on the
    /// fds of the scheduler (see [`NodePoll::take_over`]).
    released: Box<[Condvar]>,
    /// The scheduler that has the read select of fd 0, or [`NO_SCHEDULER`].
    stdin: AtomicUsize,
    /// The one-shot socket selects: each fd and direction with the process
    /// that gets the select message, and the message.
    selects: Mutex<HashMap<Ready, (ProcessId, Term)>>,
}

impl NodePoll {
    /// Return the poll state of a node of `count` schedulers.
    pub(in crate::runtime) fn new(count: usize) -> Arc<Self> {
        Arc::new(Self {
            wake: OnceLock::new(),
            node: OnceLock::new(),
            sleeping: AtomicBool::new(false),
            changed: AtomicBool::new(false),
            fds: Mutex::default(),
            marks: (0..count).map(|_| AtomicBool::new(false)).collect(),
            served: (0..count).map(|_| Mutex::default()).collect(),
            released: (0..count).map(|_| Condvar::new()).collect(),
            stdin: AtomicUsize::new(NO_SCHEDULER),
            selects: Mutex::default(),
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
            let _ = poll.node.set(node.clone());
            thread::Builder::new()
                .name("pbeam-poll".into())
                .spawn(move || run(&node, reader))
                .expect("the poll thread starts");
            writer
        });
        poll.changed();
    }

    /// Record that a port or fd 0 has a new fd to wait on, and wake the
    /// thread when it waits on its old set. The thread can wait before
    /// [`NodePoll::start`] stores the write end of its wake socket. A change
    /// in that time leaves `sleeping` set, so that the call of `changed` at
    /// the end of `start` wakes the thread; else no call would wake it.
    pub(in crate::runtime) fn changed(&self) {
        self.changed.store(true, Ordering::SeqCst);
        if let Some(mut wake) = self.wake.get()
            && self.sleeping.swap(false, Ordering::SeqCst)
        {
            let _ = wake.write(&[1]);
        }
    }

    fn fds(&self) -> MutexGuard<'_, HashMap<(c_int, Direction), (usize, PortId)>> {
        self.fds.lock().expect("the poll fd table is not poisoned")
    }

    fn served(&self, index: usize) -> MutexGuard<'_, Served> {
        self.served[index]
            .lock()
            .expect("the poll ready list is not poisoned")
    }

    /// Change the fd that the port `port` of the scheduler `index` waits on
    /// in `direction` from `before` to `after`, as `erts_poll_control`
    /// changes the pollset (`erl_check_io.c:1422-1434`). A new fd wakes the
    /// thread, or the scheduler `index` when it waits on its own fds in its
    /// idle wait, as `erts_poll_control` wakes the poller of the pollset
    /// (`erl_poll.c:1447-1448`): a port task of another scheduler can add
    /// the fd. A removed fd wakes nothing: the next wait leaves it out.
    pub(in crate::runtime) fn register(
        &self,
        port: PortId,
        index: usize,
        direction: Direction,
        before: Option<c_int>,
        after: Option<c_int>,
    ) {
        if before == after {
            return;
        }
        let mut fds = self.fds();
        if let Some(fd) = before
            && fds
                .get(&(fd, direction))
                .is_some_and(|&(_, owner)| owner == port)
        {
            fds.remove(&(fd, direction));
        }
        if let Some(fd) = after {
            fds.insert((fd, direction), (index, port));
            drop(fds);
            self.changed();
            if self.served(index).owned
                && let Some(node) = self.node.get().and_then(Weak::upgrade)
            {
                node.wake_scheduler(index);
            }
        }
    }

    /// Add the one-shot select of `fd` in `direction`, as `enif_select`
    /// does (`erl_check_io.c:1561-1790`): when the fd is ready, the thread
    /// removes the select and sends `message` to `pid`. A new select of the
    /// same fd and direction replaces the old one.
    pub(in crate::runtime) fn select(
        &self,
        fd: c_int,
        direction: Direction,
        pid: ProcessId,
        message: Term,
    ) {
        self.selects().insert((fd, direction), (pid, message));
        self.changed();
    }

    /// Remove the select of `fd` in `direction`, as `enif_select` with
    /// `ERL_NIF_SELECT_CANCEL` does. Return true when the select was still
    /// active, the `ERL_NIF_SELECT_*_CANCELLED` bit; false when its message
    /// went out or no select was made.
    pub(in crate::runtime) fn cancel_select(&self, fd: c_int, direction: Direction) -> bool {
        self.selects().remove(&(fd, direction)).is_some()
    }

    fn selects(&self) -> MutexGuard<'_, HashMap<Ready, (ProcessId, Term)>> {
        self.selects
            .lock()
            .expect("the poll select table is not poisoned")
    }

    /// Return the port that the poll waits to read `fd` for: the port that
    /// registered `fd` last.
    pub(in crate::runtime) fn reader(&self, fd: c_int) -> Option<PortId> {
        self.fds()
            .get(&(fd, Direction::Read))
            .map(|&(_, port)| port)
    }

    /// Return true when the thread waits on an fd for the scheduler
    /// `index`, or holds a ready fd for it.
    pub(in crate::runtime) fn serves(&self, index: usize) -> bool {
        self.marks[index].load(Ordering::SeqCst)
            || self.fds().values().any(|&(owner, _)| owner == index)
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

    /// Take the ready list of the scheduler `index`. The check reads one
    /// flag and takes no lock when the list is empty.
    pub(in crate::runtime) fn take_ready(&self, index: usize) -> Vec<Ready> {
        if !self.marks[index].load(Ordering::Relaxed) {
            return Vec::new();
        }
        let mut served = self.served(index);
        self.marks[index].store(false, Ordering::SeqCst);
        std::mem::take(&mut served.ready)
    }

    /// Add `entry` to the ready list of the scheduler `index`, as the
    /// thread does when it finds the fd ready.
    #[cfg(test)]
    pub(in crate::runtime) fn report(&self, index: usize, entry: Ready) {
        self.served(index).ready.push(entry);
        self.marks[index].store(true, Ordering::SeqCst);
    }

    /// Make the thread wait again on the fds of `ready`, which the
    /// scheduler `index` served.
    pub(in crate::runtime) fn rearm(&self, index: usize, ready: &[Ready]) {
        self.served(index)
            .disarmed
            .retain(|entry| !ready.contains(entry));
        self.changed();
    }

    /// Make the scheduler `index` the only waiter on its fds for its idle
    /// wait, and return these fds. When the thread waits on them, the
    /// scheduler wakes the thread and waits until the thread stopped, so
    /// that an fd never has two waiters (`erl_check_io.c:90-111`).
    pub(in crate::runtime) fn take_over(&self, index: usize) -> Vec<Ready> {
        let mut served = self.served(index);
        served.owned = true;
        if served.polled {
            self.changed();
            while served.polled {
                served = self.released[index]
                    .wait(served)
                    .expect("the poll ready list is not poisoned");
            }
        }
        let disarmed = served.disarmed.clone();
        drop(served);
        let mut fds: Vec<Ready> = self
            .fds()
            .iter()
            .filter(|(_, (owner, _))| *owner == index)
            .map(|(&entry, _)| entry)
            .collect();
        if self.stdin.load(Ordering::SeqCst) == index {
            fds.push((libc::STDIN_FILENO, Direction::Read));
        }
        fds.retain(|entry| !disarmed.contains(entry));
        fds
    }

    /// Give the fds of the scheduler `index` back to the thread after its
    /// idle wait.
    pub(in crate::runtime) fn hand_back(&self, index: usize) {
        self.served(index).owned = false;
        self.changed();
    }
}

/// An fd that the thread waits on, with the scheduler that serves it.
pub(in crate::runtime) type Owned = (Ready, usize);

/// Run the poll thread until its node ends: the node drop closes the write
/// end of the wake socket, which ends the wait.
fn run(node: &Weak<Node>, mut reader: UnixStream) {
    let wake_fd = reader.as_raw_fd();
    loop {
        let Some(strong) = node.upgrade() else { return };
        let poll = &strong.poll;
        poll.changed.store(false, Ordering::SeqCst);
        let owned = collect(poll);
        poll.sleeping.store(true, Ordering::SeqCst);
        if poll.changed.load(Ordering::SeqCst) {
            poll.sleeping.store(false, Ordering::SeqCst);
            continue;
        }
        drop(strong);
        let (readiness, writable) = wait(&owned, Some(wake_fd), None);
        let Some(strong) = node.upgrade() else { return };
        strong.poll.sleeping.store(false, Ordering::SeqCst);
        let reads = match readiness {
            Readiness::Ready(ready) => ready,
            // An fd that a port closed after the collect: the close removed
            // it from the table, and the next collect leaves it out.
            Readiness::Pending | Readiness::Invalid => Vec::new(),
        };
        if reads.contains(&wake_fd) {
            drain(&mut reader);
        }
        let found = ready(&owned, &reads, &writable);
        // The reports go out before a scheduler takes its fds over.
        let mut served: Vec<_> = (0..strong.poll.served.len())
            .map(|index| strong.poll.served(index))
            .collect();
        for slot in &mut served {
            slot.polled = false;
        }
        let mut woken = Vec::new();
        for (entry, index) in found {
            if index == SELECT {
                // A cancel after the wait removed the select: no message.
                if let Some((pid, message)) = strong.poll.selects().remove(&entry) {
                    strong.send_message(0, None, message, pid, None);
                }
                continue;
            }
            served[index].ready.push(entry);
            served[index].disarmed.push(entry);
            if !strong.poll.marks[index].swap(true, Ordering::SeqCst) {
                woken.push(index);
            }
        }
        drop(served);
        for (index, released) in strong.poll.released.iter().enumerate() {
            released.notify_all();
            if woken.contains(&index) {
                strong.wake_scheduler(index);
            }
        }
    }
}

/// Return the entries of `owned` that are ready: a read entry in `reads`
/// or a write entry in `writes`. A wait in which only write fds are ready
/// gives no read fds, and its write fds still count.
pub(in crate::runtime) fn ready(owned: &[Owned], reads: &[c_int], writes: &[c_int]) -> Vec<Owned> {
    owned
        .iter()
        .copied()
        .filter(|&((fd, direction), _)| match direction {
            Direction::Read => reads.contains(&fd),
            Direction::Write => writes.contains(&fd),
        })
        .collect()
}

/// Wait on the read and write fds of `owned` and on `wake`.
pub(in crate::runtime) fn wait(
    owned: &[Owned],
    wake: Option<c_int>,
    timeout: Option<Duration>,
) -> (Readiness, Vec<c_int>) {
    let (mut reads, mut writes) = (Vec::new(), Vec::new());
    for &((fd, direction), _) in owned {
        match direction {
            Direction::Read => reads.push(fd),
            Direction::Write => writes.push(fd),
        }
    }
    reads.extend(wake);
    input::wait(&reads, &writes, timeout)
}

/// Return the registered fds and fd 0, each with its scheduler, without
/// the fds that a scheduler did not serve yet.
fn collect(poll: &NodePoll) -> Vec<Owned> {
    let disarmed: Vec<Option<Vec<Ready>>> = (0..poll.served.len())
        .map(|index| {
            let mut served = poll.served(index);
            served.polled = !served.owned;
            poll.released[index].notify_all();
            served.polled.then(|| served.disarmed.clone())
        })
        .collect();
    let armed = |entry: &Ready, index: usize| {
        let Some(disarmed) = &disarmed[index] else {
            return false;
        };
        !disarmed
            .iter()
            .any(|(fd, direction)| *fd == entry.0 && (*direction == entry.1 || entry.0 == 0))
    };
    let mut owned: Vec<Owned> = poll
        .fds()
        .iter()
        .filter(|(entry, (index, _))| armed(entry, *index))
        .map(|(&entry, &(index, _))| (entry, index))
        .collect();
    let stdin = poll.stdin.load(Ordering::SeqCst);
    let entry = (libc::STDIN_FILENO, Direction::Read);
    if stdin != NO_SCHEDULER && armed(&entry, stdin) {
        owned.push((entry, stdin));
    }
    owned.extend(poll.selects().keys().map(|&entry| (entry, SELECT)));
    owned
}

/// Read the wake bytes. The end of file comes only after the node ended,
/// and then the upgrade of the node in [`run`] ends the thread first.
fn drain(reader: &mut UnixStream) {
    let mut bytes = [0; 64];
    // The socket is non-blocking: an error means that no more bytes wait.
    while matches!(reader.read(&mut bytes), Ok(count) if count > 0) {}
}

#[cfg(test)]
#[path = "../../../tests/runtime/scheduler/poll.rs"]
mod tests;

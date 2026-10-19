//! The state that the scheduler threads of one run share.
//!
//! Each scheduler owns its processes, its run queue, its code table, and its
//! services. The node holds what must be the same for the whole run: the
//! owner of each live process, the registered names, the identifier
//! counters, the inbox of each scheduler for the deliveries that cross a
//! thread boundary, and the shutdown state. A sender that runs on another
//! scheduler puts its message or signal into the inbox of the owner and
//! wakes it; the owner moves the delivery into the signal queue of the
//! target, in inbox order, at its next scheduling step. Thus the deliveries
//! of one sender to one receiver keep their order, and cross-thread code
//! never touches a process of another scheduler.

use std::{
    collections::{HashMap, VecDeque},
    io::{self, Read, Write},
    os::unix::net::UnixStream,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, AtomicI64, AtomicU64, AtomicUsize, Ordering},
    },
};

use super::Signal;
use crate::term::{ProcessId, Term};

/// One delivery that crosses a scheduler thread boundary.
#[derive(Debug)]
pub(in crate::runtime) enum Delivery {
    /// A message for `target`.
    Message {
        target: ProcessId,
        from: ProcessId,
        term: Term,
    },
    /// A control signal for `target`.
    Signal {
        target: ProcessId,
        sender: Option<ProcessId>,
        signal: Signal,
    },
}

/// The inbox of one scheduler: the deliveries that it did not apply yet,
/// and the write end of its wake socket.
#[derive(Debug)]
struct Inbox {
    queue: Mutex<VecDeque<Delivery>>,
    wake: UnixStream,
}

/// The state of a run that more than one scheduler thread shares.
#[derive(Debug)]
pub(in crate::runtime) struct SharedNode {
    /// The scheduler of each live process.
    owners: Mutex<HashMap<ProcessId, usize>>,
    /// The registered names of the node, as the ERTS `process_reg` table.
    names: Mutex<HashMap<String, ProcessId>>,
    /// The inbox of each scheduler.
    inboxes: Vec<Inbox>,
    /// The next process identifier.
    next_pid: AtomicU64,
    /// The next ordinary reference.
    next_reference: AtomicU64,
    /// The counter of `erlang:unique_integer/0,1`.
    unique: AtomicU64,
    /// The raw counter of `erlang:unique_integer([monotonic])`, which starts
    /// at -1.
    unique_monotonic: AtomicI64,
    /// The next unlink operation identifier.
    next_unlink_id: AtomicU64,
    /// The number of roots that did not exit yet.
    roots: AtomicUsize,
    /// True after the last root exited.
    shutdown: AtomicBool,
}

impl SharedNode {
    /// Return the node of `count` schedulers and the read end of the wake
    /// socket of each scheduler.
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// let (node, readers) = SharedNode::new(2).unwrap();
    /// assert_eq!(readers.len(), 2);
    /// ```
    pub(in crate::runtime) fn new(count: usize) -> io::Result<(Arc<Self>, Vec<UnixStream>)> {
        let mut inboxes = Vec::with_capacity(count);
        let mut readers = Vec::with_capacity(count);
        for _ in 0..count {
            let (reader, writer) = UnixStream::pair()?;
            reader.set_nonblocking(true)?;
            writer.set_nonblocking(true)?;
            inboxes.push(Inbox {
                queue: Mutex::new(VecDeque::new()),
                wake: writer,
            });
            readers.push(reader);
        }
        Ok((
            Arc::new(SharedNode {
                owners: Mutex::new(HashMap::new()),
                names: Mutex::new(HashMap::new()),
                inboxes,
                next_pid: AtomicU64::new(0),
                next_reference: AtomicU64::new(0),
                unique: AtomicU64::new(0),
                unique_monotonic: AtomicI64::new(-1),
                next_unlink_id: AtomicU64::new(1),
                roots: AtomicUsize::new(count),
                shutdown: AtomicBool::new(false),
            }),
            readers,
        ))
    }

    /// Make the process `id` the process of the scheduler `index`.
    pub(in crate::runtime) fn register(&self, id: ProcessId, index: usize) {
        self.owners
            .lock()
            .expect("the process owner table is not poisoned")
            .insert(id, index);
    }

    /// Remove the process `id` from the node.
    pub(in crate::runtime) fn unregister(&self, id: ProcessId) {
        self.owners
            .lock()
            .expect("the process owner table is not poisoned")
            .remove(&id);
    }

    /// Return the scheduler of the live process `id`.
    pub(in crate::runtime) fn owner(&self, id: ProcessId) -> Option<usize> {
        self.owners
            .lock()
            .expect("the process owner table is not poisoned")
            .get(&id)
            .copied()
    }

    /// Put a message for `target` into the inbox of `index`. `pending` is
    /// the monitor signal that goes before the message, as the send path
    /// gives it.
    pub(in crate::runtime) fn send_message(
        &self,
        index: usize,
        from: ProcessId,
        term: Term,
        target: ProcessId,
        pending: Option<Signal>,
    ) {
        let inbox = &self.inboxes[index];
        {
            let mut queue = inbox
                .queue
                .lock()
                .expect("the scheduler inbox is not poisoned");
            if let Some(signal) = pending {
                queue.push_back(Delivery::Signal {
                    target,
                    sender: Some(from),
                    signal,
                });
            }
            queue.push_back(Delivery::Message { target, from, term });
        }
        wake(&inbox.wake);
    }

    /// Put the signal for `target` into the inbox of `index`. `pending` is
    /// the monitor signal that goes before it, as the send path gives it.
    pub(in crate::runtime) fn send_signal(
        &self,
        index: usize,
        target: ProcessId,
        sender: Option<ProcessId>,
        signal: Signal,
        pending: Option<Signal>,
    ) {
        let inbox = &self.inboxes[index];
        {
            let mut queue = inbox
                .queue
                .lock()
                .expect("the scheduler inbox is not poisoned");
            if let Some(signal) = pending {
                queue.push_back(Delivery::Signal {
                    target,
                    sender,
                    signal,
                });
            }
            queue.push_back(Delivery::Signal {
                target,
                sender,
                signal,
            });
        }
        wake(&inbox.wake);
    }

    /// Move each delivery of the inbox of `index` into `out`.
    pub(in crate::runtime) fn take(&self, index: usize, out: &mut Vec<Delivery>) {
        let mut queue = self.inboxes[index]
            .queue
            .lock()
            .expect("the scheduler inbox is not poisoned");
        out.extend(queue.drain(..));
    }

    /// Drain the wake socket of `index` after the scheduler waited on it.
    pub(in crate::runtime) fn drain_wake(reader: &mut UnixStream) {
        let mut bytes = [0; 64];
        while matches!(reader.read(&mut bytes), Ok(count) if count > 0) {}
    }

    /// Return the next process identifier.
    pub(in crate::runtime) fn next_pid(&self) -> u64 {
        self.next_pid.fetch_add(1, Ordering::Relaxed)
    }

    /// Return the next ordinary reference.
    pub(in crate::runtime) fn next_reference(&self) -> u64 {
        self.next_reference.fetch_add(1, Ordering::Relaxed)
    }

    /// Return the next counter value of `erlang:unique_integer/0,1`.
    pub(in crate::runtime) fn next_unique(&self) -> u64 {
        self.unique.fetch_add(1, Ordering::Relaxed)
    }

    /// Return the next raw value of `erlang:unique_integer([monotonic])`.
    pub(in crate::runtime) fn next_unique_monotonic(&self) -> i64 {
        self.unique_monotonic.fetch_add(1, Ordering::Relaxed) + 1
    }

    /// Return the next unlink operation identifier.
    pub(in crate::runtime) fn next_unlink_id(&self) -> u64 {
        self.next_unlink_id.fetch_add(1, Ordering::Relaxed)
    }

    /// Register `name` for `id`. Return false when another process has the
    /// name.
    pub(in crate::runtime) fn name_register(&self, name: &str, id: ProcessId) -> bool {
        let mut names = self.names.lock().expect("the name table is not poisoned");
        if names.contains_key(name) {
            return false;
        }
        names.insert(name.to_owned(), id);
        true
    }

    /// Remove the process `name`. Return the process that had it.
    pub(in crate::runtime) fn name_remove(&self, name: &str) -> Option<ProcessId> {
        self.names
            .lock()
            .expect("the name table is not poisoned")
            .remove(name)
    }

    /// Return the process of `name`.
    pub(in crate::runtime) fn name_get(&self, name: &str) -> Option<ProcessId> {
        self.names
            .lock()
            .expect("the name table is not poisoned")
            .get(name)
            .copied()
    }

    /// Return the registered names, in no order.
    pub(in crate::runtime) fn name_keys(&self) -> Vec<String> {
        self.names
            .lock()
            .expect("the name table is not poisoned")
            .keys()
            .cloned()
            .collect()
    }

    /// Record that the root of one scheduler exited. The last root sets the
    /// shutdown state and wakes each scheduler.
    pub(in crate::runtime) fn root_done(&self) {
        if self.roots.fetch_sub(1, Ordering::AcqRel) == 1 {
            self.shutdown.store(true, Ordering::Release);
        }
        for inbox in &self.inboxes {
            wake(&inbox.wake);
        }
    }

    /// Stop every scheduler after a root failed. Each scheduler that still
    /// runs returns from its loop, and each scheduler that serves its inbox
    /// leaves it.
    pub(in crate::runtime) fn abort(&self) {
        self.shutdown.store(true, Ordering::Release);
        for inbox in &self.inboxes {
            wake(&inbox.wake);
        }
    }

    /// Return true after the last root exited.
    pub(in crate::runtime) fn is_shutdown(&self) -> bool {
        self.shutdown.load(Ordering::Acquire)
    }

    /// Return the number of roots that did not exit yet.
    pub(in crate::runtime) fn roots_left(&self) -> usize {
        self.roots.load(Ordering::Acquire)
    }
}

/// Write one byte to the wake socket, and drop the byte when the socket is
/// full: the scheduler reads the socket, and a full socket already wakes it.
fn wake(writer: &UnixStream) {
    let mut writer = writer;
    let _ = writer.write(&[1]);
}

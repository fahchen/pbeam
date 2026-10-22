//! The state that the scheduler threads of one run share.
//!
//! Each scheduler owns the state of its processes, its run queue, its code
//! table, and its services. The node holds what must be the same for the
//! whole run: the process table, with the owner of each live process, the
//! registered names, the identifier
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
        Arc, Mutex, OnceLock,
        atomic::{AtomicBool, AtomicI64, AtomicU64, AtomicUsize, Ordering},
    },
    time::Instant,
};

use super::{Signal, process_table::ProcessTable};
use crate::{
    error::{BeamError, error},
    runtime::{dirty::Pools, host::clock},
    term::{PortId, ProcessId, Term},
};

/// One delivery that crosses a scheduler thread boundary.
#[derive(Debug)]
pub(in crate::runtime) enum Delivery {
    /// A message for `target`. `from` is None for a message of the system,
    /// which sets no self-signal mark on the receiver.
    Message {
        target: ProcessId,
        from: Option<ProcessId>,
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
pub(in crate::runtime) struct Node {
    /// The process table of the node: the scheduler, the exiting state,
    /// and the table order of each live process.
    pub(super) processes: ProcessTable,
    /// The registered names of the node, as the ERTS `process_reg` table.
    names: Mutex<HashMap<String, ProcessId>>,
    /// The scheduler of each open port.
    ports: Mutex<HashMap<PortId, usize>>,
    /// The scheduler of each active BIF timer, by its reference.
    timers: Mutex<HashMap<crate::term::Reference, usize>>,
    /// The scheduler of each compiled regular expression, by its reference.
    regexes: Mutex<HashMap<crate::term::Reference, usize>>,
    /// The scheduler of each compiled binary pattern, by its reference.
    patterns: Mutex<HashMap<crate::term::Reference, usize>>,
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
    /// The next port identifier.
    next_port: AtomicU64,
    /// The next unlink operation identifier.
    next_unlink_id: AtomicU64,
    /// The number of roots that did not exit yet.
    roots: AtomicUsize,
    /// The number of schedulers of the node.
    schedulers: usize,
    /// The number of live processes.
    live: AtomicUsize,
    /// True after the last root exited.
    shutdown: AtomicBool,
    /// The run start, as the instant and the OS system time at the same
    /// moment. Each scheduler builds its timer service on this pair, so the
    /// time of every scheduler agrees.
    clock: (Instant, i64),
    /// The dirty CPU pool and the dirty IO pool, which each scheduler of
    /// the node uses.
    dirty: Pools,
    /// The seed of the hash of `term_to_binary/2` with `local`
    /// (`local_node_hash`, external.c:134). The first call on any scheduler
    /// computes it, and each scheduler reads the same value.
    local_node_hash: OnceLock<u32>,
}

impl Node {
    /// Return the node of `count` schedulers and the read end of the wake
    /// socket of each scheduler.
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// let (node, readers) = Node::new(2).unwrap();
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
            Arc::new(Node {
                processes: ProcessTable::default(),
                names: Mutex::new(HashMap::new()),
                ports: Mutex::new(HashMap::new()),
                timers: Mutex::new(HashMap::new()),
                regexes: Mutex::new(HashMap::new()),
                patterns: Mutex::new(HashMap::new()),
                inboxes,
                next_pid: AtomicU64::new(0),
                next_reference: AtomicU64::new(0),
                unique: AtomicU64::new(0),
                unique_monotonic: AtomicI64::new(-1),
                next_port: AtomicU64::new(0),
                next_unlink_id: AtomicU64::new(1),
                roots: AtomicUsize::new(count),
                schedulers: count,
                live: AtomicUsize::new(0),
                shutdown: AtomicBool::new(false),
                clock: (Instant::now(), clock::system_time()),
                dirty: Pools::new(count),
                local_node_hash: OnceLock::new(),
            }),
            readers,
        ))
    }

    /// Return the number of schedulers of the node.
    pub(in crate::runtime) fn scheduler_count(&self) -> usize {
        self.schedulers
    }

    /// Return the dirty pools of the node.
    pub(in crate::runtime) fn dirty_pools(&self) -> &Pools {
        &self.dirty
    }

    /// Return the run start of the node, as the instant and the OS system
    /// time at the same moment.
    pub(in crate::runtime) fn clock_origin(&self) -> (Instant, i64) {
        self.clock
    }

    /// Return the seed of the local node hash. `compute` runs at the first
    /// call on any scheduler, and each scheduler reads the same value.
    pub(in crate::runtime) fn local_node_hash(&self, compute: impl FnOnce() -> u32) -> u32 {
        *self.local_node_hash.get_or_init(compute)
    }

    /// Return the node of one scheduler that has no other scheduler. It has
    /// no inbox and no root that waits for a start, so a one-thread run never
    /// wakes a scheduler.
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// let node = Node::single();
    /// assert_eq!(node.scheduler_count(), 1);
    /// ```
    pub(in crate::runtime) fn single() -> Arc<Self> {
        Arc::new(Node {
            processes: ProcessTable::default(),
            names: Mutex::new(HashMap::new()),
            ports: Mutex::new(HashMap::new()),
            timers: Mutex::new(HashMap::new()),
            regexes: Mutex::new(HashMap::new()),
            patterns: Mutex::new(HashMap::new()),
            inboxes: Vec::new(),
            // A run with one scheduler creates its root as `ProcessId(0)`
            // before it starts a process, so the first other pid is 1.
            next_pid: AtomicU64::new(1),
            next_reference: AtomicU64::new(0),
            unique: AtomicU64::new(0),
            unique_monotonic: AtomicI64::new(-1),
            next_port: AtomicU64::new(0),
            next_unlink_id: AtomicU64::new(1),
            roots: AtomicUsize::new(0),
            schedulers: 1,
            live: AtomicUsize::new(0),
            shutdown: AtomicBool::new(false),
            clock: (Instant::now(), clock::system_time()),
            dirty: Pools::new(1),
            local_node_hash: OnceLock::new(),
        })
    }

    /// Return a node whose identifier counters are at their last value, for
    /// a test of the overflow errors.
    #[cfg(test)]
    pub(in crate::runtime) fn exhausted() -> Arc<Self> {
        let mut node = Node::single();
        let inner = Arc::get_mut(&mut node).expect("the test holds the only reference");
        inner.next_pid = AtomicU64::new(u64::MAX);
        inner.next_reference = AtomicU64::new(u64::MAX);
        node
    }

    /// Make the process `id` the process of the scheduler `index`.
    pub(in crate::runtime) fn register(&self, id: ProcessId, index: usize) {
        self.processes.insert(id, index);
        self.live.fetch_add(1, Ordering::AcqRel);
    }

    /// Mark the process `id` as exiting, as `set_self_exiting`
    /// (`erl_process.c:13485-13517`) sets its `EXITING` flag. A process of
    /// another scheduler then finds no live process for the pid (see
    /// [`Self::live_owner`]), and signals still reach the owner, which
    /// answers them at the exit.
    pub(in crate::runtime) fn set_exiting(&self, id: ProcessId) {
        self.processes.set_exiting(id);
    }

    /// Remove the process `id` from the process table and each name of
    /// it. A registration of a name and this removal hold the shard of the
    /// process and the name table in the same order, so a name never
    /// belongs to a process that left.
    ///
    /// # Panics
    ///
    /// Panic when the process left the table before.
    pub(in crate::runtime) fn remove_process(&self, id: ProcessId) {
        self.processes.remove(id, || {
            let mut names = self.names.lock().expect("the name table is not poisoned");
            names.retain(|_, owner| *owner != id);
        });
    }

    /// Record the exit of a process after its exit signals. A waiting root
    /// reads the count at the impossible-wait check, so the decrement comes
    /// after the notifications of the exit and wakes each scheduler.
    pub(in crate::runtime) fn live_exit(&self) {
        self.live.fetch_sub(1, Ordering::AcqRel);
        for inbox in &self.inboxes {
            wake(&inbox.wake);
        }
    }

    /// Return the number of live processes.
    pub(in crate::runtime) fn live_processes(&self) -> usize {
        self.live.load(Ordering::Acquire)
    }

    /// Return the scheduler of the live process `id`.
    pub(in crate::runtime) fn owner(&self, id: ProcessId) -> Option<usize> {
        self.processes.owner(id)
    }

    /// Return the scheduler of the process `id` when the process does not
    /// exit, as `erts_proc_lookup` (`erl_process_lock.h:1151-1157`) finds
    /// it.
    pub(in crate::runtime) fn live_owner(&self, id: ProcessId) -> Option<usize> {
        self.processes.live_owner(id)
    }

    /// Put a message for `target` into the inbox of `index`. `pending` is
    /// the monitor signal that goes before the message, as the send path
    /// gives it.
    pub(in crate::runtime) fn send_message(
        &self,
        index: usize,
        from: Option<ProcessId>,
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
                    sender: from,
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

    /// Return the next process identifier, or the error when the counter
    /// ends.
    pub(in crate::runtime) fn next_pid(&self) -> Result<u64, BeamError> {
        let value = self.next_pid.fetch_add(1, Ordering::Relaxed);
        if value == u64::MAX {
            return Err(error("process identifier overflow"));
        }
        Ok(value)
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

    /// Make the port `id` a port of the scheduler `index`.
    pub(in crate::runtime) fn register_port(&self, id: PortId, index: usize) {
        self.ports
            .lock()
            .expect("the port table is not poisoned")
            .insert(id, index);
    }

    /// Remove the port `id` from the node.
    pub(in crate::runtime) fn remove_port(&self, id: PortId) {
        self.ports
            .lock()
            .expect("the port table is not poisoned")
            .remove(&id);
    }

    /// Return the scheduler of the open port `id`.
    pub(in crate::runtime) fn port_owner(&self, id: PortId) -> Option<usize> {
        self.ports
            .lock()
            .expect("the port table is not poisoned")
            .get(&id)
            .copied()
    }

    /// Record the BIF timer `reference` of the scheduler `index`.
    pub(in crate::runtime) fn register_timer(
        &self,
        reference: crate::term::Reference,
        index: usize,
    ) {
        self.timers
            .lock()
            .expect("the timer table is not poisoned")
            .insert(reference, index);
    }

    /// Remove the BIF timer `reference` from the node.
    pub(in crate::runtime) fn remove_timer(&self, reference: crate::term::Reference) {
        self.timers
            .lock()
            .expect("the timer table is not poisoned")
            .remove(&reference);
    }

    /// Return the scheduler of the active BIF timer `reference`.
    pub(in crate::runtime) fn timer_owner(
        &self,
        reference: crate::term::Reference,
    ) -> Option<usize> {
        self.timers
            .lock()
            .expect("the timer table is not poisoned")
            .get(&reference)
            .copied()
    }

    /// Record the compiled regular expression `reference` of the
    /// scheduler `index`.
    pub(in crate::runtime) fn register_regex(
        &self,
        reference: crate::term::Reference,
        index: usize,
    ) {
        self.regexes
            .lock()
            .expect("the regex table is not poisoned")
            .insert(reference, index);
    }

    /// Return the scheduler of the compiled regular expression
    /// `reference`.
    pub(in crate::runtime) fn regex_owner(
        &self,
        reference: crate::term::Reference,
    ) -> Option<usize> {
        self.regexes
            .lock()
            .expect("the regex table is not poisoned")
            .get(&reference)
            .copied()
    }

    /// Record the compiled binary pattern `reference` of the scheduler
    /// `index`.
    pub(in crate::runtime) fn register_pattern(
        &self,
        reference: crate::term::Reference,
        index: usize,
    ) {
        self.patterns
            .lock()
            .expect("the pattern table is not poisoned")
            .insert(reference, index);
    }

    /// Return the scheduler of the compiled binary pattern `reference`.
    pub(in crate::runtime) fn pattern_owner(
        &self,
        reference: crate::term::Reference,
    ) -> Option<usize> {
        self.patterns
            .lock()
            .expect("the pattern table is not poisoned")
            .get(&reference)
            .copied()
    }

    /// Return the next port identifier, or the error when the counter
    /// ends.
    pub(in crate::runtime) fn next_port(&self) -> Result<PortId, BeamError> {
        let value = self.next_port.fetch_add(1, Ordering::Relaxed);
        if value == u64::MAX {
            return Err(error("port identifier overflow"));
        }
        Ok(PortId(value))
    }

    /// Return the next unlink operation identifier.
    pub(in crate::runtime) fn next_unlink_id(&self) -> u64 {
        self.next_unlink_id.fetch_add(1, Ordering::Relaxed)
    }

    /// Register `name` for `id`. Return false when another process has the
    /// name.
    pub(in crate::runtime) fn name_register(&self, name: &str, id: ProcessId) -> bool {
        // Hold the shard of `id` until the name is in, so the exit of `id`
        // cannot pass the registration.
        let Some(_shard) = self.processes.hold(id) else {
            return false;
        };
        let mut names = self.names.lock().expect("the name table is not poisoned");
        if names.contains_key(name) || names.values().any(|owner| *owner == id) {
            return false;
        }
        names.insert(name.to_owned(), id);
        true
    }

    /// Remove the process of `name`. Return the process that had it.
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

    /// Return the name of the process `id`.
    pub(in crate::runtime) fn name_of(&self, id: ProcessId) -> Option<String> {
        self.names
            .lock()
            .expect("the name table is not poisoned")
            .iter()
            .find_map(|(name, owner)| (*owner == id).then(|| name.clone()))
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
}

/// Write one byte to the wake socket, and drop the byte when the socket is
/// full: the scheduler reads the socket, and a full socket already wakes it.
fn wake(writer: &UnixStream) {
    let mut writer = writer;
    let _ = writer.write(&[1]);
}

#[cfg(test)]
#[path = "../../../tests/runtime/scheduler/node.rs"]
mod tests;

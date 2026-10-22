//! The state that the scheduler threads of one run share.
//!
//! Each scheduler owns the state of its processes, its run queue, its
//! working copy of the code table, and its services. The node holds what
//! must be the same for the whole run: the published code table, the process table, with the owner of each live process, the
//! registered names, the identifier
//! counters, the inbox of each scheduler for the deliveries that cross a
//! thread boundary, the roots and their results, and the shutdown state. A sender that runs on another
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
        atomic::{AtomicBool, AtomicI64, AtomicU64, Ordering},
    },
};

use super::{Signal, process_table::ProcessTable};
use crate::runtime::process::{MonitorLinkTree, ProcessState, TimerReference};
use crate::{
    error::{BeamError, error},
    runtime::{dirty::Pools, timer::NodeClock},
    term::{LiveBytes, PortId, ProcessId, Term},
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
    /// A cancel of the BIF timer `reference` of this scheduler, which
    /// another scheduler took from the node timer table. The receiver
    /// removes the timer from its timer wheel.
    CancelTimer { reference: crate::term::Reference },
    /// A runnable process that another scheduler gives to this scheduler
    /// ([`Node::transfer`]). The receiver puts it into its run queue.
    Process {
        process: Box<ProcessState>,
        /// The count that a `low` process has left in the shared queue
        /// (see [`super::run_queue::RunQueue::push_stolen`]).
        low: Option<u8>,
    },
}

/// The inbox of one scheduler: the deliveries that it did not apply yet,
/// and the write end of its wake socket.
#[derive(Debug)]
struct Inbox {
    queue: Mutex<VecDeque<Delivery>>,
    wake: UnixStream,
}

/// The roots of a run. A root is an ordinary process with a root mark: the
/// slot of its result here. The run ends after the exit of the last root
/// completes, or after the exit of a root that failed.
#[derive(Debug)]
struct Roots {
    /// The number of roots whose exit did not complete.
    live: usize,
    /// The result of each root by its slot: the return value or the exit
    /// reason. `None` until the exit of the root starts.
    results: Vec<Option<Result<Term, Term>>>,
}

impl Roots {
    /// Return the roots of a run with `slots` root slots and no root yet.
    fn new(slots: usize) -> Mutex<Self> {
        Mutex::new(Roots {
            live: 0,
            results: vec![None; slots],
        })
    }
}

/// The alias table of the node: the owner of each alias, and the aliases
/// of each owner, so the exit of a process removes only its own aliases, as
/// the ERTS exit walks only the monitors of the process
/// (`erl_process.c:14560-14569`).
#[derive(Debug, Default)]
struct Aliases {
    owners: HashMap<crate::term::Reference, ProcessId>,
    by_owner: HashMap<ProcessId, Vec<crate::term::Reference>>,
}

/// The node entry of one active BIF timer.
#[derive(Debug, Clone, Copy)]
struct TimerEntry {
    /// The scheduler whose timer wheel holds the timer.
    owner: usize,
    /// The end tick of the timer.
    end: i64,
    /// The receiver when it is a process.
    receiver: Option<ProcessId>,
}

/// The BIF timer table of the node. The removal of an entry decides
/// between the expiry and a cancel, as the state of an ERTS `ErtsBifTimer`
/// does (`access_btm`, `erl_hl_timer.c:1922-1971`). `trees` holds the timer
/// tree of each receiver process, as the ERTS tree `p->bif_timers`
/// (`erl_hl_timer.c:579-617`), so a cancel on any scheduler removes the
/// timer from the tree at once (`erl_hl_timer.c:1904-1911`).
#[derive(Debug, Default)]
struct BifTimers {
    entries: HashMap<crate::term::Reference, TimerEntry>,
    trees: HashMap<ProcessId, MonitorLinkTree<TimerReference, ()>>,
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
    /// The owner of each alias, by its reference. ERTS aliases belong to
    /// the node, and any scheduler sends to them
    /// (`erl_proc_sig_queue.c:2814-2895`).
    aliases: Mutex<Aliases>,
    /// The active BIF timers of the node.
    timers: Mutex<BifTimers>,
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
    /// The roots of the run and their results.
    roots: Mutex<Roots>,
    /// The number of schedulers of the node.
    schedulers: usize,
    /// The number of schedulers that wait in the idle state (see
    /// [`Self::enter_idle`]).
    idle: Mutex<usize>,
    /// One flag for each scheduler: true while the scheduler has no
    /// runnable process and waits for work that another scheduler can give
    /// it ([`Self::set_hungry`], [`Self::give_work`]).
    hungry: Vec<AtomicBool>,
    /// True after the last root exited, or after a root or a scheduler
    /// failed.
    shutdown: AtomicBool,
    /// The time support of the node: the run start and the one time
    /// correction state. Each scheduler builds its timer service on it, so
    /// the time of every scheduler agrees.
    clock: NodeClock,
    /// The dirty CPU pool and the dirty IO pool, which each scheduler of
    /// the node uses.
    dirty: Pools,
    /// The seed of the hash of `term_to_binary/2` with `local`
    /// (`local_node_hash`, external.c:134). The first call on any scheduler
    /// computes it, and each scheduler reads the same value.
    local_node_hash: OnceLock<u32>,
    /// The published code state of the run (see [`super::code`]).
    pub(in crate::runtime) code: super::code::NodeCode,
    /// The atomic arrays of the node. ERTS keeps each array in a magic
    /// binary that a process of any scheduler reaches by its reference
    /// (`erl_bif_atomics.c:90-110`).
    pub(in crate::runtime) atomics: crate::runtime::atomics::AtomicArrays,
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
                aliases: Mutex::new(Aliases::default()),
                timers: Mutex::new(BifTimers::default()),
                regexes: Mutex::new(HashMap::new()),
                patterns: Mutex::new(HashMap::new()),
                inboxes,
                next_pid: AtomicU64::new(0),
                next_reference: AtomicU64::new(0),
                unique: AtomicU64::new(0),
                unique_monotonic: AtomicI64::new(-1),
                next_port: AtomicU64::new(0),
                next_unlink_id: AtomicU64::new(1),
                roots: Roots::new(count),
                schedulers: count,
                idle: Mutex::new(0),
                hungry: (0..count).map(|_| AtomicBool::new(false)).collect(),
                shutdown: AtomicBool::new(false),
                clock: NodeClock::new(),
                dirty: Pools::new(count),
                local_node_hash: OnceLock::new(),
                code: super::code::NodeCode::default(),
                atomics: Default::default(),
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

    /// Return the time support of the node.
    pub(in crate::runtime) fn clock(&self) -> NodeClock {
        self.clock.clone()
    }

    /// Return the seed of the local node hash. `compute` runs at the first
    /// call on any scheduler, and each scheduler reads the same value.
    pub(in crate::runtime) fn local_node_hash(&self, compute: impl FnOnce() -> u32) -> u32 {
        *self.local_node_hash.get_or_init(compute)
    }

    /// Return the node of one scheduler that has no other scheduler. It has
    /// no inbox, so a one-thread run never wakes a scheduler, and one root
    /// slot.
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
            aliases: Mutex::new(Aliases::default()),
            timers: Mutex::new(BifTimers::default()),
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
            roots: Roots::new(1),
            schedulers: 1,
            idle: Mutex::new(0),
            hungry: Vec::new(),
            shutdown: AtomicBool::new(false),
            clock: NodeClock::new(),
            dirty: Pools::new(1),
            local_node_hash: OnceLock::new(),
            code: super::code::NodeCode::default(),
            atomics: Default::default(),
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

    /// Count the scheduler that calls as idle, and return true when the run
    /// is stuck: each scheduler of the node is idle, and no delivery waits
    /// in an inbox. A scheduler is idle when it has no process that can
    /// run, no release work, no timer, no dirty call, and no input source
    /// (see `idle_step`), so each live process waits and no source can
    /// make a process runnable.
    ///
    /// Only a scheduler that runs sends a delivery, and it puts the
    /// delivery into the inbox before it becomes idle. A scheduler leaves
    /// the idle state with [`Self::leave_idle`] before it takes its inbox.
    /// The count and the check of the inboxes hold one lock, so a scheduler
    /// cannot leave the idle state and take a delivery while the last idle
    /// scheduler reads the inboxes.
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// let (node, _readers) = Node::new(2).unwrap();
    /// assert!(!node.enter_idle());
    /// assert!(node.enter_idle());
    /// ```
    pub(in crate::runtime) fn enter_idle(&self) -> bool {
        let mut idle = self.idle.lock().expect("the idle count is not poisoned");
        *idle += 1;
        *idle == self.schedulers
            && self.inboxes.iter().all(|inbox| {
                inbox
                    .queue
                    .lock()
                    .expect("the scheduler inbox is not poisoned")
                    .is_empty()
            })
    }

    /// Count the scheduler that calls out of the idle state, after its wait
    /// ends and before it takes its inbox or runs a process.
    pub(in crate::runtime) fn leave_idle(&self) {
        *self.idle.lock().expect("the idle count is not poisoned") -= 1;
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

    /// Put a message for `target` into the inbox of its scheduler. `pending`
    /// is the monitor signal that goes before the message, as the send path
    /// gives it. The scheduler is read again while the shard of `target` is
    /// locked, so a message after a steal goes to the new scheduler and
    /// after the process ([`Self::transfer`]). `index` is the scheduler that
    /// the sender found; the message goes there when `target` left the
    /// table, and that scheduler handles it as for a dead target.
    pub(in crate::runtime) fn send_message(
        &self,
        index: usize,
        from: Option<ProcessId>,
        term: Term,
        target: ProcessId,
        pending: Option<Signal>,
    ) {
        let inbox = self.processes.with_owner(target, |owner| {
            let inbox = &self.inboxes[owner.unwrap_or(index)];
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
            inbox
        });
        wake(&inbox.wake);
    }

    /// Put the signal for `target` into the inbox of its scheduler. `pending`
    /// is the monitor signal that goes before it, as the send path gives it.
    /// The scheduler is read as [`Self::send_message`] reads it.
    pub(in crate::runtime) fn send_signal(
        &self,
        index: usize,
        target: ProcessId,
        sender: Option<ProcessId>,
        signal: Signal,
        pending: Option<Signal>,
    ) {
        let inbox = self.processes.with_owner(target, |owner| {
            let inbox = &self.inboxes[owner.unwrap_or(index)];
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
            inbox
        });
        wake(&inbox.wake);
    }

    /// Mark the scheduler `index` as a scheduler that has no runnable
    /// process (`hungry` true) or that has work again (`hungry` false). The
    /// mark changes under the lock of the idle count, as each steal reads it
    /// ([`Self::give_work`]). Thus after a call with `false`, each process
    /// that a steal gave the scheduler is in its inbox, and the scheduler
    /// takes it at its next step, before it runs a process.
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// node.set_hungry(1, true);
    /// assert!(node.has_hungry(0));
    /// ```
    pub(in crate::runtime) fn set_hungry(&self, index: usize, hungry: bool) {
        let _idle = self.idle.lock().expect("the idle count is not poisoned");
        self.hungry[index].store(hungry, Ordering::Relaxed);
    }

    /// Return whether a scheduler other than `index` has no runnable
    /// process. The read takes no lock, so it is only a hint for
    /// [`Self::give_work`].
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// assert!(!node.has_hungry(0));
    /// ```
    pub(in crate::runtime) fn has_hungry(&self, index: usize) -> bool {
        self.hungry
            .iter()
            .enumerate()
            .any(|(thief, hungry)| thief != index && hungry.load(Ordering::Relaxed))
    }

    /// Give work of the scheduler `from` to the first scheduler after
    /// `from`, in the cyclic order of the scheduler indices, that has no
    /// runnable process. ERTS gives the same order from the side of the
    /// thief: an empty run queue tries the queues after its own index
    /// (`try_steal_task`, `erl_process.c:4682-4697`). `take` gives the
    /// processes, and each moves with [`Self::transfer`]. The lock of the
    /// idle count stays held from the read of the mark to the last move,
    /// so the receiver cannot leave its wait between them (see
    /// [`Self::set_hungry`]). The receiver keeps its mark when `take` gives
    /// no process. Return the number of processes that moved.
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// node.set_hungry(1, true);
    /// assert_eq!(node.give_work(0, || vec![(process, None)]), 1);
    /// assert!(!node.has_hungry(0));
    /// ```
    pub(in crate::runtime) fn give_work(
        &self,
        from: usize,
        take: impl FnOnce() -> Vec<(ProcessState, Option<u8>)>,
    ) -> usize {
        let _idle = self.idle.lock().expect("the idle count is not poisoned");
        let count = self.hungry.len();
        let Some(thief) = (1..count)
            .map(|step| (from + step) % count)
            .find(|&thief| self.hungry[thief].load(Ordering::Relaxed))
        else {
            return 0;
        };
        let processes = take();
        if !processes.is_empty() {
            self.hungry[thief].store(false, Ordering::Relaxed);
        }
        let moved = processes.len();
        for (process, low) in processes {
            self.transfer(from, thief, Box::new(process), low);
        }
        moved
    }

    /// Give the runnable process `process` of the scheduler `from` to the
    /// scheduler `to`. While the shard of the process is locked, the owner
    /// in the process table becomes `to`, the deliveries for the process
    /// that wait in the inbox of `from` leave it in their order, and the
    /// inbox of `to` gets the process and then those deliveries. Each
    /// sender reads the owner under the same lock ([`Self::send_message`]),
    /// so a later delivery goes to `to` after them, and the deliveries of
    /// one sender keep their order.
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// node.transfer(0, 1, Box::new(process), None);
    /// assert_eq!(node.owner(id), Some(1));
    /// ```
    pub(in crate::runtime) fn transfer(
        &self,
        from: usize,
        to: usize,
        process: Box<ProcessState>,
        low: Option<u8>,
    ) {
        let id = process.id;
        self.processes.move_owner(id, to, || {
            let mut moved = Vec::new();
            {
                let mut queue = self.inboxes[from]
                    .queue
                    .lock()
                    .expect("the scheduler inbox is not poisoned");
                let mut kept = VecDeque::with_capacity(queue.len());
                for delivery in queue.drain(..) {
                    // A process for `from` stays: `from` runs, so no
                    // steal gives it a process (`Self::give_work`).
                    let follows = matches!(
                        &delivery,
                        Delivery::Message { target, .. } | Delivery::Signal { target, .. }
                            if *target == id
                    );
                    if follows {
                        moved.push(delivery);
                    } else {
                        kept.push_back(delivery);
                    }
                }
                *queue = kept;
            }
            let mut queue = self.inboxes[to]
                .queue
                .lock()
                .expect("the scheduler inbox is not poisoned");
            queue.push_back(Delivery::Process { process, low });
            queue.extend(moved);
        });
        wake(&self.inboxes[to].wake);
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

    /// Record the process `owner` as the owner of the alias `reference`.
    pub(in crate::runtime) fn register_alias(
        &self,
        reference: crate::term::Reference,
        owner: ProcessId,
    ) {
        let mut aliases = self
            .aliases
            .lock()
            .expect("the alias table is not poisoned");
        aliases.owners.insert(reference, owner);
        aliases.by_owner.entry(owner).or_default().push(reference);
    }

    /// Return the owner of the alias `reference`.
    pub(in crate::runtime) fn alias_owner(
        &self,
        reference: crate::term::Reference,
    ) -> Option<ProcessId> {
        self.aliases
            .lock()
            .expect("the alias table is not poisoned")
            .owners
            .get(&reference)
            .copied()
    }

    /// Remove each alias of the process `owner`, at its exit.
    pub(in crate::runtime) fn remove_aliases(&self, owner: ProcessId) {
        let mut aliases = self
            .aliases
            .lock()
            .expect("the alias table is not poisoned");
        for reference in aliases.by_owner.remove(&owner).unwrap_or_default() {
            aliases.owners.remove(&reference);
        }
    }

    /// Record the BIF timer `reference` of the scheduler `index` with the
    /// end tick `end` for `receiver`, and put it into the timer tree of a
    /// receiver process. For a receiver of another scheduler (`remote`),
    /// return false and record nothing when it is not a live process. The
    /// check holds the timer table lock, and the exit of the receiver marks
    /// it as exiting before it takes its tree
    /// ([`Self::take_receiver_timers`]). Thus no timer joins the tree of a
    /// receiver after its exit took the tree.
    pub(in crate::runtime) fn register_timer(
        &self,
        reference: crate::term::Reference,
        index: usize,
        end: i64,
        receiver: Option<ProcessId>,
        remote: bool,
    ) -> bool {
        let mut timers = self.timers.lock().expect("the timer table is not poisoned");
        if let Some(receiver) = receiver {
            if remote && self.live_owner(receiver).is_none() {
                return false;
            }
            timers
                .trees
                .entry(receiver)
                .or_default()
                .insert(TimerReference(reference), ());
        }
        let entry = TimerEntry {
            owner: index,
            end,
            receiver,
        };
        timers.entries.insert(reference, entry);
        true
    }

    /// Remove the active BIF timer `reference` from the node and from the
    /// timer tree of its receiver. Return its scheduler and its end tick,
    /// or `None` when no timer with the reference is active. An exiting
    /// receiver keeps the timer in its tree for its exit, as ERTS
    /// leaves it in the tree of an exiting process (`erl_hl_timer.c:1185-1192`,
    /// `1904-1911`).
    pub(in crate::runtime) fn take_timer(
        &self,
        reference: crate::term::Reference,
    ) -> Option<(usize, i64)> {
        let mut timers = self.timers.lock().expect("the timer table is not poisoned");
        let entry = timers.entries.remove(&reference)?;
        // An exiting receiver keeps the timer in its tree for its exit.
        if let Some(receiver) = entry.receiver
            && !(self.owner(receiver).is_some() && self.live_owner(receiver).is_none())
            && let Some(tree) = timers.trees.get_mut(&receiver)
        {
            tree.remove(&TimerReference(reference));
            if tree.is_empty() {
                timers.trees.remove(&receiver);
            }
        }
        Some((entry.owner, entry.end))
    }

    /// Return the end tick of the active BIF timer `reference`.
    pub(in crate::runtime) fn timer_end(&self, reference: crate::term::Reference) -> Option<i64> {
        self.timers
            .lock()
            .expect("the timer table is not poisoned")
            .entries
            .get(&reference)
            .map(|entry| entry.end)
    }

    /// Take the timer tree of `receiver` for its exit. The timers stay
    /// active until the exit cancels them.
    pub(in crate::runtime) fn take_receiver_timers(
        &self,
        receiver: ProcessId,
    ) -> MonitorLinkTree<TimerReference, ()> {
        self.timers
            .lock()
            .expect("the timer table is not poisoned")
            .trees
            .remove(&receiver)
            .unwrap_or_default()
    }

    /// Put a cancel of the BIF timer `reference` into the inbox of its
    /// scheduler `index`.
    pub(in crate::runtime) fn cancel_timer_at(
        &self,
        index: usize,
        reference: crate::term::Reference,
    ) {
        let inbox = &self.inboxes[index];
        inbox
            .queue
            .lock()
            .expect("the scheduler inbox is not poisoned")
            .push_back(Delivery::CancelTimer { reference });
        wake(&inbox.wake);
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

    /// Count a new root of the slot `slot`, before it starts.
    ///
    /// # Panics
    ///
    /// Panic when the node has no slot `slot`.
    pub(in crate::runtime) fn add_root(&self, slot: usize) {
        let mut roots = self.roots.lock().expect("the roots are not poisoned");
        assert!(slot < roots.results.len(), "the node has the root slot");
        roots.live += 1;
    }

    /// Record the result of the root of the slot `slot` when its exit
    /// starts: the return value, or the exit reason.
    pub(in crate::runtime) fn root_result(&self, slot: usize, result: Result<Term, Term>) {
        let mut roots = self.roots.lock().expect("the roots are not poisoned");
        roots.results[slot] = Some(result);
    }

    /// Record that the exit of the root of the slot `slot` completed. A
    /// failed root stops the run, as [`Self::abort`] does. The last root
    /// sets the shutdown state. Each root exit wakes each scheduler.
    pub(in crate::runtime) fn root_exit(&self, slot: usize) {
        let mut roots = self.roots.lock().expect("the roots are not poisoned");
        roots.live -= 1;
        if roots.live == 0 || matches!(roots.results[slot], Some(Err(_))) {
            self.shutdown.store(true, Ordering::Release);
        }
        drop(roots);
        for inbox in &self.inboxes {
            wake(&inbox.wake);
        }
    }

    /// Take the result of each root, in slot order. `None` is a root whose
    /// exit did not start.
    pub(in crate::runtime) fn take_root_results(&self) -> Vec<Option<Result<Term, Term>>> {
        let mut roots = self.roots.lock().expect("the roots are not poisoned");
        std::mem::take(&mut roots.results)
    }

    /// Account the terms of the root results in `live`.
    pub(in crate::runtime) fn account_roots(&self, live: &mut LiveBytes) {
        let roots = self.roots.lock().expect("the roots are not poisoned");
        for result in roots.results.iter().flatten() {
            match result {
                Ok(term) | Err(term) => live.term(term),
            }
        }
    }

    /// Stop every scheduler after a scheduler failed. Each scheduler
    /// returns from its loop.
    pub(in crate::runtime) fn abort(&self) {
        self.shutdown.store(true, Ordering::Release);
        for inbox in &self.inboxes {
            wake(&inbox.wake);
        }
    }

    /// Return true when the run ends: after the last root exited, or after
    /// a root or a scheduler failed.
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

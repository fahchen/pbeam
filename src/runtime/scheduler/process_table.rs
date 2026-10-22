//! The process table of the node: the pid, the scheduler, the exiting
//! state, and the insert interval of each live process, in the index order
//! of the ERTS process table, and the state of the `erlang:processes/0`
//! reads. Each scheduler of the node reads the same table, so a pid lookup,
//! `erlang:processes/0`, and `erts_internal:processes_next/1` give the same
//! result on each scheduler. The state of a process stays with the
//! scheduler that runs it (see [`super::process_slots::ProcessSlots`]).
//!
//! ERTS keeps the table as an array of atomic pointers, and a lookup takes
//! no lock (`erts_ptab_pix2intptr_nob`, `erl_ptab.h:344-348`). The insert
//! and the delete of an element hold the table rwlock for a read, and a
//! chunk read of `erlang:processes/0` holds it for a write
//! (`erl_ptab.c:540-558`, `687-728`, `970-1020`). This table puts the
//! entries into [`SHARDS`] shards, each with its own lock, by blocks of
//! [`BLOCK`] table indices. A lookup locks only the shard of the pid. The
//! state of the reads has the rwlock: an insert and a delete hold it for a
//! read, and a read of a chunk holds it for a write, as in ERTS.
//!
//! The reads of `erlang:processes/0` share one deleted list, as ERTS does
//! (`erl_ptab.c:643-672`, `888-1280`). Each entry keeps the insert and the
//! delete interval of its process, and each read keeps its start interval,
//! the interval of each table chunk that it read, and its position in the
//! list. A read decides from these intervals whether it lists an entry, so
//! an exit does not look at the reads. Only the oldest read frees the
//! entries that it steps through. Each operation of the list does a fixed
//! quantity of work: it looks at the first position of the reads, and it
//! drops at most [`DROP_PER_STEP`] entries that no read can step through.

use std::collections::{BTreeMap, HashMap, VecDeque};
use std::sync::{Mutex, MutexGuard, RwLock};

use super::PTAB_SIZE;
use crate::term::ProcessId;

/// The maximum number of entries of the deleted list that one operation
/// drops when no read can step through them. Each exit adds at most one
/// entry, so the entries that no read can see go away at the same rate as
/// the exits add them, also after the exit of a read.
const DROP_PER_STEP: usize = 2;

/// The number of shards of the table.
const SHARDS: usize = 64;

/// The number of consecutive table indices in one shard. A chunk read of
/// `erlang:processes/0` of 1000 indices locks one or two shards, and the
/// processes of a burst of spawns go to more than one shard.
const BLOCK: u64 = 1024;

#[cfg(test)]
thread_local! {
    /// The entries of the deleted list and the read positions that the
    /// process table added, dropped, or read since the last reset.
    pub(super) static DELETED_WORK: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}

/// Count `n` units of work on the deleted list.
#[cfg(test)]
fn count_work(n: usize) {
    DELETED_WORK.set(DELETED_WORK.get() + n);
}

#[cfg(not(test))]
fn count_work(_: usize) {}

/// An `erlang:processes/0` call that reads the table, as the invocation
/// entry of the deleted list of ERTS (`ptlbdp->bif_invocation`,
/// `erl_ptab.c:922-951`).
#[derive(Debug)]
struct Reader {
    /// The interval at the start of the call, as the invocation interval:
    /// the call lists only a process with a smaller insert interval
    /// (`erl_ptab.c:933-934`, `1000-1003`, `1108`).
    start: u64,
    /// The first table index that the call did not read.
    tix: u64,
    /// The end index and the interval of each table chunk that the call
    /// read, in index order, as `ptlbdp->chunk[cix].interval`
    /// (`erl_ptab.c:978-982`).
    chunks: Vec<(u64, u64)>,
    /// The position in the deleted list of the next entry that the call
    /// did not step through, as the place of the invocation entry in the
    /// ERTS list (`erl_ptab.c:1168-1180`).
    pos: u64,
}

impl Reader {
    /// Return whether the call lists the deleted entry `entry`: the process
    /// was in the table at the start of the call, and it left the table
    /// before the call read the chunk of its index (`erl_ptab.c:1102-1135`).
    /// A chunk that the call did not read has a later interval than each
    /// deleted entry. The work is in proportion to the logarithm of the
    /// fixed number of chunks.
    fn lists(&self, entry: &Deleted) -> bool {
        let ix = table_index(entry.id);
        let cix = self.chunks.partition_point(|(end, _)| *end <= ix);
        entry.inserted < self.start
            && self
                .chunks
                .get(cix)
                .is_none_or(|(_, interval)| entry.deleted < *interval)
    }
}

/// An entry of the deleted list of the table: a process that left the
/// table while a call read it (`save_deleted_element`,
/// `erl_ptab.c:643-672`).
#[derive(Debug)]
struct Deleted {
    /// The pid of the process.
    id: ProcessId,
    /// The interval when the process entered the table.
    inserted: u64,
    /// The interval when the process left the table.
    deleted: u64,
}

/// Return the index of the pid `id` in the ERTS process table.
fn table_index(id: ProcessId) -> u64 {
    id.0 % PTAB_SIZE
}

/// Remove the entry of the process `id` with the table index `ix` from its
/// locked shard.
///
/// # Panics
///
/// Panic when the shard has no entry of the pid.
fn take(shard: &mut Shard, ix: u64, id: ProcessId) -> Entry {
    shard
        .remove(&(ix, id))
        .expect("a process leaves the process table once")
}

/// The entry of one live process.
#[derive(Debug, Clone, Copy)]
struct Entry {
    /// The scheduler that runs the process.
    owner: usize,
    /// True when the process exits, as the ERTS `EXITING` state flag.
    exiting: bool,
    /// The interval when the process entered the table, as the
    /// `started_interval` of an ERTS element (`erl_ptab.c:568-569`).
    inserted: u64,
}

/// The entries of the table indices of some blocks, by table index and
/// pid.
type Shard = BTreeMap<(u64, ProcessId), Entry>;

/// The state of the `erlang:processes/0` reads of the table.
#[derive(Debug, Default)]
struct Listing {
    /// The current interval of the table (`erts_ptab_interval`). The start
    /// of a read and the read of each chunk after the first step it
    /// (`erl_ptab.c:933-934`, `978-982`).
    interval: u64,
    /// The `erlang:processes/0` calls that read the table now, by the pid
    /// of the caller.
    readers: HashMap<ProcessId, Reader>,
    /// The number of reads at each position of the deleted list, as the
    /// order of the invocation entries in the ERTS list. The first key is
    /// the position of the oldest read.
    positions: BTreeMap<u64, usize>,
    /// The deleted list: the entries that a call did not step through
    /// yet, the oldest first (`ptab->list.data.deleted`).
    deleted: VecDeque<Deleted>,
    /// The position of the first entry of `deleted`.
    deleted_base: u64,
}

/// The process table of a node, which each scheduler of the node shares.
#[derive(Debug)]
pub(super) struct ProcessTable {
    /// The entries, by blocks of [`BLOCK`] table indices.
    shards: Box<[Mutex<Shard>]>,
    /// The state of the reads, as the ERTS table rwlock guards the deleted
    /// list (`erl_ptab.c:687-692`, `970-975`).
    listing: RwLock<Listing>,
}

impl Default for ProcessTable {
    fn default() -> Self {
        ProcessTable {
            shards: (0..SHARDS).map(|_| Mutex::default()).collect(),
            listing: RwLock::default(),
        }
    }
}

impl ProcessTable {
    /// Lock the shard of the table index `ix`.
    fn shard(&self, ix: u64) -> MutexGuard<'_, Shard> {
        self.shards[(ix / BLOCK) as usize % SHARDS]
            .lock()
            .expect("the process table shard is not poisoned")
    }

    /// Return the entry of the process `id`.
    fn entry(&self, id: ProcessId) -> Option<Entry> {
        let ix = table_index(id);
        self.shard(ix).get(&(ix, id)).copied()
    }

    /// Add the process `id` of the scheduler `owner`, with the current
    /// interval as its insert interval.
    ///
    /// # Panics
    ///
    /// Panic when the table has a process with the same pid: each pid is
    /// new.
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// table.insert(ProcessId(1), 0);
    /// assert_eq!(table.owner(ProcessId(1)), Some(0));
    /// ```
    pub(super) fn insert(&self, id: ProcessId, owner: usize) {
        let listing = self
            .listing
            .read()
            .expect("the process list state is not poisoned");
        let ix = table_index(id);
        let entry = Entry {
            owner,
            exiting: false,
            inserted: listing.interval,
        };
        let old = self.shard(ix).insert((ix, id), entry);
        assert!(old.is_none(), "each process has a new pid");
    }

    /// Mark the process `id` as exiting.
    ///
    /// # Panics
    ///
    /// Panic when no process has the pid.
    pub(super) fn set_exiting(&self, id: ProcessId) {
        let ix = table_index(id);
        self.shard(ix)
            .get_mut(&(ix, id))
            .expect("an exiting process is in the process table")
            .exiting = true;
    }

    /// Return the scheduler of the process `id`, also when it exits, as
    /// `erts_proc_lookup_raw` (`erl_process_lock.h:1135-1149`) finds it.
    pub(super) fn owner(&self, id: ProcessId) -> Option<usize> {
        self.entry(id).map(|entry| entry.owner)
    }

    /// Return the scheduler of the process `id` when it does not exit, as
    /// `erts_proc_lookup` (`erl_process_lock.h:1151-1157`) finds it.
    pub(super) fn live_owner(&self, id: ProcessId) -> Option<usize> {
        self.entry(id)
            .and_then(|entry| (!entry.exiting).then_some(entry.owner))
    }

    /// Call `deliver` with the scheduler of the process `id`, or `None` when
    /// the process is not in the table, while the shard of the process is
    /// locked. A steal ([`Self::move_owner`]) changes the scheduler under
    /// the same lock, so a delivery that `deliver` puts into the inbox of
    /// the scheduler is in that inbox before the steal reads it.
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// table.insert(ProcessId(1), 0);
    /// assert_eq!(table.with_owner(ProcessId(1), |owner| owner), Some(0));
    /// ```
    pub(super) fn with_owner<R>(
        &self,
        id: ProcessId,
        deliver: impl FnOnce(Option<usize>) -> R,
    ) -> R {
        let ix = table_index(id);
        let shard = self.shard(ix);
        let result = deliver(shard.get(&(ix, id)).map(|entry| entry.owner));
        drop(shard);
        result
    }

    /// Make `to` the scheduler of the process `id`, and call `handoff` while
    /// the shard of the process is locked, so that no delivery reads the
    /// scheduler while the process moves (see [`Self::with_owner`]).
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// table.move_owner(ProcessId(1), 1, || {});
    /// assert_eq!(table.owner(ProcessId(1)), Some(1));
    /// ```
    ///
    /// # Panics
    ///
    /// Panic when the process is not in the table or exits: only a
    /// runnable process that does not exit moves.
    pub(super) fn move_owner(&self, id: ProcessId, to: usize, handoff: impl FnOnce()) {
        let ix = table_index(id);
        let mut shard = self.shard(ix);
        let entry = shard
            .get_mut(&(ix, id))
            .expect("a process that moves is in the process table");
        assert!(!entry.exiting, "a process that exits does not move");
        entry.owner = to;
        handoff();
    }

    /// Return whether the process `id` is in the table, and hold its shard
    /// until the guard drops, so that the process cannot leave the table
    /// before the guard drops.
    pub(super) fn hold(&self, id: ProcessId) -> Option<impl Sized + '_> {
        let ix = table_index(id);
        let shard = self.shard(ix);
        shard.contains_key(&(ix, id)).then_some(shard)
    }

    /// Remove the process `id`, then run `after` while its shard is locked.
    /// A process that left the table while a read is in progress goes to
    /// the deleted list, as `erts_ptab_delete_element` saves it
    /// (`erl_ptab.c:687-728`). The exit of a reader ends its read, as
    /// `cleanup_ptab_list_bif_data` does (`erl_ptab.c:796-885`).
    ///
    /// # Panics
    ///
    /// Panic when no process has the pid: a process leaves the table once.
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// table.remove(ProcessId(1), || {});
    /// assert_eq!(table.owner(ProcessId(1)), None);
    /// ```
    pub(super) fn remove(&self, id: ProcessId, after: impl FnOnce()) {
        let ix = table_index(id);
        let listing = self
            .listing
            .read()
            .expect("the process list state is not poisoned");
        // ERTS saves a deleted element only while a read is in progress
        // (`erl_ptab.c:688-692`, `722-728`).
        if listing.readers.is_empty() && listing.deleted.is_empty() {
            let mut shard = self.shard(ix);
            take(&mut shard, ix, id);
            after();
            return;
        }
        drop(listing);
        let mut listing = self
            .listing
            .write()
            .expect("the process list state is not poisoned");
        let mut shard = self.shard(ix);
        let entry = take(&mut shard, ix, id);
        after();
        drop(shard);
        listing.drop_reader(id);
        if !listing.readers.is_empty() {
            count_work(1);
            let deleted = listing.interval;
            listing.deleted.push_back(Deleted {
                id,
                inserted: entry.inserted,
                deleted,
            });
        }
        listing.prune_deleted();
    }

    /// Start an `erlang:processes/0` read of the table by the process
    /// `reader`, and return the pid of each process with a table index
    /// below `end`, in index order: a process that enters the table after
    /// this start is not in the result. The start and the read of the
    /// first chunk hold the lock of the read state together, as ERTS holds
    /// the table lock from the start of the call to the end of its first
    /// chunk (`erl_ptab.c:905-1020`). A delete between them would have the
    /// interval of the start, and neither the chunk nor the deleted list
    /// would give its pid.
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// assert_eq!(table.start_list(ProcessId(0), 1000), vec![ProcessId(0)]);
    /// ```
    pub(super) fn start_list(&self, reader: ProcessId, end: u64) -> Vec<ProcessId> {
        let mut listing = self
            .listing
            .write()
            .expect("the process list state is not poisoned");
        listing.interval += 1;
        let pos = listing.deleted_base + listing.deleted.len() as u64;
        let start = Reader {
            start: listing.interval,
            tix: 0,
            chunks: Vec::new(),
            pos,
        };
        listing.readers.insert(reader, start);
        count_work(1);
        *listing.positions.entry(pos).or_default() += 1;
        self.read_chunk(&mut listing, reader, end)
    }

    /// Return the pid of each process with a table index from the first
    /// index that the read of `reader` did not read up to `end`, in index
    /// order, when the process was in the table at the start of the read.
    /// The read then continues at `end`. The work is in proportion to the
    /// processes of these indices and to the blocks of the indices.
    ///
    /// # Panics
    ///
    /// Panic when `reader` does not read the table.
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// assert_eq!(table.list_chunk(ProcessId(0), 2000), vec![ProcessId(1000)]);
    /// ```
    pub(super) fn list_chunk(&self, reader: ProcessId, end: u64) -> Vec<ProcessId> {
        let mut listing = self
            .listing
            .write()
            .expect("the process list state is not poisoned");
        self.read_chunk(&mut listing, reader, end)
    }

    /// Read the next chunk of the read of `reader` up to `end` while the
    /// caller holds the lock of the read state `listing` (see
    /// [`Self::list_chunk`]).
    fn read_chunk(&self, listing: &mut Listing, reader: ProcessId, end: u64) -> Vec<ProcessId> {
        let reader = listing
            .readers
            .get_mut(&reader)
            .expect("the process reads the table");
        let start = reader.start;
        let tix = std::mem::replace(&mut reader.tix, end);
        // The first chunk uses the invocation interval, and each later
        // chunk steps the interval (`erl_ptab.c:978-982`).
        let interval = if tix == 0 {
            start
        } else {
            listing.interval += 1;
            listing.interval
        };
        reader.chunks.push((end, interval));
        let mut ids = Vec::new();
        self.scan(tix, end, |id, entry| {
            if entry.inserted < start {
                ids.push(id);
            }
            true
        });
        ids
    }

    /// Call `visit` with each process with a table index from `start` up
    /// to `end`, in index order, until `visit` returns false. The call
    /// holds the lock of one shard at a time.
    fn scan(&self, start: u64, end: u64, mut visit: impl FnMut(ProcessId, &Entry) -> bool) {
        let mut low = start;
        while low < end {
            let high = end.min((low / BLOCK + 1) * BLOCK);
            let shard = self.shard(low);
            for ((_, id), entry) in shard.range((low, ProcessId(0))..(high, ProcessId(0))) {
                if !visit(*id, entry) {
                    return;
                }
            }
            low = high;
        }
    }

    /// Step the read of `reader` to the next entry of the deleted list.
    /// Return `None` at the end of the list. Else return the pid of the
    /// entry when the read lists it, and whether the read frees the entry:
    /// no other read must step through it, as ERTS frees the entries
    /// before the next invocation entry only in the oldest read
    /// (`erl_ptab.c:1069-1099`, `1137-1143`). The step does not look at
    /// the other reads: it compares intervals, and it reads the first
    /// position of the reads.
    ///
    /// # Panics
    ///
    /// Panic when `reader` does not read the table.
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// assert_eq!(table.next_deleted(ProcessId(0)), None);
    /// ```
    pub(super) fn next_deleted(&self, reader: ProcessId) -> Option<(Option<ProcessId>, bool)> {
        let mut listing = self
            .listing
            .write()
            .expect("the process list state is not poisoned");
        let read = &listing.readers[&reader];
        let pos = read.pos;
        let entry = listing.deleted.get((pos - listing.deleted_base) as usize)?;
        let listed = read.lists(entry).then_some(entry.id);
        // The read is the oldest when it is alone at the first position.
        count_work(1);
        let free = listing.positions.first_key_value() == Some((&pos, &1));
        listing
            .readers
            .get_mut(&reader)
            .expect("the process reads the table")
            .pos += 1;
        listing.move_position(pos, pos + 1);
        listing.prune_deleted();
        Some((listed, free))
    }

    /// Return whether the read of `reader` stepped through each entry of
    /// the deleted list.
    ///
    /// # Panics
    ///
    /// Panic when `reader` does not read the table.
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// assert!(table.deleted_done(ProcessId(0)));
    /// ```
    pub(super) fn deleted_done(&self, reader: ProcessId) -> bool {
        let listing = self
            .listing
            .read()
            .expect("the process list state is not poisoned");
        listing.readers[&reader].pos == listing.deleted_base + listing.deleted.len() as u64
    }

    /// End the read of `reader`.
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// table.end_list(ProcessId(0));
    /// ```
    pub(super) fn end_list(&self, reader: ProcessId) {
        let mut listing = self
            .listing
            .write()
            .expect("the process list state is not poisoned");
        listing.drop_reader(reader);
        listing.prune_deleted();
    }

    /// Return the index and the pid of each process in the table, also an
    /// exiting process, with an index from `start` up to `end`, in the index
    /// order of the ERTS table, up to `max` processes that `keep` accepts,
    /// as `erts_ptab_processes_next` reads the elements of the table with
    /// no table lock (`erl_ptab.c:1514-1523`). The work is in proportion to
    /// the processes of these indices and to the blocks of the indices.
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// assert_eq!(table.indexed(0, 25, 10, |_| true), vec![(0, ProcessId(0))]);
    /// ```
    pub(super) fn indexed(
        &self,
        start: u64,
        end: u64,
        max: usize,
        mut keep: impl FnMut(ProcessId) -> bool,
    ) -> Vec<(u64, ProcessId)> {
        let mut found = Vec::new();
        self.scan(start, end, |id, _| {
            if keep(id) {
                found.push((table_index(id), id));
            }
            found.len() < max
        });
        found
    }
}

impl Listing {
    /// Move one read from the position `from` to the position `to` of the
    /// deleted list.
    fn move_position(&mut self, from: u64, to: u64) {
        self.remove_position(from);
        count_work(1);
        *self.positions.entry(to).or_default() += 1;
    }

    /// Remove one read from the position `pos` of the deleted list.
    fn remove_position(&mut self, pos: u64) {
        count_work(1);
        let count = self
            .positions
            .get_mut(&pos)
            .expect("each read has a position");
        *count -= 1;
        if *count == 0 {
            self.positions.remove(&pos);
        }
    }

    /// End the read of `reader` when it reads the table.
    fn drop_reader(&mut self, reader: ProcessId) {
        if let Some(read) = self.readers.remove(&reader) {
            self.remove_position(read.pos);
        }
    }

    /// Drop at most [`DROP_PER_STEP`] entries of the deleted list that each
    /// read stepped through, the oldest first. The oldest read steps
    /// through one entry in each step, so it drops the entry that it frees.
    fn prune_deleted(&mut self) {
        let end = self.deleted_base + self.deleted.len() as u64;
        let first = self
            .positions
            .first_key_value()
            .map_or(end, |(pos, _)| *pos);
        for _ in 0..DROP_PER_STEP {
            if self.deleted_base >= first {
                break;
            }
            count_work(1);
            self.deleted.pop_front();
            self.deleted_base += 1;
        }
    }
}

#[cfg(test)]
#[path = "../../../tests/runtime/scheduler/process_table.rs"]
mod tests;

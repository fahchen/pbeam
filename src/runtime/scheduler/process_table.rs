//! The process table of a scheduler: each live process in a slot, an index
//! from the pid to the slot, and the slot of the current process.
//!
//! ERTS keeps the current process as a pointer in the scheduler data
//! (`current_process`, `erl_process.h:698`), and the JIT keeps it in a
//! register (`c_p`, `jit/arm/beam_asm.hpp:86`), so the emulator reads the
//! running process with no pid lookup. Only a lookup of another process by
//! its pid goes through the process table (`erts_proc_lookup_raw`,
//! `erl_process_lock.h:1135-1149`). This table gives the same split: the
//! slot of the current process takes the place of the pointer, and only a
//! lookup by pid hashes the pid.
//!
//! The slots do not give an order of the processes. Their only iteration,
//! [`ProcessTable::values`], adds the live bytes of each process, and that
//! sum does not depend on the order. The table also keeps the pids in the
//! index order of the ERTS process table, for the reads of
//! `erlang:processes/0` (see [`ProcessTable::list_chunk`]).
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
use std::ops::Index;

use super::PTAB_SIZE;
use crate::{runtime::process::ProcessState, term::ProcessId};

/// The maximum number of entries of the deleted list that one operation
/// drops when no read can step through them. Each exit adds at most one
/// entry, so the entries that no read can see go away at the same rate as
/// the exits add them, also after the exit of a read.
const DROP_PER_STEP: usize = 2;

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

/// The live processes of a scheduler and its current process.
///
/// Each process stays in one slot from its insert to its removal, so the slot
/// of the current process stays valid while the process runs. A removal puts
/// the slot on the free list, and the next insert uses it again. The current
/// process cannot leave the table: the scheduler switches it out first.
#[derive(Debug, Default)]
pub(super) struct ProcessTable {
    /// The slots of the processes. A free slot holds `None`.
    slots: Vec<Option<ProcessState>>,
    /// The free slots, most recent last.
    free: Vec<usize>,
    /// The slot of each live process.
    index: HashMap<ProcessId, usize>,
    /// The slot of the process that runs now, like `current_process` in the
    /// ERTS scheduler data.
    current: Option<usize>,
    /// The insert interval of each live process, in the index order of the
    /// ERTS process table, as the `started_interval` of each element of
    /// `ptab->r.o.tab` (`erl_ptab.c:996-1015`).
    order: BTreeMap<(u64, ProcessId), u64>,
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

impl ProcessTable {
    /// Store `process` in a free slot, or in a new slot when no slot is
    /// free.
    ///
    /// # Panics
    ///
    /// Panic when the table has a process with the same pid: each pid is
    /// new.
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// table.insert(ProcessState::new(ProcessId(1), Start::Entry(frame), vec![]));
    /// assert!(table.contains_key(&ProcessId(1)));
    /// ```
    pub(super) fn insert(&mut self, process: ProcessState) {
        let id = process.id;
        let slot = match self.free.pop() {
            Some(slot) => {
                self.slots[slot] = Some(process);
                slot
            }
            None => {
                self.slots.push(Some(process));
                self.slots.len() - 1
            }
        };
        let old = self.index.insert(id, slot);
        assert!(old.is_none(), "each process has a new pid");
        self.order.insert((table_index(id), id), self.interval);
    }

    /// Remove the process `id` and return it. Return `None` when no process
    /// has the pid.
    ///
    /// # Panics
    ///
    /// Panic when `id` is the current process: the scheduler switches a
    /// process out before it leaves the table.
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// let process = table.remove(&ProcessId(1)).unwrap();
    /// assert!(!table.contains_key(&ProcessId(1)));
    /// ```
    pub(super) fn remove(&mut self, id: &ProcessId) -> Option<ProcessState> {
        let slot = self.index.remove(id)?;
        assert_ne!(
            self.current,
            Some(slot),
            "the current process switches out before it leaves the table"
        );
        self.free.push(slot);
        let ix = table_index(*id);
        let inserted = self
            .order
            .remove(&(ix, *id))
            .expect("each live process has an index");
        // The exit of a reader ends its read, as
        // `cleanup_ptab_list_bif_data` does (`erl_ptab.c:796-885`).
        self.drop_reader(*id);
        // ERTS saves a deleted element only while a read is in progress
        // (`erl_ptab.c:688-692`, `722-728`).
        if !self.readers.is_empty() {
            count_work(1);
            self.deleted.push_back(Deleted {
                id: *id,
                inserted,
                deleted: self.interval,
            });
        }
        self.prune_deleted();
        self.slots[slot].take()
    }

    /// Start an `erlang:processes/0` read of the table by the process
    /// `reader`: a process that enters the table after this start is not in
    /// the result.
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// table.start_list(ProcessId(0));
    /// ```
    pub(super) fn start_list(&mut self, reader: ProcessId) {
        self.interval += 1;
        let pos = self.deleted_base + self.deleted.len() as u64;
        let start = Reader {
            start: self.interval,
            tix: 0,
            chunks: Vec::new(),
            pos,
        };
        self.readers.insert(reader, start);
        count_work(1);
        *self.positions.entry(pos).or_default() += 1;
    }

    /// Return the pid of each process with a table index from the first
    /// index that the read of `reader` did not read up to `end`, in index
    /// order, when the process was in the table at the start of the read.
    /// The read then continues at `end`. The work is in proportion to the
    /// processes of these indices.
    ///
    /// # Panics
    ///
    /// Panic when `reader` does not read the table.
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// assert_eq!(table.list_chunk(ProcessId(0), 1000).collect::<Vec<_>>(), vec![ProcessId(0)]);
    /// ```
    pub(super) fn list_chunk(
        &mut self,
        reader: ProcessId,
        end: u64,
    ) -> impl Iterator<Item = ProcessId> + '_ {
        let reader = self
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
            self.interval += 1;
            self.interval
        };
        reader.chunks.push((end, interval));
        self.order
            .range((tix, ProcessId(0))..(end, ProcessId(0)))
            .filter(move |(_, inserted)| **inserted < start)
            .map(|((_, id), _)| *id)
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
    pub(super) fn next_deleted(&mut self, reader: ProcessId) -> Option<(Option<ProcessId>, bool)> {
        let read = &self.readers[&reader];
        let pos = read.pos;
        let entry = self.deleted.get((pos - self.deleted_base) as usize)?;
        let listed = read.lists(entry).then_some(entry.id);
        // The read is the oldest when it is alone at the first position.
        count_work(1);
        let free = self.positions.first_key_value() == Some((&pos, &1));
        self.readers
            .get_mut(&reader)
            .expect("the process reads the table")
            .pos += 1;
        self.move_position(pos, pos + 1);
        self.prune_deleted();
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
        self.readers[&reader].pos == self.deleted_base + self.deleted.len() as u64
    }

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

    /// End the read of `reader`.
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// table.end_list(ProcessId(0));
    /// ```
    pub(super) fn end_list(&mut self, reader: ProcessId) {
        self.drop_reader(reader);
        self.prune_deleted();
    }

    /// Return the process `id`, or `None` when no process has the pid.
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// assert_eq!(table.get(&ProcessId(1)).map(|process| process.id), Some(ProcessId(1)));
    /// ```
    pub(super) fn get(&self, id: &ProcessId) -> Option<&ProcessState> {
        let slot = *self.index.get(id)?;
        self.slots[slot].as_ref()
    }

    /// Return the process `id` for a change, or `None` when no process has
    /// the pid.
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// table.get_mut(&ProcessId(1)).unwrap().waiting = true;
    /// ```
    pub(super) fn get_mut(&mut self, id: &ProcessId) -> Option<&mut ProcessState> {
        let slot = *self.index.get(id)?;
        self.slots[slot].as_mut()
    }

    /// Return whether a process has the pid `id`.
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// assert!(!table.contains_key(&ProcessId(99)));
    /// ```
    pub(super) fn contains_key(&self, id: &ProcessId) -> bool {
        self.index.contains_key(id)
    }

    /// Return each process of the table, in no given order.
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// assert_eq!(table.values().count(), 2);
    /// ```
    pub(super) fn values(&self) -> impl Iterator<Item = &ProcessState> {
        self.slots.iter().flatten()
    }

    /// Return whether the table has no process.
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// assert!(ProcessTable::default().is_empty());
    /// ```
    #[cfg(test)]
    pub(super) fn is_empty(&self) -> bool {
        self.index.is_empty()
    }

    /// Make the process `id` the current process and return it.
    ///
    /// # Panics
    ///
    /// Panic when no process has the pid.
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// table.set_current(ProcessId(1)).fcalls = CONTEXT_REDS;
    /// assert_eq!(table.current_id(), Some(ProcessId(1)));
    /// ```
    pub(super) fn set_current(&mut self, id: ProcessId) -> &mut ProcessState {
        let slot = self.index[&id];
        self.current = Some(slot);
        self.slots[slot]
            .as_mut()
            .expect("an indexed slot holds a process")
    }

    /// Return the pid of the current process, or `None` when no process runs.
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// assert_eq!(table.current_id(), None);
    /// ```
    pub(super) fn current_id(&self) -> Option<ProcessId> {
        self.current().map(|process| process.id)
    }

    /// Return the current process, or `None` when no process runs. The read
    /// uses the slot of the process and does not hash its pid.
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// assert_eq!(table.current().map(|process| process.id), Some(ProcessId(1)));
    /// ```
    #[inline]
    pub(super) fn current(&self) -> Option<&ProcessState> {
        self.slots.get(self.current?)?.as_ref()
    }

    /// Return the current process for a change, or `None` when no process
    /// runs. The read uses the slot of the process and does not hash its pid.
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// table.current_mut().unwrap().fcalls -= 1;
    /// ```
    #[inline]
    pub(super) fn current_mut(&mut self) -> Option<&mut ProcessState> {
        self.slots.get_mut(self.current?)?.as_mut()
    }

    /// Clear the current process and return it, or `None` when no process
    /// runs.
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// table.take_current().unwrap().reductions += used;
    /// assert_eq!(table.current_id(), None);
    /// ```
    pub(super) fn take_current(&mut self) -> Option<&mut ProcessState> {
        self.slots.get_mut(self.current.take()?)?.as_mut()
    }
}

impl Index<&ProcessId> for ProcessTable {
    type Output = ProcessState;

    /// Return the process `id`.
    ///
    /// # Panics
    ///
    /// Panic when no process has the pid.
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// assert!(!table[&ProcessId(1)].waiting);
    /// ```
    fn index(&self, id: &ProcessId) -> &ProcessState {
        self.get(id).expect("the process is alive")
    }
}

#[cfg(test)]
#[path = "../../../tests/runtime/scheduler/process_table.rs"]
mod tests;

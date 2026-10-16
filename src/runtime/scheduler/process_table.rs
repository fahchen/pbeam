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

use std::collections::{BTreeMap, HashMap, VecDeque};
use std::ops::Index;

use super::PTAB_SIZE;
use crate::{runtime::process::ProcessState, term::ProcessId};

/// An `erlang:processes/0` call that reads the table, as the invocation
/// entry of the deleted list of ERTS (`ptlbdp->bif_invocation`,
/// `erl_ptab.c:922-951`).
#[derive(Debug)]
struct Reader {
    /// The insert serial at the start of the call, as the invocation
    /// interval: the call lists only a process with a smaller serial
    /// (`erl_ptab.c:1000-1003`, `1108`).
    start: u64,
    /// The first table index that the call did not read.
    tix: u64,
    /// One entry for each process that left the table since the start of
    /// the call, the oldest first, as the deleted list
    /// (`save_deleted_element`, `erl_ptab.c:643-672`): the pid of a process
    /// that was in the table at the start and left it before the call read
    /// its index (`erl_ptab.c:1102-1135`), and `None` for each other
    /// process.
    deleted: VecDeque<Option<ProcessId>>,
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
    /// The insert serial of each live process, in the index order of the
    /// ERTS process table, as the `started_interval` of each element of
    /// `ptab->r.o.tab` (`erl_ptab.c:996-1015`).
    order: BTreeMap<(u64, ProcessId), u64>,
    /// The insert serial of the next process, as the interval of the ERTS
    /// table (`erts_ptab_interval`).
    serial: u64,
    /// The `erlang:processes/0` calls that read the table now, by the pid
    /// of the caller.
    readers: HashMap<ProcessId, Reader>,
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
        self.order.insert((table_index(id), id), self.serial);
        self.serial += 1;
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
        let serial = self
            .order
            .remove(&(ix, *id))
            .expect("each live process has an index");
        // ERTS saves a deleted element only while a read is in progress
        // (`erl_ptab.c:688-692`, `722-728`).
        self.readers.remove(id);
        for reader in self.readers.values_mut() {
            let gone = serial < reader.start && ix >= reader.tix;
            reader.deleted.push_back(gone.then_some(*id));
        }
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
        let start = Reader {
            start: self.serial,
            tix: 0,
            deleted: VecDeque::new(),
        };
        self.readers.insert(reader, start);
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
        self.order
            .range((tix, ProcessId(0))..(end, ProcessId(0)))
            .filter(move |(_, serial)| **serial < start)
            .map(|((_, id), _)| *id)
    }

    /// Return the entries of the processes that left the table during the
    /// read of `reader`, the oldest first (see [`Reader::deleted`]).
    ///
    /// # Panics
    ///
    /// Panic when `reader` does not read the table.
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// assert_eq!(table.list_deleted(ProcessId(0)).pop_front(), None);
    /// ```
    pub(super) fn list_deleted(&mut self, reader: ProcessId) -> &mut VecDeque<Option<ProcessId>> {
        &mut self
            .readers
            .get_mut(&reader)
            .expect("the process reads the table")
            .deleted
    }

    /// End the read of `reader`.
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// table.end_list(ProcessId(0));
    /// ```
    pub(super) fn end_list(&mut self, reader: ProcessId) {
        self.readers.remove(&reader);
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

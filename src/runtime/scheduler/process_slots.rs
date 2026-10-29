//! The processes that a scheduler runs: each process in a slot, an index
//! from the pid to the slot, and the slot of the current process.
//!
//! ERTS keeps the current process as a pointer in the scheduler data
//! (`current_process`, `erl_process.h:698`), and the JIT keeps it in a
//! register (`c_p`, `jit/arm/beam_asm.hpp:86`), so the emulator reads the
//! running process with no pid lookup. Only a lookup of another process by
//! its pid goes through the process table (`erts_proc_lookup_raw`,
//! `erl_process_lock.h:1135-1149`). These slots give the same split: the
//! slot of the current process takes the place of the pointer, and only a
//! lookup by pid hashes the pid.
//!
//! The process table of the node ([`super::process_table::ProcessTable`])
//! holds the pid, the scheduler, and the table order of each process of
//! the node. The slots hold the state of the processes of this scheduler.
//! The slots do not give an order of the processes. Their only iteration,
//! [`ProcessSlots::values`], adds the live bytes of each process, and that
//! sum does not depend on the order.

use std::collections::HashMap;
use std::ops::Index;

use crate::{runtime::process::ProcessState, term::ProcessId};

/// The processes of a scheduler and its current process.
///
/// Each process stays in one slot from its insert to its removal, so the slot
/// of the current process stays valid while the process runs. A removal puts
/// the slot on the free list, and the next insert uses it again. The current
/// process cannot leave the table: the scheduler switches it out first.
#[derive(Debug, Default)]
pub(super) struct ProcessSlots {
    /// The slots of the processes. A free slot holds `None`.
    slots: Vec<Option<ProcessState>>,
    /// The free slots, most recent last.
    free: Vec<usize>,
    /// The slot of each live process.
    index: HashMap<ProcessId, usize>,
    /// The slot of the process that runs now, like `current_process` in the
    /// ERTS scheduler data.
    current: Option<usize>,
}

impl ProcessSlots {
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
    /// assert!(table.contains_key(ProcessId(1)));
    /// ```
    pub(super) fn insert(&mut self, process: ProcessState) {
        let id = process.id;
        let slot = if let Some(slot) = self.free.pop() {
            self.slots[slot] = Some(process);
            slot
        } else {
            self.slots.push(Some(process));
            self.slots.len() - 1
        };
        let old = self.index.insert(id, slot);
        assert!(old.is_none(), "each process has a new pid");
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
    /// let process = table.remove(ProcessId(1)).unwrap();
    /// assert!(!table.contains_key(ProcessId(1)));
    /// ```
    pub(super) fn remove(&mut self, id: ProcessId) -> Option<ProcessState> {
        let slot = self.index.remove(&id)?;
        assert_ne!(
            self.current,
            Some(slot),
            "the current process switches out before it leaves the table"
        );
        self.free.push(slot);
        self.slots[slot].take()
    }

    /// Return the process `id`, or `None` when no process has the pid.
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// assert_eq!(table.get(ProcessId(1)).map(|process| process.id), Some(ProcessId(1)));
    /// ```
    pub(super) fn get(&self, id: ProcessId) -> Option<&ProcessState> {
        let slot = *self.index.get(&id)?;
        self.slots[slot].as_ref()
    }

    /// Return the process `id` for a change, or `None` when no process has
    /// the pid.
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// table.get_mut(ProcessId(1)).unwrap().waiting = true;
    /// ```
    pub(super) fn get_mut(&mut self, id: ProcessId) -> Option<&mut ProcessState> {
        let slot = *self.index.get(&id)?;
        self.slots[slot].as_mut()
    }

    /// Return whether a process has the pid `id`.
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// assert!(!table.contains_key(ProcessId(99)));
    /// ```
    pub(super) fn contains_key(&self, id: ProcessId) -> bool {
        self.index.contains_key(&id)
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
    /// assert!(ProcessSlots::default().is_empty());
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

impl Index<&ProcessId> for ProcessSlots {
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
        self.get(*id).expect("the process is alive")
    }
}

#[cfg(test)]
#[path = "../../../tests/runtime/scheduler/process_slots.rs"]
mod tests;

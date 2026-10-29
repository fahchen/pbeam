//! Persistent terms of the node, as the ERTS persistent term hash table
//! (`erl_bif_persistent.c`). All processes of the node share one store,
//! also on more than one scheduler thread.

use std::sync::{Arc, atomic::Ordering};

use super::scheduler::Scheduler;
use crate::term::{LiveBytes, ProcessId, Term, flat_size, internal_hash};

/// The number of scheduling steps of the node after which a new table is
/// visible. ERTS waits for thread progress, and with `erl +S 1` other
/// processes run two or more turns before the caller continues. The step
/// that follows the switch out of the caller counts too, so 3 steps give
/// other processes two turns.
const PROGRESS_STEPS: u8 = 3;

/// The change of a `put/2` or an `erase/1`.
#[derive(Debug)]
pub(in crate::runtime) enum Change {
    /// Store the value for the key.
    Put(Term, Term),
    /// Erase the key.
    Erase(Term),
}

/// A change that waits for thread progress, and the caller that it
/// suspends (`erts_schedule_thr_prgr_later_op`, `erl_bif_persistent.c:774-779`).
#[derive(Debug)]
struct Progress {
    /// The suspended caller, which holds the update permission.
    caller: ProcessId,
    /// The scheduler of the caller, which counts the steps. It takes a
    /// step only after the caller switched out, so the change is never
    /// visible before the caller suspends.
    scheduler: usize,
    /// The copied table with the change, or `None` for a change in place of
    /// the visible table (`fast_update_index`, `erl_bif_persistent.c:763-770`).
    table: Option<PersistentTerms>,
    /// The change.
    change: Change,
    /// The scheduling steps that remain.
    steps: u8,
}

/// The table copy of the holder of the update permission, which goes
/// through the slots of the visible table in steps (`copy_table`,
/// `erl_bif_persistent.c:1034-1139`).
#[derive(Debug)]
struct Copy {
    /// The new table. Its slots are made in the copy steps too.
    table: PersistentTerms,
    /// The number of slots of the new table.
    size: usize,
    /// The next slot of the visible table to copy.
    next: usize,
    /// The erased key, which the copy leaves out.
    skip: Option<Term>,
}

/// The persistent term table of the node and its update permission, as
/// `the_hash_table`, `updater_process`, and `update_queue` of
/// `erl_bif_persistent.c`. The node keeps it behind one lock.
#[derive(Debug, Default)]
pub(in crate::runtime) struct PersistentTermStore {
    /// The visible table. A reader takes a reference and reads with no
    /// lock, and an update swaps in a new table, as `the_hash_table` is an
    /// atomic pointer in ERTS.
    pub(super) terms: Arc<PersistentTerms>,
    /// The process that holds the update permission: a `put/2` or an
    /// `erase/1` until thread progress, or a `get/0` that traps until its
    /// last chunk (`erl_bif_persistent.c:1311-1365`).
    updater: Option<ProcessId>,
    /// The processes that wait for the update permission.
    waiters: Vec<ProcessId>,
    /// The changes that wait for thread progress.
    progress: Vec<Progress>,
    /// The table copy of the holder of the update permission.
    copy: Option<Copy>,
}

impl PersistentTermStore {
    /// Give the update permission to `id` and return true, or put `id`
    /// into the wait queue and return false when another process holds it,
    /// as `try_seize_update_permission` does
    /// (`erl_bif_persistent.c:1311-1335`).
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// let mut store = PersistentTermStore::default();
    /// assert!(store.seize(ProcessId(1)));
    /// assert!(!store.seize(ProcessId(2)));
    /// ```
    pub(super) fn seize(&mut self, id: ProcessId) -> bool {
        match self.updater {
            Some(holder) if holder != id => {
                if !self.waiters.contains(&id) {
                    self.waiters.push(id);
                }
                false
            }
            _ => {
                self.updater = Some(id);
                self.waiters.retain(|waiter| *waiter != id);
                true
            }
        }
    }

    /// Release the update permission of `id` and return the processes that
    /// wait for it, as `release_update_permission` takes "the entire herd"
    /// (`erl_bif_persistent.c:1337-1365`). Return no process when `id`
    /// does not hold the permission.
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// let mut store = PersistentTermStore::default();
    /// store.seize(ProcessId(1));
    /// store.seize(ProcessId(2));
    /// assert_eq!(store.release(ProcessId(1)), [ProcessId(2)]);
    /// ```
    pub(super) fn release(&mut self, id: ProcessId) -> Vec<ProcessId> {
        // The publication of a table that waits for thread progress
        // releases the permission of its caller.
        if self.updater != Some(id) || self.is_progressing(id) {
            return Vec::new();
        }
        self.updater = None;
        // The copy of a holder that exited ends with no change, as its trap
        // state destructor frees it (`erl_bif_persistent.c:295-301`).
        self.copy = None;
        // ERTS puts each waiter at the head of its queue and resumes from
        // the head (`erl_bif_persistent.c:1325-1331`, `1353-1363`).
        let mut waiters = std::mem::take(&mut self.waiters);
        waiters.reverse();
        waiters
    }

    /// Return true when the table of the caller `id` waits for thread
    /// progress.
    pub(super) fn is_progressing(&self, id: ProcessId) -> bool {
        self.progress.iter().any(|progress| progress.caller == id)
    }

    /// Start a copy of the visible table into a new table of `size`
    /// slots, which leaves out the key `skip`. The copy steps make the
    /// slots, as `copy_table` clears them in its budget
    /// (`erl_bif_persistent.c:1082-1103`).
    pub(super) fn start_copy(&mut self, size: usize, skip: Option<Term>) {
        self.copy = Some(Copy {
            table: PersistentTerms {
                buckets: Vec::with_capacity(size),
                num_entries: 0,
            },
            size,
            next: 0,
            skip,
        });
    }

    /// Copy at most `slots` slots of the visible table into the copy.
    pub(super) fn copy_step(&mut self, slots: usize) {
        if let Some(copy) = &mut self.copy {
            let made = copy.table.buckets.len();
            let new = (copy.size - made).min(slots);
            copy.table.buckets.resize(made + new, None);
            let slots = slots - new;
            if copy.table.buckets.len() < copy.size {
                return;
            }
            let end = copy
                .next
                .saturating_add(slots)
                .min(self.terms.buckets.len());
            for (key, value) in self.terms.buckets[copy.next..end].iter().flatten() {
                if copy.skip.as_ref() != Some(key) {
                    copy.table.insert(key.clone(), value.clone());
                }
            }
            copy.next = end;
        }
    }

    /// Make `change` visible after thread progress, and keep the update
    /// permission of `caller` until then. A table copy ends first, and the
    /// change goes into the copy.
    pub(super) fn schedule(&mut self, caller: ProcessId, scheduler: usize, change: Change) {
        self.copy_step(usize::MAX);
        let table = self.copy.take().map(|copy| {
            let mut table = copy.table;
            if let Change::Put(key, value) = &change {
                table.put(key, value);
            }
            table
        });
        self.progress.push(Progress {
            caller,
            scheduler,
            table,
            change,
            steps: PROGRESS_STEPS,
        });
    }
}

impl Scheduler {
    /// Count one scheduling step for each table that waits for thread
    /// progress. A table whose steps end becomes visible, its caller
    /// releases the update permission and resumes, as `table_updater` does
    /// (`erl_bif_persistent.c:1172-1234`). Return true when a table still
    /// waits, so that the run keeps stepping.
    pub(super) fn persistent_term_progress(&mut self) -> bool {
        let node = Arc::clone(self.node());
        if !node.persistent_progress().load(Ordering::Acquire) {
            return false;
        }
        let mut done = Vec::new();
        let pending = {
            let mut store = node.persistent_terms();
            let index = self.index();
            for progress in &mut store.progress {
                if progress.scheduler == index {
                    progress.steps -= 1;
                }
            }
            while let Some(index) = store
                .progress
                .iter()
                .position(|progress| progress.steps == 0)
            {
                let progress = store.progress.remove(index);
                match (progress.table, progress.change) {
                    (Some(table), _) => store.terms = Arc::new(table),
                    (None, Change::Put(key, value)) => {
                        Arc::make_mut(&mut store.terms).put(&key, &value);
                    }
                    (None, Change::Erase(key)) => {
                        Arc::make_mut(&mut store.terms).erase(&key);
                    }
                }
                done.push(progress.caller);
            }
            node.persistent_progress()
                .store(!store.progress.is_empty(), Ordering::Release);
            // Only a change of this scheduler keeps it stepping.
            store
                .progress
                .iter()
                .any(|progress| progress.scheduler == index)
        };
        // ERTS resumes the updater before its waiters
        // (`erl_bif_persistent.c:1342-1363`).
        for caller in done {
            self.resume(caller);
            self.release_persistent_terms(caller);
        }
        pending
    }

    /// Release the update permission of the persistent terms that the
    /// process `id` holds, and resume each process that waits for it. The
    /// end of a call releases it, and so does the exit of the process, as
    /// the destructor of the ERTS trap state does.
    pub(super) fn release_persistent_terms(&mut self, id: ProcessId) {
        let waiters = self.node().persistent_terms().release(id);
        for waiter in waiters {
            self.resume(waiter);
        }
    }
}

/// The number of slots of a new table (`INITIAL_SIZE`).
const INITIAL_SIZE: usize = 8;
/// The fill level in percent at which a new key doubles the table
/// (`LOAD_FACTOR` in `MUST_GROW`).
const LOAD_FACTOR: usize = 50;

/// The persistent terms of the node, in the layout of the ERTS table. ERTS
/// keeps one open-addressing table for the node. A key starts at the slot
/// `erts_internal_hash(Key)` masked by the table size, and goes to the next
/// slot, after the last slot to slot 0, until it finds a free slot or a key
/// that is exactly equal (`lookup`). A new key that finds the table half
/// full first doubles the table, which puts the old keys again in old slot
/// order (`put_common`, `copy_table`). `persistent_term:get/0` returns the
/// pairs in descending slot order (`get_all`), so the layout is visible to
/// BEAM code.
#[derive(Debug, Clone)]
pub(super) struct PersistentTerms {
    /// The slots of the table. The number of slots is a power of two.
    buckets: Vec<Option<(Term, Term)>>,
    /// The number of stored keys.
    num_entries: usize,
}

impl Default for PersistentTerms {
    /// Return an empty table of 8 slots, as `create_initial_table` does.
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// assert_eq!(PersistentTerms::default().buckets.len(), 8);
    /// ```
    fn default() -> Self {
        PersistentTerms {
            buckets: vec![None; INITIAL_SIZE],
            num_entries: 0,
        }
    }
}

impl PersistentTerms {
    /// Put the pair of a table copy into its slot.
    fn insert(&mut self, key: Term, value: Term) {
        let slot = self.lookup(&key);
        self.buckets[slot] = Some((key, value));
        self.num_entries += 1;
    }

    /// Return the value of `key`, or `None` when the key has no value. This
    /// is the lookup of `persistent_term_get_2` in `erl_bif_persistent.c`,
    /// which never raises.
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// let mut terms = PersistentTerms::default();
    /// assert_eq!(terms.get(&Term::static_atom("key")), None);
    /// terms.put(&Term::static_atom("key"), &Term::Integer(1));
    /// assert_eq!(terms.get(&Term::static_atom("key")), Some(&Term::Integer(1)));
    /// ```
    pub(super) fn get(&self, key: &Term) -> Option<&Term> {
        self.buckets[self.lookup(key)]
            .as_ref()
            .map(|(_, value)| value)
    }

    /// Count the guest terms that the table holds: the key and the value of
    /// each stored pair.
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// let mut terms = PersistentTerms::default();
    /// terms.put(&Term::static_atom("key"), &Term::tuple(vec![Term::Integer(1)]));
    /// let mut live = LiveBytes::new();
    /// terms.account_terms(&mut live);
    /// assert!(live.bytes() > 0);
    /// ```
    pub(super) fn account_terms(&self, live: &mut LiveBytes) {
        for (key, value) in self.buckets.iter().flatten() {
            live.term(key);
            live.term(value);
        }
    }

    /// Store `value` for `key`. A key that is exactly equal to a stored key
    /// keeps its slot and gets the new value. A new key first doubles the
    /// table when the table is half full. This is `put_common` of
    /// `persistent_term_put_2` in `erl_bif_persistent.c`, which accepts any
    /// key and any value.
    ///
    /// Return `None` when the key has exactly this value already, for which
    /// ERTS changes nothing (`erl_bif_persistent.c:726-729`). Else return
    /// the number of slots that the copy of a doubled table goes through:
    /// the slots of the new table, then of the old table
    /// (`copy_table`, `erl_bif_persistent.c:1082-1126`), or 0.
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// let mut terms = PersistentTerms::default();
    /// assert_eq!(terms.put(&Term::static_atom("key"), &Term::Integer(1)), Some(0));
    /// assert_eq!(terms.put(&Term::static_atom("key"), &Term::Integer(1)), None);
    /// assert_eq!(terms.put(&Term::static_atom("key"), &Term::Integer(2)), Some(0));
    /// assert_eq!(terms.get(&Term::static_atom("key")), Some(&Term::Integer(2)));
    /// ```
    pub(super) fn put(&mut self, key: &Term, value: &Term) -> Option<usize> {
        let mut slot = self.lookup(key);
        if let Some((_, existing)) = &mut self.buckets[slot] {
            if existing == value {
                return None;
            }
            *existing = value.clone();
            return Some(0);
        }
        let mut copied = 0;
        // MUST_GROW: 100 * num_entries >= LOAD_FACTOR * allocated.
        if 100 * self.num_entries >= LOAD_FACTOR * self.buckets.len() {
            copied = 3 * self.buckets.len();
            self.grow();
            slot = self.lookup(key);
        }
        self.buckets[slot] = Some((key.clone(), value.clone()));
        self.num_entries += 1;
        Some(copied)
    }

    /// Return what [`Self::put`] of `key` and `value` would return, and
    /// change nothing.
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// let terms = PersistentTerms::default();
    /// assert_eq!(terms.put_cost(&Term::static_atom("key"), &Term::Integer(1)), Some(0));
    /// ```
    pub(super) fn put_cost(&self, key: &Term, value: &Term) -> Option<usize> {
        match &self.buckets[self.lookup(key)] {
            Some((_, existing)) if existing == value => None,
            None if 100 * self.num_entries >= LOAD_FACTOR * self.buckets.len() => {
                Some(3 * self.buckets.len())
            }
            Some(_) | None => Some(0),
        }
    }

    /// Return the number of stored keys and the bytes that ERTS gives as
    /// the `memory` item of `persistent_term:info/0`: the `HashTable`
    /// struct with one slot word, the other slot words, and the literal
    /// area of each `{Key, Value}` tuple (`do_info`,
    /// `erl_bif_persistent.c:949-971`). A literal area is two header words
    /// and the words of the tuple (`ERTS_LITERAL_AREA_ALLOC_SIZE`,
    /// `global.h:948-959`). PBeam counts the words with [`flat_size`], so a
    /// subterm that the pair shares counts once for each reference, where
    /// `copy_shared_calculate` counts it once (`erl_bif_persistent.c:751`).
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// assert_eq!(PersistentTerms::default().info(), (0, 160));
    /// ```
    pub(super) fn info(&self) -> (usize, usize) {
        /// `sizeof(HashTable)` of a 64-bit emulator: five `Uint` fields,
        /// the `DeleteOp`, and one slot (`erl_bif_persistent.c:51-66`).
        const HASH_TABLE_BYTES: usize = 104;
        /// The `off_heap` and `end` words of an `ErtsLiteralArea`.
        const AREA_HEADER_WORDS: usize = 2;
        /// The header and the two elements of a `{Key, Value}` tuple.
        const PAIR_WORDS: usize = 3;
        let areas: usize = self
            .buckets
            .iter()
            .flatten()
            .map(|(key, value)| AREA_HEADER_WORDS + PAIR_WORDS + flat_size(key) + flat_size(value))
            .sum();
        let memory = HASH_TABLE_BYTES + (self.buckets.len() - 1 + areas) * size_of::<u64>();
        (self.num_entries, memory)
    }

    /// Return what [`Self::erase`] of `key` would return, and change
    /// nothing.
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// let terms = PersistentTerms::default();
    /// assert_eq!(terms.erase_cost(&Term::static_atom("key")), None);
    /// ```
    pub(super) fn erase_cost(&self, key: &Term) -> Option<(usize, usize)> {
        let slot = self.lookup(key);
        self.buckets[slot].as_ref()?;
        let must_shrink = 200 * self.num_entries <= LOAD_FACTOR * self.buckets.len()
            && self.buckets.len() > INITIAL_SIZE;
        let mask = self.buckets.len() - 1;
        if !must_shrink && self.buckets[(slot + 1) & mask].is_none() {
            return Some((0, 0));
        }
        let size = if must_shrink {
            self.buckets.len() / 2
        } else {
            self.buckets.len()
        };
        Some((self.buckets.len(), size + self.buckets.len()))
    }

    /// Erase the value of `key`, as `persistent_term_erase_1` does
    /// (`erl_bif_persistent.c:417-549`). Return `None` for a key without a
    /// value. Else return the slots of the two table copies, which are 0
    /// and 0 for an erase in place. A slot whose next slot is free is
    /// cleared in place (`is_erasable`, `erl_bif_persistent.c:1025-1030`),
    /// which copies no slot. Else the table keeps no erased entry, so the
    /// remaining pairs go again in old slot order into a table of the same
    /// size, or of half the size when the table is mostly empty
    /// (`MUST_SHRINK`, `erl_bif_persistent.c:47-48`), as `copy_table` does:
    /// a temporary copy goes through the old slots, and the new table
    /// through its own slots and the old slots. Each copy has its own limit
    /// in a call (`erl_bif_persistent.c:275-293`).
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// let mut terms = PersistentTerms::default();
    /// terms.put(&Term::static_atom("key"), &Term::Integer(1));
    /// assert_eq!(terms.erase(&Term::static_atom("key")), Some((0, 0)));
    /// assert_eq!(terms.erase(&Term::static_atom("key")), None);
    /// assert_eq!(terms.get(&Term::static_atom("key")), None);
    /// ```
    pub(super) fn erase(&mut self, key: &Term) -> Option<(usize, usize)> {
        let copied = self.erase_cost(key)?;
        let slot = self.lookup(key);
        self.num_entries -= 1;
        // It is safe to clear the slot when it is no stepping stone to the
        // next slot (`is_erasable`).
        if copied == (0, 0) {
            self.buckets[slot] = None;
            return Some(copied);
        }
        let size = copied.1 - copied.0;
        let mut old = std::mem::replace(&mut self.buckets, vec![None; size]);
        old[slot] = None;
        for (key, value) in old.into_iter().flatten() {
            let slot = self.lookup(&key);
            self.buckets[slot] = Some((key, value));
        }
        Some(copied)
    }

    /// Return all stored pairs as the elements `{Key, Value}` of a list, in
    /// descending slot order. `do_get_all` of `persistent_term_get_0` in
    /// `erl_bif_persistent.c` reads the slots from slot 0 and puts each pair
    /// at the head of the list. A trap continues the same list, so the order
    /// does not change. `persistent_term:get/0` never raises.
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// let mut terms = PersistentTerms::default();
    /// // true hashes to slot 7 of 8, and 7 to slot 3.
    /// terms.put(&Term::static_atom("true"), &Term::Integer(1));
    /// terms.put(&Term::Integer(7), &Term::Integer(4));
    /// assert_eq!(
    ///     terms.get_all(),
    ///     [
    ///         Term::tuple(vec![Term::static_atom("true"), Term::Integer(1)]),
    ///         Term::tuple(vec![Term::Integer(7), Term::Integer(4)]),
    ///     ]
    /// );
    /// ```
    pub(super) fn get_all(&self) -> Vec<Term> {
        self.buckets
            .iter()
            .rev()
            .flatten()
            .map(|(key, value)| Term::tuple(vec![key.clone(), value.clone()]))
            .collect()
    }

    /// Return the slot of `key`: the slot that holds an exactly equal key,
    /// or the first free slot on its probe path (`lookup`).
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// // The internal hash of true selects slot 7 of 8.
    /// assert_eq!(PersistentTerms::default().lookup(&Term::static_atom("true")), 7);
    /// ```
    fn lookup(&self, key: &Term) -> usize {
        let mask = self.buckets.len() - 1;
        let mut slot = internal_hash(key) as usize & mask;
        while let Some((existing, _)) = &self.buckets[slot] {
            if existing == key {
                break;
            }
            slot = (slot + 1) & mask;
        }
        slot
    }

    /// Double the table and put each stored pair again, in old slot order
    /// from slot 0 (`copy_table`).
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// let mut terms = PersistentTerms::default();
    /// terms.grow();
    /// assert_eq!(terms.buckets.len(), 16);
    /// ```
    fn grow(&mut self) {
        let size = self.buckets.len() * 2;
        let old = std::mem::replace(&mut self.buckets, vec![None; size]);
        for (key, value) in old.into_iter().flatten() {
            let slot = self.lookup(&key);
            self.buckets[slot] = Some((key, value));
        }
    }
}

#[cfg(test)]
#[path = "../../tests/runtime/persistent_term.rs"]
mod tests;

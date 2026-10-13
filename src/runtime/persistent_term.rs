//! Persistent terms of the node, as the ERTS persistent term hash table
//! (`erl_bif_persistent.c`). All processes of a run share one store.

use crate::term::{LiveBytes, Term, internal_hash};

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
#[derive(Debug)]
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

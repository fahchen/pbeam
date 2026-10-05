//! Persistent terms of the node, as the ERTS persistent term hash table
//! (`erl_bif_persistent.c`). All processes of a run share one store.

use crate::term::{Term, internal_hash};

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
/// order (`put_common`, `copy_table`). The ERTS table order is visible only
/// through `persistent_term:get/0`, which returns the `[{Key, Value}]` list
/// in table order. PBeam does not implement it yet.
/// `persistent_term:info/0` returns only `#{count := Count, memory := Memory}`.
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

    /// Store `value` for `key`. A key that is exactly equal to a stored key
    /// keeps its slot and gets the new value. A new key first doubles the
    /// table when the table is half full. This is `put_common` of
    /// `persistent_term_put_2` in `erl_bif_persistent.c`, which accepts any
    /// key and any value.
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// let mut terms = PersistentTerms::default();
    /// terms.put(&Term::static_atom("key"), &Term::Integer(1));
    /// terms.put(&Term::static_atom("key"), &Term::Integer(2));
    /// assert_eq!(terms.get(&Term::static_atom("key")), Some(&Term::Integer(2)));
    /// ```
    pub(super) fn put(&mut self, key: &Term, value: &Term) {
        let mut slot = self.lookup(key);
        if let Some((_, existing)) = &mut self.buckets[slot] {
            *existing = value.clone();
            return;
        }
        // MUST_GROW: 100 * num_entries >= LOAD_FACTOR * allocated.
        if 100 * self.num_entries >= LOAD_FACTOR * self.buckets.len() {
            self.grow();
            slot = self.lookup(key);
        }
        self.buckets[slot] = Some((key.clone(), value.clone()));
        self.num_entries += 1;
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
mod tests {
    use super::PersistentTerms;
    use crate::term::{Term, test_atom as atom};

    fn logger_config(name: &str) -> Term {
        Term::Tuple(vec![atom("logger_config"), atom(name)])
    }

    /// Return the slot of each stored key, by slot.
    fn slots(terms: &PersistentTerms) -> Vec<(usize, Term)> {
        terms
            .buckets
            .iter()
            .enumerate()
            .filter_map(|(slot, bucket)| bucket.as_ref().map(|(key, _)| (slot, key.clone())))
            .collect()
    }

    #[test]
    fn puts_and_gets_values_by_exactly_equal_keys() {
        let mut terms = PersistentTerms::default();
        assert_eq!(terms.get(&logger_config("default")), None);
        terms.put(&logger_config("default"), &Term::Integer(1));
        terms.put(&logger_config("other"), &Term::Integer(2));
        terms.put(&Term::Integer(1), &Term::Integer(3));
        // A key that is equal to a stored key, but is another term, finds its value.
        assert_eq!(
            terms.get(&logger_config("default")),
            Some(&Term::Integer(1))
        );
        assert_eq!(terms.get(&logger_config("other")), Some(&Term::Integer(2)));
        assert_eq!(terms.get(&Term::Integer(1)), Some(&Term::Integer(3)));
        assert_eq!(terms.get(&Term::Tuple(vec![atom("logger_config")])), None);
        // An equal key replaces the value and adds no key.
        terms.put(&logger_config("default"), &Term::List(vec![]));
        assert_eq!(
            terms.get(&logger_config("default")),
            Some(&Term::List(vec![]))
        );
        assert_eq!(terms.num_entries, 3);
    }

    #[test]
    fn keeps_the_erts_slots_through_collisions_and_a_resize() {
        let (true_, ok) = (Term::static_atom("true"), Term::static_atom("ok"));
        let mut terms = PersistentTerms::default();
        terms.put(&true_, &Term::Integer(1));
        terms.put(&Term::Integer(39), &Term::Integer(2));
        terms.put(&ok, &Term::Integer(3));
        terms.put(&Term::Integer(7), &Term::Integer(4));
        // true and 39 both hash to slot 7 of 8: 39 goes past the end to slot 0, and ok
        // (slot 0) goes to slot 1.
        assert_eq!(terms.buckets.len(), 8);
        assert_eq!(
            slots(&terms),
            [
                (0, Term::Integer(39)),
                (1, ok.clone()),
                (3, Term::Integer(7)),
                (7, true_.clone())
            ]
        );
        assert_eq!(terms.get(&Term::Integer(39)), Some(&Term::Integer(2)));
        // A missing key that hashes to slot 7 walks past the end to the free slot 2.
        assert_eq!(terms.get(&Term::Tuple(vec![ok.clone()])), None);

        // The fifth key finds the table half full and doubles it. The old keys go
        // again in old slot order: 39, ok, 7, true. {ok} and true both hash to slot 7.
        let ok_tuple = Term::Tuple(vec![ok.clone()]);
        terms.put(&ok_tuple, &Term::Integer(5));
        assert_eq!(terms.buckets.len(), 16);
        assert_eq!(
            slots(&terms),
            [
                (0, ok.clone()),
                (3, Term::Integer(7)),
                (7, true_.clone()),
                (8, ok_tuple.clone()),
                (15, Term::Integer(39))
            ]
        );
        assert_eq!(terms.get(&ok_tuple), Some(&Term::Integer(5)));

        // A new value for a stored key keeps its slot and adds no key.
        terms.put(&ok, &Term::Integer(30));
        assert_eq!(terms.num_entries, 5);
        assert_eq!(slots(&terms)[0], (0, ok.clone()));
        assert_eq!(terms.get(&ok), Some(&Term::Integer(30)));
    }
}

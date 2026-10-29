//! The key order of an `ordered_set` table, as the AVL tree of
//! `erl_db_tree.c` keeps it: an insert, a delete, and a lookup cost
//! O(log n), and a walk costs O(log n) to start.
//!
//! The keys go in a std `BTreeSet`. A key compares with the ERTS term order,
//! which needs the atom table, so each key holds the shared atom table of the
//! run. A lookup searches the set with a [`Locate`] probe through `Borrow`,
//! so a lookup can compare with the arithmetic term order, keep the error of
//! a comparison, or split the keys where a predicate stops to give `true`.

use std::borrow::Borrow;
use std::cell::RefCell;
use std::cmp::Ordering;
use std::collections::BTreeSet;
use std::ops::Bound;
use std::sync::Arc;

use crate::error::BeamError;
use crate::term::{AtomTable, Direction, Term};

/// The keys of an `ordered_set` table in the ERTS term order, as the tree
/// orders them with `cmp_key_eq` (`erl_db_tree.c:758-761`). Two keys that
/// compare equal (`1` and `1.0`) are one key.
#[derive(Debug, Clone)]
pub(super) struct KeyOrder {
    atoms: Arc<AtomTable>,
    keys: BTreeSet<OrderKey>,
}

/// A key of the order with the atom table to compare it.
#[derive(Debug, Clone)]
struct OrderKey {
    term: Term,
    atoms: Arc<AtomTable>,
}

/// A search of the order: the place of a key of the order against the
/// searched place.
trait Locate {
    /// Return the key of the order, which only a stored key has.
    fn term(&self) -> &Term;
    /// Return the order of the searched place against the key `key`.
    fn locate(&self, key: &Term) -> Ordering;
}

impl Locate for OrderKey {
    fn term(&self) -> &Term {
        &self.term
    }

    /// Compare a new key with a stored key. A put searches with a
    /// [`Probe`] first, so a comparison of the put cannot fail.
    fn locate(&self, key: &Term) -> Ordering {
        self.term
            .compare(key, &self.atoms)
            .unwrap_or(Ordering::Less)
    }
}

/// A search with a comparison that can fail. The probe keeps the first
/// error, and the search then gives the error.
struct Probe<F> {
    locate: F,
    error: RefCell<Option<BeamError>>,
}

impl<F: Fn(&Term) -> Result<Ordering, BeamError>> Probe<F> {
    fn new(locate: F) -> Self {
        Probe {
            locate,
            error: RefCell::new(None),
        }
    }

    /// Return `found`, or the error of a comparison of the search.
    fn result<T>(self, found: T) -> Result<T, BeamError> {
        match self.error.into_inner() {
            Some(failed) => Err(failed),
            None => Ok(found),
        }
    }
}

impl<F: Fn(&Term) -> Result<Ordering, BeamError>> Locate for Probe<F> {
    fn term(&self) -> &Term {
        unreachable!("a probe is never a key of the order")
    }

    fn locate(&self, key: &Term) -> Ordering {
        (self.locate)(key).unwrap_or_else(|failed| {
            self.error.borrow_mut().get_or_insert(failed);
            Ordering::Less
        })
    }
}

impl<'a> Borrow<dyn Locate + 'a> for OrderKey {
    fn borrow(&self) -> &(dyn Locate + 'a) {
        self
    }
}

/// The search compares the searched place (`self`) with a stored key
/// (`other`), as `BTreeSet` calls `query.cmp(stored)`.
impl Ord for dyn Locate + '_ {
    fn cmp(&self, other: &Self) -> Ordering {
        self.locate(other.term())
    }
}

impl PartialOrd for dyn Locate + '_ {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl PartialEq for dyn Locate + '_ {
    fn eq(&self, other: &Self) -> bool {
        self.cmp(other).is_eq()
    }
}

impl Eq for dyn Locate + '_ {}

impl Ord for OrderKey {
    fn cmp(&self, other: &Self) -> Ordering {
        self.locate(&other.term)
    }
}

impl PartialOrd for OrderKey {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl PartialEq for OrderKey {
    fn eq(&self, other: &Self) -> bool {
        self.cmp(other).is_eq()
    }
}

impl Eq for OrderKey {}

impl KeyOrder {
    /// Return an empty order that compares atoms with `atoms`.
    pub(super) fn new(atoms: Arc<AtomTable>) -> Self {
        KeyOrder {
            atoms,
            keys: BTreeSet::new(),
        }
    }

    /// Return the number of keys.
    pub(super) fn len(&self) -> usize {
        self.keys.len()
    }

    /// Return the keys in the term order.
    pub(super) fn iter(&self) -> impl DoubleEndedIterator<Item = &Term> {
        self.keys.iter().map(|key| &key.term)
    }

    /// Remove the greatest key, so the free of the keys of a table stays in
    /// the budget of each turn.
    pub(super) fn pop(&mut self) {
        self.keys.pop_last();
    }

    /// Remove each key.
    pub(super) fn clear(&mut self) {
        self.keys.clear();
    }

    /// Return the stored key equal to `key` in the arithmetic term order,
    /// so `1` finds `1.0` (`cmp_key_eq`, `erl_db_tree.c:758-761`).
    ///
    /// # Errors
    ///
    /// Return the error of a key comparison, which no current key has
    /// (`src/term/order.rs`).
    pub(super) fn find(&self, atoms: &AtomTable, key: &Term) -> Result<Option<&Term>, BeamError> {
        let probe = Probe::new(|stored: &Term| key.compare(stored, atoms));
        let found = self.keys.get(&probe as &dyn Locate).map(|key| &key.term);
        probe.result(found)
    }

    /// Return the keys above `key` up for [`Direction::Forward`], else the keys
    /// below `key` down, as `find_next` and `find_prev` step
    /// (`erl_db_tree.c:3659-3762`). The key `key` does not have to be in the
    /// order.
    ///
    /// # Errors
    ///
    /// Return the errors of [`KeyOrder::find`].
    pub(super) fn beyond<'a>(
        &'a self,
        atoms: &AtomTable,
        key: &Term,
        direction: Direction,
    ) -> Result<Box<dyn Iterator<Item = &'a Term> + 'a>, BeamError> {
        let probe = Probe::new(|stored: &Term| key.compare(stored, atoms));
        let place = &probe as &dyn Locate;
        let keys: Box<dyn Iterator<Item = &'a Term>> = if direction == Direction::Forward {
            let range = (Bound::Excluded(place), Bound::Unbounded);
            Box::new(self.keys.range::<dyn Locate, _>(range).map(|key| &key.term))
        } else {
            let range = (Bound::Unbounded, Bound::Excluded(place));
            Box::new(
                self.keys
                    .range::<dyn Locate, _>(range)
                    .rev()
                    .map(|key| &key.term),
            )
        };
        probe.result(keys)
    }

    /// Split the keys where `below` stops to give `true`, and return the
    /// last key before the split and the first key after it, as
    /// `find_prev_from_pb_key` and `find_next_from_pb_key` search the tree
    /// (`erl_db_tree.c:3294-3382`). `below` gives `true` for each key of a
    /// prefix of the key order.
    ///
    /// # Errors
    ///
    /// Return the error of `below`, or a runtime error when the order is not
    /// current.
    pub(super) fn split(
        &self,
        below: impl Fn(&Term) -> Result<bool, BeamError>,
    ) -> Result<(Option<&Term>, Option<&Term>), BeamError> {
        let probe = Probe::new(|stored: &Term| {
            Ok(if below(stored)? {
                Ordering::Greater
            } else {
                Ordering::Less
            })
        });
        let place = &probe as &dyn Locate;
        let before = self
            .keys
            .range::<dyn Locate, _>((Bound::Unbounded, Bound::Excluded(place)))
            .next_back();
        let after = self
            .keys
            .range::<dyn Locate, _>((Bound::Excluded(place), Bound::Unbounded))
            .next();
        probe.result((before.map(|key| &key.term), after.map(|key| &key.term)))
    }

    /// Put the key `key` in place of an equal key.
    pub(super) fn put(&mut self, key: Term) {
        let atoms = Arc::clone(&self.atoms);
        self.keys.replace(OrderKey { term: key, atoms });
    }

    /// Remove the key `key`.
    pub(super) fn remove(&mut self, atoms: &AtomTable, key: &Term) {
        let probe = Probe::new(|stored: &Term| key.compare(stored, atoms));
        self.keys.remove(&probe as &dyn Locate);
    }
}

#[cfg(test)]
#[path = "../../../tests/runtime/ets/order.rs"]
mod tests;

//! Persistent terms of the node, as the ERTS persistent term hash table
//! (`erl_bif_persistent.c`). All processes of a run share one store.

use crate::term::Term;

/// The persistent terms of the node, as key and value pairs. ERTS keeps one
/// hash table for the node and finds a key with the `EQ` exact equality
/// (`lookup` in `erl_bif_persistent.c`). `Term` has no hash, so this store
/// uses a linear search with the exact equality of `Term`. The ERTS hash
/// order is visible only through `persistent_term:get/0`, which returns the
/// `[{Key, Value}]` list in table order. PBeam does not implement it.
/// `persistent_term:info/0` returns only `#{count := Count, memory := Memory}`.
#[derive(Debug, Default)]
pub(super) struct PersistentTerms(Vec<(Term, Term)>);

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
        self.0
            .iter()
            .find(|(other, _)| other == key)
            .map(|(_, value)| value)
    }

    /// Store `value` for `key`. A key that is exactly equal to a stored key
    /// gets the new value; another key is added. This is `put_common` of
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
        match self.0.iter_mut().find(|(other, _)| other == key) {
            Some((_, old)) => *old = value.clone(),
            None => self.0.push((key.clone(), value.clone())),
        }
    }
}

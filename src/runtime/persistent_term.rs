//! Persistent terms of the node, as the ERTS persistent term hash table
//! (`erl_bif_persistent.c`). All processes of a run share one store.

use std::collections::HashMap;

use crate::term::Term;

/// The persistent terms of the node, as a hash table from key to value.
/// ERTS keeps one hash table for the node and finds a key with the `EQ`
/// exact equality (`lookup` in `erl_bif_persistent.c`). This store uses the
/// `Hash` and exact equality of `Term`. The ERTS table order is visible only
/// through `persistent_term:get/0`, which returns the `[{Key, Value}]` list
/// in table order. PBeam does not implement it.
/// `persistent_term:info/0` returns only `#{count := Count, memory := Memory}`.
#[derive(Debug, Default)]
pub(super) struct PersistentTerms(HashMap<Term, Term>);

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
        self.0.get(key)
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
        self.0.insert(key.clone(), value.clone());
    }
}

#[cfg(test)]
mod tests {
    use super::PersistentTerms;
    use crate::term::{Term, test_atom as atom};

    fn logger_config(name: &str) -> Term {
        Term::Tuple(vec![atom("logger_config"), atom(name)])
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
        assert_eq!(terms.0.len(), 3);
    }
}

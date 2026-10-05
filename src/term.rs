mod atom;
mod hash;
mod map;
mod order;

pub use atom::Atom;
pub(crate) use atom::AtomTable;
#[cfg(test)]
pub(crate) use atom::test_atom;
pub(crate) use hash::internal_hash;
pub use map::Map;

use std::fmt;

/// The smallest ERTS small integer on a 64-bit system: -(2^59).
const SMALL_INTEGER_MIN: i64 = -(1 << 59);
/// The largest ERTS small integer on a 64-bit system: 2^59 - 1.
const SMALL_INTEGER_MAX: i64 = (1 << 59) - 1;

/// A BEAM term. Lists, tuples, and maps can nest to any depth, as in ERTS,
/// so `Drop`, `Clone`, `PartialEq`, and `Debug` use explicit work
/// stacks and do not recurse on the Rust stack.
pub enum Term {
    /// An atom: its index in the atom table of the program, and its name.
    Atom(Atom),
    Binary(Vec<u8>),
    Integer(i64),
    /// A proper list of BEAM terms.
    List(Vec<Term>),
    /// A BEAM tuple.
    Tuple(Vec<Term>),
    /// A BEAM map: a flatmap up to 32 keys and a hashmap above 32 keys, as
    /// in ERTS. `Map` keeps each map in its canonical form, so two maps with
    /// the same pairs are equal. A key is an integer or an atom; a hashmap
    /// accepts atoms and small integers. Atom keys are ordered and hashed by
    /// their atom index, as in ERTS.
    Map(Map),
    /// A process identifier owned by this runtime.
    Pid(ProcessId),
    /// A reference owned by this runtime.
    Reference(Reference),
}

/// The kind of a term that contains other terms.
#[derive(Clone, Copy)]
enum Kind {
    List,
    Tuple,
    Map,
}

/// The nested terms of a list, tuple, or map, in order. A map gives each
/// key and then its value.
type Nested<'a> = Box<dyn Iterator<Item = &'a Term> + 'a>;

impl Kind {
    /// Build a term of this kind from its nested terms. For a map, the
    /// terms are keys and values in turn.
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// assert_eq!(Kind::Tuple.build(vec![]), Term::Tuple(vec![]));
    /// let map = Kind::Map.build(vec![Term::static_atom("ok"), Term::Integer(1)]);
    /// let pairs = vec![(Term::static_atom("ok"), Term::Integer(1))];
    /// assert_eq!(map, Term::Map(Map::from_pairs(pairs)?));
    /// ```
    fn build(self, terms: Vec<Term>) -> Term {
        match self {
            Kind::List => Term::List(terms),
            Kind::Tuple => Term::Tuple(terms),
            Kind::Map => {
                let mut pairs = Vec::with_capacity(terms.len() / 2);
                let mut terms = terms.into_iter();
                while let (Some(key), Some(value)) = (terms.next(), terms.next()) {
                    pairs.push((key, value));
                }
                // A hashmap copy hashes each key again; `Map` has no other way to build a map.
                Term::Map(Map::from_pairs(pairs).expect("a copied map holds only supported keys"))
            }
        }
    }
}

impl Term {
    /// Return the term of a static atom, such as `true` or `normal`. A
    /// static atom has the same index in every atom table.
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// assert_eq!(Term::static_atom("ok"), Term::Atom(Atom::from_static("ok").unwrap()));
    /// ```
    ///
    /// # Panics
    ///
    /// Panic when the name is not a static atom. Callers pass a fixed name.
    pub(crate) fn static_atom(name: &str) -> Term {
        Term::Atom(Atom::from_static(name).expect("the name is a static atom"))
    }

    /// Return the value of an integer in the ERTS small-integer range,
    /// -(2^59) to 2^59 - 1, as the ERTS checks `is_small` and `signed_val`
    /// do. Return `None` for any other term. ERTS stores a larger integer as
    /// a bignum, which is not a small integer.
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// assert_eq!(Term::Integer(-1).small_integer(), Some(-1));
    /// assert_eq!(Term::Integer(1 << 59).small_integer(), None);
    /// assert_eq!(Term::static_atom("ok").small_integer(), None);
    /// ```
    pub(crate) fn small_integer(&self) -> Option<i64> {
        match self {
            Term::Integer(value) if (SMALL_INTEGER_MIN..=SMALL_INTEGER_MAX).contains(value) => {
                Some(*value)
            }
            _ => None,
        }
    }

    /// Copy a term that has no nested terms. For a list, tuple, or map,
    /// return its nested terms, kind, and count instead, so the caller
    /// copies them.
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// assert_eq!(Term::Integer(1).shallow_clone().ok(), Some(Term::Integer(1)));
    /// assert!(Term::List(vec![]).shallow_clone().is_err());
    /// ```
    fn shallow_clone(&self) -> Result<Term, (Nested<'_>, Kind, usize)> {
        match self {
            Term::List(elements) => Err((Box::new(elements.iter()), Kind::List, elements.len())),
            Term::Tuple(elements) => Err((Box::new(elements.iter()), Kind::Tuple, elements.len())),
            Term::Map(map) => Err((
                Box::new(map.iter().flat_map(|(key, value)| [key, value])),
                Kind::Map,
                map.len() * 2,
            )),
            Term::Atom(atom) => Ok(Term::Atom(atom.clone())),
            Term::Binary(bytes) => Ok(Term::Binary(bytes.clone())),
            Term::Integer(value) => Ok(Term::Integer(*value)),
            Term::Pid(pid) => Ok(Term::Pid(*pid)),
            Term::Reference(reference) => Ok(Term::Reference(*reference)),
        }
    }

    /// Move the nested terms of a list, tuple, or map onto a work stack and
    /// leave the term empty.
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// let mut pending = Vec::new();
    /// Term::Tuple(vec![Term::Integer(1)]).take_nested(&mut pending);
    /// assert_eq!(pending, vec![Term::Integer(1)]);
    /// ```
    fn take_nested(&mut self, pending: &mut Vec<Term>) {
        match self {
            Term::List(elements) | Term::Tuple(elements) => pending.append(elements),
            Term::Map(map) => std::mem::replace(map, Map::new()).drain_into(pending),
            _ => {}
        }
    }
}

impl Drop for Term {
    /// Move nested terms onto a work stack, so dropping a deep term does not
    /// recurse.
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// let deep = (0..1_000_000).fold(Term::Tuple(vec![]), |inner, _| Term::Tuple(vec![inner]));
    /// drop(deep);
    /// ```
    fn drop(&mut self) {
        let mut pending = Vec::new();
        self.take_nested(&mut pending);
        while let Some(mut term) = pending.pop() {
            term.take_nested(&mut pending);
        }
    }
}

impl Clone for Term {
    /// Copy a term with a work stack of partly copied lists, tuples, and
    /// maps.
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// let term = Term::Tuple(vec![Term::List(vec![Term::Integer(1)])]);
    /// assert_eq!(term.clone(), term);
    /// ```
    fn clone(&self) -> Self {
        let (nested, kind, count) = match self.shallow_clone() {
            Ok(term) => return term,
            Err(container) => container,
        };
        let mut stack = vec![(nested, kind, Vec::with_capacity(count))];
        loop {
            let (source, _, copied) = stack.last_mut().expect("the clone stack is not empty");
            if let Some(term) = source.next() {
                match term.shallow_clone() {
                    Ok(term) => copied.push(term),
                    Err((nested, kind, count)) => {
                        stack.push((nested, kind, Vec::with_capacity(count)));
                    }
                }
                continue;
            }
            let (_, kind, copied) = stack.pop().expect("the clone stack is not empty");
            let term = kind.build(copied);
            match stack.last_mut() {
                Some((_, _, parent)) => parent.push(term),
                None => return term,
            }
        }
    }
}

impl PartialEq for Term {
    /// Compare two terms pair by pair with a work stack. Two maps are equal
    /// when their pairs in canonical order are equal, as the structural ERTS
    /// comparison of two flatmaps or two hashmaps is.
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// assert_eq!(Term::List(vec![Term::Integer(1)]), Term::List(vec![Term::Integer(1)]));
    /// assert_ne!(Term::List(vec![]), Term::Tuple(vec![]));
    /// ```
    fn eq(&self, other: &Self) -> bool {
        let mut pairs = vec![(self, other)];
        while let Some(pair) = pairs.pop() {
            let equal = match pair {
                (Term::List(left), Term::List(right)) | (Term::Tuple(left), Term::Tuple(right)) => {
                    pairs.extend(left.iter().zip(right));
                    left.len() == right.len()
                }
                (Term::Map(left), Term::Map(right)) => {
                    for ((left_key, left_value), (right_key, right_value)) in
                        left.iter().zip(right.iter())
                    {
                        pairs.push((left_key, right_key));
                        pairs.push((left_value, right_value));
                    }
                    left.len() == right.len()
                }
                (Term::Atom(left), Term::Atom(right)) => left == right,
                (Term::Binary(left), Term::Binary(right)) => left == right,
                (Term::Integer(left), Term::Integer(right)) => left == right,
                (Term::Pid(left), Term::Pid(right)) => left == right,
                (Term::Reference(left), Term::Reference(right)) => left == right,
                _ => false,
            };
            if !equal {
                return false;
            }
        }
        true
    }
}

impl Eq for Term {}

impl fmt::Debug for Term {
    /// Write the same text as a derived `Debug`, for example
    /// `Tuple([Atom("undef"), List([])])` or `Map([(Atom("a"), Integer(1))])`,
    /// with a work stack.
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// let term = Term::Tuple(vec![Term::static_atom("undef"), Term::List(vec![])]);
    /// assert_eq!(format!("{term:?}"), "Tuple([Atom(\"undef\"), List([])])");
    /// ```
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        enum Item<'a> {
            Term(&'a Term),
            Text(&'static str),
        }
        let mut items = vec![Item::Term(self)];
        while let Some(item) = items.pop() {
            let term = match item {
                Item::Text(text) => {
                    formatter.write_str(text)?;
                    continue;
                }
                Item::Term(term) => term,
            };
            match term {
                Term::Atom(atom) => write!(formatter, "Atom({:?})", atom.name())?,
                Term::Binary(bytes) => write!(formatter, "Binary({bytes:?})")?,
                Term::Integer(value) => write!(formatter, "Integer({value:?})")?,
                Term::Pid(pid) => write!(formatter, "Pid({pid:?})")?,
                Term::Reference(reference) => write!(formatter, "Reference({reference:?})")?,
                Term::List(elements) | Term::Tuple(elements) => {
                    let name = match term {
                        Term::List(_) => "List",
                        _ => "Tuple",
                    };
                    write!(formatter, "{name}([")?;
                    items.push(Item::Text("])"));
                    for (index, element) in elements.iter().enumerate().rev() {
                        items.push(Item::Term(element));
                        if index > 0 {
                            items.push(Item::Text(", "));
                        }
                    }
                }
                Term::Map(map) => {
                    formatter.write_str("Map([")?;
                    items.push(Item::Text("])"));
                    let pairs: Vec<_> = map.iter().collect();
                    for (index, (key, value)) in pairs.into_iter().enumerate().rev() {
                        items.push(Item::Text(")"));
                        items.push(Item::Term(value));
                        items.push(Item::Text(", "));
                        items.push(Item::Term(key));
                        items.push(Item::Text("("));
                        if index > 0 {
                            items.push(Item::Text(", "));
                        }
                    }
                }
            }
        }
        Ok(())
    }
}

/// An identifier for a process in this runtime.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct ProcessId(pub(crate) u64);

/// An identifier for a reference in this runtime.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Reference(pub(crate) u64);

#[cfg(test)]
mod tests {
    use super::{AtomTable, Map, ProcessId, Reference, Term, test_atom as atom};

    fn map(pairs: Vec<(Term, Term)>) -> Term {
        Term::Map(Map::from_pairs(pairs).unwrap())
    }

    fn deep(levels: usize) -> Term {
        (0..levels).fold(Term::List(vec![]), |inner, level| match level % 3 {
            0 => Term::Tuple(vec![inner]),
            1 => Term::List(vec![Term::Integer(1), inner]),
            _ => map(vec![(atom("k"), inner)]),
        })
    }

    #[test]
    fn clones_compares_formats_and_drops_deep_terms_without_recursion() {
        let term = deep(1_000_000);
        let copy = term.clone();
        assert_eq!(term, copy);
        assert_ne!(term, deep(999_999));
        assert!(format!("{copy:?}").starts_with("Tuple([Map([(Atom(\"k\"), List([Integer(1), "));
        drop(term);
        drop(copy);
    }

    #[test]
    fn matches_the_derived_debug_text_and_compares_every_variant() {
        let leaves = [
            atom("ok"),
            Term::Binary(vec![1, 2]),
            Term::Integer(-3),
            Term::Pid(ProcessId(4)),
            Term::Reference(Reference(5)),
        ];
        let term = Term::Tuple(vec![Term::List(leaves.to_vec()), Term::List(vec![])]);
        assert_eq!(
            format!("{term:?}"),
            "Tuple([List([Atom(\"ok\"), Binary([1, 2]), Integer(-3), Pid(ProcessId(4)), \
             Reference(Reference(5))]), List([])])"
        );
        for (index, leaf) in leaves.iter().enumerate() {
            assert_eq!(leaf, &leaf.clone());
            assert_ne!(leaf, &leaves[(index + 1) % leaves.len()]);
        }
        assert_ne!(Term::List(vec![]), Term::Tuple(vec![]));
        assert_ne!(Term::List(vec![]), Term::List(vec![Term::Integer(1)]));
        assert_ne!(Term::Tuple(vec![atom("a")]), Term::Tuple(vec![atom("b")]));
    }

    #[test]
    fn formats_and_compares_maps_by_their_pairs() {
        // A fresh table for this test makes a before b, so the Debug text
        // lists a first. A table shared with other tests could make b first.
        let mut table = AtomTable::new();
        let a = Term::Atom(table.put("a").unwrap());
        let b = Term::Atom(table.put("b").unwrap());
        let atom = |name: &str| match name {
            "a" => a.clone(),
            "b" => b.clone(),
            _ => crate::term::test_atom(name),
        };
        let pairs = || {
            vec![
                (atom("a"), Term::Integer(1)),
                (atom("b"), Term::List(vec![])),
            ]
        };
        let sorted = map(pairs());
        assert_eq!(
            format!("{sorted:?}"),
            "Map([(Atom(\"a\"), Integer(1)), (Atom(\"b\"), List([]))])"
        );
        assert_eq!(format!("{:?}", map(vec![])), "Map([])");
        assert_eq!(sorted.clone(), sorted);
        assert_ne!(sorted, map(vec![(atom("a"), Term::Integer(1))]));
        assert_ne!(
            map(vec![(atom("a"), Term::Integer(1))]),
            map(vec![(atom("a"), Term::Integer(2))])
        );
        assert_ne!(
            map(vec![(atom("a"), Term::Integer(1))]),
            map(vec![(atom("b"), Term::Integer(1))])
        );
        assert_ne!(map(vec![]), Term::List(vec![]));
        // Pairs in any order give the same map and the same text.
        let reversed = map(pairs().into_iter().rev().collect());
        assert_eq!(reversed, sorted);
        assert_eq!(format!("{reversed:?}"), format!("{sorted:?}"));
        // A hashmap copy is equal to its source and has the same text.
        let hash = |keys: &mut dyn Iterator<Item = i64>| {
            map(keys.map(|key| (Term::Integer(key), atom("v"))).collect())
        };
        let forty = hash(&mut (1..=40));
        assert_eq!(forty.clone(), hash(&mut (1..=40).rev()));
        assert_eq!(format!("{:?}", forty.clone()), format!("{forty:?}"));
        assert!(format!("{forty:?}").starts_with("Map([(Integer(18), Atom(\"v\")), (Integer(4), "));
        assert_ne!(forty, hash(&mut (2..=41)));
    }
}

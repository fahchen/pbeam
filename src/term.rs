use std::fmt;

/// A BEAM term. Lists and tuples can nest to any depth, as in ERTS, so
/// `Drop`, `Clone`, `PartialEq`, and `Debug` use explicit work stacks and do
/// not recurse on the Rust stack.
pub enum Term {
    Atom(String),
    Binary(Vec<u8>),
    Integer(i64),
    /// A proper list of BEAM terms.
    List(Vec<Term>),
    /// A BEAM tuple.
    Tuple(Vec<Term>),
    /// A process identifier owned by this runtime.
    Pid(ProcessId),
    /// A reference owned by this runtime.
    Reference(Reference),
}

/// The kind of a term that contains elements.
#[derive(Clone, Copy)]
enum Kind {
    List,
    Tuple,
}

impl Kind {
    /// Build a term of this kind from its elements.
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// assert_eq!(Kind::Tuple.build(vec![]), Term::Tuple(vec![]));
    /// ```
    fn build(self, elements: Vec<Term>) -> Term {
        match self {
            Kind::List => Term::List(elements),
            Kind::Tuple => Term::Tuple(elements),
        }
    }
}

impl Term {
    /// Copy a term that has no elements. For a list or tuple, return its
    /// elements and kind instead, so the caller copies them.
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// assert_eq!(Term::Integer(1).shallow_clone().ok(), Some(Term::Integer(1)));
    /// assert!(Term::List(vec![]).shallow_clone().is_err());
    /// ```
    fn shallow_clone(&self) -> Result<Term, (&[Term], Kind)> {
        match self {
            Term::List(elements) => Err((elements, Kind::List)),
            Term::Tuple(elements) => Err((elements, Kind::Tuple)),
            Term::Atom(name) => Ok(Term::Atom(name.clone())),
            Term::Binary(bytes) => Ok(Term::Binary(bytes.clone())),
            Term::Integer(value) => Ok(Term::Integer(*value)),
            Term::Pid(pid) => Ok(Term::Pid(*pid)),
            Term::Reference(reference) => Ok(Term::Reference(*reference)),
        }
    }
}

impl Drop for Term {
    /// Move nested elements onto a work stack, so dropping a deep term does
    /// not recurse.
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// let deep = (0..1_000_000).fold(Term::Tuple(vec![]), |inner, _| Term::Tuple(vec![inner]));
    /// drop(deep);
    /// ```
    fn drop(&mut self) {
        let (Term::List(elements) | Term::Tuple(elements)) = self else {
            return;
        };
        let mut pending = std::mem::take(elements);
        while let Some(mut term) = pending.pop() {
            if let Term::List(elements) | Term::Tuple(elements) = &mut term {
                pending.append(elements);
            }
        }
    }
}

impl Clone for Term {
    /// Copy a term with a work stack of partly copied lists and tuples.
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// let term = Term::Tuple(vec![Term::List(vec![Term::Integer(1)])]);
    /// assert_eq!(term.clone(), term);
    /// ```
    fn clone(&self) -> Self {
        let (elements, kind) = match self.shallow_clone() {
            Ok(term) => return term,
            Err(container) => container,
        };
        let mut stack = vec![(elements.iter(), kind, Vec::with_capacity(elements.len()))];
        loop {
            let (source, _, copied) = stack.last_mut().expect("the clone stack is not empty");
            if let Some(element) = source.next() {
                match element.shallow_clone() {
                    Ok(term) => copied.push(term),
                    Err((elements, kind)) => {
                        stack.push((elements.iter(), kind, Vec::with_capacity(elements.len())));
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
    /// Compare two terms pair by pair with a work stack.
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
    /// `Tuple([Atom("undef"), List([])])`, with a work stack.
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// let term = Term::Tuple(vec![Term::Atom("undef".into()), Term::List(vec![])]);
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
                Term::Atom(name) => write!(formatter, "Atom({name:?})")?,
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
    use super::{ProcessId, Reference, Term};

    fn deep(levels: usize) -> Term {
        (0..levels).fold(Term::List(vec![]), |inner, level| {
            if level % 2 == 0 {
                Term::Tuple(vec![inner])
            } else {
                Term::List(vec![Term::Integer(1), inner])
            }
        })
    }

    #[test]
    fn clones_compares_formats_and_drops_deep_terms_without_recursion() {
        let term = deep(1_000_000);
        let copy = term.clone();
        assert_eq!(term, copy);
        assert_ne!(term, deep(999_999));
        assert!(format!("{copy:?}").starts_with("List([Integer(1), Tuple([List([Integer(1), "));
        drop(term);
        drop(copy);
    }

    #[test]
    fn matches_the_derived_debug_text_and_compares_every_variant() {
        let leaves = [
            Term::Atom("ok".into()),
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
        assert_ne!(
            Term::Tuple(vec![Term::Atom("a".into())]),
            Term::Tuple(vec![Term::Atom("b".into())])
        );
    }
}

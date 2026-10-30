//! The list API: the one way that code outside the term module reads and
//! builds a BEAM list.
//!
//! A list is `[]` (`Term::Nil`) or a cons cell (`Term::Cons`) with a head
//! and a tail, as in ERTS (`erl_term.h`). A proper list ends in `[]`, and an
//! improper list ends in another term, for example `[1, 2 | x]`. A cons
//! cell is in an `Arc`, so a copy of a list shares its cells, and a new
//! cons cell shares the list of its tail. Thus `put_list`, `get_list`,
//! `get_hd`, and `get_tl` take O(1) time, as on ERTS. Only this module and
//! the other parts of the term module match `Term::Nil` and `Term::Cons`.

use std::{ops::Deref, sync::Arc};

use super::Term;

/// A cons cell: the head of a list and the tail after it. The tail of the
/// last cell of a proper list is `[]`. The term module drops a chain of
/// cells with a work stack (`Term::take_nested`), so the drop of a long
/// list does not recurse.
#[derive(Debug)]
pub struct Cons {
    /// The first element of the list.
    pub(super) head: Term,
    /// The rest of the list: `[]`, another cons cell, or the tail of an
    /// improper list.
    pub(super) tail: Term,
}

/// The shared reference of a `Term::Cons` to its cons cell. It is empty only
/// after the drop of its term takes the cell out (`Term::take_nested`):
/// `Term` has a `Drop`, so the drop cannot move the `Arc` out of the term,
/// and an empty reference needs no placeholder allocation. Each other
/// reader finds a cell.
#[derive(Clone, Debug)]
pub struct ConsRef(Option<Arc<Cons>>);

impl ConsRef {
    /// Make the reference of a new cons cell.
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// let cell = ConsRef::new(Term::Integer(1), Term::nil());
    /// assert_eq!(cell.head, Term::Integer(1));
    /// ```
    fn new(head: Term, tail: Term) -> Self {
        ConsRef(Some(Arc::new(Cons { head, tail })))
    }

    /// Return true when the two references share one cons cell.
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// let cell = ConsRef::new(Term::Integer(1), Term::nil());
    /// assert!(cell.same_cell(&cell.clone()));
    /// assert!(!cell.same_cell(&ConsRef::new(Term::Integer(1), Term::nil())));
    /// ```
    pub(super) fn same_cell(&self, other: &ConsRef) -> bool {
        Arc::ptr_eq(self.arc(), other.arc())
    }

    /// Take the cell out of the reference, and return its head and tail
    /// when this reference was the last holder of the cell. The function
    /// uses `Arc::into_inner`, which gives the cell to exactly one of the
    /// holders that drop at the same time.
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// let mut cell = ConsRef::new(Term::Integer(1), Term::nil());
    /// let mut copy = cell.clone();
    /// assert!(cell.take_last().is_none());
    /// assert!(matches!(copy.take_last(), Some(Cons { head: Term::Integer(1), .. })));
    /// ```
    pub(super) fn take_last(&mut self) -> Option<Cons> {
        self.0.take().and_then(Arc::into_inner)
    }

    /// Return the bytes of one cons cell: the size of the cell itself. The
    /// number excludes the header of the `Arc`, as the accounting of task
    /// [#559](https://github.com/fahchen/pbeam/issues/559) excludes it.
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// assert_eq!(ConsRef::allocated_bytes(), std::mem::size_of::<Cons>());
    /// ```
    pub(super) fn allocated_bytes() -> usize {
        std::mem::size_of::<Cons>()
    }

    /// Return the `Arc` of the cell.
    pub(super) fn arc(&self) -> &Arc<Cons> {
        self.0
            .as_ref()
            .expect("only the drop of a term takes its cons cell")
    }

    /// Return a value that identifies the shared cell. Two cons references
    /// have one value only when they share one cell, so the import of a
    /// host term reuses the imported term of a list that the source holds
    /// more than once. The value is the address of the cell, and no reader
    /// dereferences it.
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// let cell = ConsRef::new(Term::Integer(1), Term::nil());
    /// assert_eq!(cell.identity(), cell.clone().identity());
    /// assert_ne!(cell.identity(), ConsRef::new(Term::Integer(1), Term::nil()).identity());
    /// ```
    pub(super) fn identity(&self) -> usize {
        Arc::as_ptr(self.arc()).addr()
    }
}

impl Deref for ConsRef {
    type Target = Cons;

    fn deref(&self) -> &Cons {
        self.arc()
    }
}

/// An iterator over the elements of a list, by reference, in order. It
/// follows the tail of each cons cell. [`ListIter::rest`] gives the part
/// of the list that the iterator did not read: after the last element, it
/// is `[]` for a proper list and the tail of an improper list.
#[derive(Clone, Debug)]
pub struct ListIter<'a> {
    /// The cons cell of the next element, or the term after the last
    /// element.
    rest: &'a Term,
}

impl<'a> ListIter<'a> {
    /// Return the part of the list that the iterator did not read: the cons
    /// cell of the next element, `[]`, or the tail of an improper list. A
    /// trap of a list BIF keeps it as the rest of its list in O(1) time.
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// let list = Term::list_with_tail(vec![Term::Integer(1), Term::Integer(2)], Term::static_atom("x"));
    /// let mut elements = list.list_iter().unwrap();
    /// assert_eq!(elements.next(), Some(&Term::Integer(1)));
    /// assert_eq!(elements.rest(), &Term::list_with_tail(vec![Term::Integer(2)], Term::static_atom("x")));
    /// assert_eq!(elements.next(), Some(&Term::Integer(2)));
    /// assert_eq!(elements.rest(), &Term::static_atom("x"));
    /// ```
    pub fn rest(&self) -> &'a Term {
        self.rest
    }
}

impl<'a> Iterator for ListIter<'a> {
    type Item = &'a Term;

    /// Return the head of the next cons cell, and move to its tail. Return
    /// `None` at `[]` or at the tail of an improper list.
    fn next(&mut self) -> Option<&'a Term> {
        let (head, tail) = self.rest.cons_cell()?;
        self.rest = tail;
        Some(head)
    }
}

impl Term {
    /// Return the empty list `[]`.
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// assert!(Term::nil().is_nil());
    /// assert_eq!(Term::nil(), Term::list_from([]));
    /// ```
    pub fn nil() -> Term {
        Term::Nil
    }

    /// Build the cons cell `[head | tail]` in O(1) time, as the `put_list`
    /// instruction does. The new cell shares `tail`, which can be any term.
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// let list = Term::cons(Term::Integer(1), Term::list_from([Term::Integer(2)]));
    /// assert_eq!(list, Term::list_from([Term::Integer(1), Term::Integer(2)]));
    /// assert_eq!(Term::cons(Term::Integer(1), Term::static_atom("x")), Term::list_with_tail(vec![Term::Integer(1)], Term::static_atom("x")));
    /// ```
    pub fn cons(head: Term, tail: Term) -> Term {
        Term::Cons(ConsRef::new(head, tail))
    }

    /// Build a proper list of `elements`, in order. No elements give `[]`.
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// let list = Term::list_from([Term::Integer(1), Term::Integer(2)]);
    /// assert!(list.proper_list().unwrap().eq([&Term::Integer(1), &Term::Integer(2)]));
    /// assert!(Term::list_from([]).is_nil());
    /// ```
    pub fn list_from(elements: impl IntoIterator<Item = Term>) -> Term {
        Term::list_with_tail(elements.into_iter().collect(), Term::Nil)
    }

    /// Build a list of `elements` followed by `tail`: one cons cell for each
    /// element, and the last cell has the tail `tail`. A proper list tail
    /// gives a proper list, and another tail gives an improper list. No
    /// elements give the tail itself.
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// let one = || vec![Term::Integer(1)];
    /// let x = Term::static_atom("x");
    /// let two = Term::list_from([Term::Integer(2)]);
    /// assert_eq!(Term::list_with_tail(one(), two), Term::list_from([Term::Integer(1), Term::Integer(2)]));
    /// assert_eq!(Term::list_with_tail(one(), x.clone()), Term::cons(Term::Integer(1), x.clone()));
    /// assert_eq!(Term::list_with_tail(vec![], x.clone()), x);
    /// ```
    pub fn list_with_tail(elements: Vec<Term>, tail: Term) -> Term {
        elements
            .into_iter()
            .rev()
            .fold(tail, |tail, head| Term::cons(head, tail))
    }

    /// Return true for `[]`, as the ERTS check `is_nil` does.
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// assert!(Term::nil().is_nil());
    /// assert!(!Term::list_from([Term::Integer(1)]).is_nil());
    /// assert!(!Term::tuple(vec![]).is_nil());
    /// ```
    pub fn is_nil(&self) -> bool {
        matches!(self, Term::Nil)
    }

    /// Return true for `[]` and for a cons cell, proper or improper, as the
    /// guard BIF `is_list/1` does.
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// assert!(Term::nil().is_list());
    /// assert!(Term::list_with_tail(vec![Term::Integer(1)], Term::static_atom("x")).is_list());
    /// assert!(!Term::tuple(vec![]).is_list());
    /// ```
    pub fn is_list(&self) -> bool {
        matches!(self, Term::Nil | Term::Cons(_))
    }

    /// Return true for a cons cell, proper or improper, as the ERTS check
    /// `is_list` (a list that is not `[]`) does.
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// assert!(Term::list_from([Term::Integer(1)]).is_nonempty_list());
    /// assert!(Term::list_with_tail(vec![Term::Integer(1)], Term::static_atom("x")).is_nonempty_list());
    /// assert!(!Term::nil().is_nonempty_list());
    /// ```
    pub fn is_nonempty_list(&self) -> bool {
        matches!(self, Term::Cons(_))
    }

    /// Return the head and the tail of a cons cell, proper or improper, in
    /// O(1) time, as `CAR` and `CDR` read them. `[]` and a term that is not
    /// a list give `None`.
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// let x = Term::static_atom("x");
    /// let list = Term::list_with_tail(vec![Term::Integer(1)], x.clone());
    /// assert_eq!(list.cons_cell(), Some((&Term::Integer(1), &x)));
    /// assert_eq!(Term::nil().cons_cell(), None);
    /// ```
    pub fn cons_cell(&self) -> Option<(&Term, &Term)> {
        match self {
            Term::Cons(cell) => Some((&cell.head, &cell.tail)),
            _ => None,
        }
    }

    /// Return an iterator over the elements of a list, `[]` or a cons cell,
    /// proper or improper. A term that is not a list gives `None`. After
    /// the last element, [`ListIter::rest`] gives `[]` for a proper list and
    /// the tail of an improper list.
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// let x = Term::static_atom("x");
    /// let mut elements = Term::list_with_tail(vec![Term::Integer(1)], x.clone()).list_iter().unwrap();
    /// assert_eq!(elements.next(), Some(&Term::Integer(1)));
    /// assert_eq!(elements.next(), None);
    /// assert_eq!(elements.rest(), &x);
    /// assert_eq!(Term::nil().list_iter().unwrap().count(), 0);
    /// assert!(Term::Integer(1).list_iter().is_none());
    /// ```
    pub fn list_iter(&self) -> Option<ListIter<'_>> {
        self.is_list().then_some(ListIter { rest: self })
    }

    /// Return the elements and the tail of a list: the tail is `None` for a
    /// proper list and the tail term of an improper list. A term that is
    /// not a list gives `None`. The function walks the list once to find
    /// the tail.
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// let x = Term::static_atom("x");
    /// let (elements, tail) = Term::list_with_tail(vec![Term::Integer(1)], x.clone()).list_cells().unwrap();
    /// assert!(elements.eq([&Term::Integer(1)]));
    /// assert_eq!(tail, Some(&x));
    /// assert!(matches!(Term::nil().list_cells(), Some((_, None))));
    /// assert!(Term::Integer(1).list_cells().is_none());
    /// ```
    pub fn list_cells(&self) -> Option<(ListIter<'_>, Option<&Term>)> {
        let elements = self.list_iter()?;
        let mut walk = elements.clone();
        walk.by_ref().for_each(drop);
        let tail = Some(walk.rest()).filter(|tail| !tail.is_nil());
        Some((elements, tail))
    }

    /// Return the head of a cons cell, proper or improper. `[]` and a term
    /// that is not a list give `None`.
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// assert_eq!(Term::list_from([Term::Integer(1), Term::Integer(2)]).list_head(), Some(&Term::Integer(1)));
    /// assert_eq!(Term::list_with_tail(vec![Term::Integer(1)], Term::static_atom("x")).list_head(), Some(&Term::Integer(1)));
    /// assert_eq!(Term::nil().list_head(), None);
    /// ```
    pub fn list_head(&self) -> Option<&Term> {
        self.cons_cell().map(|(head, _)| head)
    }

    /// Return an iterator over the elements of a proper list. `[]` gives no
    /// elements. An improper list or a term that is not a list gives
    /// `None`. The function walks the list once to check its tail.
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// assert!(Term::list_from([Term::Integer(1)]).proper_list().unwrap().eq([&Term::Integer(1)]));
    /// assert!(Term::list_with_tail(vec![Term::Integer(1)], Term::static_atom("x")).proper_list().is_none());
    /// assert!(Term::Integer(1).proper_list().is_none());
    /// ```
    pub fn proper_list(&self) -> Option<ListIter<'_>> {
        match self.list_cells()? {
            (elements, None) => Some(elements),
            (_, Some(_)) => None,
        }
    }

    /// Copy the elements of a list into a vector, and tell whether the list
    /// is proper. A term that is not a list gives `None`.
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// let list = Term::list_with_tail(vec![Term::Integer(1)], Term::static_atom("x"));
    /// assert_eq!(list.list_elements(), Some((vec![Term::Integer(1)], false)));
    /// assert_eq!(Term::nil().list_elements(), Some((vec![], true)));
    /// assert_eq!(Term::static_atom("x").list_elements(), None);
    /// ```
    pub fn list_elements(&self) -> Option<(Vec<Term>, bool)> {
        let mut elements = self.list_iter()?;
        let copied = elements.by_ref().cloned().collect();
        Some((copied, elements.rest().is_nil()))
    }
}

#[cfg(test)]
#[path = "../../tests/term/list.rs"]
mod tests;

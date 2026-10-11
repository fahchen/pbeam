//! The list API: the one way that code outside the term module reads and
//! builds a BEAM list.
//!
//! A list is `[]` or a cons cell with a head and a tail, as in ERTS
//! (`erl_term.h`). A proper list ends in `[]`, and an improper list ends in
//! another term, for example `[1, 2 | x]`. PBeam stores a proper list as
//! `Term::List` and an improper list as `Term::ImproperList`. Only this
//! module and the other parts of the term module match these variants, so
//! a change of the list representation stays in the term module.

use super::Term;

impl Term {
    /// Return the empty list `[]`.
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// assert!(Term::nil().is_nil());
    /// assert_eq!(Term::nil(), Term::list_from([]));
    /// ```
    pub(crate) fn nil() -> Term {
        Term::List(Vec::new())
    }

    /// Build a proper list of `elements`, in order. No elements give `[]`.
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// let list = Term::list_from([Term::Integer(1), Term::Integer(2)]);
    /// assert_eq!(list.proper_list(), Some(&[Term::Integer(1), Term::Integer(2)][..]));
    /// assert!(Term::list_from([]).is_nil());
    /// ```
    pub(crate) fn list_from(elements: impl IntoIterator<Item = Term>) -> Term {
        Term::List(elements.into_iter().collect())
    }

    /// Build a list of `elements` followed by `tail`, in the one form of
    /// each list: a proper list tail gives a proper list, an improper list
    /// tail gives an improper list with its tail, and no elements give the
    /// tail itself. Each other tail gives an improper list.
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// let one = || vec![Term::Integer(1)];
    /// let x = Term::static_atom("x");
    /// let two = Term::list_from([Term::Integer(2)]);
    /// assert_eq!(Term::list_with_tail(one(), two), Term::list_from([Term::Integer(1), Term::Integer(2)]));
    /// assert_eq!(Term::list_with_tail(one(), x.clone()).list_cells(), Some((&one()[..], Some(&x))));
    /// assert_eq!(Term::list_with_tail(vec![], x.clone()), x);
    /// ```
    pub(crate) fn list_with_tail(mut elements: Vec<Term>, mut tail: Term) -> Term {
        if elements.is_empty() {
            return tail;
        }
        match &mut tail {
            Term::List(rest) => {
                elements.append(rest);
                Term::List(elements)
            }
            Term::ImproperList(rest, end) => {
                elements.append(rest);
                Term::ImproperList(elements, std::mem::replace(end, Box::new(Term::nil())))
            }
            _ => Term::ImproperList(elements, Box::new(tail)),
        }
    }

    /// Return true for `[]`, as the ERTS check `is_nil` does.
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// assert!(Term::nil().is_nil());
    /// assert!(!Term::list_from([Term::Integer(1)]).is_nil());
    /// assert!(!Term::Tuple(vec![]).is_nil());
    /// ```
    pub(crate) fn is_nil(&self) -> bool {
        matches!(self, Term::List(elements) if elements.is_empty())
    }

    /// Return true for `[]` and for a cons cell, proper or improper, as the
    /// guard BIF `is_list/1` does.
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// assert!(Term::nil().is_list());
    /// assert!(Term::list_with_tail(vec![Term::Integer(1)], Term::static_atom("x")).is_list());
    /// assert!(!Term::Tuple(vec![]).is_list());
    /// ```
    pub(crate) fn is_list(&self) -> bool {
        matches!(self, Term::List(_) | Term::ImproperList(..))
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
    pub(crate) fn is_nonempty_list(&self) -> bool {
        self.is_list() && !self.is_nil()
    }

    /// Return the elements of a list and the tail of an improper list. `[]`
    /// gives no elements and no tail. A term that is not a list gives
    /// `None`.
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// let x = Term::static_atom("x");
    /// let improper = Term::list_with_tail(vec![Term::Integer(1)], x.clone());
    /// assert_eq!(improper.list_cells(), Some((&[Term::Integer(1)][..], Some(&x))));
    /// assert_eq!(Term::nil().list_cells(), Some((&[][..], None)));
    /// assert_eq!(Term::Integer(1).list_cells(), None);
    /// ```
    pub(crate) fn list_cells(&self) -> Option<(&[Term], Option<&Term>)> {
        match self {
            Term::List(elements) => Some((elements, None)),
            Term::ImproperList(elements, tail) => Some((elements, Some(tail))),
            _ => None,
        }
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
    pub(crate) fn list_head(&self) -> Option<&Term> {
        self.list_cells()?.0.first()
    }

    /// Return the elements of a proper list. `[]` gives no elements. An
    /// improper list or a term that is not a list gives `None`.
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// assert_eq!(Term::list_from([Term::Integer(1)]).proper_list(), Some(&[Term::Integer(1)][..]));
    /// assert_eq!(Term::list_with_tail(vec![Term::Integer(1)], Term::static_atom("x")).proper_list(), None);
    /// assert_eq!(Term::Integer(1).proper_list(), None);
    /// ```
    pub(crate) fn proper_list(&self) -> Option<&[Term]> {
        match self.list_cells() {
            Some((elements, None)) => Some(elements),
            _ => None,
        }
    }

    /// Move the elements of a list out of `self`, and tell whether the list
    /// is proper. The list keeps its tail. A term that is not a list gives
    /// `None` and does not change.
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// let mut list = Term::list_with_tail(vec![Term::Integer(1)], Term::static_atom("x"));
    /// assert_eq!(list.take_list_elements(), Some((vec![Term::Integer(1)], false)));
    /// assert_eq!(Term::nil().take_list_elements(), Some((vec![], true)));
    /// assert_eq!(Term::static_atom("x").take_list_elements(), None);
    /// ```
    pub(crate) fn take_list_elements(&mut self) -> Option<(Vec<Term>, bool)> {
        match self {
            Term::List(elements) => Some((std::mem::take(elements), true)),
            Term::ImproperList(elements, _) => Some((std::mem::take(elements), false)),
            _ => None,
        }
    }

    /// Split a cons cell into its head and its tail, as the `get_list`
    /// instruction reads them. `[]` and a term that is not a list give
    /// `Err` with the term.
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// let list = Term::list_from([Term::Integer(1), Term::Integer(2)]);
    /// assert_eq!(list.split_cons(), Ok((Term::Integer(1), Term::list_from([Term::Integer(2)]))));
    /// let x = Term::static_atom("x");
    /// assert_eq!(Term::list_with_tail(vec![Term::Integer(1)], x.clone()).split_cons(), Ok((Term::Integer(1), x)));
    /// assert_eq!(Term::nil().split_cons(), Err(Term::nil()));
    /// ```
    pub(crate) fn split_cons(mut self) -> Result<(Term, Term), Term> {
        let (mut values, tail) = match &mut self {
            Term::List(values) if !values.is_empty() => (std::mem::take(values), Term::nil()),
            Term::ImproperList(values, tail) => (
                std::mem::take(values),
                std::mem::replace(&mut **tail, Term::nil()),
            ),
            _ => return Err(self),
        };
        let head = values.remove(0);
        Ok((head, Term::list_with_tail(values, tail)))
    }
}

#[cfg(test)]
#[path = "../../tests/term/list.rs"]
mod tests;

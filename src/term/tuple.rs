//! The layout of a BEAM tuple.
//!
//! A tuple holds its elements in one array. Copies of a tuple share the
//! array, as an ERTS copy of a tuple term in a register or a stack slot is
//! the same boxed pointer to the `Arityval` on the process heap. A change
//! through one copy first copies a shared array, so the other copies do not
//! change. `setelement/3` copies the tuple (`bif.c:2976-3001`), and the
//! `set_tuple_element` instruction changes a new tuple in place.
//!
//! `Tuple` owns that array. It is in an `Option`, because `Term` has a
//! `Drop`: the drop cannot move the `Arc` out of the term, so it takes the
//! `Arc` out of the `Option` and calls `Arc::into_inner` (see
//! [`Tuple::take_last`]). Each other reader finds the array.

use std::{ops::Deref, sync::Arc};

use super::Term;

/// The elements of a BEAM tuple. All copies of the tuple own one reference
/// to the array. `Arc::make_mut` copies it first when another copy also
/// holds it. The last copy that drops releases it.
#[derive(Clone, Debug)]
#[repr(transparent)]
#[expect(
    clippy::rc_buffer,
    reason = "`Tuple` shares its elements as `Arc<Vec<Term>>` because `Arc::make_mut` copies and writes them (`set_tuple_element`); an `Arc<[Term]>` cannot do that"
)]
pub struct Tuple(Option<Arc<Vec<Term>>>);

impl Tuple {
    /// Return the number of elements.
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// assert_eq!(Tuple::from(vec![Term::Integer(1)]).len(), 1);
    /// assert_eq!(Tuple::from(vec![]).len(), 0);
    /// ```
    pub(crate) fn len(&self) -> usize {
        self.arc().len()
    }

    /// Return the bytes of the element array of the tuple: the capacity of
    /// the array times the size of one element. The number excludes the
    /// header of the `Arc` and the header of the `Vec`, as the accounting of
    /// task [#559](https://github.com/fahchen/pbeam/issues/559) excludes
    /// them.
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// let tuple = Tuple::from(vec![Term::Integer(1), Term::Integer(2)]);
    /// assert!(tuple.allocated_bytes() >= 2 * std::mem::size_of::<Term>());
    /// ```
    pub(crate) fn allocated_bytes(&self) -> usize {
        self.arc().capacity() * std::mem::size_of::<Term>()
    }

    /// Return the array of the tuple.
    #[expect(
        clippy::rc_buffer,
        reason = "`Tuple` shares its elements as `Arc<Vec<Term>>` because `Arc::make_mut` copies and writes them (`set_tuple_element`); an `Arc<[Term]>` cannot do that"
    )]
    pub(super) fn arc(&self) -> &Arc<Vec<Term>> {
        self.0
            .as_ref()
            .expect("only the drop of a term takes its tuple array")
    }

    /// Return a value that identifies the shared array. Two tuples have one
    /// value only when they share one array, so the import of a host term
    /// reuses the imported term of a tuple that the source holds more than
    /// once. The value is the address of the array, and no reader
    /// dereferences it.
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// let tuple = Tuple::from(vec![Term::Integer(1)]);
    /// assert_eq!(tuple.identity(), tuple.clone().identity());
    /// assert_ne!(tuple.identity(), Tuple::from(vec![Term::Integer(1)]).identity());
    /// ```
    pub(super) fn identity(&self) -> usize {
        Arc::as_ptr(self.arc()) as usize
    }

    /// Return the array of the tuple for a change. This is the receiver of
    /// `Arc::make_mut`: the caller copies the array first when another copy
    /// also holds it.
    #[expect(
        clippy::rc_buffer,
        reason = "`Tuple` shares its elements as `Arc<Vec<Term>>` because `Arc::make_mut` copies and writes them (`set_tuple_element`); an `Arc<[Term]>` cannot do that"
    )]
    pub(crate) fn arc_mut(&mut self) -> &mut Arc<Vec<Term>> {
        self.0
            .as_mut()
            .expect("only the drop of a term takes its tuple array")
    }

    /// Return true when the two tuples share one array, as two ERTS tuple
    /// terms with the same boxed pointer are the same tuple.
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// let tuple = Tuple::from(vec![Term::Integer(1)]);
    /// assert!(tuple.same_array(&tuple.clone()));
    /// assert!(!tuple.same_array(&Tuple::from(vec![Term::Integer(1)])));
    /// ```
    pub(crate) fn same_array(&self, other: &Tuple) -> bool {
        Arc::ptr_eq(self.arc(), other.arc())
    }

    /// Take the array out of the tuple, and return its elements when this
    /// tuple was the last holder of the array. The function uses
    /// `Arc::into_inner`, which gives the array to exactly one of the
    /// holders that drop at the same time.
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// let mut tuple = Tuple::from(vec![Term::Integer(1)]);
    /// let mut copy = tuple.clone();
    /// assert!(tuple.take_last().is_none());
    /// assert_eq!(copy.take_last(), Some(vec![Term::Integer(1)]));
    /// ```
    pub(crate) fn take_last(&mut self) -> Option<Vec<Term>> {
        self.0.take().and_then(Arc::into_inner)
    }
}

impl Deref for Tuple {
    type Target = [Term];

    fn deref(&self) -> &[Term] {
        self.arc()
    }
}

impl From<Vec<Term>> for Tuple {
    /// Make a tuple of `elements`. The array of the tuple holds the
    /// elements, and copies of the tuple share it.
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// let tuple = Tuple::from(vec![Term::Integer(1)]);
    /// assert_eq!(tuple.len(), 1);
    /// ```
    fn from(elements: Vec<Term>) -> Self {
        Tuple(Some(Arc::new(elements)))
    }
}

#[cfg(test)]
#[path = "../../tests/term/tuple.rs"]
mod tests;

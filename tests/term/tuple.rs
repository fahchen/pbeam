//! Unit tests of the tuple newtype of `src/term/tuple.rs`.

use super::*;

/// Return a tuple of the integers `values`.
fn tuple(values: &[i64]) -> Term {
    Term::tuple(values.iter().copied().map(Term::Integer).collect())
}

/// Return the elements of a tuple term.
fn elements(term: &Term) -> &[Term] {
    let Term::Tuple(elements) = term else {
        panic!("not a tuple: {term:?}");
    };
    elements
}

/// Return the array of a tuple term.
fn array(term: &Term) -> &Arc<Vec<Term>> {
    let Term::Tuple(tuple) = term else {
        panic!("not a tuple: {term:?}");
    };
    tuple.arc()
}

#[test]
fn shares_the_array_between_the_copies_of_a_tuple() {
    // A copy of a tuple shares the array of the source, as an ERTS copy of a
    // tuple term is the same boxed pointer (`copy_struct`, copy.c:812-842).
    let source = tuple(&[1, 2, 3]);
    let copy = source.clone();
    assert!(Arc::ptr_eq(array(&source), array(&copy)));
    // A new tuple of equal elements is a different array.
    assert!(!Arc::ptr_eq(array(&source), array(&tuple(&[1, 2, 3]))));
    assert_eq!(source, copy);
}

#[test]
fn copies_the_array_of_a_shared_tuple_before_a_change() {
    // `Arc::make_mut` copies the array of a tuple that another copy also
    // holds, so the other copy does not change.
    let source = tuple(&[1, 2, 3]);
    let mut copy = source.clone();
    let Term::Tuple(fields) = &mut copy else {
        unreachable!()
    };
    Arc::make_mut(fields.arc_mut())[1] = Term::Integer(9);
    assert_eq!(
        elements(&copy),
        &[Term::Integer(1), Term::Integer(9), Term::Integer(3)]
    );
    assert_eq!(
        elements(&source),
        &[Term::Integer(1), Term::Integer(2), Term::Integer(3)]
    );
    assert!(!Arc::ptr_eq(array(&source), array(&copy)));
    // A tuple that no other copy holds changes in place.
    let in_place = Arc::as_ptr(array(&copy)) as usize;
    let Term::Tuple(fields) = &mut copy else {
        unreachable!()
    };
    Arc::make_mut(fields.arc_mut())[0] = Term::Integer(7);
    assert_eq!(Arc::as_ptr(array(&copy)) as usize, in_place);
    assert_eq!(elements(&copy)[0], Term::Integer(7));
}

#[test]
fn compares_tuples_by_their_lengths_and_elements() {
    assert_eq!(tuple(&[]), tuple(&[]));
    assert_eq!(tuple(&[1]), tuple(&[1]));
    assert_ne!(tuple(&[]), tuple(&[1]));
    assert_ne!(tuple(&[1]), tuple(&[1, 2]));
    assert_ne!(tuple(&[1, 2]), tuple(&[1, 3]));
    assert_ne!(tuple(&[]), Term::nil());
    // A copy of a deep tuple is equal to its source with no walk of the
    // copied array.
    let deep = (0..100).fold(tuple(&[1]), |inner, _| Term::tuple(vec![inner]));
    assert_eq!(deep, deep.clone());
}

#[test]
fn takes_the_array_of_a_tuple_only_from_the_last_copy() {
    // A tuple that another copy holds gives no elements, and the other copy
    // keeps them.
    let mut first = tuple(&[1, 2]);
    let mut second = first.clone();
    let Term::Tuple(first_tuple) = &mut first else {
        unreachable!()
    };
    assert_eq!(first_tuple.take_last(), None);
    assert_eq!(elements(&second), &[Term::Integer(1), Term::Integer(2)]);
    // The last holder takes the elements.
    let Term::Tuple(second_tuple) = &mut second else {
        unreachable!()
    };
    drop(first);
    assert_eq!(
        second_tuple.take_last(),
        Some(vec![Term::Integer(1), Term::Integer(2)])
    );
    // The array of the last holder is empty after the take.
    assert_eq!(second_tuple.take_last(), None);
}

#[test]
fn drops_a_deep_tuple_without_recursion() {
    // `Drop` moves the elements of a tuple onto its work stack, so a tuple
    // that nests 1,000,000 deep does not overflow the Rust stack.
    let mut deep = tuple(&[1]);
    for _ in 0..1_000_000 {
        deep = Term::tuple(vec![deep]);
    }
    // The shared array of a copy gives no elements to the drop of the copy.
    let copy = deep.clone();
    drop(deep);
    assert_eq!(elements(&copy).len(), 1);
    drop(copy);
}

#[test]
fn drains_the_elements_of_a_tuple_with_the_work_stack() {
    // `Term::take_nested` moves the elements of a tuple onto the work stack
    // and leaves the array empty, so the drop of a deep tuple walks a stack
    // and does not recurse.
    let mut pending = Vec::new();
    let mut term = tuple(&[1, 2, 3]);
    Term::take_nested(&mut term, &mut pending);
    assert_eq!(
        pending,
        vec![Term::Integer(1), Term::Integer(2), Term::Integer(3)]
    );
    // The array of the tuple is gone after the move: `take_nested` took it.
    let Term::Tuple(tuple) = &mut term else {
        unreachable!()
    };
    assert_eq!(tuple.take_last(), None);
}

#[test]
fn reports_the_bytes_of_the_element_array() {
    let term = tuple(&[1, 2, 3]);
    let Term::Tuple(tuple) = &term else {
        unreachable!()
    };
    assert_eq!(
        tuple.allocated_bytes(),
        tuple.arc().capacity() * std::mem::size_of::<Term>()
    );
    assert_eq!(tuple.allocated_bytes(), 3 * std::mem::size_of::<Term>());
}

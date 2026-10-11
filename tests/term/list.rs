use super::super::{Term, test_atom as atom};

fn integers(values: &[i64]) -> Vec<Term> {
    values.iter().copied().map(Term::Integer).collect()
}

/// `[1, 2 | x]`.
fn improper() -> Term {
    Term::list_with_tail(integers(&[1, 2]), atom("x"))
}

#[test]
fn builds_proper_lists_and_nil() {
    assert_eq!(Term::nil(), Term::List(vec![]));
    assert_eq!(Term::list_from([]), Term::List(vec![]));
    assert_eq!(
        Term::list_from((1..=3).map(Term::Integer)),
        Term::List(integers(&[1, 2, 3]))
    );
}

#[test]
fn tests_the_list_kind_of_each_term() {
    let one = Term::list_from(integers(&[1]));
    // (term, is_nil, is_list, is_nonempty_list), as the guard BIFs
    // `is_list/1` and the `is_nil` and `is_nonempty_list` tests give.
    let cases = [
        (Term::nil(), true, true, false),
        (one, false, true, true),
        (improper(), false, true, true),
        (Term::Tuple(vec![]), false, false, false),
        (atom("x"), false, false, false),
    ];
    for (term, nil, list, nonempty) in cases {
        assert_eq!(term.is_nil(), nil, "is_nil {term:?}");
        assert_eq!(term.is_list(), list, "is_list {term:?}");
        assert_eq!(
            term.is_nonempty_list(),
            nonempty,
            "is_nonempty_list {term:?}"
        );
    }
}

#[test]
fn reads_the_elements_and_the_tail_of_a_list() {
    let proper = Term::list_from(integers(&[1, 2]));
    let x = atom("x");
    assert_eq!(proper.list_cells(), Some((&integers(&[1, 2])[..], None)));
    assert_eq!(
        improper().list_cells(),
        Some((&integers(&[1, 2])[..], Some(&x)))
    );
    assert_eq!(Term::nil().list_cells(), Some((&[][..], None)));
    assert_eq!(x.list_cells(), None);

    assert_eq!(proper.proper_list(), Some(&integers(&[1, 2])[..]));
    assert_eq!(Term::nil().proper_list(), Some(&[][..]));
    assert_eq!(improper().proper_list(), None);
    assert_eq!(x.proper_list(), None);

    assert_eq!(proper.list_head(), Some(&Term::Integer(1)));
    assert_eq!(improper().list_head(), Some(&Term::Integer(1)));
    assert_eq!(Term::nil().list_head(), None);
    assert_eq!(x.list_head(), None);
}

#[test]
fn takes_the_elements_of_a_list() {
    let mut proper = Term::list_from(integers(&[1, 2]));
    assert_eq!(proper.take_list_elements(), Some((integers(&[1, 2]), true)));
    let mut list = improper();
    assert_eq!(list.take_list_elements(), Some((integers(&[1, 2]), false)));
    assert_eq!(Term::nil().take_list_elements(), Some((vec![], true)));
    let mut x = atom("x");
    assert_eq!(x.take_list_elements(), None);
    assert_eq!(x, atom("x"));
}

#[test]
fn splits_a_cons_cell_into_its_head_and_tail() {
    let proper = Term::list_from(integers(&[1, 2, 3]));
    assert_eq!(
        proper.split_cons(),
        Ok((Term::Integer(1), Term::list_from(integers(&[2, 3]))))
    );
    assert_eq!(
        improper().split_cons(),
        Ok((
            Term::Integer(1),
            Term::list_with_tail(integers(&[2]), atom("x"))
        ))
    );
    // The tail of the last cons cell of an improper list is its tail term.
    assert_eq!(
        Term::list_with_tail(integers(&[1]), atom("x")).split_cons(),
        Ok((Term::Integer(1), atom("x")))
    );
    assert_eq!(
        Term::list_from(integers(&[1])).split_cons(),
        Ok((Term::Integer(1), Term::nil()))
    );
    assert_eq!(Term::nil().split_cons(), Err(Term::nil()));
    assert_eq!(atom("x").split_cons(), Err(atom("x")));
}

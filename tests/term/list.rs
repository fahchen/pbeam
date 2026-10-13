use super::super::{Term, test_atom as atom};
use super::{Cons, ConsRef};

fn integers(values: &[i64]) -> Vec<Term> {
    values.iter().copied().map(Term::Integer).collect()
}

/// `[1, 2 | x]`.
fn improper() -> Term {
    Term::list_with_tail(integers(&[1, 2]), atom("x"))
}

/// The cons cell of a list term.
fn cell(term: &Term) -> &ConsRef {
    match term {
        Term::Cons(cell) => cell,
        other => panic!("{other:?} is not a cons cell"),
    }
}

#[test]
fn builds_lists_as_cons_cells() {
    assert_eq!(Term::nil(), Term::Nil);
    assert_eq!(Term::list_from([]), Term::Nil);
    let list = Term::list_from((1..=2).map(Term::Integer));
    assert_eq!(
        list.cons_cell(),
        Some((&Term::Integer(1), &Term::list_from(integers(&[2]))))
    );
    let rest = list.cons_cell().unwrap().1;
    assert_eq!(rest.cons_cell(), Some((&Term::Integer(2), &Term::Nil)));
    assert_eq!(
        improper(),
        Term::cons(Term::Integer(1), Term::cons(Term::Integer(2), atom("x")))
    );
    // A list tail gives a longer list, and no elements give the tail.
    assert_eq!(
        Term::list_with_tail(integers(&[1]), Term::list_from(integers(&[2]))),
        Term::list_from(integers(&[1, 2]))
    );
    assert_eq!(Term::list_with_tail(vec![], atom("x")), atom("x"));
}

#[test]
fn shares_the_tail_of_a_new_cons_cell_and_the_cells_of_a_copy() {
    let tail = Term::list_from(integers(&[2, 3]));
    let list = Term::cons(Term::Integer(1), tail.clone());
    let (_, shared) = list.cons_cell().unwrap();
    assert!(cell(shared).same_cell(cell(&tail)));
    let copy = list.clone();
    assert!(cell(&copy).same_cell(cell(&list)));
    assert!(!cell(&list).same_cell(cell(&Term::cons(Term::Integer(1), tail))));
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
        (Term::tuple(vec![]), false, false, false),
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
    let elements = |term: &Term| {
        term.list_iter()
            .map(|elements| elements.cloned().collect::<Vec<_>>())
    };
    assert_eq!(elements(&proper), Some(integers(&[1, 2])));
    assert_eq!(elements(&improper()), Some(integers(&[1, 2])));
    assert_eq!(elements(&Term::nil()), Some(vec![]));
    assert_eq!(elements(&x), None);

    let list = improper();
    let mut walk_list = list.list_iter().unwrap();
    assert_eq!(walk_list.next(), Some(&Term::Integer(1)));
    assert_eq!(
        walk_list.rest(),
        &Term::list_with_tail(integers(&[2]), atom("x"))
    );
    assert_eq!(walk_list.by_ref().count(), 1);
    assert_eq!(walk_list.rest(), &x);

    let tail = |term: &Term| term.list_cells().map(|(_, tail)| tail.cloned());
    assert_eq!(tail(&proper), Some(None));
    assert_eq!(tail(&improper()), Some(Some(x.clone())));
    assert_eq!(tail(&Term::nil()), Some(None));
    assert_eq!(tail(&x), None);

    let proper_elements = |term: &Term| {
        term.proper_list()
            .map(|elements| elements.cloned().collect::<Vec<_>>())
    };
    assert_eq!(proper_elements(&proper), Some(integers(&[1, 2])));
    assert_eq!(proper_elements(&Term::nil()), Some(vec![]));
    assert_eq!(proper_elements(&improper()), None);
    assert_eq!(proper_elements(&x), None);

    assert_eq!(proper.list_head(), Some(&Term::Integer(1)));
    assert_eq!(improper().list_head(), Some(&Term::Integer(1)));
    assert_eq!(Term::nil().list_head(), None);
    assert_eq!(x.list_head(), None);

    assert_eq!(
        improper().cons_cell().map(|(head, _)| head.clone()),
        Some(Term::Integer(1))
    );
    assert_eq!(Term::nil().cons_cell(), None);
}

#[test]
fn copies_the_elements_of_a_list() {
    let proper = Term::list_from(integers(&[1, 2]));
    assert_eq!(proper.list_elements(), Some((integers(&[1, 2]), true)));
    assert_eq!(improper().list_elements(), Some((integers(&[1, 2]), false)));
    assert_eq!(Term::nil().list_elements(), Some((vec![], true)));
    assert_eq!(atom("x").list_elements(), None);
}

#[test]
fn drops_a_long_list_and_a_shared_list_without_recursion() {
    // A recursive drop of 1000000 cons cells overflows the Rust stack.
    let long = Term::list_from((0..1_000_000).map(Term::Integer));
    let copy = long.clone();
    drop(long);
    assert_eq!(copy.list_head(), Some(&Term::Integer(0)));
    drop(copy);
    let nested = (0..1_000_000).fold(Term::nil(), |inner, _| Term::cons(inner, Term::nil()));
    drop(nested);
}

#[test]
fn reports_the_bytes_of_a_cons_cell() {
    let term = Term::cons(Term::Integer(1), Term::nil());
    let Term::Cons(cell) = &term else {
        unreachable!()
    };
    assert_eq!(cell.allocated_bytes(), std::mem::size_of::<Cons>());
}

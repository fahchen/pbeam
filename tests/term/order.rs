use std::{
    cmp::Ordering::{Equal, Greater, Less},
    sync::Arc,
};

use super::super::{
    Atom, AtomTable, Binary, Fun, Lambda, Map, MatchState, PortId, ProcessId, Reference, Term,
    test_atom as atom,
};

#[test]
#[should_panic(expected = "a match context reached a term operation")]
fn panics_for_a_match_context() {
    // The interpreter gives a runtime error before a match context can
    // reach a term operation.
    let _ = Term::MatchContext(Arc::new(MatchState::new(Binary::from(b"a"))))
        .compare(&Term::Integer(1));
}

fn map(pairs: Vec<(Term, Term)>) -> Term {
    Term::Map(Map::from_pairs(pairs).unwrap())
}

/// Return a fun of a lambda of `module` with an index, an old unique
/// value, and free variables.
fn fun(module: &str, index: i32, old_unique: i32, free: Vec<Term>) -> Term {
    let module = Atom::from_static(module).unwrap();
    Term::Fun(Fun::new(
        Lambda::new(module, index, old_unique, 53, 1),
        free,
    ))
}

#[test]
fn orders_terms_of_different_types_by_type() {
    // ERTS 29.1.1: lists:sort/1 gives
    // [-1,1,ab,#Ref<...>,<0.10.0>,{},#{},[],[a],<<>>] for these terms,
    // and puts a fun between a reference and a pid.
    let sorted = [
        Term::Integer(-1),
        Term::Integer(1),
        atom("ab"),
        Term::Reference(Reference(1)),
        fun("lists", 0, 0, vec![]),
        Term::Pid(ProcessId(0)),
        Term::tuple(vec![]),
        map(vec![]),
        Term::nil(),
        Term::list_from(vec![atom("a")]),
        Term::binary(vec![]),
    ];
    for (index, left) in sorted.iter().enumerate() {
        for (other, right) in sorted.iter().enumerate() {
            assert_eq!(left.compare(right).unwrap(), index.cmp(&other));
        }
    }
}

#[test]
fn orders_atoms_by_name_and_not_by_index() {
    // A fresh table gives z the lower index. ERTS 29.1.1: z >= a is true,
    // ab >= abc is false, and 'é' >= z is true.
    let mut table = AtomTable::new();
    let z = Term::Atom(table.put("pbeam_order_z").unwrap());
    let a = Term::Atom(table.put("pbeam_order_a").unwrap());
    assert_eq!(z.compare(&a).unwrap(), Greater);
    assert_eq!(atom("ab").compare(&atom("abc")).unwrap(), Less);
    let accent = Term::Atom(table.put("é").unwrap());
    assert_eq!(accent.compare(&atom("z")).unwrap(), Greater);
    // Map values compare in the term order of the keys: a, then z.
    let first = map(vec![
        (z.clone(), Term::Integer(1)),
        (a.clone(), Term::Integer(2)),
    ]);
    let second = map(vec![(z, Term::Integer(2)), (a, Term::Integer(1))]);
    assert_eq!(first.compare(&second).unwrap(), Greater);
}

#[test]
fn orders_nested_terms_as_pinned_erts() {
    let list = |names: &[&str]| Term::list_from(names.iter().map(|name| atom(name)));
    let tuple = |names: &[&str]| Term::tuple(names.iter().map(|name| atom(name)).collect());
    let pairs = [
        (tuple(&["b"]), tuple(&["a", "a"]), Less),
        (tuple(&["a", "b"]), tuple(&["a", "a"]), Greater),
        (list(&["a", "b"]), list(&["a", "c"]), Less),
        (list(&["a", "b"]), list(&["a"]), Greater),
        (list(&["b"]), list(&["a", "z"]), Greater),
        (Term::binary(vec![1, 2]), Term::binary(vec![1, 3]), Less),
        (Term::binary(vec![1, 2]), Term::binary(vec![1]), Greater),
        (Term::binary(vec![2]), Term::binary(vec![1, 9]), Greater),
        (Term::Pid(ProcessId(2)), Term::Pid(ProcessId(1)), Greater),
        (
            map(vec![(atom("a"), Term::Integer(1))]),
            map(vec![]),
            Greater,
        ),
        (
            map(vec![(atom("z"), Term::Integer(1))]),
            map(vec![
                (atom("a"), Term::Integer(1)),
                (atom("b"), Term::Integer(1)),
            ]),
            Less,
        ),
        (
            map(vec![(atom("a"), Term::Integer(2))]),
            map(vec![(atom("b"), Term::Integer(1))]),
            Less,
        ),
        (
            map(vec![(Term::Integer(1), atom("x"))]),
            map(vec![(atom("a"), atom("x"))]),
            Less,
        ),
        (
            Term::tuple(vec![Term::Reference(Reference(1)), list(&["a"])]),
            Term::tuple(vec![Term::Reference(Reference(1)), list(&["a"])]),
            Equal,
        ),
    ];
    for (left, right, order) in pairs {
        assert_eq!(
            left.compare(&right).unwrap(),
            order,
            "{left:?} and {right:?}"
        );
    }
    let deep = |levels| (0..levels).fold(Term::nil(), |inner, _| Term::list_from(vec![inner]));
    assert_eq!(deep(1_000_000).compare(&deep(999_999)).unwrap(), Greater);
}

#[test]
fn orders_improper_lists_by_elements_then_tails_as_pinned_erts() {
    // ERTS 29.1.1 (`erts_cmp_compound`, `utils.c:1827-1851`): with equal
    // lengths the tails compare, and a proper list has the tail []. The
    // tail of a shorter list compares with a list that is not empty.
    let list = |elements: &[i64]| Term::list_from(elements.iter().map(|e| Term::Integer(*e)));
    let improper = |elements: &[i64], tail| {
        Term::list_with_tail(
            elements
                .iter()
                .map(|element| Term::Integer(*element))
                .collect(),
            tail,
        )
    };
    let tuple = |name| Term::tuple(vec![atom(name)]);
    let pairs = [
        (improper(&[1], atom("x")), list(&[1, 2]), Less),
        (improper(&[1], Term::binary(vec![])), list(&[1, 2]), Greater),
        (
            improper(&[1, 2], atom("a")),
            improper(&[1, 2], atom("b")),
            Less,
        ),
        (improper(&[1], atom("a")), list(&[1]), Less),
        (
            improper(&[1], Term::Integer(2)),
            improper(&[1], atom("x")),
            Less,
        ),
        (list(&[]), improper(&[1], atom("x")), Less),
        (tuple("a"), improper(&[1], atom("x")), Less),
        (improper(&[1], atom("x")), improper(&[1], atom("x")), Equal),
        (improper(&[1], Term::tuple(vec![])), list(&[1]), Less),
        (list(&[1, 2]), improper(&[1], atom("x")), Greater),
        (list(&[1]), improper(&[1], atom("a")), Greater),
        (improper(&[1], tuple("a")), improper(&[1], tuple("b")), Less),
        (
            improper(&[1, 2], atom("x")),
            improper(&[1], Term::binary(vec![])),
            Less,
        ),
        (improper(&[2], atom("x")), list(&[1, 2]), Greater),
        (improper(&[1], map(vec![])), list(&[1]), Less),
    ];
    for (left, right, order) in pairs {
        assert_eq!(
            left.compare(&right).unwrap(),
            order,
            "{left:?} and {right:?}"
        );
    }
}

#[test]
fn orders_local_funs_by_fun_entry_then_free_variables() {
    // `erts_cmp_compound` (`utils.c:2157-2187`) compares the module name,
    // the index, the old unique value, the number of free variables, and
    // then the free variables. ERTS 29.1.1: `fun_probe:make_pair(b) >=
    // fun_probe:make_pair(a)` is true.
    let pairs = [
        (fun("lists", 9, 9, vec![]), fun("maps", 0, 0, vec![]), Less),
        (
            fun("lists", 1, 0, vec![]),
            fun("lists", 0, 9, vec![]),
            Greater,
        ),
        (
            fun("lists", 0, -1, vec![]),
            fun("lists", 0, 1, vec![]),
            Less,
        ),
        (
            fun("lists", 0, 0, vec![atom("a")]),
            fun("lists", 0, 0, vec![]),
            Greater,
        ),
        (
            fun("lists", 0, 0, vec![atom("b")]),
            fun("lists", 0, 0, vec![atom("a")]),
            Greater,
        ),
        (
            fun("lists", 0, 0, vec![atom("a")]),
            fun("lists", 0, 0, vec![atom("a")]),
            Equal,
        ),
    ];
    for (left, right, order) in pairs {
        assert_eq!(
            left.compare(&right).unwrap(),
            order,
            "{left:?} and {right:?}"
        );
    }
}

#[test]
fn orders_external_funs_by_mfa_and_after_local_funs() {
    // `erts_cmp_compound` (`utils.c:2189-2207`) compares the module name,
    // the function name, and the arity of two external funs, and puts a
    // local fun before an external fun. ERTS 29.1.1: `fun fun_probe:id/1
    // >= fun_probe:make_pair(a)` is true, and `fun fun_probe:mfas/0 >=
    // fun fun_probe:id/1` is true.
    let external = |module, function, arity| {
        let atom = |name| Atom::from_static(name).unwrap();
        Term::Fun(Fun::external(atom(module), atom(function), arity))
    };
    let pairs = [
        (
            external("lists", "put", 2),
            external("maps", "get", 2),
            Less,
        ),
        (
            external("lists", "put", 2),
            external("lists", "get", 3),
            Greater,
        ),
        (
            external("lists", "put", 1),
            external("lists", "put", 2),
            Less,
        ),
        (
            external("lists", "put", 2),
            external("lists", "put", 2),
            Equal,
        ),
        (
            fun("maps", 9, 9, vec![atom("a")]),
            external("lists", "put", 0),
            Less,
        ),
        (
            external("lists", "put", 0),
            fun("maps", 9, 9, vec![]),
            Greater,
        ),
    ];
    for (left, right, order) in pairs {
        assert_eq!(
            left.compare(&right).unwrap(),
            order,
            "{left:?} and {right:?}"
        );
    }
}

#[test]
fn rejects_the_order_of_two_different_references() {
    // ERTS 29.1.1: a reference made later on scheduler 1 is less than one
    // made earlier on scheduler 2, so PBeam cannot give the ERTS order.
    let pair = Term::tuple(vec![atom("a"), Term::Reference(Reference(1))]);
    let other = Term::tuple(vec![atom("a"), Term::Reference(Reference(2))]);
    assert_eq!(
        pair.compare(&other).unwrap_err().to_string(),
        "unsupported term order of two references Reference(Reference(1)) and \
         Reference(Reference(2))"
    );
    let earlier = Term::tuple(vec![atom("a"), Term::Reference(Reference(2))]);
    assert_eq!(
        Term::tuple(vec![atom("b"), Term::Reference(Reference(1))])
            .compare(&earlier)
            .unwrap(),
        Greater
    );
}

#[test]
fn orders_ports_by_number_between_funs_and_pids() {
    // ERTS: #Port<0.3> < #Port<0.10>, fun lists:reverse/1 < #Port<0.3>,
    // and #Port<0.3> < <0.1.0> (utils.c:1751-1755; erl_term.h order).
    let (three, ten) = (Term::Port(PortId(3)), Term::Port(PortId(10)));
    let static_atom = |name| Atom::from_static(name).unwrap();
    let reverse = Term::Fun(Fun::external(
        static_atom("lists"),
        static_atom("reverse"),
        1,
    ));
    assert_eq!(three.compare(&ten).unwrap(), Less);
    assert_eq!(three.compare(&three).unwrap(), Equal);
    assert_eq!(reverse.compare(&three).unwrap(), Less);
    assert_eq!(three.compare(&Term::Pid(ProcessId(1))).unwrap(), Less);
    assert_eq!(three.compare(&Term::tuple(vec![])).unwrap(), Less);
}

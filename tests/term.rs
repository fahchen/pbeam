use std::sync::Arc;

use super::{
    AtomTable, Binary, Fun, Lambda, Map, MatchState, PortId, ProcessId, Reference, Term,
    debug_term, test_atom as atom, test_atom_table,
};

fn map(pairs: Vec<(Term, Term)>) -> Term {
    Term::Map(Map::from_pairs(pairs, &test_atom_table()).unwrap())
}

/// Return a fun of a lambda of `lists` with an index and free variables.
fn fun(index: i32, free: Vec<Term>) -> Term {
    let module = super::Atom::from_static("lists").unwrap();
    Term::Fun(Fun::new(Lambda::new(module, index, index, 7, 53, 1), free))
}

fn deep(levels: usize) -> Term {
    (0..levels).fold(Term::nil(), |inner, level| match level % 4 {
        0 => Term::tuple(vec![inner]),
        1 => Term::list_from(vec![Term::Integer(1), inner]),
        2 => map(vec![(atom("k"), inner)]),
        _ => fun(0, vec![inner]),
    })
}

#[test]
fn holds_no_inline_atom_name_in_a_term() {
    // #460 removed the inline `String` of an atom, and #596 removed the
    // `Arc<str>` name, so the atom holds only its 4-byte index. `Term` is 16
    // bytes, which is the target of `docs/term-layout-design.md`. This test
    // holds that size and stops a later change from making a term larger.
    assert_eq!(size_of::<super::Atom>(), 4);
    assert_eq!(size_of::<Term>(), 16);
}

#[test]
fn drops_a_term_with_many_nested_terms_without_recursion() {
    // The walk of `Term::drop` is iterative. A tuple and a fun give the array
    // that they own to the walk, and a map holds its pairs on a vector, so a
    // tuple, a fun, and a map with many nested terms walk every one of them
    // without a Rust stack overflow.
    let wide = Term::tuple((0..100).map(Term::Integer).collect());
    assert_eq!(wide, Term::tuple((0..100).map(Term::Integer).collect()));
    drop(wide);

    let fun = fun(0, (0..100).map(Term::Integer).collect());
    drop(fun);

    let big = map((0..100)
        .map(|key| (Term::Integer(key), Term::Integer(key)))
        .collect());
    drop(big);
}

#[test]
fn drops_a_shared_map_and_a_cloned_map_with_the_same_values() {
    let form = || {
        map(vec![
            (atom("k"), Term::Integer(1)),
            (Term::Integer(2), Term::list_from([atom("v")])),
        ])
    };
    // A shared map: the drop of one holder moves no pair, and the other
    // holder keeps every pair.
    let shared = form();
    let copy = shared.clone();
    assert!(shared.shares_storage_with(&copy));
    drop(shared);
    assert_eq!(copy, form());
    drop(copy);

    // A clone whose original has dropped: the clone is the last holder of
    // the form, so its drop moves every pair.
    let original = form();
    let clone = original.clone();
    drop(original);
    assert_eq!(clone, form());
    drop(clone);
}

#[test]
fn clones_compares_formats_and_drops_deep_terms_without_recursion() {
    let term = deep(1_000_000);
    let copy = term.clone();
    assert_eq!(term, copy);
    assert_ne!(term, deep(999_999));
    assert!(debug_term(&copy, &test_atom_table()).starts_with(
        "Fun(Local(Lambda { module: Atom(\"lists\"), index: 0, \
         old_unique: 7, label: 53, arity: 1 }), [Map([(Atom(\"k\"), List([Integer(1), Tuple([Fun("
    ));
    drop(term);
    drop(copy);
}

#[test]
fn clones_compares_formats_and_drops_deep_improper_lists_without_recursion() {
    // Each level nests through the tail or through the first element.
    let deep = |levels: usize| {
        (0..levels).fold(atom("x"), |inner, level| match level % 2 {
            0 => Term::list_with_tail(vec![Term::Integer(1)], Term::tuple(vec![inner])),
            _ => Term::list_with_tail(vec![inner], atom("t")),
        })
    };
    let term = deep(1_000_000);
    let copy = term.clone();
    assert_eq!(term, copy);
    assert_ne!(term, deep(999_999));
    assert!(format!("{copy:?}").starts_with(
        "ImproperList([ImproperList([Integer(1)], Tuple([ImproperList([ImproperList(["
    ));
    drop(term);
    drop(copy);
}

#[test]
fn compares_and_formats_improper_lists_by_elements_and_tail() {
    // ERTS 29.1.1: [1|x] =:= [1|x] is true, and [1,2|a] =:= [1,2|b],
    // [1|a] =:= [1], and [1|2] =:= [1|x] are false (`eq`, `utils.c:1137-1162`).
    let improper = |elements: &[i64], tail| {
        Term::list_with_tail(
            elements
                .iter()
                .map(|element| Term::Integer(*element))
                .collect(),
            tail,
        )
    };
    assert_eq!(improper(&[1], atom("x")), improper(&[1], atom("x")).clone());
    assert_ne!(improper(&[1, 2], atom("a")), improper(&[1, 2], atom("b")));
    assert_ne!(
        improper(&[1], atom("a")),
        Term::list_from(vec![Term::Integer(1)])
    );
    assert_ne!(improper(&[1], Term::Integer(2)), improper(&[1], atom("x")));
    assert_ne!(improper(&[1], atom("x")), improper(&[1, 2], atom("x")));
    assert_ne!(improper(&[1], atom("x")), improper(&[2], atom("x")));
    assert_eq!(
        debug_term(
            &Term::tuple(vec![improper(&[1, 2], Term::tuple(vec![atom("t")]))]),
            &test_atom_table()
        ),
        "Tuple([ImproperList([Integer(1), Integer(2)], Tuple([Atom(\"t\")]))])"
    );
}

#[test]
fn builds_each_list_in_its_one_form() {
    let one = || vec![Term::Integer(1)];
    let x = || Box::new(atom("x"));
    assert_eq!(
        Term::list_with_tail(one(), Term::list_from(vec![Term::Integer(2)])),
        Term::list_from(vec![Term::Integer(1), Term::Integer(2)])
    );
    assert_eq!(
        Term::list_with_tail(one(), Term::nil()),
        Term::list_from(one())
    );
    assert_eq!(
        Term::list_with_tail(one(), Term::list_with_tail(vec![Term::Integer(2)], *x())),
        Term::cons(Term::Integer(1), Term::cons(Term::Integer(2), *x()))
    );
    assert_eq!(
        Term::list_with_tail(one(), *x()),
        Term::cons(Term::Integer(1), *x())
    );
    assert_eq!(Term::list_with_tail(vec![], *x()), *x());
    assert_eq!(
        Term::list_with_tail(vec![], Term::list_from(one())),
        Term::list_from(one())
    );
}

#[test]
fn compares_funs_by_their_lambda_and_free_variables() {
    // ERTS 29.1.1: two funs of one lambda are equal when their free
    // variables are equal (`eq` in `utils.c:1241-1276`).
    let tagged = |tag| fun(0, vec![atom(tag)]);
    assert_eq!(tagged("a"), tagged("a").clone());
    assert_ne!(tagged("a"), tagged("b"));
    assert_ne!(tagged("a"), fun(1, vec![atom("a")]));
    assert_ne!(tagged("a"), fun(0, vec![]));
    assert_ne!(fun(0, vec![]), Term::tuple(vec![]));
    assert_eq!(fun(0, vec![]), fun(0, vec![]));
    // ERTS identifies a fun entry by its module, index, arity, and module
    // MD5, and not by its old unique value (`fun_cmp` in
    // `erl_fun.c:79-85`). The entry label is only the address of the code.
    let module = || super::Atom::from_static("lists").unwrap();
    let entry = |label, arity| {
        Term::Fun(Fun::new(
            Lambda::new(module(), 0, 0, 7, label, arity),
            vec![],
        ))
    };
    assert_eq!(entry(53, 1), entry(61, 1));
    assert_ne!(entry(53, 1), entry(53, 2));
    assert_eq!(
        fun(0, vec![]),
        Term::Fun(Fun::new(Lambda::new(module(), 0, 0, 8, 53, 1), vec![]))
    );
}

#[test]
fn compares_external_funs_by_their_mfa() {
    // ERTS 29.1.1: two external funs are equal when their modules,
    // functions, and arities are equal (`eq` in `utils.c:1270-1273`), and
    // an external fun is not equal to a local fun.
    let external = |function, arity| {
        let atom = |name| super::Atom::from_static(name).unwrap();
        Term::Fun(Fun::external(atom("lists"), atom(function), arity))
    };
    assert_eq!(external("get", 1), external("get", 1).clone());
    assert_ne!(external("get", 1), external("get", 2));
    assert_ne!(external("get", 1), external("put", 1));
    assert_ne!(external("get", 1), fun(0, vec![]));
    assert_eq!(
        format!("{:?}", external("get", 1)),
        "Fun(External { module: Atom { index: 971 }, function: Atom { index: 781 }, \
         arity: 1 }, [])"
    );
}

#[test]
fn prints_each_free_variable_of_a_fun() {
    // The `Debug` walk writes the free variables of a fun and separates them
    // with `, `. A tuple and a fun shared one arm of the walk before #454, and
    // the test of a tuple with two elements covered the separator. The arm of
    // a fun needs this test of its own.
    let term = fun(0, vec![Term::Integer(1), Term::Integer(2)]);
    assert_eq!(
        format!("{term:?}"),
        "Fun(Local(Lambda { module: Atom { index: 971 }, index: 0, \
         old_index: 0, old_unique: 7, uniq: [0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0], label: 53, arity: 1 }), [Integer(1), Integer(2)])"
    );
}

#[test]
fn matches_the_derived_debug_text_and_compares_every_variant() {
    let leaves = [
        atom("ok"),
        Term::binary(vec![1, 2]),
        Term::Integer(-3),
        Term::Float(-0.0),
        Term::Pid(ProcessId(4)),
        Term::Reference(Reference(5)),
        Term::MatchContext(Arc::new(MatchState::new(Binary::from(&[6])))),
    ];
    let term = Term::tuple(vec![Term::list_from(leaves.to_vec()), Term::nil()]);
    assert_eq!(
        debug_term(&term, &test_atom_table()),
        "Tuple([List([Atom(\"ok\"), Binary([1, 2]), Integer(-3), Float(-0.0), Pid(ProcessId(4)), \
         Reference(Reference(5)), MatchContext(MatchState { bytes: [6], position: 0 })]), \
         List([])])"
    );
    for (index, leaf) in leaves.iter().enumerate() {
        assert_eq!(leaf, &leaf.clone());
        assert_ne!(leaf, &leaves[(index + 1) % leaves.len()]);
    }
    assert_ne!(Term::nil(), Term::tuple(vec![]));
    assert_ne!(Term::nil(), Term::list_from(vec![Term::Integer(1)]));
    assert_ne!(Term::tuple(vec![atom("a")]), Term::tuple(vec![atom("b")]));
}

#[test]
fn prints_an_atom_index_without_a_table_and_a_name_with_one() {
    // `Debug` prints the atom index, because a bare term carries no table and
    // the caller of `Debug::fmt` cannot pass one. `debug_term` reads the name
    // through a table. ERTS gives the static atom `undef` index 725.
    let term = Term::static_atom("undef");
    assert_eq!(format!("{term:?}"), "Atom(725)");
    assert_eq!(debug_term(&term, &AtomTable::new()), "Atom(\"undef\")");
}

#[test]
fn prints_the_module_and_the_function_name_of_a_fun_target() {
    // A bare atom term carries no table, so `Debug` writes the index of the
    // module and the function of an external fun. `debug_term` reads both
    // names through the table, and a local fun reads the name of its module.
    let table = test_atom_table();
    let atom_of = |name| super::Atom::from_static(name).unwrap();
    let external = Term::Fun(Fun::external(atom_of("lists"), atom_of("get"), 1));
    assert_eq!(
        debug_term(&external, &table),
        "Fun(External { module: Atom(\"lists\"), function: Atom(\"get\"), arity: 1 }, [])"
    );
    let local = fun(0, vec![]);
    assert_eq!(
        debug_term(&local, &table),
        "Fun(Local(Lambda { module: Atom(\"lists\"), index: 0, old_unique: 7, label: 53, arity: 1 }), [])"
    );
}

#[test]
fn formats_and_compares_maps_by_their_pairs() {
    // A fresh table for this test makes a before b, so the Debug text
    // lists a first. A table shared with other tests could make b first.
    let table = AtomTable::new();
    let a = Term::Atom(table.put("a").unwrap());
    let b = Term::Atom(table.put("b").unwrap());
    let atom = |name: &str| match name {
        "a" => a.clone(),
        "b" => b.clone(),
        _ => crate::term::test_atom(name),
    };
    let pairs = || vec![(atom("a"), Term::Integer(1)), (atom("b"), Term::nil())];
    let sorted = map(pairs());
    assert_eq!(
        debug_term(&sorted, &table),
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
    assert_ne!(map(vec![]), Term::nil());
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
    assert!(
        debug_term(&forty, &test_atom_table())
            .starts_with("Map([(Integer(18), Atom(\"v\")), (Integer(4), ")
    );
    assert_ne!(forty, hash(&mut (2..=41)));
}

#[test]
fn gives_a_reference_its_erts_reference_numbers() {
    // ERTS 29.1.1 prints the reference with the counter value
    // 0xF55F56815BA88196 of scheduler 1 as
    // #Ref<0.4116665985.1537736705.33174>: `#Ref<0.N2.N1.N0>`.
    assert_eq!(
        Reference(0xF55F_5681_5BA8_8196).numbers(),
        [33174, 1537736705, 4116665985]
    );
    assert_eq!(Reference(0).numbers(), [0, 1, 0]);
}

#[test]
fn keeps_the_creation_order_and_the_pid_reference_marker() {
    let alias = Reference(1).pid_reference();
    assert!(alias.is_pid_reference() && !Reference(1).is_pid_reference());
    assert!(Reference(0) < alias && alias < Reference(2));
    assert_eq!(Reference::ordinary(1), Some(Reference(1)));
    assert_eq!(Reference::ordinary(alias.0), None);
}

#[test]
fn clones_compares_and_formats_ports() {
    let port = Term::tuple(vec![Term::Port(PortId(5))]);
    assert_eq!(port.clone(), port);
    assert_ne!(Term::Port(PortId(5)), Term::Port(PortId(6)));
    assert_ne!(Term::Port(PortId(5)), Term::Pid(ProcessId(5)));
    assert_eq!(format!("{port:?}"), "Tuple([Port(PortId(5))])");
}

#[test]
fn finds_a_fun_at_any_depth_without_recursion() {
    let element = |term| Term::list_from(vec![Term::Integer(1), term]);
    assert!(!Term::Integer(1).contains_fun());
    assert!(!deep(3).contains_fun());
    assert!(deep(4).contains_fun());
    assert!(fun(0, vec![]).contains_fun());
    assert!(element(Term::tuple(vec![fun(0, vec![])])).contains_fun());
    assert!(map(vec![(atom("k"), fun(0, vec![]))]).contains_fun());
    assert!(!map(vec![(atom("k"), atom("v"))]).contains_fun());
    let improper = |tail| Term::list_with_tail(vec![Term::Integer(1)], tail);
    assert!(improper(fun(0, vec![])).contains_fun());
    assert!(!improper(atom("x")).contains_fun());
    assert!(Term::list_with_tail(vec![fun(0, vec![])], atom("x")).contains_fun());
    let nested = (0..1_000_000).fold(fun(0, vec![]), |inner, _| Term::tuple(vec![inner]));
    assert!(nested.contains_fun());
    let nested = (0..1_000_000).fold(Term::Integer(1), |inner, _| element(inner));
    assert!(!nested.contains_fun());
}

#[test]
fn compares_a_deep_term_and_a_long_list_without_a_stack_overflow() {
    // The work stack of the comparison is iterative, so a deep term and a
    // long list compare on the heap rather than on the Rust stack. The two
    // terms of each pair come from two builds, so they share no storage and
    // the comparison walks every level or every cell.
    let deep = (0..1_000_000).fold(Term::nil(), |inner, _| Term::tuple(vec![inner]));
    let other_deep = (0..1_000_000).fold(Term::nil(), |inner, _| Term::tuple(vec![inner]));
    assert_eq!(Term::Nil, Term::Nil);
    assert_eq!(deep, other_deep);
    assert_eq!(other_deep, deep);
    let different = (0..1_000_001).fold(Term::nil(), |inner, _| Term::tuple(vec![inner]));
    assert_ne!(deep, different);

    let list = Term::list_from((0..1_000_000).map(Term::Integer));
    let other_list = Term::list_from((0..1_000_000).map(Term::Integer));
    assert_eq!(list, other_list);
    assert_eq!(other_list, list);
    let different = Term::list_from((0..999_999).map(Term::Integer));
    assert_ne!(list, different);
}

#[test]
fn compares_a_shared_term_and_an_equal_term_of_another_storage() {
    // A copy of a compound term shares its storage, so the comparison of
    // two copies reads one pointer. Two terms that hold equal content in
    // different storage walk their elements.
    let deep = (0..100).fold(Term::nil(), |inner, _| Term::tuple(vec![inner]));
    assert_eq!(deep, deep.clone());
    let other = (0..100).fold(Term::nil(), |inner, _| Term::tuple(vec![inner]));
    assert_eq!(deep, other);
    let list = Term::list_from((0..100).map(Term::Integer));
    assert_eq!(list, list.clone());
    assert_eq!(list, Term::list_from((0..100).map(Term::Integer)));
    let held_map = map(vec![
        (atom("a"), Term::Integer(1)),
        (atom("b"), Term::Integer(2)),
    ]);
    assert_eq!(held_map, held_map.clone());
    assert_eq!(
        held_map,
        map(vec![
            (atom("a"), Term::Integer(1)),
            (atom("b"), Term::Integer(2))
        ])
    );
    let held = fun(0, vec![Term::Integer(1)]);
    assert_eq!(held, held.clone());
    assert_eq!(held, fun(0, vec![Term::Integer(1)]));
}

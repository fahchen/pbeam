use super::{MatchSpec, Pattern, SpecError, variable};
use crate::term::{
    AtomTable, Binary, Fun, Map, PortId, ProcessId, Reference, Term, test_atom_table,
};
use num_bigint::BigInt;

fn atom(atoms: &AtomTable, name: &str) -> Term {
    Term::Atom(atoms.put(name).unwrap())
}

#[test]
fn reads_the_variable_numbers_as_db_is_variable() {
    let atoms = test_atom_table();
    let number = |name| {
        let term = atom(&atoms, name);
        variable(&atoms, &term)
    };
    assert_eq!(number("$0"), Some(0));
    assert_eq!(number("$12"), Some(12));
    assert_eq!(number("$01"), None);
    assert_eq!(number("$"), None);
    assert_eq!(number("$1a"), None);
    assert_eq!(number("_"), None);
    // The C `int` arithmetic wraps to a negative number, which is no
    // variable (erl_db_util.c:3913-3926).
    assert_eq!(number("$2147483648"), None);
    assert_eq!(variable(&test_atom_table(), &Term::Integer(1)), None);
}

#[test]
fn matches_lists_maps_and_ground_tuples() {
    let atoms = test_atom_table();
    let one = Term::Integer(1);
    let variable = atom(&atoms, "$1");
    let list = Term::list_with_tail(vec![variable.clone(), one.clone()], atom(&atoms, "_"));
    let pattern = Pattern::compile(&atoms, &Term::tuple(vec![list])).unwrap();
    let object = Term::list_from([Term::Integer(5), one.clone(), Term::Integer(7)]);
    assert_eq!(
        pattern.run(&[object]),
        Some(Term::list_from([Term::Integer(5)]))
    );
    assert_eq!(pattern.run(&[Term::list_from([Term::Integer(5)])]), None);
    assert_eq!(pattern.run(&[Term::nil()]), None);
    // A tuple with no variable stays a tuple step at the root, so it has a
    // bound key and its size.
    let ground = Pattern::compile(&atoms, &Term::tuple(vec![one.clone(), Term::nil()])).unwrap();
    assert_eq!(ground.bound_key(1), Some(&one));
    assert!(ground.matches_nothing(3));
    assert_eq!(ground.run(&[one.clone(), Term::nil()]), Some(Term::nil()));
    let nested =
        Pattern::compile(&atoms, &Term::tuple(vec![Term::tuple(vec![one.clone()])])).unwrap();
    assert!(nested.bound_key(1).is_some());
    let map = Term::Map(
        crate::term::Map::from_pairs(vec![(one.clone(), variable.clone())], &atoms).unwrap(),
    );
    let pattern = Pattern::compile(&atoms, &map).unwrap();
    assert!(pattern.matches_nothing(1));
    assert_eq!(pattern.run(std::slice::from_ref(&one)), None);
    let twice = Term::tuple(vec![variable.clone(), variable]);
    let pattern = Pattern::compile(&atoms, &twice).unwrap();
    assert_eq!(pattern.run(&[one.clone(), Term::Integer(2)]), None);
    assert_eq!(
        pattern.run(&[one.clone(), one.clone()]),
        Some(Term::list_from([one]))
    );
}

/// Compile a one-clause specification and run it.
fn run(
    atoms: &AtomTable,
    head: Term,
    guards: Vec<Term>,
    body: Vec<Term>,
    object: &[Term],
) -> Option<Term> {
    let spec = Term::list_from(vec![Term::tuple(vec![
        head,
        Term::list_from(guards),
        Term::list_from(body),
    ])]);
    MatchSpec::compile(atoms, &spec).unwrap().run(atoms, object)
}

/// Return the guard call `{Name, Arguments...}`.
fn call(atoms: &AtomTable, name: &str, arguments: Vec<Term>) -> Term {
    let mut elements = vec![atom(atoms, name)];
    elements.extend(arguments);
    Term::tuple(elements)
}

#[test]
#[expect(
    clippy::too_many_lines,
    reason = "case table: one entry for each guard function of the match specification"
)]
fn runs_each_guard_function_of_the_match_specification() {
    let atoms = test_atom_table();
    let var1 = atom(&atoms, "$1");
    let var2 = atom(&atoms, "$2");
    let head = Term::tuple(vec![var1.clone(), var2.clone()]);
    let ok = Term::static_atom("true");
    let map =
        Term::Map(Map::from_pairs(vec![(atom(&atoms, "a"), Term::Integer(1))], &atoms).unwrap());
    let fun = Term::Fun(Fun::external(
        atoms.put("m").unwrap(),
        atoms.put("f").unwrap(),
        1,
    ));
    let binary = Term::binary(vec![1, 2]);
    let key_a = atom(&atoms, "a");
    let node = atom(&atoms, "nonode@nohost");
    let equal = |atoms: &AtomTable, guard: Term, expected: Term| {
        Term::tuple(vec![
            atom(atoms, "=:="),
            guard,
            Term::tuple(vec![atom(atoms, "const"), expected]),
        ])
    };
    let bool_cases: Vec<(Term, Term)> = vec![
        (
            call(&atoms, "is_atom", vec![var1.clone()]),
            atom(&atoms, "a"),
        ),
        (
            call(&atoms, "is_float", vec![var1.clone()]),
            Term::Float(1.0),
        ),
        (
            call(&atoms, "is_integer", vec![var1.clone()]),
            Term::Integer(1),
        ),
        (
            call(&atoms, "is_list", vec![var1.clone()]),
            Term::list_from([Term::Integer(1)]),
        ),
        (
            call(&atoms, "is_number", vec![var1.clone()]),
            Term::Integer(1),
        ),
        (
            call(&atoms, "is_pid", vec![var1.clone()]),
            Term::Pid(ProcessId(0)),
        ),
        (
            call(&atoms, "is_port", vec![var1.clone()]),
            Term::Port(PortId(0)),
        ),
        (
            call(&atoms, "is_reference", vec![var1.clone()]),
            Term::Reference(Reference(0)),
        ),
        (
            call(&atoms, "is_tuple", vec![var1.clone()]),
            Term::tuple(vec![Term::Integer(1)]),
        ),
        (call(&atoms, "is_map", vec![var1.clone()]), map.clone()),
        (
            call(&atoms, "is_binary", vec![var1.clone()]),
            binary.clone(),
        ),
        (
            call(&atoms, "is_bitstring", vec![var1.clone()]),
            binary.clone(),
        ),
        (call(&atoms, "is_boolean", vec![var1.clone()]), ok.clone()),
        (call(&atoms, "is_function", vec![var1.clone()]), fun.clone()),
        (
            call(&atoms, "is_function", vec![var1.clone(), Term::Integer(1)]),
            fun,
        ),
        (
            call(&atoms, "is_map_key", vec![key_a.clone(), var1.clone()]),
            map.clone(),
        ),
    ];
    for (guard, value) in bool_cases {
        let object = vec![value.clone(), Term::Integer(2)];
        assert_eq!(
            run(
                &atoms,
                head.clone(),
                vec![guard.clone()],
                vec![ok.clone()],
                &object
            ),
            Some(ok.clone()),
            "{guard:?} {value:?}"
        );
    }
    let value_cases: Vec<(Term, Term, Term)> = vec![
        (
            call(&atoms, "abs", vec![var1.clone()]),
            Term::Integer(-1),
            Term::Integer(1),
        ),
        (
            call(&atoms, "element", vec![Term::Integer(2), var1.clone()]),
            Term::tuple(vec![Term::Integer(1), Term::Integer(2)]),
            Term::Integer(2),
        ),
        (
            call(&atoms, "hd", vec![var1.clone()]),
            Term::list_from([Term::Integer(1), Term::Integer(2)]),
            Term::Integer(1),
        ),
        (
            call(&atoms, "tl", vec![var1.clone()]),
            Term::list_from([Term::Integer(1), Term::Integer(2)]),
            Term::list_from([Term::Integer(2)]),
        ),
        (
            call(&atoms, "length", vec![var1.clone()]),
            Term::list_from([Term::Integer(1), Term::Integer(2)]),
            Term::Integer(2),
        ),
        (
            call(&atoms, "map_size", vec![var1.clone()]),
            map.clone(),
            Term::Integer(1),
        ),
        (
            call(&atoms, "map_get", vec![key_a, var1.clone()]),
            map,
            Term::Integer(1),
        ),
        (
            call(&atoms, "size", vec![var1.clone()]),
            Term::tuple(vec![Term::Integer(1)]),
            Term::Integer(1),
        ),
        (
            call(&atoms, "size", vec![var1.clone()]),
            binary.clone(),
            Term::Integer(2),
        ),
        (
            call(&atoms, "bit_size", vec![var1.clone()]),
            binary.clone(),
            Term::Integer(16),
        ),
        (
            call(&atoms, "byte_size", vec![var1.clone()]),
            binary,
            Term::Integer(2),
        ),
        (
            call(&atoms, "tuple_size", vec![var1.clone()]),
            Term::tuple(vec![Term::Integer(1)]),
            Term::Integer(1),
        ),
        (call(&atoms, "node", vec![]), Term::Integer(0), node.clone()),
        (
            call(&atoms, "node", vec![var1.clone()]),
            Term::Pid(ProcessId(0)),
            node,
        ),
        (
            call(&atoms, "+", vec![var1.clone()]),
            Term::Integer(1),
            Term::Integer(1),
        ),
        (
            call(&atoms, "-", vec![var1.clone()]),
            Term::Integer(1),
            Term::Integer(-1),
        ),
        (
            call(&atoms, "+", vec![var1.clone(), var2.clone()]),
            Term::Integer(1),
            Term::Integer(3),
        ),
        (
            call(&atoms, "-", vec![var1.clone(), var2.clone()]),
            Term::Integer(1),
            Term::Integer(-1),
        ),
        (
            call(&atoms, "*", vec![var1.clone(), var2.clone()]),
            Term::Integer(1),
            Term::Integer(2),
        ),
        (
            call(&atoms, "/", vec![var1.clone(), var2.clone()]),
            Term::Integer(1),
            Term::Float(0.5),
        ),
        (
            call(&atoms, "div", vec![var1.clone(), var2.clone()]),
            Term::Integer(1),
            Term::Integer(0),
        ),
        (
            call(&atoms, "rem", vec![var1.clone(), var2.clone()]),
            Term::Integer(1),
            Term::Integer(1),
        ),
        (
            call(&atoms, "band", vec![var1.clone(), var2.clone()]),
            Term::Integer(1),
            Term::Integer(0),
        ),
        (
            call(&atoms, "bor", vec![var1.clone(), var2.clone()]),
            Term::Integer(1),
            Term::Integer(3),
        ),
        (
            call(&atoms, "bxor", vec![var1.clone(), var2.clone()]),
            Term::Integer(1),
            Term::Integer(3),
        ),
        (
            call(&atoms, "bnot", vec![var1.clone()]),
            Term::Integer(1),
            Term::Integer(-2),
        ),
        (
            call(&atoms, "bsl", vec![var1.clone(), var2.clone()]),
            Term::Integer(1),
            Term::Integer(4),
        ),
        (
            call(&atoms, "bsr", vec![var1.clone(), var2.clone()]),
            Term::Integer(4),
            Term::Integer(1),
        ),
    ];
    for (guard, value, expected) in value_cases {
        let shown = guard.clone();
        let guard = equal(&atoms, guard, expected.clone());
        let object = vec![value.clone(), Term::Integer(2)];
        assert_eq!(
            run(&atoms, head.clone(), vec![guard], vec![ok.clone()], &object),
            Some(ok.clone()),
            "{shown:?} {value:?} {expected:?}"
        );
    }
    for (name, value) in [
        (">", Term::Integer(2)),
        (">=", Term::Integer(1)),
        ("<", Term::Integer(0)),
        ("=<", Term::Integer(1)),
        ("==", Term::Integer(1)),
        ("=:=", Term::Integer(1)),
        ("=/=", Term::Integer(2)),
    ] {
        let guard = call(&atoms, name, vec![var1.clone(), var2.clone()]);
        assert_eq!(
            run(
                &atoms,
                head.clone(),
                vec![guard],
                vec![ok.clone()],
                &[value, Term::Integer(1)],
            ),
            Some(ok.clone())
        );
    }
    for (name, expected) in [("min", Term::Integer(1)), ("max", Term::Integer(2))] {
        let call = call(&atoms, name, vec![var1.clone(), var2.clone()]);
        let guard = equal(&atoms, call, expected);
        assert_eq!(
            run(
                &atoms,
                head.clone(),
                vec![guard],
                vec![ok.clone()],
                &[Term::Integer(2), Term::Integer(1)]
            ),
            Some(ok.clone())
        );
    }
    let guard = call(&atoms, "/=", vec![var1.clone(), var2.clone()]);
    assert_eq!(
        run(
            &atoms,
            head.clone(),
            vec![guard],
            vec![ok.clone()],
            &[Term::Integer(2), Term::Integer(1)]
        ),
        Some(ok.clone())
    );
    for (guard, first, second) in [
        (
            "not",
            Term::static_atom("false"),
            Term::static_atom("false"),
        ),
        ("and", Term::static_atom("true"), Term::static_atom("true")),
        ("or", Term::static_atom("false"), Term::static_atom("true")),
        ("xor", Term::static_atom("true"), Term::static_atom("false")),
    ] {
        let arguments = if guard == "not" {
            vec![var1.clone()]
        } else {
            vec![var1.clone(), var2.clone()]
        };
        let guard = call(&atoms, guard, arguments);
        assert_eq!(
            run(
                &atoms,
                head.clone(),
                vec![guard],
                vec![ok.clone()],
                &[first, second]
            ),
            Some(ok.clone())
        );
    }
    let andalso = Term::tuple(vec![
        atom(&atoms, "andalso"),
        call(&atoms, "is_integer", vec![var1.clone()]),
        Term::static_atom("true"),
    ]);
    let orelse = Term::tuple(vec![
        atom(&atoms, "orelse"),
        call(&atoms, "is_atom", vec![var1]),
        Term::static_atom("true"),
    ]);
    assert_eq!(
        run(
            &atoms,
            head.clone(),
            vec![andalso, orelse],
            vec![ok.clone()],
            &[Term::Integer(1), Term::Integer(2)]
        ),
        Some(ok.clone())
    );
    // Short-circuit values and a non-boolean operand fail the clause.
    let andalso = Term::tuple(vec![
        atom(&atoms, "andalso"),
        Term::static_atom("false"),
        Term::static_atom("true"),
    ]);
    let orelse = Term::tuple(vec![
        atom(&atoms, "orelse"),
        Term::static_atom("true"),
        Term::static_atom("false"),
    ]);
    for (guard, expected) in [
        (andalso, Term::static_atom("false")),
        (orelse, Term::static_atom("true")),
    ] {
        let guard = equal(&atoms, guard, expected.clone());
        assert_eq!(
            run(
                &atoms,
                head.clone(),
                vec![guard],
                vec![ok.clone()],
                &[Term::Integer(1), Term::Integer(2)]
            ),
            Some(ok.clone()),
            "{expected:?}"
        );
    }
    for guard in [
        Term::tuple(vec![
            atom(&atoms, "andalso"),
            Term::Integer(1),
            Term::static_atom("true"),
        ]),
        Term::tuple(vec![
            atom(&atoms, "orelse"),
            Term::Integer(1),
            Term::static_atom("true"),
        ]),
    ] {
        assert_eq!(
            run(
                &atoms,
                head.clone(),
                vec![guard],
                vec![ok.clone()],
                &[Term::Integer(1), Term::Integer(2)]
            ),
            None
        );
    }
}

#[test]
fn builds_match_specification_terms_and_rejects_bad_ones() {
    let atoms = test_atom_table();
    let at = |atoms: &AtomTable, name| atom(atoms, name);
    let head = Term::tuple(vec![at(&atoms, "$1"), at(&atoms, "$2")]);
    let spec = Term::list_from(vec![Term::tuple(vec![
        head.clone(),
        Term::nil(),
        Term::list_from(vec![
            Term::static_atom("$_"),
            Term::static_atom("$$"),
            Term::tuple(vec![Term::tuple(vec![at(&atoms, "$1"), Term::Integer(1)])]),
            Term::list_with_tail(vec![Term::static_atom("$$")], Term::static_atom("$_")),
        ]),
    ])]);
    let compiled = MatchSpec::compile(&atoms, &spec).unwrap();
    assert_eq!(compiled.specification(), spec);
    assert_eq!(
        compiled.run(&atoms, &[Term::Integer(1), Term::Integer(2)]),
        Some(Term::list_with_tail(
            vec![Term::list_from([Term::Integer(1), Term::Integer(2)])],
            Term::tuple(vec![Term::Integer(1), Term::Integer(2)]),
        ))
    );
    // The first clause whose guards hold decides the result, and a bad body
    // BIF gives the atom EXIT.
    let clauses = Term::list_from(vec![
        Term::tuple(vec![
            head.clone(),
            Term::list_from([at(&atoms, "false")]),
            Term::list_from([Term::Integer(1)]),
        ]),
        Term::tuple(vec![
            head.clone(),
            Term::nil(),
            Term::list_from([Term::Integer(2)]),
        ]),
    ]);
    assert_eq!(
        MatchSpec::compile(&atoms, &clauses)
            .unwrap()
            .run(&atoms, &[Term::Integer(1), Term::Integer(2)]),
        Some(Term::Integer(2))
    );
    let body = Term::list_from(vec![call(
        &atoms,
        "element",
        vec![Term::Integer(3), Term::static_atom("$_")],
    )]);
    let spec = Term::list_from(vec![Term::tuple(vec![head, Term::nil(), body])]);
    assert_eq!(
        MatchSpec::compile(&atoms, &spec)
            .unwrap()
            .run(&atoms, &[Term::Integer(1), Term::Integer(2)]),
        Some(Term::static_atom("EXIT"))
    );
}

#[test]
fn builds_match_specification_bodies_with_bignums_and_failing_division() {
    let atoms = test_atom_table();
    let at = |atoms: &AtomTable, name| atom(atoms, name);
    let head = Term::tuple(vec![at(&atoms, "$1"), at(&atoms, "$2")]);
    // A large arithmetic result becomes a bignum, and division by zero fails.
    let big = Term::Integer(1 << 40);
    let body = Term::list_from(vec![call(&atoms, "*", vec![big.clone(), big])]);
    let spec = Term::list_from(vec![Term::tuple(vec![head.clone(), Term::nil(), body])]);
    assert!(matches!(
        MatchSpec::compile(&atoms, &spec)
            .unwrap()
            .run(&atoms, &[Term::Integer(1), Term::Integer(2)]),
        Some(Term::BigInt(_))
    ));
    let body = Term::list_from(vec![call(
        &atoms,
        "/",
        vec![Term::Float(1.0), Term::Float(0.0)],
    )]);
    let spec = Term::list_from(vec![Term::tuple(vec![head, Term::nil(), body])]);
    assert_eq!(
        MatchSpec::compile(&atoms, &spec)
            .unwrap()
            .run(&atoms, &[Term::Integer(1), Term::Integer(2)]),
        Some(Term::static_atom("EXIT"))
    );
}

#[test]
fn rejects_bad_match_specifications() {
    let atoms = test_atom_table();
    let at = |atoms: &AtomTable, name| atom(atoms, name);
    let head = Term::tuple(vec![at(&atoms, "$1"), at(&atoms, "$2")]);
    let ok = Term::static_atom("true");
    // An unsupported guard, a map expression, and a bad specification.
    let unsupported = Term::list_from(vec![Term::tuple(vec![
        head.clone(),
        Term::list_from([call(&atoms, "round", vec![Term::Float(1.0)])]),
        Term::nil(),
    ])]);
    assert!(matches!(
        MatchSpec::compile(&atoms, &unsupported),
        Err(SpecError::Unsupported(_))
    ));
    let map =
        Term::Map(Map::from_pairs(vec![(at(&atoms, "a"), Term::Integer(1))], &atoms).unwrap());
    let spec = Term::list_from(vec![Term::tuple(vec![
        head.clone(),
        Term::nil(),
        Term::list_from([map]),
    ])]);
    assert!(matches!(
        MatchSpec::compile(&atoms, &spec),
        Err(SpecError::Unsupported(_))
    ));
    let bad = [
        Term::Integer(1),
        Term::list_from(vec![Term::Integer(1)]),
        Term::list_from(vec![Term::tuple(vec![
            head.clone(),
            Term::nil(),
            Term::nil(),
        ])]),
        Term::list_from(vec![Term::tuple(vec![
            head.clone(),
            Term::Integer(1),
            Term::nil(),
        ])]),
        Term::list_from(vec![Term::tuple(vec![
            head.clone(),
            Term::nil(),
            Term::Integer(1),
        ])]),
        Term::list_from(vec![Term::tuple(vec![
            head.clone(),
            Term::nil(),
            Term::list_from([at(&atoms, "$3")]),
        ])]),
        Term::list_from(vec![Term::tuple(vec![
            head.clone(),
            Term::nil(),
            Term::list_from([call(&atoms, "bogus", vec![])]),
        ])]),
        Term::list_from(vec![Term::tuple(vec![head.clone(), Term::nil()])]),
        Term::list_from(vec![Term::tuple(vec![
            head.clone(),
            Term::nil(),
            Term::list_from([Term::tuple(vec![Term::Integer(1), Term::Integer(2)])]),
        ])]),
        Term::list_from(vec![Term::tuple(vec![
            head.clone(),
            Term::nil(),
            Term::list_from([Term::tuple(vec![atom(&atoms, "and")])]),
        ])]),
        Term::list_from(vec![Term::tuple(vec![
            head.clone(),
            Term::nil(),
            Term::list_from([Term::tuple(vec![atom(&atoms, "andalso")])]),
        ])]),
    ];
    for spec in bad {
        assert!(
            matches!(MatchSpec::compile(&atoms, &spec), Err(SpecError::Badarg)),
            "{spec:?}"
        );
    }
    assert!(MatchSpec::compile(&atoms, &Term::nil()).is_ok());
    let compiled = MatchSpec::compile(
        &atoms,
        &Term::list_from(vec![Term::tuple(vec![
            head,
            Term::nil(),
            Term::list_from([ok.clone()]),
        ])]),
    )
    .unwrap();
    assert_eq!(
        compiled.run(&atoms, &[Term::Integer(1), Term::Integer(2)]),
        Some(ok)
    );
}

#[test]
fn gives_the_key_and_the_match_nothing_test_of_a_match_specification() {
    let atoms = test_atom_table();
    let at = |atoms: &AtomTable, name| atom(atoms, name);
    let head = Term::tuple(vec![at(&atoms, "$1"), at(&atoms, "$2")]);
    let ok = Term::static_atom("true");
    let key_k = atom(&atoms, "k");
    // A single clause gives its key and its match-nothing test, and a
    // specification with two clauses gives neither.
    let one = Term::list_from(vec![Term::tuple(vec![
        Term::tuple(vec![key_k.clone(), Term::Integer(1)]),
        Term::nil(),
        Term::list_from([ok.clone()]),
    ])]);
    let two = Term::list_from(vec![
        Term::tuple(vec![
            Term::tuple(vec![key_k.clone(), Term::Integer(1)]),
            Term::nil(),
            Term::list_from([ok.clone()]),
        ]),
        Term::tuple(vec![
            Term::static_atom("_"),
            Term::nil(),
            Term::list_from([ok.clone()]),
        ]),
    ]);
    assert_eq!(
        MatchSpec::compile(&atoms, &one).unwrap().bound_key(1),
        Some(&key_k)
    );
    assert!(!MatchSpec::compile(&atoms, &one).unwrap().matches_nothing(1));
    assert_eq!(MatchSpec::compile(&atoms, &two).unwrap().bound_key(1), None);
    assert!(!MatchSpec::compile(&atoms, &two).unwrap().matches_nothing(1));
    let nope = atom(&atoms, "nope");
    assert!(
        MatchSpec::compile(
            &atoms,
            &Term::list_from(vec![Term::tuple(vec![
                nope,
                Term::nil(),
                Term::list_from([ok.clone()]),
            ])])
        )
        .unwrap()
        .matches_nothing(1)
    );
    // A guard that is not `true` fails the clause, and a guard BIF that
    // fails also fails it.
    for guard in [
        Term::Integer(1),
        call(
            &atoms,
            "element",
            vec![Term::Integer(3), Term::static_atom("$_")],
        ),
    ] {
        let spec = Term::list_from(vec![Term::tuple(vec![
            head.clone(),
            Term::list_from([guard]),
            Term::list_from([ok.clone()]),
        ])]);
        assert_eq!(
            MatchSpec::compile(&atoms, &spec)
                .unwrap()
                .run(&atoms, &[Term::Integer(1), Term::Integer(2)]),
            None
        );
    }
}

#[test]
#[expect(
    clippy::too_many_lines,
    reason = "case table: one `fails` call for each guard and expression that fails"
)]
fn fails_each_guard_and_expression_like_the_match_program() {
    let atoms = test_atom_table();
    let var1 = atom(&atoms, "$1");
    let var2 = atom(&atoms, "$2");
    let head = Term::tuple(vec![var1.clone(), var2]);
    let ok = Term::static_atom("true");
    let key_a = atom(&atoms, "a");
    let object = |value: Term| vec![value, Term::Integer(2)];
    let fails = |atoms: &AtomTable, guard: Term, value: Term| {
        assert_eq!(
            run(
                atoms,
                head.clone(),
                vec![guard.clone()],
                vec![ok.clone()],
                &[value, Term::Integer(2)],
            ),
            None,
            "{guard:?}"
        );
    };
    let map_value =
        Term::Map(Map::from_pairs(vec![(key_a.clone(), Term::Integer(1))], &atoms).unwrap());
    let size_map = call(&atoms, "size", vec![var1.clone()]);
    fails(&atoms, size_map, map_value);
    let map_key = call(&atoms, "is_map_key", vec![key_a.clone(), var1.clone()]);
    fails(&atoms, map_key, key_a.clone());
    for guard in [
        call(&atoms, "is_function", vec![var1.clone(), Term::Integer(1)]),
        call(&atoms, "element", vec![Term::Integer(2), var1.clone()]),
        call(&atoms, "map_size", vec![var1.clone()]),
        call(&atoms, "map_get", vec![key_a.clone(), var1.clone()]),
        call(&atoms, "size", vec![var1.clone()]),
        call(&atoms, "bit_size", vec![var1.clone()]),
        call(&atoms, "byte_size", vec![var1.clone()]),
        call(&atoms, "tuple_size", vec![var1.clone()]),
        call(&atoms, "node", vec![var1]),
        call(&atoms, "not", vec![Term::Integer(1)]),
        call(
            &atoms,
            "and",
            vec![Term::static_atom("false"), Term::static_atom("false")],
        ),
        call(
            &atoms,
            "xor",
            vec![Term::Integer(1), Term::static_atom("true")],
        ),
        call(&atoms, "div", vec![Term::Integer(1), Term::Integer(0)]),
        call(&atoms, "rem", vec![Term::Integer(1), Term::Integer(0)]),
        call(&atoms, "bsl", vec![Term::Integer(1), Term::Integer(-1)]),
        call(&atoms, "+", vec![key_a.clone()]),
        call(&atoms, "-", vec![key_a.clone()]),
        call(&atoms, "-", vec![Term::Float(1.5)]),
        call(&atoms, "abs", vec![key_a.clone()]),
        call(&atoms, "abs", vec![Term::Float(-1.5)]),
        call(
            &atoms,
            "*",
            vec![Term::Float(1.0e308), Term::Float(1.0e308)],
        ),
        call(&atoms, "+", vec![Term::Float(1.5), Term::Float(2.5)]),
        call(&atoms, "-", vec![Term::Float(1.5), Term::Float(2.5)]),
        call(&atoms, "+", vec![Term::Float(1.0), key_a.clone()]),
    ] {
        fails(&atoms, guard, key_a.clone());
    }
    let big = Term::from_bigint(BigInt::from(1_i32) << 100_usize);
    let equal = |atoms: &AtomTable, guard: Term, expected: Term| {
        Term::tuple(vec![
            atom(atoms, "=:="),
            guard,
            Term::tuple(vec![atom(atoms, "const"), expected]),
        ])
    };
    // A bignum operand runs the bignum branches of the numeric helpers.
    let greater = call(&atoms, ">", vec![big.clone(), Term::Integer(0)]);
    let guard = equal(&atoms, greater, Term::static_atom("true"));
    assert_eq!(
        run(
            &atoms,
            head.clone(),
            vec![guard],
            vec![ok.clone()],
            &object(Term::Integer(1))
        ),
        Some(ok.clone())
    );
    let negation = call(&atoms, "-", vec![big.clone()]);
    let guard = Term::tuple(vec![
        atom(&atoms, "=:="),
        negation,
        Term::tuple(vec![
            atom(&atoms, "const"),
            Term::from_bigint(-(BigInt::from(1_i32) << 100_usize)),
        ]),
    ]);
    assert_eq!(
        run(
            &atoms,
            head.clone(),
            vec![guard],
            vec![ok.clone()],
            &object(Term::Integer(1))
        ),
        Some(ok.clone())
    );
    let absolute = call(
        &atoms,
        "abs",
        vec![Term::from_bigint(-(BigInt::from(1_i32) << 100_usize))],
    );
    let guard = Term::tuple(vec![
        atom(&atoms, "=:="),
        absolute,
        Term::tuple(vec![atom(&atoms, "const"), big.clone()]),
    ]);
    assert_eq!(
        run(
            &atoms,
            head.clone(),
            vec![guard],
            vec![ok.clone()],
            &object(Term::Integer(1))
        ),
        Some(ok.clone())
    );
    let guard = Term::tuple(vec![
        atom(&atoms, "=:="),
        call(&atoms, "+", vec![big, Term::Float(1.0)]),
        Term::tuple(vec![
            atom(&atoms, "const"),
            Term::Float(1.267_650_600_228_229_4e30),
        ]),
    ]);
    assert_eq!(
        run(
            &atoms,
            head.clone(),
            vec![guard],
            vec![ok.clone()],
            &object(Term::Integer(1))
        ),
        Some(ok.clone())
    );
}

#[test]
#[expect(
    clippy::too_many_lines,
    reason = "case table: one entry for each expression that shifts a list or a number"
)]
fn shifts_lists_and_boolean_forms_like_the_match_program() {
    let atoms = test_atom_table();
    let head = Term::static_atom("_");
    let run_expression = |atoms: &AtomTable, expression: Term| {
        let spec = Term::list_from([Term::tuple(vec![
            head.clone(),
            Term::nil(),
            Term::list_from([expression]),
        ])]);
        MatchSpec::compile(atoms, &spec)
            .unwrap()
            .run(atoms, &[Term::Integer(1)])
    };
    let big = |power: u32| Term::from_bigint(BigInt::from(10).pow(power));
    let cases = [
        (
            call(&atoms, "bsl", vec![Term::Integer(8), Term::Integer(-1)]),
            Term::Integer(4),
        ),
        (
            call(&atoms, "bsr", vec![Term::Integer(8), Term::Integer(-1)]),
            Term::Integer(16),
        ),
        (
            call(&atoms, "bsl", vec![Term::Integer(1), Term::Integer(0)]),
            Term::Integer(1),
        ),
        (
            call(
                &atoms,
                "bsl",
                vec![Term::Integer(0), Term::Integer(1_000_000)],
            ),
            Term::Integer(0),
        ),
        (
            call(&atoms, "bsr", vec![Term::Integer(1), big(20)]),
            Term::Integer(0),
        ),
        (
            call(&atoms, "bsr", vec![Term::Integer(-1), big(20)]),
            Term::Integer(-1),
        ),
        (
            call(&atoms, "bsl", vec![Term::Integer(0), big(20)]),
            Term::Integer(0),
        ),
        (
            call(&atoms, "bsl", vec![Term::Integer(1), big(20)]),
            Term::static_atom("EXIT"),
        ),
        (
            call(
                &atoms,
                "bsl",
                vec![
                    Term::Integer(1),
                    Term::from_bigint(-(BigInt::from(10_i32).pow(20))),
                ],
            ),
            Term::Integer(0),
        ),
        (
            call(
                &atoms,
                "bsr",
                vec![
                    Term::Integer(1),
                    Term::from_bigint(-(BigInt::from(10_i32).pow(20))),
                ],
            ),
            Term::static_atom("EXIT"),
        ),
        (
            call(
                &atoms,
                "bsl",
                vec![Term::Integer(1), Term::Integer(4_194_239)],
            ),
            Term::from_bigint(BigInt::from(1_i32) << 4_194_239_usize),
        ),
        (
            call(
                &atoms,
                "bsl",
                vec![Term::Integer(1), Term::Integer(4_194_240)],
            ),
            Term::static_atom("EXIT"),
        ),
        (
            call(
                &atoms,
                "bsl",
                vec![Term::Integer(1), Term::Integer(i64::MAX)],
            ),
            Term::static_atom("EXIT"),
        ),
        (
            call(
                &atoms,
                "bsl",
                vec![Term::Integer(1), Term::Integer(i64::MIN)],
            ),
            Term::Integer(0),
        ),
    ];
    for (expression, expected) in cases {
        assert_eq!(
            run_expression(&atoms, expression.clone()),
            Some(expected),
            "{expression:?}"
        );
    }
    // A failing list tail keeps the list and gives EXIT in the tail.
    let tail = call(
        &atoms,
        "element",
        vec![Term::Integer(5), Term::static_atom("$_")],
    );
    let list = Term::list_with_tail(vec![Term::Integer(1)], tail);
    assert_eq!(
        run_expression(&atoms, list),
        Some(Term::list_with_tail(
            vec![Term::Integer(1)],
            Term::static_atom("EXIT")
        ))
    );
}

#[test]
fn takes_the_boolean_forms_like_the_match_program() {
    let atoms = test_atom_table();
    let head = Term::static_atom("_");
    let ok = Term::static_atom("true");
    let run_expression = |atoms: &AtomTable, expression: Term| {
        let spec = Term::list_from([Term::tuple(vec![
            head.clone(),
            Term::nil(),
            Term::list_from([expression]),
        ])]);
        MatchSpec::compile(atoms, &spec)
            .unwrap()
            .run(atoms, &[Term::Integer(1)])
    };
    // The boolean forms take two or more operands.
    for (form, operands, expected) in [
        ("and", vec![ok.clone(), ok.clone(), ok.clone()], ok.clone()),
        (
            "or",
            vec![
                Term::static_atom("false"),
                Term::static_atom("false"),
                Term::static_atom("false"),
            ],
            Term::static_atom("false"),
        ),
        (
            "andalso",
            vec![ok.clone(), ok.clone(), ok.clone()],
            ok.clone(),
        ),
        (
            "andalso",
            vec![Term::static_atom("false"), ok.clone(), ok.clone()],
            Term::static_atom("false"),
        ),
        (
            "orelse",
            vec![
                Term::static_atom("false"),
                Term::static_atom("false"),
                ok.clone(),
            ],
            ok.clone(),
        ),
        (
            "orelse",
            vec![
                ok.clone(),
                Term::static_atom("false"),
                Term::static_atom("false"),
            ],
            ok,
        ),
    ] {
        let mut elements = vec![atom(&atoms, form)];
        elements.extend(operands);
        assert_eq!(
            run_expression(&atoms, Term::tuple(elements)),
            Some(expected),
            "{form}"
        );
    }
}

#[test]
fn takes_one_operand_of_the_boolean_forms_like_the_match_program() {
    let atoms = test_atom_table();
    let head = Term::static_atom("_");
    let ok = Term::static_atom("true");
    let run_expression = |atoms: &AtomTable, expression: Term| {
        let spec = Term::list_from([Term::tuple(vec![
            head.clone(),
            Term::nil(),
            Term::list_from([expression]),
        ])]);
        MatchSpec::compile(atoms, &spec)
            .unwrap()
            .run(atoms, &[Term::Integer(1)])
    };
    // One operand is allowed. `and` and `or` check the boolean; `andalso`
    // and `orelse` give the operand (`erl_db_util.c:4701-4855`).
    for (form, operand, expected) in [
        ("and", ok.clone(), ok.clone()),
        ("or", Term::static_atom("false"), Term::static_atom("false")),
        ("andalso", Term::Integer(1), Term::Integer(1)),
        ("orelse", Term::Integer(1), Term::Integer(1)),
        ("and", Term::Integer(1), Term::static_atom("EXIT")),
        ("or", Term::Integer(1), Term::static_atom("EXIT")),
    ] {
        let one = Term::tuple(vec![atom(&atoms, form), operand]);
        assert_eq!(run_expression(&atoms, one), Some(expected), "{form}");
    }
    // A failing nested guard propagates; it does not become `EXIT`.
    let guard = Term::tuple(vec![
        atom(&atoms, "=="),
        call(&atoms, "hd", vec![Term::Integer(1)]),
        Term::static_atom("EXIT"),
    ]);
    let spec = Term::list_from([Term::tuple(vec![
        head.clone(),
        Term::list_from([guard]),
        Term::list_from([ok.clone()]),
    ])]);
    assert_eq!(
        MatchSpec::compile(&atoms, &spec)
            .unwrap()
            .run(&atoms, &[Term::Integer(1)]),
        None
    );
    // `min` and `max` keep the first operand on a tie.
    let tie = Term::tuple(vec![
        atom(&atoms, "=:="),
        call(&atoms, "min", vec![Term::Integer(1), Term::Float(1.0)]),
        Term::tuple(vec![atom(&atoms, "const"), Term::Integer(1)]),
    ]);
    assert_eq!(run_expression(&atoms, tie), Some(ok.clone()));
    for (name, expected) in [("min", 1), ("max", 2)] {
        let less = Term::tuple(vec![
            atom(&atoms, "=:="),
            call(&atoms, name, vec![Term::Integer(1), Term::Integer(2)]),
            Term::tuple(vec![atom(&atoms, "const"), Term::Integer(expected)]),
        ]);
        assert_eq!(run_expression(&atoms, less), Some(ok.clone()), "{name}");
    }
}

#[test]
fn tests_and_sizes_a_bitstring_with_tail_bits_like_the_match_program() {
    // The pinned ERTS gives `[1,3]` for `byte_size`, `[0,2]` for `size`, and
    // `[<<1,2>>,<<>>]` for the `is_binary` guard on `<<1:3>>`,
    // `<<1,2,3:2>>`, `<<1,2>>`, and `<<>>` (`erl_db_util.c:641-642`,
    // `731-732`, `761-762`). A fixture cannot build a bitstring with tail
    // bits yet, so the test builds it in Rust.
    let atoms = test_atom_table();
    let var1 = atom(&atoms, "$1");
    let head = Term::tuple(vec![var1.clone()]);
    let three_bits = Term::Binary(Binary::from(&[0x20_u8]).bits(0, 3));
    let eighteen_bits = Term::Binary(Binary::from(&[1_u8, 2, 0xC0]).bits(0, 18));
    let whole = Term::binary(vec![1, 2]);
    let empty = Term::binary(vec![]);
    for (name, value, expected) in [
        ("byte_size", three_bits.clone(), 1),
        ("byte_size", eighteen_bits.clone(), 3),
        ("size", three_bits.clone(), 0),
        ("size", eighteen_bits.clone(), 2),
    ] {
        let body = call(&atoms, name, vec![var1.clone()]);
        assert_eq!(
            run(&atoms, head.clone(), vec![], vec![body], &[value]),
            Some(Term::Integer(expected)),
            "{name}"
        );
    }
    let guard = call(&atoms, "is_binary", vec![var1.clone()]);
    for (value, passes) in [
        (three_bits, false),
        (eighteen_bits, false),
        (whole, true),
        (empty, true),
    ] {
        assert_eq!(
            run(
                &atoms,
                head.clone(),
                vec![guard.clone()],
                vec![var1.clone()],
                std::slice::from_ref(&value)
            ),
            passes.then_some(value)
        );
    }
}

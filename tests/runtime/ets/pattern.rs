use super::{MatchSpec, Pattern, SpecError, variable};
use crate::term::{AtomTable, Fun, Map, PortId, ProcessId, Reference, Term, test_atom_table};
use num_bigint::BigInt;

fn atom(atoms: &mut AtomTable, name: &str) -> Term {
    Term::Atom(atoms.put(name).unwrap())
}

#[test]
fn reads_the_variable_numbers_as_db_is_variable() {
    let mut atoms = test_atom_table();
    let mut number = |name| {
        let term = atom(&mut atoms, name);
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
    let mut atoms = test_atom_table();
    let one = Term::Integer(1);
    let variable = atom(&mut atoms, "$1");
    let list = Term::list_with_tail(vec![variable.clone(), one.clone()], atom(&mut atoms, "_"));
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
    let twice = Term::tuple(vec![variable.clone(), variable.clone()]);
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
    object: Vec<Term>,
) -> Option<Term> {
    let spec = Term::list_from(vec![Term::tuple(vec![
        head,
        Term::list_from(guards),
        Term::list_from(body),
    ])]);
    MatchSpec::compile(atoms, &spec)
        .unwrap()
        .run(atoms, &object)
}

/// Return the guard call `{Name, Arguments...}`.
fn call(atoms: &mut AtomTable, name: &str, arguments: Vec<Term>) -> Term {
    let mut elements = vec![atom(atoms, name)];
    elements.extend(arguments);
    Term::tuple(elements)
}

#[test]
fn runs_each_guard_function_of_the_match_specification() {
    let mut atoms = test_atom_table();
    let var1 = atom(&mut atoms, "$1");
    let var2 = atom(&mut atoms, "$2");
    let head = Term::tuple(vec![var1.clone(), var2.clone()]);
    let ok = Term::static_atom("true");
    let map = Term::Map(
        Map::from_pairs(vec![(atom(&mut atoms, "a"), Term::Integer(1))], &atoms).unwrap(),
    );
    let fun = Term::Fun(Fun::external(
        atoms.put("m").unwrap(),
        atoms.put("f").unwrap(),
        1,
    ));
    let binary = Term::binary(vec![1, 2]);
    let key_a = atom(&mut atoms, "a");
    let node = atom(&mut atoms, "nonode@nohost");
    let equal = |atoms: &mut AtomTable, guard: Term, expected: Term| {
        Term::tuple(vec![
            atom(atoms, "=:="),
            guard,
            Term::tuple(vec![atom(atoms, "const"), expected]),
        ])
    };
    let bool_cases: Vec<(Term, Term)> = vec![
        (
            call(&mut atoms, "is_atom", vec![var1.clone()]),
            atom(&mut atoms, "a"),
        ),
        (
            call(&mut atoms, "is_float", vec![var1.clone()]),
            Term::Float(1.0),
        ),
        (
            call(&mut atoms, "is_integer", vec![var1.clone()]),
            Term::Integer(1),
        ),
        (
            call(&mut atoms, "is_list", vec![var1.clone()]),
            Term::list_from([Term::Integer(1)]),
        ),
        (
            call(&mut atoms, "is_number", vec![var1.clone()]),
            Term::Integer(1),
        ),
        (
            call(&mut atoms, "is_pid", vec![var1.clone()]),
            Term::Pid(ProcessId(0)),
        ),
        (
            call(&mut atoms, "is_port", vec![var1.clone()]),
            Term::Port(PortId(0)),
        ),
        (
            call(&mut atoms, "is_reference", vec![var1.clone()]),
            Term::Reference(Reference(0)),
        ),
        (
            call(&mut atoms, "is_tuple", vec![var1.clone()]),
            Term::tuple(vec![Term::Integer(1)]),
        ),
        (call(&mut atoms, "is_map", vec![var1.clone()]), map.clone()),
        (
            call(&mut atoms, "is_binary", vec![var1.clone()]),
            binary.clone(),
        ),
        (
            call(&mut atoms, "is_bitstring", vec![var1.clone()]),
            binary.clone(),
        ),
        (
            call(&mut atoms, "is_boolean", vec![var1.clone()]),
            ok.clone(),
        ),
        (
            call(&mut atoms, "is_function", vec![var1.clone()]),
            fun.clone(),
        ),
        (
            call(
                &mut atoms,
                "is_function",
                vec![var1.clone(), Term::Integer(1)],
            ),
            fun.clone(),
        ),
        (
            call(&mut atoms, "is_map_key", vec![key_a.clone(), var1.clone()]),
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
                object
            ),
            Some(ok.clone()),
            "{guard:?} {value:?}"
        );
    }
    let value_cases: Vec<(Term, Term, Term)> = vec![
        (
            call(&mut atoms, "abs", vec![var1.clone()]),
            Term::Integer(-1),
            Term::Integer(1),
        ),
        (
            call(&mut atoms, "element", vec![Term::Integer(2), var1.clone()]),
            Term::tuple(vec![Term::Integer(1), Term::Integer(2)]),
            Term::Integer(2),
        ),
        (
            call(&mut atoms, "hd", vec![var1.clone()]),
            Term::list_from([Term::Integer(1), Term::Integer(2)]),
            Term::Integer(1),
        ),
        (
            call(&mut atoms, "tl", vec![var1.clone()]),
            Term::list_from([Term::Integer(1), Term::Integer(2)]),
            Term::list_from([Term::Integer(2)]),
        ),
        (
            call(&mut atoms, "length", vec![var1.clone()]),
            Term::list_from([Term::Integer(1), Term::Integer(2)]),
            Term::Integer(2),
        ),
        (
            call(&mut atoms, "map_size", vec![var1.clone()]),
            map.clone(),
            Term::Integer(1),
        ),
        (
            call(&mut atoms, "map_get", vec![key_a.clone(), var1.clone()]),
            map.clone(),
            Term::Integer(1),
        ),
        (
            call(&mut atoms, "size", vec![var1.clone()]),
            Term::tuple(vec![Term::Integer(1)]),
            Term::Integer(1),
        ),
        (
            call(&mut atoms, "size", vec![var1.clone()]),
            binary.clone(),
            Term::Integer(2),
        ),
        (
            call(&mut atoms, "bit_size", vec![var1.clone()]),
            binary.clone(),
            Term::Integer(16),
        ),
        (
            call(&mut atoms, "byte_size", vec![var1.clone()]),
            binary.clone(),
            Term::Integer(2),
        ),
        (
            call(&mut atoms, "tuple_size", vec![var1.clone()]),
            Term::tuple(vec![Term::Integer(1)]),
            Term::Integer(1),
        ),
        (
            call(&mut atoms, "node", vec![]),
            Term::Integer(0),
            node.clone(),
        ),
        (
            call(&mut atoms, "node", vec![var1.clone()]),
            Term::Pid(ProcessId(0)),
            node.clone(),
        ),
        (
            call(&mut atoms, "+", vec![var1.clone()]),
            Term::Integer(1),
            Term::Integer(1),
        ),
        (
            call(&mut atoms, "-", vec![var1.clone()]),
            Term::Integer(1),
            Term::Integer(-1),
        ),
        (
            call(&mut atoms, "+", vec![var1.clone(), var2.clone()]),
            Term::Integer(1),
            Term::Integer(3),
        ),
        (
            call(&mut atoms, "-", vec![var1.clone(), var2.clone()]),
            Term::Integer(1),
            Term::Integer(-1),
        ),
        (
            call(&mut atoms, "*", vec![var1.clone(), var2.clone()]),
            Term::Integer(1),
            Term::Integer(2),
        ),
        (
            call(&mut atoms, "/", vec![var1.clone(), var2.clone()]),
            Term::Integer(1),
            Term::Float(0.5),
        ),
        (
            call(&mut atoms, "div", vec![var1.clone(), var2.clone()]),
            Term::Integer(1),
            Term::Integer(0),
        ),
        (
            call(&mut atoms, "rem", vec![var1.clone(), var2.clone()]),
            Term::Integer(1),
            Term::Integer(1),
        ),
        (
            call(&mut atoms, "band", vec![var1.clone(), var2.clone()]),
            Term::Integer(1),
            Term::Integer(0),
        ),
        (
            call(&mut atoms, "bor", vec![var1.clone(), var2.clone()]),
            Term::Integer(1),
            Term::Integer(3),
        ),
        (
            call(&mut atoms, "bxor", vec![var1.clone(), var2.clone()]),
            Term::Integer(1),
            Term::Integer(3),
        ),
        (
            call(&mut atoms, "bnot", vec![var1.clone()]),
            Term::Integer(1),
            Term::Integer(-2),
        ),
        (
            call(&mut atoms, "bsl", vec![var1.clone(), var2.clone()]),
            Term::Integer(1),
            Term::Integer(4),
        ),
        (
            call(&mut atoms, "bsr", vec![var1.clone(), var2.clone()]),
            Term::Integer(4),
            Term::Integer(1),
        ),
    ];
    for (guard, value, expected) in value_cases {
        let shown = guard.clone();
        let guard = equal(&mut atoms, guard, expected.clone());
        let object = vec![value.clone(), Term::Integer(2)];
        assert_eq!(
            run(&atoms, head.clone(), vec![guard], vec![ok.clone()], object),
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
        let guard = call(&mut atoms, name, vec![var1.clone(), var2.clone()]);
        assert_eq!(
            run(
                &atoms,
                head.clone(),
                vec![guard],
                vec![ok.clone()],
                vec![value, Term::Integer(1)],
            ),
            Some(ok.clone())
        );
    }
    for (name, expected) in [("min", Term::Integer(1)), ("max", Term::Integer(2))] {
        let call = call(&mut atoms, name, vec![var1.clone(), var2.clone()]);
        let guard = equal(&mut atoms, call, expected);
        assert_eq!(
            run(
                &atoms,
                head.clone(),
                vec![guard],
                vec![ok.clone()],
                vec![Term::Integer(2), Term::Integer(1)]
            ),
            Some(ok.clone())
        );
    }
    let guard = call(&mut atoms, "/=", vec![var1.clone(), var2.clone()]);
    assert_eq!(
        run(
            &atoms,
            head.clone(),
            vec![guard],
            vec![ok.clone()],
            vec![Term::Integer(2), Term::Integer(1)]
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
        let guard = call(&mut atoms, guard, arguments);
        assert_eq!(
            run(
                &atoms,
                head.clone(),
                vec![guard],
                vec![ok.clone()],
                vec![first, second]
            ),
            Some(ok.clone())
        );
    }
    let andalso = Term::tuple(vec![
        atom(&mut atoms, "andalso"),
        call(&mut atoms, "is_integer", vec![var1.clone()]),
        Term::static_atom("true"),
    ]);
    let orelse = Term::tuple(vec![
        atom(&mut atoms, "orelse"),
        call(&mut atoms, "is_atom", vec![var1.clone()]),
        Term::static_atom("true"),
    ]);
    assert_eq!(
        run(
            &atoms,
            head.clone(),
            vec![andalso, orelse],
            vec![ok.clone()],
            vec![Term::Integer(1), Term::Integer(2)]
        ),
        Some(ok.clone())
    );
    // Short-circuit values and a non-boolean operand fail the clause.
    let andalso = Term::tuple(vec![
        atom(&mut atoms, "andalso"),
        Term::static_atom("false"),
        Term::static_atom("true"),
    ]);
    let orelse = Term::tuple(vec![
        atom(&mut atoms, "orelse"),
        Term::static_atom("true"),
        Term::static_atom("false"),
    ]);
    for (guard, expected) in [
        (andalso, Term::static_atom("false")),
        (orelse, Term::static_atom("true")),
    ] {
        let guard = equal(&mut atoms, guard, expected.clone());
        assert_eq!(
            run(
                &atoms,
                head.clone(),
                vec![guard],
                vec![ok.clone()],
                vec![Term::Integer(1), Term::Integer(2)]
            ),
            Some(ok.clone()),
            "{expected:?}"
        );
    }
    for guard in [
        Term::tuple(vec![
            atom(&mut atoms, "andalso"),
            Term::Integer(1),
            Term::static_atom("true"),
        ]),
        Term::tuple(vec![
            atom(&mut atoms, "orelse"),
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
                vec![Term::Integer(1), Term::Integer(2)]
            ),
            None
        );
    }
}

#[test]
fn builds_match_specification_terms_and_rejects_bad_ones() {
    let mut atoms = test_atom_table();
    let at = |atoms: &mut AtomTable, name| atom(atoms, name);
    let head = Term::tuple(vec![at(&mut atoms, "$1"), at(&mut atoms, "$2")]);
    let ok = Term::static_atom("true");
    let key_k = atom(&mut atoms, "k");
    let spec = Term::list_from(vec![Term::tuple(vec![
        head.clone(),
        Term::nil(),
        Term::list_from(vec![
            Term::static_atom("$_"),
            Term::static_atom("$$"),
            Term::tuple(vec![Term::tuple(vec![
                at(&mut atoms, "$1"),
                Term::Integer(1),
            ])]),
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
            Term::list_from([at(&mut atoms, "false")]),
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
        &mut atoms,
        "element",
        vec![Term::Integer(3), Term::static_atom("$_")],
    )]);
    let spec = Term::list_from(vec![Term::tuple(vec![head.clone(), Term::nil(), body])]);
    assert_eq!(
        MatchSpec::compile(&atoms, &spec)
            .unwrap()
            .run(&atoms, &[Term::Integer(1), Term::Integer(2)]),
        Some(Term::static_atom("EXIT"))
    );
    // A large arithmetic result becomes a bignum, and division by zero fails.
    let big = Term::Integer(1 << 40);
    let body = Term::list_from(vec![call(&mut atoms, "*", vec![big.clone(), big.clone()])]);
    let spec = Term::list_from(vec![Term::tuple(vec![head.clone(), Term::nil(), body])]);
    assert!(matches!(
        MatchSpec::compile(&atoms, &spec)
            .unwrap()
            .run(&atoms, &[Term::Integer(1), Term::Integer(2)]),
        Some(Term::BigInt(_))
    ));
    let body = Term::list_from(vec![call(
        &mut atoms,
        "/",
        vec![Term::Float(1.0), Term::Float(0.0)],
    )]);
    let spec = Term::list_from(vec![Term::tuple(vec![head.clone(), Term::nil(), body])]);
    assert_eq!(
        MatchSpec::compile(&atoms, &spec)
            .unwrap()
            .run(&atoms, &[Term::Integer(1), Term::Integer(2)]),
        Some(Term::static_atom("EXIT"))
    );
    // An unsupported guard, a map expression, and a bad specification.
    let unsupported = Term::list_from(vec![Term::tuple(vec![
        head.clone(),
        Term::list_from([call(&mut atoms, "round", vec![Term::Float(1.0)])]),
        Term::nil(),
    ])]);
    assert!(matches!(
        MatchSpec::compile(&atoms, &unsupported),
        Err(SpecError::Unsupported(_))
    ));
    let map =
        Term::Map(Map::from_pairs(vec![(at(&mut atoms, "a"), Term::Integer(1))], &atoms).unwrap());
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
            Term::list_from([at(&mut atoms, "$3")]),
        ])]),
        Term::list_from(vec![Term::tuple(vec![
            head.clone(),
            Term::nil(),
            Term::list_from([call(&mut atoms, "bogus", vec![])]),
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
            Term::list_from([Term::tuple(vec![
                atom(&mut atoms, "andalso"),
                Term::static_atom("true"),
            ])]),
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
            head.clone(),
            Term::nil(),
            Term::list_from([ok.clone()]),
        ])]),
    )
    .unwrap();
    assert_eq!(
        compiled.run(&atoms, &[Term::Integer(1), Term::Integer(2)]),
        Some(ok.clone())
    );
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
        Some(&key_k.clone())
    );
    assert!(!MatchSpec::compile(&atoms, &one).unwrap().matches_nothing(1));
    assert_eq!(MatchSpec::compile(&atoms, &two).unwrap().bound_key(1), None);
    assert!(!MatchSpec::compile(&atoms, &two).unwrap().matches_nothing(1));
    let nope = atom(&mut atoms, "nope");
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
            &mut atoms,
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
fn fails_each_guard_and_expression_like_the_match_program() {
    let mut atoms = test_atom_table();
    let var1 = atom(&mut atoms, "$1");
    let var2 = atom(&mut atoms, "$2");
    let head = Term::tuple(vec![var1.clone(), var2.clone()]);
    let ok = Term::static_atom("true");
    let key_a = atom(&mut atoms, "a");
    let object = |value: Term| vec![value, Term::Integer(2)];
    let fails = |atoms: &AtomTable, guard: Term, value: Term| {
        assert_eq!(
            run(
                atoms,
                head.clone(),
                vec![guard.clone()],
                vec![ok.clone()],
                vec![value, Term::Integer(2)],
            ),
            None,
            "{guard:?}"
        );
    };
    let map_value =
        Term::Map(Map::from_pairs(vec![(key_a.clone(), Term::Integer(1))], &atoms).unwrap());
    let size_map = call(&mut atoms, "size", vec![var1.clone()]);
    fails(&atoms, size_map, map_value);
    let map_key = call(&mut atoms, "is_map_key", vec![key_a.clone(), var1.clone()]);
    fails(&atoms, map_key, key_a.clone());
    for guard in [
        call(
            &mut atoms,
            "is_function",
            vec![var1.clone(), Term::Integer(1)],
        ),
        call(&mut atoms, "element", vec![Term::Integer(2), var1.clone()]),
        call(&mut atoms, "map_size", vec![var1.clone()]),
        call(&mut atoms, "map_get", vec![key_a.clone(), var1.clone()]),
        call(&mut atoms, "size", vec![var1.clone()]),
        call(&mut atoms, "bit_size", vec![var1.clone()]),
        call(&mut atoms, "byte_size", vec![var1.clone()]),
        call(&mut atoms, "tuple_size", vec![var1.clone()]),
        call(&mut atoms, "node", vec![var1.clone()]),
        call(&mut atoms, "not", vec![Term::Integer(1)]),
        call(
            &mut atoms,
            "and",
            vec![Term::static_atom("false"), Term::static_atom("false")],
        ),
        call(
            &mut atoms,
            "xor",
            vec![Term::Integer(1), Term::static_atom("true")],
        ),
        call(&mut atoms, "div", vec![Term::Integer(1), Term::Integer(0)]),
        call(&mut atoms, "rem", vec![Term::Integer(1), Term::Integer(0)]),
        call(&mut atoms, "bsl", vec![Term::Integer(1), Term::Integer(-1)]),
        call(&mut atoms, "+", vec![key_a.clone()]),
        call(&mut atoms, "-", vec![key_a.clone()]),
        call(&mut atoms, "-", vec![Term::Float(1.5)]),
        call(&mut atoms, "abs", vec![key_a.clone()]),
        call(&mut atoms, "abs", vec![Term::Float(-1.5)]),
        call(
            &mut atoms,
            "*",
            vec![Term::Float(1.0e308), Term::Float(1.0e308)],
        ),
        call(&mut atoms, "+", vec![Term::Float(1.5), Term::Float(2.5)]),
        call(&mut atoms, "-", vec![Term::Float(1.5), Term::Float(2.5)]),
        call(&mut atoms, "+", vec![Term::Float(1.0), key_a.clone()]),
    ] {
        fails(&atoms, guard, key_a.clone());
    }
    let big = Term::from_bigint(BigInt::from(1_i32) << 100_usize);
    let equal = |atoms: &mut AtomTable, guard: Term, expected: Term| {
        Term::tuple(vec![
            atom(atoms, "=:="),
            guard,
            Term::tuple(vec![atom(atoms, "const"), expected]),
        ])
    };
    // A bignum operand runs the bignum branches of the numeric helpers.
    let greater = call(&mut atoms, ">", vec![big.clone(), Term::Integer(0)]);
    let guard = equal(&mut atoms, greater, Term::static_atom("true"));
    assert_eq!(
        run(
            &atoms,
            head.clone(),
            vec![guard],
            vec![ok.clone()],
            object(Term::Integer(1))
        ),
        Some(ok.clone())
    );
    let negation = call(&mut atoms, "-", vec![big.clone()]);
    let guard = Term::tuple(vec![
        atom(&mut atoms, "=:="),
        negation,
        Term::tuple(vec![
            atom(&mut atoms, "const"),
            Term::from_bigint(-(BigInt::from(1_i32) << 100_usize)),
        ]),
    ]);
    assert_eq!(
        run(
            &atoms,
            head.clone(),
            vec![guard],
            vec![ok.clone()],
            object(Term::Integer(1))
        ),
        Some(ok.clone())
    );
    let absolute = call(
        &mut atoms,
        "abs",
        vec![Term::from_bigint(-(BigInt::from(1_i32) << 100_usize))],
    );
    let guard = Term::tuple(vec![
        atom(&mut atoms, "=:="),
        absolute,
        Term::tuple(vec![atom(&mut atoms, "const"), big.clone()]),
    ]);
    assert_eq!(
        run(
            &atoms,
            head.clone(),
            vec![guard],
            vec![ok.clone()],
            object(Term::Integer(1))
        ),
        Some(ok.clone())
    );
    let guard = Term::tuple(vec![
        atom(&mut atoms, "=:="),
        call(&mut atoms, "+", vec![big, Term::Float(1.0)]),
        Term::tuple(vec![
            atom(&mut atoms, "const"),
            Term::Float(1.2676506002282294e30),
        ]),
    ]);
    assert_eq!(
        run(
            &atoms,
            head.clone(),
            vec![guard],
            vec![ok.clone()],
            object(Term::Integer(1))
        ),
        Some(ok.clone())
    );
}

use std::process::Command;

use pbeam_runtime::{BeamProgram, Map, Term};

const PERSISTENT_TERM_PROBE: &[u8] =
    include_bytes!("../../fixtures/persistent_term/persistent_term_probe.beam");

#[test]
fn keeps_persistent_terms_for_all_processes_as_pinned_erts() {
    // `main/0` reads a missing key, puts a value, puts the same value again,
    // and lets a spawned process replace the value. The keys start with the
    // probe module name, so the persistent terms of the ERTS logger do not
    // change the reference result.
    let reference = Command::new("erl")
        .args([
            "-noshell",
            "-pa",
            "fixtures/persistent_term",
            "-eval",
            "io:format(\"~p~n\", [persistent_term_probe:main()]), halt().",
        ])
        .output()
        .unwrap();
    assert!(reference.status.success());
    assert!(reference.stderr.is_empty());
    assert_eq!(
        reference.stdout,
        b"{default,ok,1,ok,ok,{value,[a,<<\"b\">>]},undefined}\n"
    );

    let program = BeamProgram::load(&[PERSISTENT_TERM_PROBE]).unwrap();
    let atom = |name: &str| Term::Atom(program.atom(name).unwrap());
    assert_eq!(
        program
            .execute("persistent_term_probe", "main", &[])
            .unwrap()
            .term,
        Term::tuple(vec![
            atom("default"),
            atom("ok"),
            Term::Integer(1),
            atom("ok"),
            atom("ok"),
            Term::tuple(vec![
                atom("value"),
                Term::list_from(vec![atom("a"), Term::binary(b"b".to_vec())]),
            ]),
            atom("undefined"),
        ])
    );
}

#[test]
fn returns_all_persistent_terms_in_the_erts_table_order() {
    // `table/0` puts keys that collide in slot 7 of 8 and wrap to slot 0, that
    // collide again after the resize to 16 slots, and that grow the table to
    // 32 slots. At `-eval` time the ERTS table already holds the 25 terms of
    // the boot in 64 slots, so the reference run first resets the table to 8
    // empty slots. The probe module loads before the reset, because code
    // loading reads `persistent_term:get(code_server)`. The run puts the boot
    // terms back before `io:format/2`, because the IO path reads them too.
    let reference = Command::new("erl")
        .args([
            "-noshell",
            "-pa",
            "fixtures/persistent_term",
            "-eval",
            "{module, M} = code:ensure_loaded(persistent_term_probe), \
             Saved = persistent_term:get(), \
             true = erts_internal:erase_persistent_terms(), \
             Result = M:table(), \
             true = erts_internal:erase_persistent_terms(), \
             [persistent_term:put(K, V) || {K, V} <- Saved], \
             io:format(\"~w~n\", [Result]), halt().",
        ])
        .output()
        .unwrap();
    assert!(reference.status.success());
    assert!(reference.stderr.is_empty());
    assert_eq!(
        String::from_utf8(reference.stdout).unwrap(),
        "{[],[{true,1},{7,4},{ok,3},{39,2}],[{39,2},{1152921504606846976,8},{{ok},5},{true,1},\
         {[97,98,99],7},{7,4},{<<48,49,50,51,52,53,54,55,56,57,97,98,99,100,101,102,103>>,6},\
         {ok,3}],[{39,2},{[],9},{#{18 => 18,4 => 4,12 => 12,19 => 19,29 => 29,13 => 13,2 => 2,\
         7 => 7,31 => 31,8 => 8,10 => 10,23 => 23,9 => 9,15 => 15,32 => 32,1 => 1,25 => 25,\
         28 => 28,20 => 20,6 => 6,11 => 11,17 => 17,24 => 24,14 => 14,33 => 33,3 => 3,16 => 16,\
         30 => 30,21 => 21,5 => 5,27 => 27,26 => 26,22 => 22},13},{{ok},5},{[97,98,99],7},\
         {<<48,49,50,51,52,53,54,55,56,57,97,98,99,100,101,102,103>>,6},{#{1 => <<>>,ok => []},12},\
         {[ok,{}],10},{1152921504606846976,8},{[1|2],14},{true,1},{7,4},{-1,11},{ok,30}]}\n"
    );

    let program = BeamProgram::load(&[PERSISTENT_TERM_PROBE]).unwrap();
    let atom = |name: &str| Term::Atom(program.atom(name).unwrap());
    let pair = |key: Term, value: i64| Term::tuple(vec![key, Term::Integer(value)]);
    let ok_tuple = Term::tuple(vec![atom("ok")]);
    let binary = Term::binary(b"0123456789abcdefg".to_vec());
    let string = Term::list_from(b"abc".iter().map(|&c| Term::Integer(c.into())));
    let small_map = Term::Map(
        Map::from_pairs(
            vec![
                (atom("ok"), Term::nil()),
                (Term::Integer(1), Term::binary(vec![])),
            ],
            program.atoms(),
        )
        .unwrap(),
    );
    let large_map = Term::Map(
        Map::from_pairs(
            (1..=33)
                .map(|key| (Term::Integer(key), Term::Integer(key)))
                .collect(),
            program.atoms(),
        )
        .unwrap(),
    );
    assert_eq!(
        program
            .execute("persistent_term_probe", "table", &[])
            .unwrap()
            .term,
        Term::tuple(vec![
            Term::nil(),
            Term::list_from(vec![
                pair(atom("true"), 1),
                pair(Term::Integer(7), 4),
                pair(atom("ok"), 3),
                pair(Term::Integer(39), 2),
            ]),
            Term::list_from(vec![
                pair(Term::Integer(39), 2),
                pair(Term::Integer(1 << 60), 8),
                pair(ok_tuple.clone(), 5),
                pair(atom("true"), 1),
                pair(string.clone(), 7),
                pair(Term::Integer(7), 4),
                pair(binary.clone(), 6),
                pair(atom("ok"), 3),
            ]),
            Term::list_from(vec![
                pair(Term::Integer(39), 2),
                pair(Term::nil(), 9),
                pair(large_map, 13),
                pair(ok_tuple, 5),
                pair(string, 7),
                pair(binary, 6),
                pair(small_map, 12),
                pair(Term::list_from(vec![atom("ok"), Term::tuple(vec![])]), 10),
                pair(Term::Integer(1 << 60), 8),
                pair(
                    Term::list_with_tail(vec![Term::Integer(1)], Term::Integer(2)),
                    14,
                ),
                pair(atom("true"), 1),
                pair(Term::Integer(7), 4),
                pair(Term::Integer(-1), 11),
                pair(atom("ok"), 30),
            ]),
        ])
    );
}

#[test]
fn erases_persistent_terms_as_pinned_erts() {
    // `erase/0` erases a missing key, a stored key, the same key again, and
    // a stored tuple key. The keys start with the probe module name, so the
    // persistent terms of the ERTS logger do not change the result.
    let reference = Command::new("erl")
        .args([
            "-noshell",
            "-pa",
            "fixtures/persistent_term",
            "-eval",
            "io:format(\"~p~n\", [persistent_term_probe:erase()]), halt().",
        ])
        .output()
        .unwrap();
    assert!(reference.status.success());
    assert!(reference.stderr.is_empty());
    assert_eq!(reference.stdout, b"{false,true,default,false,true}\n");

    let program = BeamProgram::load(&[PERSISTENT_TERM_PROBE]).unwrap();
    let atom = |name: &str| Term::Atom(program.atom(name).unwrap());
    assert_eq!(
        program
            .execute("persistent_term_probe", "erase", &[])
            .unwrap()
            .term,
        Term::tuple(vec![
            atom("false"),
            atom("true"),
            atom("default"),
            atom("false"),
            atom("true"),
        ])
    );
}

#[test]
fn keeps_the_erts_table_order_after_an_erase() {
    // `erase_table/0` puts the four keys of `table/0`, erases a key whose
    // next slot holds another key, and erases a key whose next slot is
    // free. The ERTS table holds the boot terms at `-eval` time, so the
    // reference run resets the table to 8 empty slots first, as the `table`
    // test does.
    let reference = Command::new("erl")
        .args([
            "-noshell",
            "-pa",
            "fixtures/persistent_term",
            "-eval",
            "{module, M} = code:ensure_loaded(persistent_term_probe), \
             Saved = persistent_term:get(), \
             true = erts_internal:erase_persistent_terms(), \
             Result = M:erase_table(), \
             true = erts_internal:erase_persistent_terms(), \
             [persistent_term:put(K, V) || {K, V} <- Saved], \
             io:format(\"~w~n\", [Result]), halt().",
        ])
        .output()
        .unwrap();
    assert!(reference.status.success());
    assert!(reference.stderr.is_empty());
    assert_eq!(reference.stdout, b"[{39,2},{ok,3}]\n");

    let program = BeamProgram::load(&[PERSISTENT_TERM_PROBE]).unwrap();
    let atom = |name: &str| Term::Atom(program.atom(name).unwrap());
    let pair = |key: Term, value: i64| Term::tuple(vec![key, Term::Integer(value)]);
    assert_eq!(
        program
            .execute("persistent_term_probe", "erase_table", &[])
            .unwrap()
            .term,
        Term::list_from(vec![pair(Term::Integer(39), 2), pair(atom("ok"), 3),])
    );
}

use std::process::Command;

use pbeam_runtime::{BeamProgram, Map, Term};

mod common;

use common::pinned_beam;

const LOGGER_START_PROBE: &[u8] =
    include_bytes!("../fixtures/logger_start/logger_start_probe.beam");

/// Run `logger_start_probe:Function()` on the pinned ERTS and return its
/// `~w` output.
fn reference(function: &str) -> Vec<u8> {
    reference_call(&format!("{function}()"))
}

/// Run `logger_start_probe:Call` on the pinned ERTS, where `Call` is a
/// function name with its arguments, and return its `~w` output.
fn reference_call(call: &str) -> Vec<u8> {
    let reference = Command::new("erl")
        .args([
            "-noshell",
            "-pa",
            "fixtures/logger_start",
            "-eval",
            &format!("io:format(\"~w~n\", [logger_start_probe:{call}]), halt()."),
        ])
        .output()
        .unwrap();
    assert!(reference.status.success());
    assert!(reference.stderr.is_empty());
    reference.stdout
}

/// Load the probe with the pinned `erlang`, `lists`, and `maps`, so
/// `erlang:spawn_monitor/1`, `lists:reverse/1`, `lists:seq/2`, and
/// `maps:without/2` run their BEAM code.
fn load() -> BeamProgram {
    let erlang = pinned_beam("erlang");
    let lists = pinned_beam("lists");
    let maps = pinned_beam("maps");
    BeamProgram::load(&[&erlang, &lists, &maps, LOGGER_START_PROBE]).unwrap()
}

#[test]
fn reverses_lists_as_pinned_erts() {
    // `logger_start_probe:reverse/0` runs `lists:reverse/2`
    // (`erl_bif_lists.c:1246-1263`), directly and through the pinned
    // `lists:reverse/1` (`lists.erl:298-299`). An empty first list returns
    // the second term itself, and a first term that is not a list raises
    // `badarg`.
    let program = load();
    let atom = |name: &str| Term::Atom(program.atom(name).unwrap());
    let list = |names: &[&str]| Term::List(names.iter().map(|name| atom(name)).collect());
    let expected = vec![
        list(&["c", "b", "a", "d"]),
        atom("foo"),
        list(&["a"]),
        list(&[]),
        list(&["d", "c", "b", "a"]),
        atom("badarg"),
        atom("badarg"),
    ];
    assert_eq!(
        program
            .execute("logger_start_probe", "reverse", &[])
            .unwrap(),
        Term::List(expected)
    );
    assert_eq!(
        reference("reverse"),
        b"[[c,b,a,d],foo,[a],[],[d,c,b,a],badarg,badarg]\n"
    );
}

#[test]
fn removes_map_keys_as_pinned_erts() {
    // `logger_start_probe:remove/0` runs `maps:remove/2`
    // (`erl_map.c:1929-1937`). A key that the map does not hold, also a
    // tuple key, returns the map. A hashmap that keeps 32 keys becomes a
    // flatmap (`erl_map.c:2989-3025`), so the keys come in integer order. A
    // hashmap that keeps more keys has the ERTS HAMT key order. A term that
    // is not a map raises `{badmap, Map}`.
    let program = load();
    let atom = |name: &str| Term::Atom(program.atom(name).unwrap());
    let map = |pairs: Vec<(Term, Term)>| Term::Map(Map::from_pairs(pairs).unwrap());
    let integers = |keys: &[i64]| Term::List(keys.iter().copied().map(Term::Integer).collect());
    let both = map(vec![
        (Term::Integer(1), atom("one")),
        (Term::Integer(2), atom("two")),
    ]);
    let without_33 = integers(&[
        18, 4, 12, 19, 29, 13, 2, 7, 31, 8, 10, 23, 9, 15, 32, 1, 25, 28, 20, 6, 11, 17, 24, 14,
        33, 3, 16, 30, 21, 5, 27, 26, 22,
    ]);
    let expected = vec![
        map(vec![(Term::Integer(2), atom("two"))]),
        both.clone(),
        both,
        map(vec![]),
        integers(&(1..=32).collect::<Vec<_>>()),
        without_33.clone(),
        without_33,
        integers(&[
            18, 4, 34, 12, 19, 29, 13, 2, 7, 31, 8, 10, 23, 9, 15, 32, 25, 28, 20, 6, 11, 17, 24,
            14, 33, 3, 16, 30, 21, 5, 27, 26, 22,
        ]),
        integers(&[
            54, 34, 46, 52, 41, 42, 55, 38, 56, 61, 31, 44, 49, 64, 35, 36, 32, 58, 60, 43, 57, 48,
            37, 40, 33, 53, 62, 47, 63, 51, 50, 45, 59, 39,
        ]),
        Term::Tuple(vec![atom("badmap"), atom("foo")]),
        Term::Tuple(vec![atom("badmap"), Term::List(vec![])]),
    ];
    assert_eq!(
        program
            .execute("logger_start_probe", "remove", &[])
            .unwrap(),
        Term::List(expected)
    );
    assert_eq!(
        reference("remove"),
        b"[#{2 => two},#{1 => one,2 => two},#{1 => one,2 => two},#{},\
          [1,2,3,4,5,6,7,8,9,10,11,12,13,14,15,16,17,18,19,20,21,22,23,24,25,26,27,28,29,30,31,32],\
          [18,4,12,19,29,13,2,7,31,8,10,23,9,15,32,1,25,28,20,6,11,17,24,14,33,3,16,30,21,5,27,26,22],\
          [18,4,12,19,29,13,2,7,31,8,10,23,9,15,32,1,25,28,20,6,11,17,24,14,33,3,16,30,21,5,27,26,22],\
          [18,4,34,12,19,29,13,2,7,31,8,10,23,9,15,32,25,28,20,6,11,17,24,14,33,3,16,30,21,5,27,26,22],\
          [54,34,46,52,41,42,55,38,56,61,31,44,49,64,35,36,32,58,60,43,57,48,37,40,33,53,62,47,63,51,50,45,59,39],\
          {badmap,foo},{badmap,[]}]\n"
    );
}

#[test]
fn removes_map_keys_through_maps_without() {
    // `logger_start_probe:without/0` runs the pinned `maps:without/2`
    // (`maps.erl:1139-1140`), which folds `fun maps:remove/2` over the keys
    // through `call_fun2`. `logger_h_common:adding_handler/1`
    // (`logger_h_common.erl:71`) removes keys that the map does not hold,
    // and `logger_h_common:filter_config/1` (`logger_h_common.erl:187-188`)
    // removes the `olp` key that it holds.
    let program = load();
    let atom = |name: &str| Term::Atom(program.atom(name).unwrap());
    let map = |pairs: Vec<(Term, Term)>| Term::Map(Map::from_pairs(pairs).unwrap());
    let expected = vec![
        map(vec![(atom("type"), atom("standard_io"))]),
        map(vec![
            (Term::Integer(1), atom("one")),
            (Term::Integer(2), atom("two")),
        ]),
    ];
    assert_eq!(
        program
            .execute("logger_start_probe", "without", &[])
            .unwrap(),
        Term::List(expected)
    );
    assert_eq!(
        reference("without"),
        b"[#{type => standard_io},#{1 => one,2 => two}]\n"
    );
}

#[test]
fn reads_and_deletes_ets_keys_as_pinned_erts() {
    // `logger_start_probe:ets/0` runs `ets:whereis/1`
    // (`erl_db.c:2849-2872`), `ets:lookup/2` (`erl_db.c:2877-2900`),
    // `ets:member/2` (`erl_db.c:2905-2928`), and `ets:delete/2`
    // (`erl_db.c:3328-3349`), as `logger_config` (`logger_config.erl:45-98`)
    // does. `whereis/1` gives the table reference of a named table and
    // `undefined` for an unnamed table and a free name. A set table compares
    // keys with exact equality, also with the key position 2. `delete/2`
    // returns `true` also for a key that the table does not hold. A name
    // that is not an atom, a free name, and a term that is not an atom or a
    // reference raise `badarg`.
    let program = load();
    let atom = |name: &str| Term::Atom(program.atom(name).unwrap());
    let pair = |first: Term, second: Term| Term::Tuple(vec![first, second]);
    let found = |key: Term, value: Term| Term::List(vec![pair(key, value)]);
    let badarg = pair(atom("error"), atom("badarg"));
    let mut expected = vec![
        atom("logger_start_probe"),
        atom("true"),
        atom("true"),
        atom("undefined"),
        atom("undefined"),
        found(atom("a"), Term::Integer(1)),
        found(pair(atom("h"), atom("b")), Term::Integer(2)),
        found(Term::List(vec![atom("x")]), Term::Integer(3)),
        Term::List(vec![]),
        found(atom("v"), atom("k")),
        Term::List(vec![]),
        atom("true"),
        atom("true"),
        atom("false"),
        atom("true"),
        atom("true"),
        atom("true"),
        atom("true"),
        Term::List(vec![]),
        atom("false"),
        found(pair(atom("h"), atom("b")), Term::Integer(2)),
    ];
    expected.extend(std::iter::repeat_n(badarg, 8));
    assert_eq!(
        program.execute("logger_start_probe", "ets", &[]).unwrap(),
        Term::List(expected)
    );
    assert_eq!(
        reference("ets"),
        b"[logger_start_probe,true,true,undefined,undefined,[{a,1}],[{{h,b},2}],[{[x],3}],[],[{v,k}],[],\
          true,true,false,true,true,true,true,[],false,[{{h,b},2}],\
          {error,badarg},{error,badarg},{error,badarg},{error,badarg},{error,badarg},{error,badarg},{error,badarg},{error,badarg}]\n"
    );
}

#[test]
fn checks_ets_access_rights_as_pinned_erts() {
    // `logger_start_probe:ets_access/0` reads the tables of the caller from
    // another process. `ets:lookup/2` and `ets:member/2` need `DB_READ`, and
    // `ets:delete/2` needs `DB_WRITE` (`erl_db_util.h:449-450`), so a
    // protected table raises `badarg` only for `delete/2`, and a private
    // table raises `badarg` for all three. `ets:whereis/1` needs only
    // `DB_INFO` (`erl_db_util.h:451`), so it finds the private table. The
    // exit of the owner deletes its table and frees its name.
    let program = load();
    let atom = |name: &str| Term::Atom(program.atom(name).unwrap());
    let badarg = Term::Tuple(vec![atom("error"), atom("badarg")]);
    let expected = vec![
        Term::List(vec![Term::Tuple(vec![atom("k"), atom("v")])]),
        atom("true"),
        badarg.clone(),
        badarg.clone(),
        badarg.clone(),
        badarg.clone(),
        atom("true"),
        atom("undefined"),
        badarg,
    ];
    assert_eq!(
        program
            .execute("logger_start_probe", "ets_access", &[])
            .unwrap(),
        Term::List(expected)
    );
    assert_eq!(
        reference("ets_access"),
        b"[[{k,v}],true,{error,badarg},{error,badarg},{error,badarg},{error,badarg},true,undefined,{error,badarg}]\n"
    );
}

#[test]
fn finds_map_keys_as_pinned_erts() {
    // `logger_start_probe:find/0` runs `maps:find/2`
    // (`erl_map.c:225-243`), as `supervisor:find_child/2`
    // (`supervisor.erl:1871-1872`) does. A key that the map holds gives
    // `{ok, Value}`, also in a hashmap, and another key gives `error`. A
    // term that is not a map raises `{badmap, Map}`.
    let program = load();
    let atom = |name: &str| Term::Atom(program.atom(name).unwrap());
    let pair = |first: Term, second: Term| Term::Tuple(vec![first, second]);
    let ok = |value: Term| pair(atom("ok"), value);
    let badmap = |term: Term| pair(atom("error"), pair(atom("badmap"), term));
    let expected = vec![
        ok(Term::Integer(1)),
        ok(Term::Integer(2)),
        ok(atom("three")),
        atom("error"),
        atom("error"),
        ok(Term::Integer(40)),
        atom("error"),
        badmap(atom("foo")),
        badmap(Term::List(vec![])),
    ];
    assert_eq!(
        program.execute("logger_start_probe", "find", &[]).unwrap(),
        Term::List(expected)
    );
    assert_eq!(
        reference("find"),
        b"[{ok,1},{ok,2},{ok,three},error,error,{ok,40},error,{error,{badmap,foo}},{error,{badmap,[]}}]\n"
    );
}

#[test]
fn builds_maps_from_lists_as_pinned_erts() {
    // `logger_start_probe:from_list/0` runs `maps:from_list/1`
    // (`erl_map.c:321-363`), which `maps:with/2` (`maps.erl:1165-1166`)
    // calls for the logger handler config. A later pair of a key wins. 40
    // keys give a hashmap with the ERTS HAMT key order, and 43 pairs with 3
    // keys give a flatmap. A term that is not a proper list, an element that
    // is not a tuple, and a tuple with an arity other than 2 raise `badarg`.
    let program = load();
    let atom = |name: &str| Term::Atom(program.atom(name).unwrap());
    let map = |pairs: Vec<(Term, Term)>| Term::Map(Map::from_pairs(pairs).unwrap());
    let integer_map = |pairs: &[(i64, i64)]| {
        map(pairs
            .iter()
            .map(|&(key, value)| (Term::Integer(key), Term::Integer(value)))
            .collect())
    };
    let badarg = Term::Tuple(vec![atom("error"), atom("badarg")]);
    let keys = [
        18, 4, 34, 12, 19, 29, 13, 2, 7, 38, 31, 8, 10, 23, 9, 15, 35, 36, 32, 1, 25, 28, 20, 6,
        11, 17, 24, 37, 40, 14, 33, 3, 16, 30, 21, 5, 27, 26, 22, 39,
    ];
    let mut expected = vec![
        map(vec![]),
        map(vec![
            (Term::Integer(1), atom("a")),
            (Term::Integer(2), atom("c")),
        ]),
        map(vec![(atom("type"), atom("standard_io"))]),
        Term::List(keys.into_iter().map(Term::Integer).collect()),
        integer_map(&[(0, 1), (1, 2), (2, 80)]),
    ];
    expected.extend(std::iter::repeat_n(badarg, 4));
    assert_eq!(
        program
            .execute("logger_start_probe", "from_list", &[])
            .unwrap(),
        Term::List(expected)
    );
    assert_eq!(
        reference("from_list"),
        b"[#{},#{1 => a,2 => c},#{type => standard_io},\
          [18,4,34,12,19,29,13,2,7,38,31,8,10,23,9,15,35,36,32,1,25,28,20,6,11,17,24,37,40,14,33,3,16,30,21,5,27,26,22,39],\
          #{0 => 1,1 => 2,2 => 80},{error,badarg},{error,badarg},{error,badarg},{error,badarg}]\n"
    );
}

#[test]
fn tests_map_keys_as_pinned_erts() {
    // `logger_start_probe:is_key/0` runs `erlang:is_map_key/2`
    // (`erl_map.c:1185-1196`), in a body and in a guard, as
    // `logger:add_handler/3` (`logger.erl:832`) does. A term that is not a
    // map raises `{badmap, Map}` in a body. In a guard, the `bif`
    // instruction goes to its fail label, so `key_guard/2` gives `no`.
    let program = load();
    let atom = |name: &str| Term::Atom(program.atom(name).unwrap());
    let badmap =
        |term: Term| Term::Tuple(vec![atom("error"), Term::Tuple(vec![atom("badmap"), term])]);
    let expected = vec![
        atom("true"),
        atom("false"),
        atom("true"),
        atom("false"),
        atom("yes"),
        atom("no"),
        atom("no"),
        badmap(atom("foo")),
        badmap(Term::List(vec![])),
    ];
    assert_eq!(
        program
            .execute("logger_start_probe", "is_key", &[])
            .unwrap(),
        Term::List(expected)
    );
    assert_eq!(
        reference("is_key"),
        b"[true,false,true,false,yes,no,no,{error,{badmap,foo}},{error,{badmap,[]}}]\n"
    );
}

#[test]
fn takes_list_heads_as_pinned_erts() {
    // `logger_start_probe:hd/0` runs `erlang:hd/1` (`bif.c:2918-2925`), in a
    // body and in a guard, as `gen_server:decode_msg/6`
    // (`gen_server.erl:2380`) does. It gives the head of a proper and of an
    // improper list. `[]` and a term that is not a list raise `badarg` in a
    // body. In a guard, the `bif` instruction goes to its fail label, so
    // `hd_guard/1` gives `no`.
    let program = load();
    let atom = |name: &str| Term::Atom(program.atom(name).unwrap());
    let badarg = Term::Tuple(vec![atom("error"), atom("badarg")]);
    let expected = vec![
        atom("a"),
        atom("a"),
        atom("c"),
        atom("yes"),
        atom("no"),
        atom("no"),
        badarg.clone(),
        badarg,
    ];
    assert_eq!(
        program.execute("logger_start_probe", "hd", &[]).unwrap(),
        Term::List(expected)
    );
    assert_eq!(
        reference("hd"),
        b"[a,a,c,yes,no,no,{error,badarg},{error,badarg}]\n"
    );
}

/// The keys of `big(40)` and `big(100)` of the probe in the ERTS HAMT order.
const BIG_40_KEYS: [i64; 40] = [
    18, 4, 34, 12, 19, 29, 13, 2, 7, 38, 31, 8, 10, 23, 9, 15, 35, 36, 32, 1, 25, 28, 20, 6, 11,
    17, 24, 37, 40, 14, 33, 3, 16, 30, 21, 5, 27, 26, 22, 39,
];
const BIG_100_KEYS: [i64; 100] = [
    54, 18, 96, 94, 84, 4, 72, 68, 93, 34, 46, 52, 92, 12, 19, 91, 41, 100, 29, 13, 42, 55, 2, 98,
    7, 90, 38, 56, 61, 99, 65, 31, 70, 8, 44, 10, 23, 9, 82, 49, 79, 71, 66, 75, 64, 78, 80, 15,
    35, 36, 76, 97, 32, 58, 1, 25, 28, 60, 20, 87, 6, 95, 83, 43, 57, 11, 81, 73, 89, 88, 48, 17,
    24, 37, 40, 14, 33, 53, 3, 16, 30, 62, 86, 21, 5, 77, 27, 85, 47, 63, 26, 51, 22, 50, 45, 67,
    69, 59, 74, 39,
];

/// Return the `~w` text of `[{K,K}, ...]` for integer keys.
fn pairs_text(keys: &[i64]) -> String {
    let pairs: Vec<_> = keys.iter().map(|key| format!("{{{key},{key}}}")).collect();
    format!("[{}]", pairs.join(","))
}

/// Return the list `[{K, K}, ...]` of integer keys.
fn integer_pairs(keys: &[i64]) -> Term {
    let pair = |key: &i64| Term::Tuple(vec![Term::Integer(*key), Term::Integer(*key)]);
    Term::List(keys.iter().map(pair).collect())
}

#[test]
fn lists_map_pairs_as_pinned_erts() {
    // `logger_start_probe:to_list/0` runs the pinned `maps:to_list/1`
    // (`maps.erl:525-542`), which calls `erts_internal:map_next/3`
    // (`erl_map.c:3637-4135`) until no path remains, as
    // `logger_h_common:filter_config/1` (`logger_h_common.erl:441`) does. A
    // flatmap gives its pairs in the flatmap order. A hashmap gives the last
    // 32 pairs at path 0 and the rest in the next call, in the HAMT order.
    let program = load();
    let atom = |name: &str| Term::Atom(program.atom(name).unwrap());
    let pair = |key: Term, value: Term| Term::Tuple(vec![key, value]);
    let expected = vec![
        Term::List(vec![]),
        Term::List(vec![
            pair(Term::Integer(1), atom("a")),
            pair(Term::Integer(2), atom("b")),
            pair(atom("three"), Term::Integer(3)),
        ]),
        Term::List(vec![pair(atom("type"), atom("standard_io"))]),
        integer_pairs(&BIG_40_KEYS),
        integer_pairs(&BIG_100_KEYS),
    ];
    assert_eq!(
        program
            .execute("logger_start_probe", "to_list", &[])
            .unwrap(),
        Term::List(expected)
    );
    let text = format!(
        "[[],[{{1,a}},{{2,b}},{{three,3}}],[{{type,standard_io}}],{},{}]\n",
        pairs_text(&BIG_40_KEYS),
        pairs_text(&BIG_100_KEYS)
    );
    assert_eq!(String::from_utf8(reference("to_list")).unwrap(), text);
}

#[test]
fn walks_maps_with_a_path_as_pinned_erts() {
    // `logger_start_probe:map_next/0` calls `erts_internal:map_next/3` in a
    // new process, which has a full reduction budget on ERTS. A flatmap
    // gives all pairs for a small path from 0 to its size. In list mode, a
    // hashmap at path 0 gives its last 32 pairs and the path 43, and the
    // next call adds the 8 other pairs in front. A path that is not a small
    // integer for a flatmap, a negative small integer (2^60 - 1 as a path),
    // a term that is not a map, a third argument that is not `iterator` or
    // a list, and a path element that selects no entry of a node raise
    // `badarg`.
    let program = load();
    let atom = |name: &str| Term::Atom(program.atom(name).unwrap());
    let pair = |key: i64, value: &str| Term::Tuple(vec![Term::Integer(key), atom(value)]);
    let small = Term::List(vec![pair(1, "a"), pair(2, "b")]);
    let iterator = Term::Tuple(vec![
        Term::Integer(1),
        atom("a"),
        Term::Tuple(vec![Term::Integer(2), atom("b"), atom("none")]),
    ]);
    let badarg = Term::Tuple(vec![atom("error"), atom("badarg")]);
    let mut expected = vec![
        small.clone(),
        Term::List(vec![pair(1, "a"), pair(2, "b"), atom("x")]),
        small,
        iterator,
        atom("none"),
        Term::Integer(43),
        Term::Integer(32),
        integer_pairs(&BIG_40_KEYS[8..]),
        integer_pairs(&BIG_40_KEYS),
        Term::Integer(43),
        atom("end_"),
    ];
    expected.extend(std::iter::repeat_n(badarg, 8));
    assert_eq!(
        program
            .execute("logger_start_probe", "map_next", &[])
            .unwrap(),
        Term::List(expected)
    );
    let text = format!(
        "[[{{1,a}},{{2,b}}],[{{1,a}},{{2,b}},x],[{{1,a}},{{2,b}}],{{1,a,{{2,b,none}}}},none,43,32,{},{},43,end_,{}]\n",
        pairs_text(&BIG_40_KEYS[8..]),
        pairs_text(&BIG_40_KEYS),
        ["{error,badarg}"; 8].join(",")
    );
    assert_eq!(String::from_utf8(reference("map_next")).unwrap(), text);
}

#[test]
fn iterates_maps_as_pinned_erts() {
    // `logger_start_probe:next/0` runs the pinned `maps:next/1`
    // (`maps.erl:1094-1115`) on `maps:iterator/1`, which is `[0 | Map]`. In
    // iterator mode, a hashmap at path 0 gives its first 32 pairs and then
    // `[26 | Map]`, and the call with path 26 gives the rest.
    let program = load();
    let atom = |name: &str| Term::Atom(program.atom(name).unwrap());
    let integers = |keys: &[i64]| Term::List(keys.iter().copied().map(Term::Integer).collect());
    let expected = vec![
        integers(&[1, 2]),
        integers(&BIG_40_KEYS),
        Term::Tuple(vec![Term::Integer(18), Term::Integer(18)]),
        atom("true"),
        Term::Integer(26),
        atom("true"),
    ];
    assert_eq!(
        program.execute("logger_start_probe", "next", &[]).unwrap(),
        Term::List(expected)
    );
    let keys: Vec<_> = BIG_40_KEYS.iter().map(i64::to_string).collect();
    let text = format!("[[1,2],[{}],{{18,18}},true,26,true]\n", keys.join(","));
    assert_eq!(String::from_utf8(reference("next")).unwrap(), text);
}

#[test]
fn iterates_maps_in_a_given_key_order_as_pinned_erts() {
    // `logger_start_probe:ordered/0` runs the ordered iterator of
    // `erts_internal:map_next/3` (`erl_map.c:3653-3707`): a list path of
    // keys, as `maps:iterator/2` (`maps.erl:1062-1072`) makes it. The BIF
    // gives the keys in the order of the list, also for a hashmap, and `[]`
    // gives `none`. A call takes at most 125 keys, so 130 keys need a
    // second call with the rest of the list. A key that the map does not
    // hold, a tail that is not `[]`, and a list path in list mode raise
    // `badarg`. The second-call badarg case is in
    // `ordered_second_call_badarg/0`, which crashes ERTS; the test below
    // checks PBeam for it without a reference run.
    let program = load();
    let atom = |name: &str| Term::Atom(program.atom(name).unwrap());
    let badarg = Term::Tuple(vec![atom("error"), atom("badarg")]);
    let mut expected = vec![
        Term::List(vec![atom("a"), atom("b"), atom("c")]),
        Term::List((1..=40).rev().map(Term::Integer).collect()),
        atom("true"),
        Term::Tuple(vec![
            atom("c"),
            Term::Integer(3),
            Term::Tuple(vec![atom("a"), Term::Integer(1), atom("none")]),
        ]),
        atom("none"),
    ];
    expected.extend(std::iter::repeat_n(badarg, 3));
    assert_eq!(
        program
            .execute("logger_start_probe", "ordered", &[])
            .unwrap(),
        Term::List(expected)
    );
    let keys: Vec<_> = (1..=40).rev().map(|key: i64| key.to_string()).collect();
    let text = format!(
        "[[a,b,c],[{}],true,{{c,3,{{a,1,none}}}},none,{{error,badarg}},{{error,badarg}},{{error,badarg}}]\n",
        keys.join(",")
    );
    assert_eq!(String::from_utf8(reference("ordered")).unwrap(), text);
}

#[test]
fn returns_badarg_for_an_ordered_path_that_crashes_pinned_erts() {
    // `logger_start_probe:ordered_second_call_badarg/0` runs the ordered
    // iterator of `erts_internal:map_next/3` with a 130-key path whose tail
    // is not `[]`, so the badarg is raised in the second call, after the
    // first chunk of keys. PBeam returns `{error, badarg}` every time. On the
    // pinned ERTS (OTP 29.1.1, JIT) this input crashes the emulator with
    // SIGSEGV in about one run in four: the BIF builds one tuple per key in a
    // heap fragment and raises badarg before it sets the last tuple's third
    // element (`erl_map.c:3670-3707`); `HRelease` keeps the half-built tuples
    // in the fragment, the GC after the BIF call copies and scans the whole
    // fragment (`collect_live_heap_frags`, `erl_gc.c`), and the unset element
    // holds uninitialised heap memory that reads as a wild boxed or list
    // pointer. There is thus no stable reference output to compare, so this
    // test checks only PBeam; `docs/erts-differences.md` records the crash.
    let program = load();
    let atom = |name: &str| Term::Atom(program.atom(name).unwrap());
    assert_eq!(
        program
            .execute("logger_start_probe", "ordered_second_call_badarg", &[])
            .unwrap(),
        Term::Tuple(vec![atom("error"), atom("badarg")])
    );
}

#[test]
fn removes_monitors_as_pinned_erts() {
    // `logger_start_probe:demonitor/0` runs `erlang:demonitor/1`
    // (`bif.c:598-611`), as `logger_simple_h:adding_handler/1`
    // (`logger_simple_h.erl:45`) does. It removes an active monitor, so no
    // `DOWN` message comes. It gives `true` also when the monitor already
    // fired, and the queued `DOWN` message with reason `normal` stays. A
    // term that is not a reference raises `badarg`.
    let program = load();
    let atom = |name: &str| Term::Atom(program.atom(name).unwrap());
    let badarg = Term::Tuple(vec![atom("error"), atom("badarg")]);
    let expected = vec![
        atom("true"),
        atom("none"),
        atom("true"),
        atom("normal"),
        atom("true"),
        atom("true"),
        badarg.clone(),
        badarg,
    ];
    assert_eq!(
        program
            .execute("logger_start_probe", "demonitor", &[])
            .unwrap(),
        Term::List(expected)
    );
    assert_eq!(
        reference("demonitor"),
        b"[true,none,true,normal,true,true,{error,badarg},{error,badarg}]\n"
    );
}

#[test]
fn removes_links_as_pinned_erts() {
    // `logger_start_probe:unlink/0` runs `erlang:unlink/1`
    // (`bif.c:1223-1364`), as `logger_simple_h:loop/2`
    // (`logger_simple_h.erl:124`) does. After the unlink, the `boom` exit of
    // the child sends no exit signal to the caller. The unlink of a process
    // that is gone gives `true`, and its queued `EXIT` message with reason
    // `bang` stays. The caller itself and a pid without a link give `true`.
    // A term that is not a pid raises `badarg`.
    let program = load();
    let atom = |name: &str| Term::Atom(program.atom(name).unwrap());
    let badarg = Term::Tuple(vec![atom("error"), atom("badarg")]);
    let expected = vec![
        atom("true"),
        atom("true"),
        atom("bang"),
        atom("true"),
        atom("true"),
        badarg.clone(),
        badarg,
    ];
    assert_eq!(
        program
            .execute("logger_start_probe", "unlink", &[])
            .unwrap(),
        Term::List(expected)
    );
    assert_eq!(
        reference("unlink"),
        b"[true,true,bang,true,true,{error,badarg},{error,badarg}]\n"
    );
}

#[test]
fn sets_the_system_logger_as_pinned_erts() {
    // `logger_start_probe:system_logger/0` runs `erlang:system_flag/2` with
    // `system_logger` (`bif.c:5294-5296`, `erl_trace.c:2486-2491`), as
    // `logger_proxy:init/1` (`logger_proxy.erl:117`) does. `logger`,
    // `undefined`, and a local pid set the node value and give the old
    // value. Each other value raises `badarg`.
    let program = load();
    let atom = |name: &str| Term::Atom(program.atom(name).unwrap());
    let badarg = Term::Tuple(vec![atom("error"), atom("badarg")]);
    let expected = vec![
        atom("logger"),
        atom("undefined"),
        atom("true"),
        atom("logger"),
        badarg.clone(),
        badarg.clone(),
        badarg,
    ];
    assert_eq!(
        program
            .execute("logger_start_probe", "system_logger", &[])
            .unwrap(),
        Term::List(expected)
    );
    assert_eq!(
        reference("system_logger"),
        b"[logger,undefined,true,logger,{error,badarg},{error,badarg},{error,badarg}]\n"
    );
}

#[test]
fn rejects_system_flag_items_as_pinned_erts() {
    // `logger_start_probe:system_flag/0`: an item that `system_flag_2`
    // (`bif.c:4972-5304`) does not know raises `badarg`, also `time_offset`
    // with a value that is not `finalize`. `debug_flags` gives `true`. It
    // also raises `badarg` for a value that it rejects for an item that
    // needs a subsystem that PBeam does not have, before it uses the
    // subsystem.
    let program = load();
    let atom = |name: &str| Term::Atom(program.atom(name).unwrap());
    let badarg = Term::Tuple(vec![atom("error"), atom("badarg")]);
    let mut expected = vec![badarg.clone(); 3];
    expected.push(atom("true"));
    expected.extend(vec![badarg; 29]);
    assert_eq!(
        program
            .execute("logger_start_probe", "system_flag", &[])
            .unwrap(),
        Term::List(expected)
    );
    assert_eq!(
        String::from_utf8(reference("system_flag")).unwrap(),
        format!(
            "[{},true,{}]\n",
            ["{error,badarg}"; 3].join(","),
            ["{error,badarg}"; 29].join(",")
        )
    );
}

#[test]
fn sets_the_message_queue_data_flag_as_pinned_erts() {
    // `logger_start_probe:message_queue_data/0` runs
    // `process_flag(message_queue_data, Value)` (`bif.c:2167-2172`), as
    // `logger_olp:init/1` (`logger_olp.erl:172`) does, and the spawn option
    // `{message_queue_data, off_heap}`, as
    // `logger_simple_h:adding_handler/1` (`logger_simple_h.erl:39-40`)
    // does. The flag gives the old value, and the start value is `on_heap`.
    // The last spawn option wins (`erl_process.c:12305-12319`). Each other
    // value raises `badarg`.
    let program = load();
    let atom = |name: &str| Term::Atom(program.atom(name).unwrap());
    let badarg = Term::Tuple(vec![atom("error"), atom("badarg")]);
    let expected = vec![
        atom("on_heap"),
        atom("off_heap"),
        atom("off_heap"),
        atom("off_heap"),
        atom("off_heap"),
        badarg.clone(),
        badarg,
    ];
    assert_eq!(
        program
            .execute("logger_start_probe", "message_queue_data", &[])
            .unwrap(),
        Term::List(expected)
    );
    assert_eq!(
        reference("message_queue_data"),
        b"[on_heap,off_heap,off_heap,off_heap,off_heap,{error,badarg},{error,badarg}]\n"
    );
}

#[test]
fn rejects_process_flags_as_pinned_erts() {
    // `logger_start_probe:process_flag/0`: `process_flag_2`
    // (`bif.c:2028-2237`) raises `badarg` for `trap_exit` with a value that
    // is not a boolean, for `message_queue_data` with a value that is not an
    // atom, for a flag that it does not know, and for a tuple
    // that is not `{monitor_nodes, Options}`. It also raises `badarg` for a
    // value of `async_dist`, `sensitive`, `monitor_nodes`, `error_handler`,
    // `priority`, `save_calls`, `scheduler`, `fullsweep_after`,
    // `min_heap_size`, or `min_bin_vheap_size` that it rejects, before it
    // uses a subsystem that PBeam does not have.
    let program = load();
    let atom = |name: &str| Term::Atom(program.atom(name).unwrap());
    let badarg = Term::Tuple(vec![atom("error"), atom("badarg")]);
    assert_eq!(
        program
            .execute("logger_start_probe", "process_flag", &[])
            .unwrap(),
        Term::List(vec![badarg; 20])
    );
    assert_eq!(
        String::from_utf8(reference("process_flag")).unwrap(),
        format!("[{}]\n", ["{error,badarg}"; 20].join(","))
    );
}

#[test]
fn returns_a_runtime_error_for_a_flag_that_needs_a_missing_subsystem() {
    // `logger_start_probe:unsupported_flag/1`: ERTS gives a value for each
    // flag (`bif.c:5108-5115`, `bif.c:5203-5224`, `bif.c:5116-5172`,
    // `bif.c:2052-2057`, `bif.c:1907-1915`, `bif.c:2114-2130`,
    // `bif.c:2212-2226`). PBeam has no stack traces, time offset service,
    // tracing, priorities, saved calls, garbage collector, or distribution
    // (rows "System flags" and "Process flags" in
    // `docs/erts-differences.md`).
    let program = load();
    for (case, message, erts) in [
        (
            "backtrace_depth",
            "unsupported erlang:system_flag/2 item backtrace_depth",
            "8",
        ),
        (
            "time_offset",
            "unsupported erlang:system_flag/2 item time_offset",
            "volatile",
        ),
        (
            "trace_control_word",
            "unsupported erlang:system_flag/2 item trace_control_word",
            "0",
        ),
        (
            "sequential_tracer",
            "unsupported erlang:system_flag/2 item sequential_tracer",
            "false",
        ),
        (
            "reset_seq_trace",
            "unsupported erlang:system_flag/2 item reset_seq_trace",
            "true",
        ),
        (
            "priority",
            "unsupported erlang:process_flag/2 flag priority",
            "normal",
        ),
        (
            "save_calls",
            "unsupported erlang:process_flag/2 flag save_calls",
            "0",
        ),
        (
            "min_heap_size",
            "unsupported erlang:process_flag/2 flag min_heap_size",
            "233",
        ),
        (
            "monitor_nodes",
            "unsupported erlang:process_flag/2 flag {monitor_nodes, Options}",
            "0",
        ),
    ] {
        let error = program
            .execute(
                "logger_start_probe",
                "unsupported_flag",
                &[Term::Atom(program.atom(case).unwrap())],
            )
            .unwrap_err();
        assert!(error.to_string().contains(message), "{error}");
        assert_eq!(
            String::from_utf8(reference_call(&format!("unsupported_flag({case})"))).unwrap(),
            format!("{erts}\n")
        );
    }
}

#[test]
fn raises_exceptions_as_pinned_erts() {
    // `logger_start_probe:raise/0`: `erlang:error/1` (`bif.c:1436-1440`)
    // raises class `error`. `erlang:raise/3` (`bif.c:1549-1692`) raises the
    // class `error` or `exit` for a valid stack trace, and returns the atom
    // `badarg` for a class or a stack trace that is not valid.
    let program = load();
    let atom = |name: &str| Term::Atom(program.atom(name).unwrap());
    let pair = |class: &str, reason: Term| Term::Tuple(vec![atom(class), reason]);
    let mut expected = vec![
        pair("error", atom("foo")),
        pair("error", Term::Tuple(vec![atom("a"), atom("b")])),
        pair("error", atom("r")),
        pair("exit", Term::Tuple(vec![atom("r")])),
    ];
    expected.extend(std::iter::repeat_n(atom("badarg"), 12));
    assert_eq!(
        program.execute("logger_start_probe", "raise", &[]).unwrap(),
        Term::List(expected)
    );
    assert_eq!(
        String::from_utf8(reference("raise")).unwrap(),
        format!(
            "[{{error,foo}},{{error,{{a,b}}}},{{error,r}},{{exit,{{r}}}},{}]\n",
            ["badarg"; 12].join(",")
        )
    );
}

#[test]
fn returns_a_runtime_error_for_a_raise_of_class_throw() {
    // `logger_start_probe:raise_throw/0`: ERTS raises class `throw`, which
    // PBeam does not implement (row "Exception raise sites" in
    // `docs/erts-differences.md`).
    let program = load();
    let error = program
        .execute("logger_start_probe", "raise_throw", &[])
        .unwrap_err();
    assert!(
        error
            .to_string()
            .contains("unsupported erlang:raise/3 class throw"),
        "{error}"
    );
    assert_eq!(reference("raise_throw"), b"{throw,r}\n");
}

#[test]
fn walks_collision_nodes_and_bignum_paths_as_pinned_erts() {
    // `logger_start_probe:map_paths/0` calls `erts_internal:map_next/3`
    // (`erl_map.c:3593-4135`) in a new process. Two list keys share a
    // collision node, and a chunk at path 0 takes the whole node, so it
    // gives 33 pairs and the path 369. Path 113 selects the collision node.
    // The bignum path 2^59 and the path -(2^59) walk all pairs. The negative
    // bignum -(2^60), and a bignum path for a flatmap, raise `badarg`. The
    // paths 2^60 + 18 and 2^60 + 786 have 16 elements and select a node at
    // level 15.
    let program = load();
    let atom = |name: &str| Term::Atom(program.atom(name).unwrap());
    let integers = |keys: &[i64]| keys.iter().copied().map(Term::Integer).collect::<Vec<_>>();
    let a = [246, 157, 203, 77, 19, 254, 19, 93];
    let b = [161, 92, 65, 50, 231, 24, 153, 137];
    let collision = [Term::List(integers(&b)), Term::List(integers(&a))];
    let before = [
        19, 213, 340, 110, 91, 164, 399, 139, 361, 212, 41, 114, 100, 29, 347, 365, 171, 140, 13,
        42, 149, 55, 2, 146, 372, 317, 191, 316, 202, 192, 233,
    ];
    let after = [312, 98, 333];
    let tail = [
        7, 398, 285, 90, 343, 38, 325, 326, 284, 281, 56, 153, 61, 99, 113, 383, 201, 237, 314, 65,
        322, 137, 141, 308, 315, 230, 104, 187,
    ];
    let deep = [
        329898321527737274,
        446694106686689078,
        413866500823436792,
        -101714049538469044,
        -74925715755171563,
    ];
    let keys = |parts: &[&[Term]]| Term::List(parts.concat());
    let chunk = |keys: Term, path: Term| Term::Tuple(vec![keys, path]);
    let badarg = Term::Tuple(vec![atom("error"), atom("badarg")]);
    let all = keys(&[&integers(&before), &collision, &integers(&after)]);
    let expected = vec![
        chunk(keys(&[&integers(&before), &collision]), Term::Integer(369)),
        chunk(keys(&[&collision, &integers(&after)]), atom("none")),
        chunk(all.clone(), atom("none")),
        chunk(all, atom("none")),
        badarg.clone(),
        chunk(
            keys(&[&collision, &integers(&after), &integers(&tail)]),
            Term::Integer(97),
        ),
        badarg,
        chunk(keys(&[&integers(&deep[3..])]), atom("none")),
        chunk(keys(&[&integers(&deep[..4])]), atom("none")),
    ];
    assert_eq!(
        program
            .execute("logger_start_probe", "map_paths", &[])
            .unwrap(),
        Term::List(expected)
    );
    let text = |keys: &[i64]| {
        keys.iter()
            .map(i64::to_string)
            .collect::<Vec<_>>()
            .join(",")
    };
    let collision = format!("[{}],[{}]", text(&b), text(&a));
    let (before, after, tail) = (text(&before), text(&after), text(&tail));
    let all = format!("{before},{collision},{after}");
    assert_eq!(
        String::from_utf8(reference("map_paths")).unwrap(),
        format!(
            "[{{[{before},{collision}],369}},{{[{collision},{after}],none}},{{[{all}],none}},{{[{all}],none}},{{error,badarg}},{{[{collision},{after},{tail}],97}},{{error,badarg}},{{[{}],none}},{{[{}],none}}]\n",
            text(&deep[3..]),
            text(&deep[..4])
        )
    );
}

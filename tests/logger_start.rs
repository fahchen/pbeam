use std::process::Command;

use pbeam_runtime::{BeamProgram, Map, Term};

mod common;

use common::pinned_beam;

const LOGGER_START_PROBE: &[u8] =
    include_bytes!("../fixtures/logger_start/logger_start_probe.beam");

/// Run `logger_start_probe:Function()` on the pinned ERTS and return its
/// `~w` output.
fn reference(function: &str) -> Vec<u8> {
    let reference = Command::new("erl")
        .args([
            "-noshell",
            "-pa",
            "fixtures/logger_start",
            "-eval",
            &format!("io:format(\"~w~n\", [logger_start_probe:{function}()]), halt()."),
        ])
        .output()
        .unwrap();
    assert!(reference.status.success());
    assert!(reference.stderr.is_empty());
    reference.stdout
}

/// Load the probe with the pinned `erlang`, `lists`, and `maps`, so
/// `erlang:spawn_monitor/1`, `lists:reverse/1`, and `maps:without/2` run
/// their BEAM code.
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

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

/// Load the probe with the pinned `lists` and `maps`, so `lists:reverse/1`
/// and `maps:without/2` run their BEAM code.
fn load() -> BeamProgram {
    let lists = pinned_beam("lists");
    let maps = pinned_beam("maps");
    BeamProgram::load(&[&lists, &maps, LOGGER_START_PROBE]).unwrap()
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

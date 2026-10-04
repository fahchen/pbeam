use std::process::Command;

use pbeam_runtime::{BeamProgram, Map, Term};

const MAP_PROBE: &[u8] = include_bytes!("../fixtures/map/map_probe.beam");
const MAP_ORDER_PROBE: &[u8] = include_bytes!("../fixtures/map/map_order_probe.beam");

fn atom(name: &str) -> Term {
    Term::Atom(name.into())
}

fn map(pairs: &[(&str, &str)]) -> Term {
    let pairs = pairs
        .iter()
        .map(|(key, value)| (atom(key), atom(value)))
        .collect();
    Term::Map(Map::from_pairs(pairs).unwrap())
}

fn integers(keys: &[i64]) -> Term {
    Term::List(keys.iter().map(|key| Term::Integer(*key)).collect())
}

/// Run `main/0` of a map fixture under the pinned ERTS and return its stdout.
fn erl_reference(module: &str, format: &str) -> Vec<u8> {
    let reference = Command::new("erl")
        .args([
            "-noshell",
            "-pa",
            "fixtures/map",
            "-eval",
            &format!("io:format(\"{format}~n\", [{module}:main()]), halt()."),
        ])
        .output()
        .unwrap();
    assert!(reference.status.success());
    assert!(reference.stderr.is_empty());
    reference.stdout
}

#[test]
fn builds_matches_and_compares_maps_like_erts() {
    let program = BeamProgram::load(&[MAP_PROBE]).unwrap();
    assert_eq!(
        program.execute("map_probe", "main", &[]).unwrap(),
        Term::Tuple(vec![
            atom("raw"),
            map(&[("input", "cooked"), ("ofd", "stdout"), ("output", "raw")]),
            atom("other"),
            atom("other"),
            atom("same"),
        ])
    );

    assert_eq!(
        erl_reference("map_probe", "~p"),
        b"{raw,#{input => cooked,output => raw,ofd => stdout},other,other,same}\n"
    );
}

#[test]
fn uses_a_flatmap_up_to_32_keys_and_a_hashmap_above_like_erts() {
    let hash33 = [
        18, 4, 12, 19, 29, 13, 2, 7, 31, 8, 10, 23, 9, 15, 32, 1, 25, 28, 20, 6, 11, 17, 24, 14,
        33, 3, 16, 30, 21, 5, 27, 26, 22,
    ];
    let hash40 = [
        18, 4, 34, 12, 19, 29, 13, 2, 7, 38, 31, 8, 10, 23, 9, 15, 35, 36, 32, 1, 25, 28, 20, 6,
        11, 17, 24, 37, 40, 14, 33, 3, 16, 30, 21, 5, 27, 26, 22, 39,
    ];
    let program = BeamProgram::load(&[MAP_ORDER_PROBE]).unwrap();
    assert_eq!(
        program.execute("map_order_probe", "main", &[]).unwrap(),
        Term::Tuple(vec![
            integers(&(1..=32).collect::<Vec<_>>()),
            integers(&hash33),
            integers(&hash33),
            atom("same"),
            integers(&hash40),
            atom("same"),
            atom("different"),
            atom("v"),
            atom("missing"),
            Term::List(vec![Term::Integer(1), Term::Integer(2), atom("a")]),
            integers(&[-101714049538469044, -1, 5]),
        ])
    );

    assert_eq!(
        erl_reference("map_order_probe", "~w"),
        b"{[1,2,3,4,5,6,7,8,9,10,11,12,13,14,15,16,17,18,19,20,21,22,23,24,25,26,27,28,29,30,31,32],\
          [18,4,12,19,29,13,2,7,31,8,10,23,9,15,32,1,25,28,20,6,11,17,24,14,33,3,16,30,21,5,27,26,22],\
          [18,4,12,19,29,13,2,7,31,8,10,23,9,15,32,1,25,28,20,6,11,17,24,14,33,3,16,30,21,5,27,26,22],\
          same,\
          [18,4,34,12,19,29,13,2,7,38,31,8,10,23,9,15,35,36,32,1,25,28,20,6,11,17,24,37,40,14,33,3,16,30,21,5,27,26,22,39],\
          same,different,v,missing,[1,2,a],[-101714049538469044,-1,5]}\n"
    );
}

#[test]
fn takes_the_badmap_path_when_add_gets_a_non_map() {
    // `add/1` tests is_map and then calls erlang:error({badmap, M}), which
    // PBeam does not resolve yet: `erlang:error/1` is not implemented.
    let program = BeamProgram::load(&[MAP_PROBE]).unwrap();
    assert!(
        program
            .execute("map_probe", "add", &[Term::Integer(1)])
            .unwrap_err()
            .to_string()
            .contains("unresolved external MFA erlang:error/1")
    );
}

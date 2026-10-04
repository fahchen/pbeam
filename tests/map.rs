use std::process::Command;

use pbeam_runtime::{BeamProgram, Term};

const MAP_PROBE: &[u8] = include_bytes!("../fixtures/map/map_probe.beam");

fn atom(name: &str) -> Term {
    Term::Atom(name.into())
}

fn map(pairs: &[(&str, &str)]) -> Term {
    Term::Map(
        pairs
            .iter()
            .map(|(key, value)| (atom(key), atom(value)))
            .collect(),
    )
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

    let reference = Command::new("erl")
        .args([
            "-noshell",
            "-pa",
            "fixtures/map",
            "-eval",
            "io:format(\"~p~n\", [map_probe:main()]), halt().",
        ])
        .output()
        .unwrap();
    assert!(reference.status.success());
    assert!(reference.stderr.is_empty());
    assert_eq!(
        reference.stdout,
        b"{raw,#{input => cooked,output => raw,ofd => stdout},other,other,same}\n"
    );
}

#[test]
fn takes_the_badmap_path_when_add_gets_a_non_map() {
    // `add/1` tests is_map and then calls erlang:error({badmap, M}), which
    // PBeam does not resolve until exceptions arrive in #73.
    let program = BeamProgram::load(&[MAP_PROBE]).unwrap();
    assert!(
        program
            .execute("map_probe", "add", &[Term::Integer(1)])
            .unwrap_err()
            .to_string()
            .contains("unresolved external MFA erlang:error/1")
    );
}

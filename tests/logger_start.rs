use std::process::Command;

use pbeam_runtime::{BeamProgram, Term};

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

/// Load the probe with the pinned `lists`, so `lists:reverse/1` runs its
/// BEAM code.
fn load() -> BeamProgram {
    let lists = pinned_beam("lists");
    BeamProgram::load(&[&lists, LOGGER_START_PROBE]).unwrap()
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

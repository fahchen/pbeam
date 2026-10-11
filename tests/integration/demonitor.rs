use std::process::Command;

use pbeam_runtime::{BeamProgram, Term};

#[path = "../common/mod.rs"]
mod common;

use common::pinned_beam;

const DEMONITOR_PROBE: &[u8] = include_bytes!("../../fixtures/demonitor/demonitor_probe.beam");

#[test]
fn matches_erts_demonitor_options_and_argument_errors() {
    // `spawn_monitor/3` runs from the pinned `erlang.beam`, and the flush trap
    // of `demonitor/2` runs `erts_internal:flush_monitor_messages/3`.
    let erlang = pinned_beam("erlang");
    let erts_internal = pinned_beam("erts_internal");
    let program = BeamProgram::load(&[&erlang, &erts_internal, DEMONITOR_PROBE]).unwrap();
    let atom = |name: &str| Term::Atom(program.atom(name).unwrap());
    let atoms = |names: &[&str]| Term::list_from(names.iter().map(|name| atom(name)));
    let removed = ["true", "true", "false", "false", "false", "false"];
    let fired = [
        ("true", "down"),
        ("true", "none"),
        ("false", "down"),
        ("false", "none"),
        ("false", "down"),
        ("false", "none"),
    ];

    assert_eq!(
        program.execute("demonitor_probe", "main", &[]).unwrap(),
        Term::tuple(vec![
            Term::tuple(vec![atom("badarg"); 5]),
            Term::tuple(vec![atom("value"), atom("false")]),
            atoms(&["true"; 6]),
            atom("true"),
            atoms(&removed),
            atoms(&removed),
            Term::list_from(
                fired
                    .iter()
                    .map(|(result, down)| Term::tuple(vec![atom(result), atom(down)]))
            ),
        ])
    );

    let reference = Command::new("erl")
        .args([
            "-noshell",
            "-pa",
            "fixtures/demonitor",
            "-eval",
            "io:format(\"~w~n\", [demonitor_probe:main()]), halt().",
        ])
        .output()
        .unwrap();
    assert!(reference.status.success());
    assert!(reference.stderr.is_empty());
    assert_eq!(
        reference.stdout,
        b"{{badarg,badarg,badarg,badarg,badarg},{value,false},[true,true,true,true,true,true],true,[true,true,false,false,false,false],[true,true,false,false,false,false],[{true,down},{true,none},{false,down},{false,none},{false,down},{false,none}]}\n"
    );
}

#[test]
fn creates_no_monitor_when_a_process_monitors_itself() {
    let program = BeamProgram::load(&[DEMONITOR_PROBE]).unwrap();

    assert_eq!(
        program
            .execute("demonitor_probe", "self_monitor", &[])
            .unwrap(),
        Term::Atom(program.atom("false").unwrap())
    );

    let reference = Command::new("erl")
        .args([
            "-noshell",
            "-pa",
            "fixtures/demonitor",
            "-eval",
            "io:format(\"~w~n\", [demonitor_probe:self_monitor()]), halt().",
        ])
        .output()
        .unwrap();
    assert!(reference.status.success());
    assert!(reference.stderr.is_empty());
    assert_eq!(reference.stdout, b"false\n");
}

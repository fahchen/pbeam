use std::process::Command;

use pbeam_runtime::{BeamProgram, Term};

mod common;

use common::pinned_beam;

const EXIT_SIGNAL_PROBE: &[u8] = include_bytes!("../fixtures/exit_signal/exit_signal_probe.beam");

#[test]
fn delivers_link_exit_before_monitor_down_and_normal_exit_to_trapping_process() {
    // `erlang:spawn_monitor/3` runs from the pinned `erlang.beam`.
    let erlang = pinned_beam("erlang");
    let program = BeamProgram::load(&[&erlang, EXIT_SIGNAL_PROBE]).unwrap();
    let atom = |name: &str| Term::Atom(program.atom(name).unwrap());
    let pair = |tag: &str, reason: &str| Term::Tuple(vec![atom(tag), atom(reason)]);
    assert_eq!(
        program.execute("exit_signal_probe", "main", &[]).unwrap(),
        Term::Tuple(vec![
            Term::Tuple(vec![pair("EXIT", "boom"), pair("DOWN", "boom")]),
            pair("EXIT", "normal"),
        ])
    );

    let reference = Command::new("erl")
        .args([
            "-noshell",
            "-pa",
            "fixtures/exit_signal",
            "-eval",
            "io:format(\"~p~n\", [exit_signal_probe:main()]), halt().",
        ])
        .output()
        .unwrap();
    assert!(reference.status.success());
    assert!(reference.stderr.is_empty());
    assert_eq!(
        reference.stdout,
        b"{{{'EXIT',boom},{'DOWN',boom}},{'EXIT',normal}}\n"
    );
}

const EXIT_PROBE: &[u8] = include_bytes!("../fixtures/exit_signal/exit_probe.beam");

#[test]
fn sends_exit_signals_with_exit_2_as_erts_does() {
    // `erlang:spawn_monitor/1,3` run from the pinned `erlang.beam`.
    let erlang = pinned_beam("erlang");
    let program = BeamProgram::load(&[&erlang, EXIT_PROBE]).unwrap();
    let atom = |name: &str| Term::Atom(program.atom(name).unwrap());
    let returned = Term::Tuple(vec![atom("returned"), atom("true"), atom("true")]);
    let pair = |tag: &str, value: &str| Term::Tuple(vec![atom(tag), atom(value)]);
    assert_eq!(
        program.execute("exit_probe", "main", &[]).unwrap(),
        Term::List(vec![
            atom("normal"),
            atom("boom"),
            atom("killed"),
            returned.clone(),
            returned,
            atom("killed"),
            pair("alive", "none"),
            atom("boom"),
            atom("killed"),
            pair("alive", "normal"),
            atom("killed"),
            atom("badarg"),
            atom("badarg"),
            atom("true"),
            atom("true"),
            pair("linked", "boom"),
        ])
    );

    let reference = Command::new("erl")
        .args([
            "-noshell",
            "-pa",
            "fixtures/exit_signal",
            "-eval",
            "io:format(\"~w~n\", [exit_probe:main()]), halt().",
        ])
        .output()
        .unwrap();
    assert!(reference.status.success());
    assert!(reference.stderr.is_empty());
    assert_eq!(
        reference.stdout,
        b"[normal,boom,killed,{returned,true,true},{returned,true,true},killed,{alive,none},boom,killed,{alive,normal},killed,badarg,badarg,true,true,{linked,boom}]\n"
    );
}

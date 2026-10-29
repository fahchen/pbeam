//! Checks the order of link and monitor signals and the `exit/2` and `exit/3` signals on PBeam
//! against the pinned ERTS.
#![expect(
    clippy::unwrap_used,
    reason = "test helpers report a failure by a panic"
)]

use std::process::Command;

use pbeam_runtime::{BeamProgram, Term};

#[path = "../common/mod.rs"]
mod common;

use common::pinned_beam;

const EXIT_SIGNAL_PROBE: &[u8] =
    include_bytes!("../../fixtures/exit_signal/exit_signal_probe.beam");

#[test]
fn delivers_link_exit_before_monitor_down_and_normal_exit_to_trapping_process() {
    // `erlang:spawn_monitor/3` runs from the pinned `erlang.beam`.
    let erlang = pinned_beam("erlang");
    let program = BeamProgram::load(&[&erlang, EXIT_SIGNAL_PROBE]).unwrap();
    let atom = |name: &str| Term::Atom(program.atom(name).unwrap());
    let pair = |tag: &str, reason: &str| Term::tuple(vec![atom(tag), atom(reason)]);
    assert_eq!(
        program
            .execute("exit_signal_probe", "main", &[])
            .unwrap()
            .term,
        Term::tuple(vec![
            Term::tuple(vec![pair("EXIT", "boom"), pair("DOWN", "boom")]),
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
    assert_eq!(reference.stderr, [] as [u8; 0]);
    assert_eq!(
        reference.stdout,
        b"{{{'EXIT',boom},{'DOWN',boom}},{'EXIT',normal}}\n"
    );
}

const EXIT_PROBE: &[u8] = include_bytes!("../../fixtures/exit_signal/exit_probe.beam");

#[test]
fn sends_exit_signals_with_exit_2_as_erts_does() {
    // `erlang:spawn_monitor/1,3` run from the pinned `erlang.beam`, and
    // `re:compile/2` gives the NIF resource reference from the pinned `re.beam`.
    let erlang = pinned_beam("erlang");
    let re = pinned_beam("re");
    let program = BeamProgram::load(&[&erlang, &re, EXIT_PROBE]).unwrap();
    let atom = |name: &str| Term::Atom(program.atom(name).unwrap());
    let returned = Term::tuple(vec![atom("returned"), atom("true"), atom("true")]);
    let pair = |tag: &str, value: &str| Term::tuple(vec![atom(tag), atom(value)]);
    assert_eq!(
        program.execute("exit_probe", "main", &[]).unwrap().term,
        Term::list_from(vec![
            atom("normal"),
            atom("boom"),
            atom("killed"),
            returned.clone(),
            returned,
            atom("killed"),
            pair("alive", "none"),
            atom("boom"),
            atom("killed"),
            Term::tuple(vec![atom("alive"), pair("normal", "true")]),
            atom("killed"),
            atom("badarg"),
            atom("badarg"),
            atom("true"),
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
    assert_eq!(reference.stderr, [] as [u8; 0]);
    assert_eq!(
        reference.stdout,
        b"[normal,boom,killed,{returned,true,true},{returned,true,true},killed,{alive,none},boom,killed,{alive,{normal,true}},killed,badarg,badarg,true,true,true,{linked,boom}]\n"
    );
}

#[test]
fn sends_exit_signals_with_exit_3_as_erts_does() {
    // `erlang:exit/3` takes a list of the atom `priority` and sends the
    // signal of `erlang:exit/2`; another option raises `badarg` with the
    // cause `badopt` before `Id` is checked (`bif.c:1727-1744`, `1877-1880`).
    let erlang = pinned_beam("erlang");
    let re = pinned_beam("re");
    let program = BeamProgram::load(&[&erlang, &re, EXIT_PROBE]).unwrap();
    let atom = |name: &str| Term::Atom(program.atom(name).unwrap());
    let pair = |tag: &str, value: &str| Term::tuple(vec![atom(tag), atom(value)]);
    let alive = |value: Term| Term::tuple(vec![atom("alive"), value]);
    let badopt = || pair("badarg", "badopt");
    assert_eq!(
        program.execute("exit_probe", "exit3", &[]).unwrap().term,
        Term::list_from(vec![
            atom("normal"),
            Term::tuple(vec![atom("returned"), atom("true"), atom("true")]),
            atom("killed"),
            alive(pair("boom", "true")),
            alive(atom("none")),
            atom("killed"),
            atom("true"),
            badopt(),
            badopt(),
            badopt(),
            badopt(),
            badopt(),
            pair("badarg", "none"),
            atom("true"),
        ])
    );
    let expected = "[normal,{returned,true,true},killed,{alive,{boom,true}},{alive,none},killed,true,{badarg,badopt},{badarg,badopt},{badarg,badopt},{badarg,badopt},{badarg,badopt},{badarg,none},true]";

    let reference = Command::new("erl")
        .args([
            "-noshell",
            "-pa",
            "fixtures/exit_signal",
            "-eval",
            "io:format(\"~w~n\", [exit_probe:exit3()]), halt().",
        ])
        .output()
        .unwrap();
    assert!(reference.status.success());
    assert_eq!(reference.stderr, [] as [u8; 0]);
    assert_eq!(
        String::from_utf8(reference.stdout).unwrap(),
        format!("{expected}\n")
    );
}

use std::process::Command;

use pbeam_runtime::{BeamProgram, Term};

#[path = "../common/mod.rs"]
mod common;

use common::pinned_beam;

const IO_CLIENT_PROBE: &[u8] = include_bytes!("../../fixtures/io_client/io_client_probe.beam");

#[test]
fn sends_an_io_request_to_a_probe_group_leader_and_flushes_monitors() {
    assert_probe_matches_erts("main");
}

/// Elixir `IO.puts/2` passes the chardata `[<<"Hello, world!">>, 10]` to
/// `io:put_chars/2`. `io:io_request/2` converts it to one binary with the
/// BIF `unicode:characters_to_binary/2`, so the probe server gets the same
/// request as for `main/0`.
#[test]
fn converts_chardata_in_the_io_request_to_one_binary() {
    assert_probe_matches_erts("list");
}

/// `erts_internal:group_leader/2` returns the atom `badarg`, and raises no
/// exception, for a target process that exited and for a leader that is not
/// a pid (`bif.c`). `erlang:group_leader/2` in `erlang.beam` then raises
/// `badarg` itself.
#[test]
fn returns_badarg_from_erts_internal_group_leader_for_a_dead_target_or_a_non_pid_leader() {
    let erlang = pinned_beam("erlang");
    let program = BeamProgram::load(&[&erlang, IO_CLIENT_PROBE]).unwrap();
    let badarg = Term::Atom(program.atom("badarg").unwrap());
    assert_eq!(
        program
            .execute("io_client_probe", "group_leader_badarg", &[])
            .unwrap(),
        Term::tuple(vec![badarg.clone(), badarg])
    );

    let reference = Command::new("erl")
        .args([
            "-noshell",
            "-pa",
            "fixtures/io_client",
            "-eval",
            "io:format(\"~p~n\", [io_client_probe:group_leader_badarg()]), halt().",
        ])
        .output()
        .unwrap();
    assert!(reference.status.success());
    assert!(reference.stderr.is_empty());
    assert_eq!(reference.stdout, b"{badarg,badarg}\n");
}

/// Run an `io_client_probe` function in PBeam with the pinned `erlang`,
/// `io`, and `erts_internal` modules, and in the pinned ERTS 29.1.1. Both
/// runs must give the probe term below. `erlang:group_leader/2` and
/// `erlang:spawn_monitor/3` run from `erlang.beam`, which ERTS preloads
/// first.
fn assert_probe_matches_erts(function: &str) {
    let erlang = pinned_beam("erlang");
    let io = pinned_beam("io");
    let erts_internal = pinned_beam("erts_internal");
    let program = BeamProgram::load(&[&erlang, &io, &erts_internal, IO_CLIENT_PROBE]).unwrap();
    let atom = |name: &str| Term::Atom(program.atom(name).unwrap());
    assert_eq!(
        program.execute("io_client_probe", function, &[]).unwrap(),
        Term::tuple(vec![
            atom("ok"),
            Term::tuple(vec![
                Term::tuple(vec![
                    atom("put_chars"),
                    atom("unicode"),
                    Term::Binary(b"Hello, world!\n".to_vec()),
                ]),
                atom("same"),
            ]),
            atom("noise_ok"),
            atom("none"),
            Term::tuple(vec![atom("true"), atom("no_down")]),
        ])
    );

    let reference = Command::new("erl")
        .args([
            "-noshell",
            "-pa",
            "fixtures/io_client",
            "-eval",
            &format!("io:format(\"~p~n\", [io_client_probe:{function}()]), halt()."),
        ])
        .output()
        .unwrap();
    assert!(reference.status.success());
    assert!(reference.stderr.is_empty());
    assert_eq!(
        String::from_utf8(reference.stdout).unwrap(),
        "{ok,{{put_chars,unicode,<<\"Hello, world!\\n\">>},same},\n    noise_ok,none,\n    {true,no_down}}\n"
    );
}

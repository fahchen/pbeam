use std::process::Command;

use pbeam_runtime::{BeamProgram, Term};

const IO_CLIENT_PROBE: &[u8] = include_bytes!("../fixtures/io_client/io_client_probe.beam");

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

/// Run an `io_client_probe` function in PBeam with the pinned `io` and
/// `erts_internal` modules, and in the pinned ERTS 29.1.1. Both runs must
/// give the probe term below.
fn assert_probe_matches_erts(function: &str) {
    let io = pinned_otp_beam("io");
    let erts_internal = pinned_erts_beam("erts_internal");
    let program = BeamProgram::load(&[&io, &erts_internal, IO_CLIENT_PROBE]).unwrap();
    let atom = |name: &str| Term::Atom(program.atom(name).unwrap());
    assert_eq!(
        program.execute("io_client_probe", function, &[]).unwrap(),
        Term::Tuple(vec![
            atom("ok"),
            Term::Tuple(vec![
                Term::Tuple(vec![
                    atom("put_chars"),
                    atom("unicode"),
                    Term::Binary(b"Hello, world!\n".to_vec()),
                ]),
                atom("same"),
            ]),
            atom("noise_ok"),
            atom("none"),
            Term::Tuple(vec![atom("true"), atom("no_down")]),
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

/// Read the pinned OTP 29.1.1 BEAM file for a kernel or stdlib module from
/// the mise-managed Erlang installation that this repository pins.
fn pinned_otp_beam(module: &str) -> Vec<u8> {
    let path = Command::new("erl")
        .args([
            "-noshell",
            "-eval",
            &format!("io:format(\"~s\", [code:which({module})]), halt()."),
        ])
        .output()
        .unwrap();
    assert!(path.status.success());
    std::fs::read(String::from_utf8(path.stdout).unwrap()).unwrap()
}

/// Read a pinned OTP 29.1.1 preloaded ERTS module, such as `erts_internal`,
/// from the `erts` application of the mise-managed Erlang installation.
/// `code:which/1` returns `preloaded` for these modules, so the path comes
/// from `code:lib_dir(erts)`.
fn pinned_erts_beam(module: &str) -> Vec<u8> {
    let path = Command::new("erl")
        .args([
            "-noshell",
            "-eval",
            &format!("io:format(\"~s/ebin/{module}.beam\", [code:lib_dir(erts)]), halt()."),
        ])
        .output()
        .unwrap();
    assert!(path.status.success());
    std::fs::read(String::from_utf8(path.stdout).unwrap()).unwrap()
}

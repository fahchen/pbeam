use std::process::Command;

use pbeam_runtime::{BeamProgram, Term};

mod common;

use common::pinned_beam;

const USER_DRV_PROBE: &[u8] = include_bytes!("../fixtures/user_drv/user_drv_probe.beam");

#[test]
fn runs_the_pinned_unicode_characters_to_binary_as_pinned_erts() {
    // The pinned `unicode` module has `bs_get_utf16` and `bs_get_utf32`.
    // `characters_to_binary/1` runs the module code, which calls
    // `unicode:characters_to_binary/2`. That call runs the BIF, not the
    // `erlang:nif_error(undef)` stub of the module.
    let unicode = pinned_beam("unicode");
    let program = BeamProgram::load(&[&unicode]).unwrap();
    assert_eq!(
        program
            .execute(
                "unicode",
                "characters_to_binary",
                &[Term::List(vec![
                    Term::Binary(b"Hello".to_vec()),
                    Term::Integer(10),
                ])],
            )
            .unwrap(),
        Term::Binary(b"Hello\n".to_vec())
    );

    let reference = Command::new("erl")
        .args([
            "-noshell",
            "-eval",
            "io:format(\"~p~n\", [unicode:characters_to_binary([<<\"Hello\">>, 10])]), halt().",
        ])
        .output()
        .unwrap();
    assert!(reference.status.success());
    assert!(reference.stderr.is_empty());
    assert_eq!(reference.stdout, b"<<\"Hello\\n\">>\n");
}

#[test]
fn runs_iolist_to_iovec_of_terms_that_are_not_lists_as_pinned_erts() {
    // `iolist_to_iovec/1` returns `[]` for `<<>>` and `[]`, and raises
    // `badarg` for `foo`, as `iolist_to_iovec_1` (`erl_iolist.c`) does.
    let program = BeamProgram::load(&[USER_DRV_PROBE]).unwrap();
    assert_eq!(
        program.execute("user_drv_probe", "iovec", &[]).unwrap(),
        Term::List(vec![
            Term::List(vec![]),
            Term::List(vec![]),
            Term::Atom(program.atom("badarg").unwrap()),
        ])
    );

    let reference = Command::new("erl")
        .args([
            "-noshell",
            "-pa",
            "fixtures/user_drv",
            "-eval",
            "io:format(\"~p~n\", [user_drv_probe:iovec()]), halt().",
        ])
        .output()
        .unwrap();
    assert!(reference.status.success());
    assert!(reference.stderr.is_empty());
    assert_eq!(reference.stdout, b"[[],[],badarg]\n");
}

#[test]
fn executes_the_pinned_user_drv_server_for_one_put_chars_sync_request() {
    // `main/0` starts a driver with `gen_statem:enter_loop/4` and the state of
    // a disabled-mode `user_drv`, and sends one `put_chars_sync` request from
    // the group. `user_drv:server/3` runs `is_ne_exact` once for each event
    // (user_drv.erl:542). The request compares the group with the group, so
    // it jumps to the fail label. The `{WriterRef, ok}` reply of the writer
    // compares the reference with the group, so it continues, and the next
    // `is_eq_exact` fails to label 160. `prim_tty:write/3` sends the iovec of
    // `erlang:iolist_to_iovec/1` to the writer. The writer loops, so the
    // monitor is active and `demonitor(MonRef, [flush])` does not trap to
    // `erts_internal`. `proc_lib`, `lists`, `erlang`, and `erts_internal` are
    // not loaded. The driver and the writer wait in `receive` after `main/0`
    // returns, and PBeam still returns the result of the root process.
    let modules = [
        "gen_statem",
        "gen",
        "user_drv",
        "prim_tty",
        "queue",
        "unicode",
    ]
    .map(pinned_beam);
    let mut beams = vec![USER_DRV_PROBE];
    beams.extend(modules.iter().map(Vec::as_slice));
    let program = BeamProgram::load(&beams).unwrap();
    assert_eq!(
        program.execute("user_drv_probe", "main", &[]).unwrap(),
        Term::Tuple(vec![
            Term::List(vec![Term::Binary(b"Hello, world!\n".to_vec())]),
            Term::Atom(program.atom("ok").unwrap()),
        ])
    );

    let reference = Command::new("erl")
        .args([
            "-noshell",
            "-pa",
            "fixtures/user_drv",
            "-eval",
            "io:format(\"~p~n~p~n\", [user_drv_probe:main(), user_drv_probe:iovec()]), halt().",
        ])
        .output()
        .unwrap();
    assert!(reference.status.success());
    assert!(reference.stderr.is_empty());
    assert_eq!(
        reference.stdout,
        b"{[<<\"Hello, world!\\n\">>],ok}\n[[],[],badarg]\n"
    );
}

#[test]
fn builds_the_state_of_a_real_disabled_user_drv_in_pinned_erts() {
    // `user_drv_state_check:main/0` starts a real disabled-mode `user_drv`
    // and compares its state with `user_drv_probe:state/3` built from the
    // same pids and reference. Only the tty handle and `ansi_regexp` come
    // from the real state. `prim_tty:init/1` sets `have_termcap` from
    // `setupterm/0` and `unicode` from the locale, so `TERM` is set to a
    // terminal that every terminfo database has. The locale of the
    // environment must be UTF-8.
    let reference = Command::new("erl")
        .env("TERM", "dumb")
        .args([
            "-noshell",
            "-nouser",
            "-pa",
            "fixtures/user_drv",
            "-eval",
            "user_drv_state_check:main(), halt().",
        ])
        .output()
        .unwrap();
    assert!(reference.status.success());
    assert!(reference.stderr.is_empty());
    assert_eq!(reference.stdout, b"true\n");
}

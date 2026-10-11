use std::process::Command;

use pbeam_runtime::{BeamProgram, Term};

#[path = "../common/mod.rs"]
mod common;

use common::pinned_beam;

const USER_DRV_PROBE: &[u8] = include_bytes!("../../fixtures/user_drv/user_drv_probe.beam");

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
                &[Term::list_from(vec![
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
        Term::list_from(vec![
            Term::nil(),
            Term::nil(),
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
fn runs_iolist_to_iovec_of_lists_as_pinned_erts() {
    // `iolist_to_iovec_engine` (`erl_iolist.c`) walks nested lists in order.
    // It copies bytes and each binary of at most 512 bytes
    // (`IOL2V_COPY_THRESHOLD`, 4096 bits) into one merged binary, and keeps
    // each larger binary as its own element. An empty merged binary is not
    // in the result. An element that is not iodata raises `badarg`, also
    // after valid elements. The list nested 100000 deep gives `[<<1>>]`. A
    // bare binary that is not empty gives `[Bin]` at every size.
    let program = BeamProgram::load(&[USER_DRV_PROBE]).unwrap();
    let badarg = Term::Atom(program.atom("badarg").unwrap());
    let ones = |size| Term::Binary(vec![255; size]);
    let bytes = |bytes: &[u8]| Term::Binary(bytes.to_vec());
    let merged = |head: &[u8], size| {
        let mut bytes = head.to_vec();
        bytes.resize(head.len() + size, 255);
        Term::Binary(bytes)
    };
    let huge = Term::Binary(vec![0; 100_000]);
    let deep = (0..100_000).fold(Term::list_from(vec![Term::Integer(1)]), |list, _| {
        Term::list_from(vec![list])
    });
    let expected = Term::list_from(vec![
        Term::nil(),
        Term::nil(),
        Term::list_from(vec![bytes(&[1, 2])]),
        Term::list_from(vec![bytes(b"ab\x03")]),
        Term::list_from(vec![bytes(&[1])]),
        Term::nil(),
        Term::list_from(vec![bytes(&[1, 2, 3, 4, 5, 6])]),
        Term::list_from(vec![ones(64)]),
        Term::list_from(vec![merged(&[1], 64)]),
        Term::list_from(vec![ones(512)]),
        Term::list_from(vec![merged(&[1], 512)]),
        Term::list_from(vec![ones(1024)]),
        Term::list_from(vec![ones(513)]),
        Term::list_from(vec![bytes(&[1]), ones(513)]),
        Term::list_from(vec![bytes(&[1]), ones(513), bytes(&[2])]),
        Term::list_from(vec![ones(513), ones(513)]),
        Term::list_from(vec![ones(513), bytes(&[1, 2]), ones(513)]),
        Term::list_from(vec![ones(513)]),
        Term::list_from(vec![ones(513)]),
        Term::list_from(vec![huge.clone()]),
        Term::list_from(vec![bytes(&[1]), huge.clone(), bytes(b"ab")]),
        Term::list_from(vec![bytes(&[1])]),
        badarg.clone(),
        badarg.clone(),
        badarg.clone(),
        badarg.clone(),
        badarg.clone(),
        badarg,
        Term::list_from(vec![ones(64)]),
        Term::list_from(vec![ones(513)]),
    ]);
    assert_eq!(
        program
            .execute(
                "user_drv_probe",
                "iolists",
                &[
                    ones(64),
                    ones(512),
                    ones(513),
                    huge,
                    deep,
                    Term::Integer(256),
                    Term::Integer(-1),
                ],
            )
            .unwrap(),
        expected
    );

    let reference = Command::new("erl")
        .args([
            "-noshell",
            "-pa",
            "fixtures/user_drv",
            "-eval",
            "io:format(\"~w~n\", [user_drv_probe:iolists(<<-1:512>>, <<-1:4096>>, <<-1:4104>>, <<0:800000>>, lists:foldl(fun(_, L) -> [L] end, [1], lists:seq(1, 100000)), 256, -1)]), halt().",
        ])
        .output()
        .unwrap();
    assert!(reference.status.success());
    assert!(reference.stderr.is_empty());
    assert_eq!(
        String::from_utf8(reference.stdout).unwrap(),
        format!("{}\n", erl_w(&expected))
    );
}

/// Format a list of binaries, lists, and atoms as `io:format("~w", ...)` does
/// on ERTS, for example `[[<<1,2>>],badarg]`.
///
/// # Examples
///
/// ```rust,ignore
/// let term = Term::list_from(vec![Term::Binary(vec![1, 2])]);
/// assert_eq!(erl_w(&term), "[<<1,2>>]");
/// ```
fn erl_w(term: &Term) -> String {
    let join = |items: Vec<String>| items.join(",");
    match term {
        Term::Binary(bytes) => format!("<<{}>>", join(bytes.iter().map(u8::to_string).collect())),
        _ if let Some(items) = term.proper_list() => {
            format!("[{}]", join(items.map(erl_w).collect()))
        }
        Term::Atom(atom) => atom.name().to_owned(),
        other => panic!("no ~w format for {other:?}"),
    }
}

#[test]
fn runs_is_ne_exact_in_an_orelse_guard_as_pinned_erts() {
    // `ne/3` has the `orelse` guard shape of `user_drv:server/3`:
    // `{test,is_ne_exact,{f,24},[{x,0},{x,1}]}` jumps to the `match` body
    // when A =:= B, and otherwise continues to `is_eq_exact` on A and C.
    // `{a, a, b}` returns `match` only through the fail jump, and
    // `{a, b, c}` returns `no_match` only when equal operands do not jump.
    let program = BeamProgram::load(&[USER_DRV_PROBE]).unwrap();
    let atom = |name| Term::Atom(program.atom(name).unwrap());
    assert_eq!(
        program.execute("user_drv_probe", "ne", &[]).unwrap(),
        Term::list_from(vec![atom("match"), atom("match"), atom("no_match")])
    );

    let reference = Command::new("erl")
        .args([
            "-noshell",
            "-pa",
            "fixtures/user_drv",
            "-eval",
            "io:format(\"~p~n\", [user_drv_probe:ne()]), halt().",
        ])
        .output()
        .unwrap();
    assert!(reference.status.success());
    assert!(reference.stderr.is_empty());
    assert_eq!(reference.stdout, b"[match,match,no_match]\n");
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
            Term::list_from(vec![Term::Binary(b"Hello, world!\n".to_vec())]),
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
    // terminal that every terminfo database has, and `LC_ALL` to a UTF-8
    // locale that macOS and glibc both have.
    let reference = Command::new("erl")
        .env("TERM", "dumb")
        .env("LC_ALL", "C.UTF-8")
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

use std::process::Command;

use pbeam_runtime::{BeamProgram, Term};

#[path = "../common/mod.rs"]
mod common;

use common::pinned_beam;

const OTP_IO_CALLS: &[u8] = include_bytes!("../../fixtures/otp_io/otp_io_calls.beam");

#[test]
fn resolves_calls_into_each_pinned_otp_io_module() {
    let io = pinned_beam("io");
    let group = pinned_beam("group");
    let user_drv = pinned_beam("user_drv");
    let prim_tty = pinned_beam("prim_tty");
    let program = BeamProgram::load(&[&io, &group, &user_drv, &prim_tty, OTP_IO_CALLS]).unwrap();

    // `otp_io_calls:io_call/0` calls `io:columns/0`, which resolves inside
    // the loaded `io` module. The first process is its own group leader, so
    // the request returns `{error, calling_self}`. `io:columns/1` then fails
    // its `is_integer` test (opcode 45) and returns `{error, enotsup}`
    // (`io.erl:297-303`), as pinned ERTS does for a process that is its own
    // group leader. The result proves the call ran `io`'s own code.
    assert_eq!(
        program
            .execute("otp_io_calls", "io_call", &[])
            .unwrap()
            .term,
        Term::tuple(vec![
            Term::Atom(program.atom("error").unwrap()),
            Term::Atom(program.atom("enotsup").unwrap()),
        ])
    );
    let reference = Command::new("erl")
        .args([
            "-noshell",
            "-pa",
            "fixtures/otp_io",
            "-eval",
            "group_leader(self(), self()), io:format(standard_error, \"~w~n\", [otp_io_calls:io_call()]), halt().",
        ])
        .output()
        .unwrap();
    assert!(reference.status.success());
    assert!(reference.stdout.is_empty());
    assert_eq!(reference.stderr, b"{error,enotsup}\n");

    // `group:whereis_shell/0` resolves inside the loaded `group` module,
    // calls `erlang:group_leader/0`, runs `node/1` (`bif1`) and `node/0`
    // (`bif0`), and finds that the group leader is on the local node. It
    // then calls `user_drv:whereis_group/0`. `user_drv:whereis_group/0`
    // resolves inside the loaded `user_drv` module. No `user_drv` process
    // runs, so `whereis(user_drv)` gives `undefined`, and
    // `process_info(undefined, dictionary)` raises `badarg`
    // (`user_drv.erl:159-162`). The pinned ERTS run without `user_drv`
    // (`-nouser`) below gives the same `error:badarg` and the same frames.
    // Each frame of an OTP module has the `file` and `line` location of
    // its call, as in ERTS.
    let bif_frame = "{erlang,process_info,[undefined,dictionary],\
                     [{error_info,#{module=>erl_erts_errors}}]}";
    for (function, callers) in [
        (
            "group_call",
            "{user_drv,whereis_group,0,[{file,\"user_drv.erl\"},{line,161}]},\
             {group,whereis_shell,0,[{file,\"group.erl\"},{line,185}]}",
        ),
        (
            "user_drv_call",
            "{user_drv,whereis_group,0,[{file,\"user_drv.erl\"},{line,161}]}",
        ),
    ] {
        let error = program.execute("otp_io_calls", function, &[]).unwrap_err();
        assert_eq!(
            error.to_string(),
            format!("root process exited with {{badarg,[{bif_frame},{callers}]}}"),
            "{function}"
        );
    }
    let reference = Command::new("erl")
        .args([
            "-noshell",
            "-nouser",
            "-pa",
            "fixtures/otp_io",
            "-eval",
            "io:format(standard_error, \"~w~n\", [[try otp_io_calls:F() of V -> {ok, V} catch C:R:S -> {C, R, [{M, Fn, A} || {M, Fn, A, _} <- S, M =/= erl_eval, M =/= init]} end || F <- [group_call, user_drv_call]]]), halt().",
        ])
        .output()
        .unwrap();
    assert!(reference.status.success());
    assert!(reference.stdout.is_empty());
    assert_eq!(
        reference.stderr,
        b"[{error,badarg,[{erlang,process_info,[undefined,dictionary]},\
{user_drv,whereis_group,0},{group,whereis_shell,0}]},\
{error,badarg,[{erlang,process_info,[undefined,dictionary]},{user_drv,whereis_group,0}]}]\n"
    );

    // `prim_tty:ansi_regexp/0` resolves inside the loaded `prim_tty` module
    // and returns its literal binary without reaching any unresolved call.
    // The expected bytes come from a pinned OTP 29.1.1 run of
    // `prim_tty:ansi_regexp/0`.
    assert_eq!(
        program
            .execute("otp_io_calls", "prim_tty_call", &[])
            .unwrap()
            .term,
        Term::binary(ANSI_REGEXP.to_vec())
    );
}

const ANSI_REGEXP: &[u8] = &[
    94, 91, 27, 194, 155, 93, 91, 91, 92, 93, 40, 41, 35, 59, 63, 93, 42, 40, 63, 58, 40, 63, 58,
    40, 63, 58, 40, 63, 58, 59, 91, 45, 97, 45, 122, 65, 45, 90, 92, 100, 92, 47, 35, 38, 46, 58,
    61, 63, 37, 64, 126, 95, 93, 43, 41, 42, 124, 91, 97, 45, 122, 65, 45, 90, 92, 100, 93, 43, 40,
    63, 58, 59, 91, 45, 97, 45, 122, 65, 45, 90, 92, 100, 92, 47, 35, 38, 46, 58, 61, 63, 37, 64,
    126, 95, 93, 42, 41, 42, 41, 63, 7, 41, 124, 40, 63, 58, 40, 63, 58, 92, 100, 123, 49, 44, 52,
    125, 40, 63, 58, 59, 92, 100, 123, 48, 44, 52, 125, 41, 42, 41, 63, 91, 92, 100, 65, 45, 80,
    82, 45, 84, 90, 99, 102, 45, 110, 113, 45, 117, 121, 61, 62, 60, 126, 93, 41, 41,
];

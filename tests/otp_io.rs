use std::process::Command;

use pbeam_runtime::{BeamProgram, Term};

const OTP_IO_CALLS: &[u8] = include_bytes!("../fixtures/otp_io/otp_io_calls.beam");

#[test]
fn resolves_calls_into_each_pinned_otp_io_module() {
    let io = pinned_otp_beam("io");
    let group = pinned_otp_beam("group");
    let user_drv = pinned_otp_beam("user_drv");
    let prim_tty = pinned_otp_beam("prim_tty");
    let program = BeamProgram::load(&[&io, &group, &user_drv, &prim_tty, OTP_IO_CALLS]).unwrap();

    // `otp_io_calls:io_call/0` calls `io:columns/0`, which resolves inside
    // the loaded `io` module. The first process is its own group leader, so
    // the request returns `{error, calling_self}`, and `io:columns/1` then
    // reaches its `is_integer` test (opcode 45), which PBeam does not run
    // yet. The error proves the call ran `io`'s own code.
    let error = program.execute("otp_io_calls", "io_call", &[]).unwrap_err();
    assert!(
        error.to_string().contains("unsupported BEAM opcode 45"),
        "expected resolution into io, got {error}"
    );

    // `group:whereis_shell/0` resolves inside the loaded `group` module,
    // calls `erlang:group_leader/0`, runs `node/1` (`bif1`) and `node/0`
    // (`bif0`), and finds that the group leader is on the local node. It
    // then calls `user_drv:whereis_group/0`, which reaches the unimplemented
    // process registry lookup.
    let error = program
        .execute("otp_io_calls", "group_call", &[])
        .unwrap_err();
    assert!(
        error
            .to_string()
            .contains("unresolved external MFA erlang:whereis/1"),
        "expected resolution into group, got {error}"
    );

    // `user_drv:whereis_group/0` resolves inside the loaded `user_drv`
    // module and reaches the unimplemented process registry lookup.
    let error = program
        .execute("otp_io_calls", "user_drv_call", &[])
        .unwrap_err();
    assert!(
        error
            .to_string()
            .contains("unresolved external MFA erlang:whereis/1"),
        "expected resolution into user_drv, got {error}"
    );

    // `prim_tty:ansi_regexp/0` resolves inside the loaded `prim_tty` module
    // and returns its literal binary without reaching any unresolved call.
    // The expected bytes come from a pinned OTP 29.1.1 run of
    // `prim_tty:ansi_regexp/0`.
    assert_eq!(
        program
            .execute("otp_io_calls", "prim_tty_call", &[])
            .unwrap(),
        Term::Binary(ANSI_REGEXP.to_vec())
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

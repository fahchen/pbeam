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
    // the loaded `io` module and reaches the unimplemented group-leader
    // lookup. The error proves the call crossed into `io`'s own code.
    let error = program.execute("otp_io_calls", "io_call", &[]).unwrap_err();
    assert!(
        error
            .to_string()
            .contains("unresolved external MFA erlang:group_leader/0"),
        "expected resolution into io, got {error}"
    );

    // `group:whereis_shell/0` resolves inside the loaded `group` module and
    // reaches the same group-leader boundary as `io`, which `group` also
    // calls directly.
    let error = program
        .execute("otp_io_calls", "group_call", &[])
        .unwrap_err();
    assert!(
        error
            .to_string()
            .contains("unresolved external MFA erlang:group_leader/0"),
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
    assert!(matches!(
        program
            .execute("otp_io_calls", "prim_tty_call", &[])
            .unwrap(),
        Term::Binary(_)
    ));
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

//! Checks that the display bytes of the `pbeam` binary match the pinned ERTS.

use std::{
    process::{Command, Stdio},
    time::Duration,
};

#[expect(dead_code, reason = "this test crate does not use a broken pipe")]
#[path = "../common/run.rs"]
mod run;

use run::run_dumb_utf8;

const RUN_TIME_LIMIT: Duration = Duration::from_secs(10);

#[test]
fn writes_the_display_bytes_of_pinned_erts() {
    // `erlang:display/1` writes the `%T` text and `\r\n` to the standard
    // output (`bif.c:4144-4148`; `erts_printf_add_cr_to_stdout` in
    // `sys/unix/sys.c:342`). `erlang:display_string/2` writes the UTF-8
    // bytes of the string to the chosen descriptor without a newline
    // (`bif.c:4192-4285`).
    let erts = run_dumb_utf8(
        Command::new("erl").args([
            "-noshell",
            "-pa",
            "fixtures/diagnostics",
            "-eval",
            "ok = diagnostics_probe:display(), halt().",
        ]),
        Stdio::piped(),
        RUN_TIME_LIMIT,
    );
    let pbeam = run_dumb_utf8(
        Command::new(env!("CARGO_BIN_EXE_pbeam")).args([
            "diagnostics_probe",
            "display",
            "--",
            "fixtures/diagnostics/diagnostics_probe.beam",
        ]),
        Stdio::piped(),
        RUN_TIME_LIMIT,
    );
    assert_eq!(
        (
            erts.stdout.as_slice(),
            erts.stderr.as_slice(),
            erts.status.code(),
            pbeam.stdout.as_slice(),
            pbeam.stderr.as_slice(),
            pbeam.status.code(),
        ),
        (
            &b"{hello,\"world\"}\r\nout\xf0\x9f\x98\x80\n"[..],
            &b"err"[..],
            Some(0),
            &b"{hello,\"world\"}\r\nout\xf0\x9f\x98\x80\n"[..],
            &b"err"[..],
            Some(0),
        )
    );
}

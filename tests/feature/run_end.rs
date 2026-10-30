//! Checks how a `pbeam` run ends: root exit, `halt` status code and crash dump slogan.
#![expect(
    clippy::unwrap_used,
    reason = "test helpers report a failure by a panic"
)]

use std::{
    process::{Command, Stdio},
    time::Duration,
};

#[path = "../common/boot.rs"]
mod boot;
#[expect(
    dead_code,
    reason = "this test crate gives BEAM paths, not bytes, to the binary"
)]
#[path = "../common/mod.rs"]
mod common;
#[expect(dead_code, reason = "this test crate does not use a broken pipe")]
#[path = "../common/run.rs"]
mod run;

use boot::{BOOT_MODULES, boot_beam, otp_root};
use common::pinned_beam_path;
use run::run_dumb_utf8;

const RUN_END_PROBE: &str = "fixtures/run_end/run_end_probe.beam";
const RUN_TIME_LIMIT: Duration = Duration::from_secs(10);

#[test]
fn ends_the_run_at_the_root_exit_while_a_timer_stays_active() {
    // Each function of `run_end_probe` starts a process that starts a
    // 5000 ms timer again at each expiry, as the `repeated_filesync` of
    // `logger_h_common` does (logger_h_common.erl:298-311, 454-457), and a
    // process that writes "late\n" after 300 ms. ERTS stops the node at
    // `halt()` after `-eval` (erlang.erl:3357-3358), so "late\n" does not
    // print. When the `-eval` expression raises, `start_it/1` writes the
    // `Error!` line and raises again (init.erl:1552-1567), and `init` halts
    // with status 1 at the boot process exit that is not `normal`
    // (init.erl:710-712, 678-679). PBeam ends the run at the root process
    // exit with the same stdout and status, and writes its own stderr line
    // (see "Run end" in docs/erts-differences.md).
    let beams = BOOT_MODULES.map(pinned_beam_path);
    for (function, erts_stderr, pbeam_stderr, stdout, status) in [
        ("main", "", "", "Hello, world!\n", 0),
        (
            "crash",
            "Error! Failed to eval: run_end_probe:crash(), halt().\r\n\r\nRuntime terminating during boot (boom)\r\n",
            "pbeam: root process exited with boom\n",
            "",
            1,
        ),
        (
            "killed",
            "Runtime terminating during boot (killed)\r\n",
            "pbeam: root process exited with killed\n",
            "",
            1,
        ),
    ] {
        // Without `ERL_CRASH_DUMP_SECONDS=0`, ERTS writes `erl_crash.dump`
        // into the working directory for the crash cases.
        let erts = run_dumb_utf8(
            Command::new("erl")
                .args([
                    "-noshell",
                    "-pa",
                    "fixtures/run_end",
                    "-eval",
                    &format!("run_end_probe:{function}(), halt()."),
                ])
                .env("ERL_CRASH_DUMP_SECONDS", "0"),
            Stdio::piped(),
            RUN_TIME_LIMIT,
        );
        let pbeam = run_dumb_utf8(
            Command::new(env!("CARGO_BIN_EXE_pbeam"))
                .args([
                    "pbeam_boot",
                    "start",
                    "run_end_probe",
                    function,
                    "--root",
                    otp_root().to_str().unwrap(),
                    "--",
                    RUN_END_PROBE,
                ])
                .args(&beams)
                .arg(boot_beam()),
            Stdio::piped(),
            RUN_TIME_LIMIT,
        );
        for (name, output, stderr) in [("ERTS", erts, erts_stderr), ("PBeam", pbeam, pbeam_stderr)]
        {
            assert_eq!(
                (
                    String::from_utf8_lossy(&output.stdout).as_ref(),
                    String::from_utf8_lossy(&output.stderr).as_ref(),
                    output.status.code()
                ),
                (stdout, stderr, Some(status)),
                "{name} {function}"
            );
        }
    }
}

#[test]
fn exits_with_the_low_8_bits_of_the_halt_status_code() {
    // `halt_probe:status/0` calls `halt(259)`. ERTS exits with the status
    // code through the C `exit` (bif.c:4346-4362), so the host status is
    // 3, and nothing is written (tests/integration/halt.rs).
    let output = run_dumb_utf8(
        Command::new(env!("CARGO_BIN_EXE_pbeam"))
            .args([
                "halt_probe",
                "status",
                "--",
                "fixtures/halt/halt_probe.beam",
            ])
            .arg(pinned_beam_path("erlang")),
        Stdio::piped(),
        RUN_TIME_LIMIT,
    );
    assert_eq!(
        (
            output.status.code(),
            output.stdout.as_slice(),
            output.stderr.as_slice()
        ),
        (Some(3), &b""[..], &b""[..])
    );
}

#[test]
fn writes_the_slogan_of_a_crash_dump_halt_as_pinned_erts() {
    // `halt/1` with a string writes the slogan, cut to 1023 characters, as
    // UTF-8 and a newline to stderr, with `\r\n` for each `\n`, and exits
    // with status 1 (bif.c:4372-4386, erl_init.c:2712-2713, 2738-2739,
    // sys/unix/sys.c:343). With `ERL_CRASH_DUMP_SECONDS=0`, ERTS writes no
    // crash dump and no line about it; PBeam never writes one.
    for (function, stderr) in [
        ("line_slogan", "\u{e9}\r\nb\r\n".to_owned()),
        ("long_slogan", format!("{}\r\n", "a".repeat(1023))),
    ] {
        let erts = run_dumb_utf8(
            Command::new("erl")
                .args([
                    "-noshell",
                    "-pa",
                    "fixtures/halt",
                    "-eval",
                    &format!("halt_probe:{function}()."),
                ])
                .env("ERL_CRASH_DUMP_SECONDS", "0"),
            Stdio::piped(),
            RUN_TIME_LIMIT,
        );
        let pbeam = run_dumb_utf8(
            Command::new(env!("CARGO_BIN_EXE_pbeam"))
                .args([
                    "halt_probe",
                    function,
                    "--",
                    "fixtures/halt/halt_probe.beam",
                ])
                .arg(pinned_beam_path("erlang")),
            Stdio::piped(),
            RUN_TIME_LIMIT,
        );
        for (name, output) in [("ERTS", erts), ("PBeam", pbeam)] {
            assert_eq!(
                (
                    output.status.code(),
                    String::from_utf8_lossy(&output.stdout).as_ref(),
                    String::from_utf8_lossy(&output.stderr).as_ref()
                ),
                (Some(1), "", stderr.as_str()),
                "{name} {function}"
            );
        }
    }
}

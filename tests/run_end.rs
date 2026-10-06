use std::{
    process::{Command, Output},
    time::Duration,
};

#[path = "common/boot.rs"]
mod boot;
#[expect(
    dead_code,
    reason = "this test crate gives BEAM paths, not bytes, to the binary"
)]
mod common;
#[path = "common/run.rs"]
mod run;

use boot::{BOOT_BEAM, BOOT_MODULES};
use common::pinned_beam_path;
use run::run_with_time_limit;

const RUN_END_PROBE: &str = "fixtures/run_end/run_end_probe.beam";
const RUN_TIME_LIMIT: Duration = Duration::from_secs(10);

/// Run a command with `TERM=dumb` and `LC_ALL=C.UTF-8`, and return its
/// output. Kill it and panic when it does not end in `RUN_TIME_LIMIT`.
///
/// # Examples
///
/// ```rust,ignore
/// let output = run_to_end(Command::new("erl").args(["-noshell", "-eval", "halt()."]));
/// assert_eq!(output.status.code(), Some(0));
/// ```
fn run_to_end(command: &mut Command) -> Output {
    run_with_time_limit(
        command.env("TERM", "dumb").env("LC_ALL", "C.UTF-8"),
        RUN_TIME_LIMIT,
    )
}

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
            "pbeam: root process exited with Atom(\"boom\")\n",
            "",
            1,
        ),
        (
            "killed",
            "Runtime terminating during boot (killed)\r\n",
            "pbeam: root process exited with Atom(\"killed\")\n",
            "",
            1,
        ),
    ] {
        // Without `ERL_CRASH_DUMP_SECONDS=0`, ERTS writes `erl_crash.dump`
        // into the working directory for the crash cases.
        let erts = run_to_end(
            Command::new("erl")
                .args([
                    "-noshell",
                    "-pa",
                    "fixtures/run_end",
                    "-eval",
                    &format!("run_end_probe:{function}(), halt()."),
                ])
                .env("ERL_CRASH_DUMP_SECONDS", "0"),
        );
        let pbeam = run_to_end(
            Command::new(env!("CARGO_BIN_EXE_pbeam"))
                .args([
                    "pbeam_boot",
                    "start",
                    "run_end_probe",
                    function,
                    "--",
                    RUN_END_PROBE,
                ])
                .args(&beams)
                .arg(BOOT_BEAM),
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

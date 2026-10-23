use std::{
    path::PathBuf,
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

const AT_EXIT_PROBE: &str = "fixtures/at_exit/at_exit_probe.beam";
const RUN_TIME_LIMIT: Duration = Duration::from_secs(20);

/// Return the library directory of the pinned Elixir, which holds the
/// `elixir` and `logger` applications.
fn elixir_lib() -> PathBuf {
    let output = Command::new("elixir")
        .args(["-e", "IO.write(:code.lib_dir(:elixir))"])
        .output()
        .unwrap();
    assert!(output.status.success(), "{output:?}");
    PathBuf::from(String::from_utf8(output.stdout).unwrap())
        .parent()
        .unwrap()
        .to_path_buf()
}

/// Replace the `HH:MM:SS.mmm` time at the start of each line with `<time>`.
fn mask_time(text: &str) -> String {
    text.lines()
        .map(|line| match line.split_at_checked(12) {
            Some((time, rest)) if time.as_bytes()[8] == b'.' && time.as_bytes()[2] == b':' => {
                format!("<time>{rest}\n")
            }
            _ => format!("{line}\n"),
        })
        .collect()
}

#[test]
fn runs_the_at_exit_callbacks_and_flushes_the_logger_after_the_entry_returns() {
    // `elixir -e` runs the command and then `Kernel.CLI.run/1` runs the
    // `at_exit` callbacks, the newest first, each with the status 0, then
    // calls `Logger.flush()`, and then `System.halt(0)`
    // (kernel/cli.ex:58-71, 129-133; system.ex:474-477). The flush calls
    // `filesync/1` of each handler, so the event in the queue of the async
    // default handler is written before the halt (logger.ex:785-796). The
    // `pbeam_boot` root does the same after the entry returns, when the
    // elixir application runs.
    let lib = elixir_lib();
    let beams = BOOT_MODULES.map(pinned_beam_path);
    let erts = run_dumb_utf8(
        Command::new("elixir").args(["-pa", "fixtures/at_exit", "-e", ":at_exit_probe.main()"]),
        Stdio::piped(),
        RUN_TIME_LIMIT,
    );
    let pbeam = run_dumb_utf8(
        Command::new(env!("CARGO_BIN_EXE_pbeam"))
            .args(["pbeam_boot", "start", "at_exit_probe", "main", "--root"])
            .arg(otp_root())
            .arg("--pa")
            .arg(lib.join("elixir/ebin"))
            .arg("--pa")
            .arg(lib.join("logger/ebin"))
            .args(["--", AT_EXIT_PROBE])
            .args(&beams)
            .arg(boot_beam()),
        Stdio::piped(),
        RUN_TIME_LIMIT,
    );
    for (name, output) in [("ERTS", erts), ("PBeam", pbeam)] {
        assert_eq!(
            (
                mask_time(&String::from_utf8_lossy(&output.stdout)).as_str(),
                String::from_utf8_lossy(&output.stderr).as_ref(),
                output.status.code()
            ),
            (
                "\n<time> [warning] last event\n",
                "second 0\nfirst 0\n",
                Some(0)
            ),
            "{name}"
        );
    }
}

#[test]
fn keeps_a_line_without_a_time() {
    assert_eq!(mask_time("ab\n12:34:56,789 x\n"), "ab\n12:34:56,789 x\n");
}

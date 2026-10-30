//! Compare `erlang:process_info/2` of the pinned ERTS with the same call
//! on the `pbeam` binary. The probes print registered names and item values
//! that need no pid of another run.
#![expect(
    clippy::unwrap_used,
    reason = "test helpers report a failure by a panic"
)]

use std::process::Command;

#[path = "../common/boot.rs"]
mod boot;
#[expect(
    dead_code,
    reason = "this test crate gives BEAM paths, not bytes, to the binary"
)]
#[path = "../common/mod.rs"]
mod common;

use boot::{BOOT_MODULES, boot_beam, otp_root};
use common::pinned_beam_path;

const PROCESS_INFO_PROBE: &str = "fixtures/boot/process_info_probe.beam";

#[test]
fn reads_the_messages_of_a_process_as_pinned_erts() {
    // The `messages` item gives the queue in arrival order: a sleeping
    // child holds [a,b], the caller holds [] (`erl_bif_info.c`).
    assert_eq!(run(&mut erts("messages")), run(&mut pbeam("messages")));
}

#[test]
fn reads_the_items_of_another_process_as_pinned_erts() {
    // A read of another process reports each item from the values that
    // `process_info/1` reads: a child in `wait/0` waits with an empty
    // dictionary, a false `trap_exit`, the default error handler, its
    // current function, and its start MFA (`erl_bif_info.c`).
    assert_eq!(run(&mut erts("items")), run(&mut pbeam("items")));
}

#[test]
fn reads_the_links_of_a_process_as_pinned_erts() {
    // `process_info(Pid, links)` gives `{links, [Pid | Port]}` from the
    // link tree, without a link that waits for its unlink acknowledgment,
    // in the reverse of the tree walk (`erl_bif_info.c:574-585`,
    // `1764-1775`). The probe checks the caller and another process with
    // the same two nameable links, so the boot links do not differ. It
    // also prints `ok` when reads of a target with 4096 links cost more
    // reductions than reads of a target without links, which checks the
    // `floor(Links / 4)` charge of the item (`erl_bif_info.c:1424,1777`).
    for function in ["links", "charges"] {
        let erts = run(&mut erts(function));
        let pbeam = run(&mut pbeam(function));
        assert_eq!(erts, pbeam, "{function}");
        assert_eq!(erts.0, Some(0), "{function}");
        assert_eq!(erts.2, "", "{function}");
    }
}

/// The exit status code, the stdout, and the stderr of a run.
type Output = (Option<i32>, String, String);

/// Run `command` and return its exit status, its stdout, and its stderr.
fn run(command: &mut Command) -> Output {
    let output = command.output().expect("the command runs");
    (
        output.status.code(),
        String::from_utf8_lossy(&output.stdout).into_owned(),
        String::from_utf8_lossy(&output.stderr).into_owned(),
    )
}

/// Return the `erl` command that runs `process_info_probe:Function()`.
fn erts(function: &str) -> Command {
    let mut command = Command::new("erl");
    command
        .args([
            "-noshell",
            "-pa",
            "fixtures/boot",
            "-eval",
            &format!("process_info_probe:{function}(), halt()."),
        ])
        .env("ERL_CRASH_DUMP_SECONDS", "0")
        .env("TERM", "dumb")
        .env("LC_ALL", "C.UTF-8");
    command
}

/// Return the `pbeam` command that boots with `pbeam_boot` and runs
/// `process_info_probe:Function()`.
fn pbeam(function: &str) -> Command {
    let mut command = Command::new(env!("CARGO_BIN_EXE_pbeam"));
    command
        .args([
            "pbeam_boot",
            "start",
            "process_info_probe",
            function,
            "--root",
            otp_root().to_str().unwrap(),
            "--",
            PROCESS_INFO_PROBE,
        ])
        .args(BOOT_MODULES.map(pinned_beam_path))
        .arg(boot_beam())
        .env("TERM", "dumb")
        .env("LC_ALL", "C.UTF-8");
    command
}

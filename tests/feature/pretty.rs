//! `io_lib:format/2` with `~p` and `~w` under the PBeam boot.
#[path = "../common/boot.rs"]
mod boot;
#[expect(
    dead_code,
    reason = "this test crate gives BEAM paths, not bytes, to the binary"
)]
#[path = "../common/mod.rs"]
mod common;

use std::process::Command;

use boot::{BOOT_MODULES, boot_beam, otp_root};
use common::pinned_beam_path;

/// The probe that writes the `~p` text and the `~w` text of its terms.
const PRETTY_PROBE: &str = "fixtures/pretty/pretty_probe";

/// The pinned OTP modules that `io:format/2` with `~p` calls after the
/// boot, in addition to `BOOT_MODULES`. `io_lib_pretty` calls
/// `epp:default_encoding/0` (`io_lib_pretty.erl:102`), and `io_lib` quotes
/// an atom with `erl_scan:reserved_word/1`, which calls
/// `erl_features:keywords/0` (`erl_scan.erl:1357`).
const FORMAT_MODULES: [&str; 4] = ["io_lib_pretty", "erl_scan", "erl_features", "epp"];

/// Run `command` and return its stdout. The run must exit with status 0
/// and write nothing to stderr.
///
/// # Examples
///
/// ```rust,ignore
/// assert_eq!(run(Command::new("echo").arg("a")), "a\n");
/// ```
fn run(command: &mut Command) -> String {
    let output = command
        .env("TERM", "dumb")
        .env("LC_ALL", "C.UTF-8")
        .output()
        .unwrap();
    assert!(
        output.status.success() && output.stderr.is_empty(),
        "{command:?}: {output:?}"
    );
    String::from_utf8(output.stdout).unwrap()
}

#[test]
fn formats_terms_with_p_and_w_as_pinned_erts() {
    // `~p` and `~w` of a term test it with the guard `is_record/1`
    // (`io_lib:write1/4`, io_lib.erl:843; `io_lib_pretty:print_length/7`,
    // io_lib_pretty.erl:938), the `is_any_native_record` instruction. No
    // term is a native record, so each term takes its own clause.
    let reference = run(Command::new("erl").args([
        "-noshell",
        "-pa",
        "fixtures/pretty",
        "-eval",
        "pretty_probe:main(), halt().",
    ]));
    let text = run(Command::new(env!("CARGO_BIN_EXE_pbeam"))
        .args([
            "pbeam_boot",
            "start",
            "pretty_probe",
            "main",
            "--root",
            otp_root().to_str().unwrap(),
            "--",
        ])
        .arg(format!("{PRETTY_PROBE}.beam"))
        .arg(boot_beam())
        .args(
            BOOT_MODULES
                .iter()
                .chain(&FORMAT_MODULES)
                .map(|module| pinned_beam_path(module)),
        ));
    assert_eq!(text, reference);
}

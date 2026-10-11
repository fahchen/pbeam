//! The export entry of a function that a module does not export calls
//! `error_handler:undefined_function/3` (`beam_common.c:1294-1342`), and that
//! function loads the module through `code:ensure_loaded/1` or
//! `init:ensure_loaded/1` (`error_handler.erl:85-98`, `182-198`). The boot of
//! `examples/hello_world` answers `{ensure_loaded, Module}` for the init
//! process (`init.erl:730-750`).

use std::{
    path::{Path, PathBuf},
    process::{Command, Output, Stdio},
    sync::OnceLock,
};

#[path = "../common/boot.rs"]
mod boot;
#[expect(
    dead_code,
    reason = "this test crate gives BEAM paths, not bytes, to the binary"
)]
#[path = "../common/mod.rs"]
mod common;

use boot::{BOOT_MODULES, boot_beam};
use common::pinned_beam_path;

/// The probe that calls a module that the run does not load. The module
/// `error_handler_load_target` is on the code path of the run only, so the
/// call goes through the error handler of the process.
const LOAD_PROBE: &str = "fixtures/error_handler/error_handler_load_probe";

/// The code path directory of the runs. It has
/// `error_handler_load_target.beam`, which no run loads at its start.
const CODE_PATH: &str = "fixtures/error_handler";

/// The pinned OTP modules that the run needs in addition to `BOOT_MODULES`:
/// `code` and `code_server`, the modules that `code:do_start/0` loads before
/// it starts `code_server` and that `BOOT_MODULES` does not have
/// (code.erl:1699-1712), and the modules that `io:format/2` with `~p` calls.
const CODE_SERVER_MODULES: [&str; 12] = [
    "code",
    "code_server",
    "beam_lib",
    "binary",
    "ets",
    "filename",
    "gb_sets",
    "gb_trees",
    "io_lib_pretty",
    "erl_scan",
    "erl_features",
    "epp",
];

#[test]
fn loads_a_module_that_the_run_does_not_load_as_pinned_erts() {
    // `code:ensure_loaded/1` asks `code_server` for the object code of a
    // module that is not loaded, and `error_handler:undefined_function/3`
    // then runs the function (`code.erl:581-613`;
    // `code_server.erl:1112-1143,1303-1384`). The ERTS run gives
    // `-boot start_clean` and `-pa CODE_PATH`, as `pbeam_boot` does, so the
    // two runs have the same code path.
    let erts = run(&mut erts());
    assert_eq!(erts, "{loaded,a}\n");
    assert_eq!(run(&mut pbeam()), erts);
}

/// Return the root directory of the pinned OTP, as `code:root_dir/0` gives
/// it, from one `erl` run for each test process.
///
/// # Examples
///
/// ```rust,ignore
/// assert!(root().join("lib").is_dir());
/// ```
fn root() -> &'static Path {
    static ROOT: OnceLock<PathBuf> = OnceLock::new();
    ROOT.get_or_init(|| {
        let output = Command::new("erl")
            .args([
                "-noshell",
                "-eval",
                "io:put_chars(code:root_dir()), halt().",
            ])
            .output()
            .unwrap();
        assert!(output.status.success(), "{output:?}");
        PathBuf::from(String::from_utf8(output.stdout).unwrap())
    })
}

/// Return the command of the probe on the pinned ERTS with the code path
/// directory.
///
/// # Examples
///
/// ```rust,ignore
/// let output = run(&mut erts());
/// ```
fn erts() -> Command {
    let mut command = Command::new("erl");
    command.args([
        "+S",
        "1",
        "-noshell",
        "-boot",
        "start_clean",
        "-pa",
        CODE_PATH,
        "-eval",
        "error_handler_load_probe:main().",
    ]);
    command
}

/// Return the command of the probe on the `pbeam` binary with `--root` and
/// the code path directory. The start modules hold no
/// `error_handler_load_target`.
///
/// # Examples
///
/// ```rust,ignore
/// let output = run(&mut pbeam());
/// ```
fn pbeam() -> Command {
    let mut command = Command::new(env!("CARGO_BIN_EXE_pbeam"));
    command
        .args([
            "pbeam_boot",
            "start",
            "error_handler_load_probe",
            "main",
            "--root",
        ])
        .arg(root())
        .args(["--pa", CODE_PATH])
        .arg("--")
        .arg(format!("{LOAD_PROBE}.beam"))
        .arg(boot_beam())
        .args(
            BOOT_MODULES
                .iter()
                .chain(&CODE_SERVER_MODULES)
                .map(|module| pinned_beam_path(module)),
        );
    command
}

/// Run a command and return its stdout. The run must exit with status 0 and
/// write nothing to stderr.
///
/// # Examples
///
/// ```rust,ignore
/// assert_eq!(run(Command::new("echo").arg("a")), "a\n");
/// ```
fn run(command: &mut Command) -> String {
    let output: Output = command
        .env("TERM", "dumb")
        .env("LC_ALL", "C.UTF-8")
        .stdin(Stdio::null())
        .output()
        .unwrap();
    assert!(
        output.status.success() && output.stderr.is_empty(),
        "{command:?}: {output:?}"
    );
    String::from_utf8(output.stdout).unwrap()
}

use std::process::{Command, Output, Stdio};

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

/// The probe that writes the code server results.
const CODE_SERVER_PROBE: &str = "fixtures/code_server/code_server_probe";

/// The code path directory of the runs. It has `code_server_target.beam`,
/// which no run loads at its start.
const CODE_PATH: &str = "fixtures/code_server";

/// The pinned OTP modules that the run needs in addition to `BOOT_MODULES`:
/// `code` and `code_server`, the modules that `code:do_start/0` loads
/// before it starts `code_server` and that `BOOT_MODULES` does not have
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

/// The pinned OTP modules that the kernel start of `pbeam_boot` calls in
/// addition to `BOOT_MODULES` and `CODE_SERVER_MODULES`: the
/// `application_master` of `kernel`, the `kernel_sup` children and the
/// modules that they call, `c` of `c:erlangrc/0`, and `prim_inet`, a
/// preloaded module of ERTS. In embedded mode, `code_server` loads no module
/// on demand (code.erl:601-602), so the run must hold them, as the
/// `primLoad` commands of `start_clean.boot` load them in ERTS
/// (init.erl:1335-1346).
const KERNEL_START_MODULES: [&str; 20] = [
    "application_master",
    "c",
    "erl_distribution",
    "erpc",
    "file",
    "global",
    "global_group",
    "inet",
    "inet_config",
    "inet_db",
    "inet_parse",
    "inet_udp",
    "kernel",
    "kernel_config",
    "kernel_refc",
    "net_kernel",
    "peer",
    "prim_inet",
    "rand",
    "rpc",
];

#[test]
fn starts_code_server_and_loads_a_module_from_the_code_path_as_pinned_erts() {
    // With `--root`, the kernel start of `pbeam_boot` runs
    // `code:start_link/0` as the first `kernel_sup` child
    // (kernel.erl:110-115), and the boot first sets the code path of the
    // last `path` command of `start_clean.boot` (init.erl:1323-1332).
    // `code_server` makes the interactive path of `Root/lib`
    // (code_server.erl:91-125, 513-535). `code:ensure_loaded/1` of a module
    // that is not loaded asks `code_server` for its object code, prepares
    // it, and lets `code_server` finish the load (code.erl:581-613;
    // code_server.erl:1112-1143, 1303-1384). A module that no path
    // directory has gives `{error,nofile}`. `code:which/1` gives the file of
    // a module that `code_server` loaded, `preloaded` for a module of
    // `erlang:pre_loaded/0`, and else the first file on the path
    // (code.erl:1782-1789). The ERTS run gives `-boot start_clean`, so that
    // its boot path is that of `pbeam_boot`, and loads the probe with
    // `code:load_abs/1`, so that `code_server` has no other module of the
    // code path.
    let erts = run(&mut erts(&[]));
    assert!(
        erts.starts_with(
            "[interactive,\n [false,\n  {module,code_server_target},\n  true,\n  \
             {module,code_server_target},\n  {error,nofile}],\n code_server_target_value,\n"
        ),
        "{erts}"
    );
    assert_eq!(run(&mut pbeam(&[])), erts);
}

#[test]
fn starts_code_server_in_embedded_mode_as_pinned_erts() {
    // With `--mode embedded`, `code:ensure_loaded/1` of a module that is not
    // loaded returns `{error,embedded}` (code.erl:601-602), and the path is
    // the `erl_prim_loader` path (code_server.erl:112-114, 519-520).
    let flags = ["--mode", "embedded"];
    let erts = run(&mut erts(&flags));
    assert!(
        erts.starts_with(
            "[embedded,\n [false,{error,embedded},false,{error,embedded},{error,embedded}],\n \
             not_loaded,\n"
        ),
        "{erts}"
    );
    assert_eq!(run(&mut pbeam(&flags)), erts);
}

#[test]
fn stops_when_the_start_modules_do_not_hold_the_atom_of_a_boot_flag() {
    // The binary makes the boot flag list from the atoms of the loaded
    // modules, so a flag name that no loaded module holds stops the run
    // before it starts (row "Run arguments" of docs/erts-differences.md).
    // The pinned `init` module holds the atom `mode` (init.erl:938, 1206).
    // Without it, `--mode` stops the run. A start set of this test has only
    // `pbeam_boot` and the probe, because the other pins of the boot would
    // put the atom in the table.
    let output = Command::new(env!("CARGO_BIN_EXE_pbeam"))
        .args([
            "pbeam_boot",
            "start",
            "code_server_probe",
            "main",
            "--root",
            "/pbeam-test-root-that-does-not-exist",
            "--mode",
            "embedded",
            "--",
        ])
        .arg(format!("{CODE_SERVER_PROBE}.beam"))
        .arg(boot_beam())
        .env("TERM", "dumb")
        .env("LC_ALL", "C.UTF-8")
        .stdin(Stdio::null())
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(1), "{output:?}");
    assert!(output.stdout.is_empty(), "{output:?}");
    assert_eq!(
        String::from_utf8_lossy(&output.stderr),
        "pbeam: boot flag mode is not an atom of the loaded modules\n"
    );
}

/// Return the command of the probe on the pinned ERTS with the code path
/// directory and the `erl` flag of each `--mode` flag.
///
/// # Examples
///
/// ```rust,ignore
/// let output = run(&mut erts(&["--mode", "embedded"]));
/// ```
fn erts(flags: &[&str]) -> Command {
    let mut command = Command::new("erl");
    command.args([
        "+S",
        "1",
        "-noshell",
        "-boot",
        "start_clean",
        "-pa",
        CODE_PATH,
    ]);
    command.args(
        flags
            .iter()
            .map(|flag| flag.strip_prefix('-').unwrap_or(flag)),
    );
    command.args([
        "-eval",
        &format!(
            "{{module, _}} = code:load_abs(\"{CODE_SERVER_PROBE}\"), \
             code_server_probe:main(), halt()."
        ),
    ]);
    command
}

/// Return the command of the probe on the `pbeam` binary with `--root`,
/// the code path directory, and `flags`.
///
/// # Examples
///
/// ```rust,ignore
/// let output = run(&mut pbeam(&["--mode", "embedded"]));
/// ```
fn pbeam(flags: &[&str]) -> Command {
    let mut command = Command::new(env!("CARGO_BIN_EXE_pbeam"));
    command
        .args(["pbeam_boot", "start", "code_server_probe", "main", "--root"])
        .arg(otp_root())
        .args(["--pa", CODE_PATH])
        .args(flags)
        .arg("--")
        .arg(format!("{CODE_SERVER_PROBE}.beam"))
        .arg(boot_beam())
        .args(
            BOOT_MODULES
                .iter()
                .chain(&CODE_SERVER_MODULES)
                .chain(&KERNEL_START_MODULES)
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

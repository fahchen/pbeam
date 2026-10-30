//! The export entry of a function that a module does not export calls
//! `error_handler:undefined_function/3` (`beam_common.c:1294-1342`), and that
//! function loads the module through `code:ensure_loaded/1` or
//! `init:ensure_loaded/1` (`error_handler.erl:85-98`, `182-198`). The boot of
//! `examples/hello_world` answers `{ensure_loaded, Module}` for the init
//! process (`init.erl:730-750`).
#![expect(
    clippy::unwrap_used,
    reason = "test helpers report a failure by a panic"
)]

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

/// The probe that calls a module that the run does not load. The module
/// `error_handler_load_target` is on the code path of the run only, so the
/// call goes through the error handler of the process.
const LOAD_PROBE: &str = "fixtures/error_handler/error_handler_load_probe";

/// The probe that prints the reductions of the error handler path of
/// `fixtures/error_handler/error_handler_probe.beam`.
const REDUCTIONS_PROBE: &str = "fixtures/error_handler/error_handler_reductions_probe";

/// The fixture modules of the measured calls: the probe, the module of the
/// failed calls, and the custom error handler. The run loads them at its
/// start, as the ERTS run of the reference loads them.
const FIXTURE_MODULES: [&str; 3] = [
    "fixtures/error_handler/error_handler_probe",
    "fixtures/error_handler/error_handler_target",
    "fixtures/error_handler/error_handler_custom",
];

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
    let erts = run(&mut erts("error_handler_load_probe:main()."));
    assert_eq!(erts, "{loaded,a}\n");
    assert_eq!(
        run(&mut pbeam(
            "error_handler_load_probe",
            "main",
            &[format!("{LOAD_PROBE}.beam")]
        )),
        erts
    );
}

#[test]
fn calls_undefined_lambda_for_a_fun_of_a_module_that_is_not_loaded_as_pinned_erts() {
    // `call_fun` calls `error_handler:undefined_lambda/3` for a fun of a
    // module with no current code (`beam_common.c:1708-1728`). The module
    // load fails, so the handler raises undef with the frame
    // `{Fun, Args, []}` (`error_handler.erl:117-127,162-178`).
    let erts = run(&mut erts("error_handler_load_probe:lambda()."));
    assert_eq!(erts, "{error,undef,{#Fun<no_such.2.-1>,[1],[]}}\n");
    assert_eq!(
        run(&mut pbeam(
            "error_handler_load_probe",
            "lambda",
            &[format!("{LOAD_PROBE}.beam")]
        )),
        erts
    );
}

#[test]
fn charges_the_reductions_of_the_error_handler_path_as_pinned_erts() {
    // The reductions of the error handler path depend on the load path of
    // `error_handler:ensure_loaded/1`: a run with a live code server calls
    // `code:ensure_loaded/1` (`error_handler.erl:182-198`), which returns at
    // `erlang:module_loaded/1` for a loaded module (`code.erl:581-583`). The
    // boot run of `pbeam_boot` with `--root` has that code server, as ERTS
    // does, so the two runs compare, and PBeam charges the reductions of the
    // pinned `error_handler.beam` itself. A run of the library API has no
    // code server and takes the longer `init:ensure_loaded/1` path
    // (`tests/integration/error_handler.rs`).
    let mut probes = FIXTURE_MODULES
        .map(|module| format!("{module}.beam"))
        .to_vec();
    probes.push(format!("{REDUCTIONS_PROBE}.beam"));
    let erts = run(&mut erts(
        "{module,_} = code:ensure_loaded(error_handler_target), \
         {module,_} = code:ensure_loaded(error_handler_custom), \
         error_handler_reductions_probe:main().",
    ));
    assert_eq!(erts, "[{plain,16},{handled,8}]\n");
    assert_eq!(
        run(&mut pbeam(
            "error_handler_reductions_probe",
            "main",
            &probes
        )),
        erts
    );
}

/// Return the command of a probe on the pinned ERTS with the code path
/// directory.
///
/// # Examples
///
/// ```rust,ignore
/// let output = run(&mut erts("error_handler_load_probe:main()."));
/// ```
fn erts(eval: &str) -> Command {
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
        eval,
    ]);
    command
}

/// Return the command of a probe on the `pbeam` binary with `--root` and the
/// code path directory. The start modules of `probes` hold no
/// `error_handler_load_target`.
///
/// # Examples
///
/// ```rust,ignore
/// let output = run(&mut pbeam("error_handler_load_probe", "main", &[LOAD_PROBE.to_owned()]));
/// ```
fn pbeam(module: &str, function: &str, probes: &[String]) -> Command {
    let mut command = Command::new(env!("CARGO_BIN_EXE_pbeam"));
    command
        .args(["pbeam_boot", "start", module, function, "--root"])
        .arg(otp_root())
        .args(["--pa", CODE_PATH])
        .arg("--")
        .args(probes)
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

#[test]
fn starts_an_entry_module_that_the_start_files_do_not_hold_as_pinned_erts() {
    let erts_loaded = run(&mut erts("error_handler_load_probe:main()."));
    assert_eq!(
        run(&mut pbeam("error_handler_load_probe", "main", &[])),
        erts_loaded
    );

    let erts_absent = output(&mut erts("pbeam_absent_entry:main()."));
    let erts_stderr = String::from_utf8_lossy(&erts_absent.stderr);
    assert_eq!(erts_absent.status.code(), Some(1), "{erts_absent:?}");
    assert!(
        erts_stderr.contains("({undef,[{pbeam_absent_entry,main,[],[]},"),
        "{erts_stderr}"
    );
    let pbeam_absent = output(&mut pbeam("pbeam_absent_entry", "main", &[]));
    let pbeam_stderr = String::from_utf8_lossy(&pbeam_absent.stderr);
    assert_eq!(pbeam_absent.status.code(), Some(1), "{pbeam_absent:?}");
    // `pbeam_boot:start/3` calls the entry as its last call, so the stack
    // trace holds only the frame of the entry.
    assert_eq!(
        pbeam_stderr,
        "pbeam: root process exited with {undef,[{pbeam_absent_entry,main,[],[]}]}\n"
    );
}

/// Run `command` and return its output, with the environment of [`run`].
fn output(command: &mut Command) -> Output {
    command
        .env("TERM", "dumb")
        .env("LC_ALL", "C.UTF-8")
        .stdin(Stdio::null())
        .output()
        .unwrap()
}

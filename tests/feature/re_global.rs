//! `re:run/2,3` with `global`, or with a unicode subject or pattern that
//! needs the Erlang conversion, traps to `re:grun/3` or `re:urun/3`
//! (`erl_bif_re.c:1530-1534`, `1557-1605`). ERTS resolves a BIF trap
//! target through its export entry. A function of a module that is not
//! loaded has a stub entry that calls `error_handler:undefined_function/3`
//! (`erts_export_get_or_make_stub`, export.c:228-242; `call_error_handler`,
//! beam_common.c:1294-1342), which loads the pinned `re` module and enters
//! its Erlang helper (`error_handler.erl:85-98`). The probe makes its first
//! `re` call before the run loads `re`, so both runs take that path.

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

/// The probe that runs the global matches. The run loads the probe at its
/// start and loads `re` on demand.
const PROBE: &str = "fixtures/re/re_global_probe";

/// The code path directory of the runs.
const CODE_PATH: &str = "fixtures/re";

/// The pinned OTP modules that the run needs in addition to `BOOT_MODULES`:
/// `code` and `code_server`, the modules that `code:do_start/0` loads
/// before it starts `code_server` and that `BOOT_MODULES` does not have
/// (code.erl:1699-1712), and the modules that `io:format/2` with `~w`
/// calls.
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
fn runs_the_global_helpers_of_re_that_the_run_loads_on_demand_as_pinned_erts() {
    // The first call runs before the run loads `re`, so it loads the module
    // through `error_handler` and `code_server`. The result, the exceptions,
    // and the reductions of the following calls come from the pinned
    // `re.erl` helpers over the `re:internal_run/4` BIF (re.erl:1817-1889,
    // 1754-1790).
    let erts = run(&mut erts("re_global_probe:main()."));
    assert!(erts.starts_with("[{match,[[{1,1}]]},["), "{erts}");
    assert_eq!(
        run(&mut pbeam(
            "re_global_probe",
            "main",
            &[format!("{PROBE}.beam")]
        )),
        erts
    );
}

#[test]
fn switches_out_at_a_trap_to_a_module_that_the_run_did_not_load_as_pinned_erts() {
    // A BIF trap switches out before the export stub enters the error
    // handler (`instr_bif.cpp:525-536`, `676-701`). The custom handler loads
    // no module, so only that switch lets the peer process send its message
    // before the `receive` with `after 0`.
    let erts = run(&mut erts("re_global_probe:scheduling()."));
    assert_eq!(erts, "{handled,ran}\n");
    assert_eq!(
        run(&mut pbeam(
            "re_global_probe",
            "scheduling",
            &[format!("{PROBE}.beam")]
        )),
        erts
    );
}

#[test]
fn ends_a_process_when_its_error_handler_has_no_undefined_function_as_pinned_erts() {
    // The handler lookup of `undefined_function` fails, because the process
    // error handler has no `undefined_function/3`. The fallback raises
    // `undef`, and `undefined_function` returns the outcome of the exit.
    let erts = run(&mut erts("re_global_probe:uncaught()."));
    assert_eq!(erts, "undef\n");
    assert_eq!(
        run(&mut pbeam(
            "re_global_probe",
            "uncaught",
            &[format!("{PROBE}.beam")]
        )),
        erts
    );
}

/// Return the command of the probe on the pinned ERTS with the code path
/// directory.
///
/// # Examples
///
/// ```rust,ignore
/// let output = run(&mut erts("re_global_probe:main()."));
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
/// code path directory.
///
/// # Examples
///
/// ```rust,ignore
/// let output = run(&mut pbeam("re_global_probe", "main", &[format!("{PROBE}.beam")]));
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

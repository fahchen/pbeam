use std::{
    process::{Command, Stdio},
    time::Duration,
};

#[expect(
    dead_code,
    reason = "this test crate gives BEAM paths, not bytes, to the binary"
)]
#[path = "../common/mod.rs"]
mod common;
#[expect(
    dead_code,
    reason = "this test crate uses no terminal settings and no broken pipe"
)]
#[path = "../common/run.rs"]
mod run;

use common::pinned_beam_path;
use run::run_with_time_limit;

const RUN_TIME_LIMIT: Duration = Duration::from_secs(10);

/// Run `pbeam --policy erl_policy policy_probe Function` with the policy
/// fixtures and return the status code, the stdout, and the stderr.
///
/// # Examples
///
/// ```rust,ignore
/// assert_eq!(pbeam("granted"), (Some(0), String::new(), String::new()));
/// ```
fn pbeam(function: &str) -> (Option<i32>, String, String) {
    run(&[
        "--policy",
        "erl_policy",
        "policy_probe",
        function,
        "--",
        "fixtures/policy/policy_probe.beam",
        "fixtures/policy/erl_policy.beam",
    ])
}

/// Run `pbeam` with `options` before `capability_probe report Function`,
/// with the capability probe, and return the status code, the stdout, and
/// the stderr. The root process exits with `{result, Result}`.
///
/// # Examples
///
/// ```rust,ignore
/// let (status, _, stderr) = capability_probe(&["--no-capability-checks"], "reduce");
/// ```
fn capability_probe(options: &[&str], function: &str) -> (Option<i32>, String, String) {
    let mut arguments = options.to_vec();
    arguments.extend([
        "capability_probe",
        "report",
        function,
        "--",
        "fixtures/capability/capability_probe.beam",
        "fixtures/policy/erl_policy.beam",
    ]);
    run(&arguments)
}

/// Run `pbeam` with `arguments` and the pinned `erlang` and `erts_internal`
/// BEAM files at the end, and return the status code, the stdout, and the
/// stderr.
fn run(arguments: &[&str]) -> (Option<i32>, String, String) {
    let output = run_with_time_limit(
        Command::new(env!("CARGO_BIN_EXE_pbeam"))
            .args(arguments)
            .arg(pinned_beam_path("erlang"))
            .arg(pinned_beam_path("erts_internal")),
        Stdio::piped(),
        RUN_TIME_LIMIT,
    );
    (
        output.status.code(),
        String::from_utf8(output.stdout).unwrap(),
        String::from_utf8(output.stderr).unwrap(),
    )
}

#[test]
fn runs_the_root_process_with_the_policy_module_of_the_policy_option() {
    // `policy_probe:granted/0` requests env read for HOME, which the policy
    // grants, and returns. The binary writes nothing.
    assert_eq!(pbeam("granted"), (Some(0), String::new(), String::new()));
    // `initial/1` of the policy denies the root process of
    // `policy_probe:bad_initial/0`, so the root process does not start.
    let denied =
        "pbeam: the policy denied the root process: initial/1 returned [#{kind=>process}]\n";
    assert_eq!(
        pbeam("bad_initial"),
        (Some(1), String::new(), String::from(denied))
    );
}

#[test]
fn skips_each_capability_check_without_a_policy_and_with_the_flag() {
    // `capability_probe:reduce/0` runs each spawn in a child that asks for
    // only `spawn` and `monitor`. With `AllowAll`, the linked spawns are
    // denied. With no policy, and with the flag, the run skips each check,
    // so each spawn gives ok.
    let unchecked = (
        Some(1),
        String::new(),
        String::from("pbeam: root process exited with {result,[ok,ok,ok,ok,ok]}\n"),
    );
    assert_eq!(capability_probe(&[], "reduce"), unchecked);
    assert_eq!(
        capability_probe(&["--no-capability-checks"], "reduce"),
        unchecked
    );
    // With a policy, the checks run: `erl_policy` permits spawns of
    // `policy_probe` only, so the first spawn of the probe is denied.
    assert_eq!(
        capability_probe(&["--policy", "erl_policy"], "reduce"),
        (
            Some(1),
            String::new(),
            String::from("pbeam: root process exited with {result,denied}\n"),
        )
    );
    // The flag cannot be used with a policy: clap gives a usage error.
    let (status, stdout, stderr) = capability_probe(
        &["--no-capability-checks", "--policy", "erl_policy"],
        "reduce",
    );
    assert_eq!((status, stdout.as_str()), (Some(2), ""));
    assert!(
        stderr.starts_with(
            "error: the argument '--no-capability-checks' cannot be used with '--policy <MODULE>'"
        ),
        "{stderr}"
    );
}

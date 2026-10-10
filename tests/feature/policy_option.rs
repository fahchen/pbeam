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
    let output = run_with_time_limit(
        Command::new(env!("CARGO_BIN_EXE_pbeam"))
            .args([
                "--policy",
                "erl_policy",
                "policy_probe",
                function,
                "--",
                "fixtures/policy/policy_probe.beam",
                "fixtures/policy/erl_policy.beam",
            ])
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

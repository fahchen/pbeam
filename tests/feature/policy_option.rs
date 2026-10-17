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

/// Run `pbeam --boot --policy boot_policy boot_authority_probe Function`
/// with the boot fixtures and the pinned `lists`, and return the status
/// code, the stdout, and the stderr.
///
/// # Examples
///
/// ```rust,ignore
/// let (status, _, stderr) = boot("denied");
/// ```
fn boot(function: &str) -> (Option<i32>, String, String) {
    let lists = pinned_beam_path("lists");
    run(&[
        "--boot",
        "--policy",
        "boot_policy",
        "boot_authority_probe",
        function,
        "--",
        "fixtures/policy/boot_authority_probe.beam",
        "fixtures/policy/boot_policy.beam",
        lists.to_str().unwrap(),
    ])
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

#[test]
fn runs_the_boot_with_boot_authority_and_enters_the_runtime_with_the_policy_capabilities() {
    // `boot_authority_probe:main/0` runs in the root process in boot mode.
    // With no copies, it reads an environment variable that does not exist,
    // and its child `Service` is a boot process. A boot spawn with
    // `{capabilities, Map}` is denied, and a boot `pbeam_cap:request/2`
    // returns `ok` without the policy, whose `grant/3` denies each request.
    // After `pbeam_cap:enter_runtime/2`, the root process has only the
    // `process` capability of `initial/1`, which got `{Module, Function, 0}`.
    // The runtime to boot gate denies each operation on `Service` except a
    // monitor and a message to its alias. A policy revocation of `Service`
    // has no effect. The same operations on a runtime process are permitted.
    let denied = "{error,capability_denied}";
    let results = [
        String::from("{boot_getenv,false}"),
        format!("{{boot_spawn_restricted,{denied}}}"),
        String::from("{boot_request,ok}"),
        String::from("{boot_enter_badarg,{error,badarg}}"),
        String::from("{enter_again,ok}"),
        String::from(
            "{requester,#{initial_call=>{boot_authority_probe,guest,0},parent=>none,pid=><0.0.0>}}",
        ),
        format!("{{getenv,{denied}}}"),
        format!("{{spawn_other,{denied}}}"),
        format!("{{send_pid,{denied}}}"),
        format!("{{send_name,{denied}}}"),
        format!("{{timer_pid,{denied}}}"),
        format!("{{timer_name,{denied}}}"),
        format!("{{timer_pid_4,{denied}}}"),
        format!("{{timer_name_4,{denied}}}"),
        format!("{{link,{denied}}}"),
        format!("{{exit,{denied}}}"),
        format!("{{exit_alias,{denied}}}"),
        format!("{{group_leader,{denied}}}"),
        format!("{{process_info,{denied}}}"),
        format!("{{is_process_alive,{denied}}}"),
        format!("{{unregister,{denied}}}"),
        String::from("{monitor,true}"),
        String::from("{alias,via_alias}"),
        String::from("{policy_revoke,ok}"),
        String::from("{boot_spawn_after_revoke,true}"),
        String::from("{runtime_send,x}"),
        String::from("{runtime_link,true}"),
        String::from("{runtime_process_info,[]}"),
        String::from("{runtime_unregister,true}"),
    ];
    let exited = format!(
        "pbeam: root process exited with {{result,[{}]}}\n",
        results.join(",")
    );
    assert_eq!(boot("main"), (Some(1), String::new(), exited));
    // `pbeam_cap:serve/2` registers a live boot process as an IO service.
    // The IO service entry then passes an IO request of a runtime process
    // only when its capabilities permit it: `boot_policy` gives no
    // `terminal` capability.
    let results = [
        "{serve_not_pid,{error,badarg}}",
        "{serve_not_io,{error,badarg}}",
        "{serve_dead,{error,badarg}}",
        "{serve,ok}",
        "{serve_again,ok}",
        "{put_chars,{error,capability_denied}}",
        "{empty_requests,sent}",
        "{runtime_serve_not_io,{error,badarg}}",
    ];
    let exited = format!(
        "pbeam: root process exited with {{result,[{}]}}\n",
        results.join(",")
    );
    assert_eq!(boot("serve"), (Some(1), String::new(), exited));
    // A denial of `initial/1` in boot mode gives the error text of a denial
    // without boot mode.
    assert_eq!(
        boot("denied"),
        (
            Some(1),
            String::new(),
            String::from("pbeam: the policy denied the root process: initial/1 returned deny\n")
        )
    );
    // Only the root process can enter the runtime.
    assert_eq!(
        boot("child_enter"),
        (
            Some(1),
            String::new(),
            String::from(
                "pbeam: unsupported pbeam_cap:enter_runtime/2 in a boot process that is not the root process: see \"Boot authority\" in docs/erts-differences.md\n"
            )
        )
    );
    // Boot mode needs a policy module: clap gives a usage error.
    let (status, stdout, stderr) = capability_probe(&["--boot"], "reduce");
    assert_eq!((status, stdout.as_str()), (Some(2), ""));
    assert!(
        stderr.starts_with("error: the following required arguments were not provided:"),
        "{stderr}"
    );
}

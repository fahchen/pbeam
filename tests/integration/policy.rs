//! The policy module and the policy process: `initial/1` gives the
//! capabilities of the root process, and `grant/3` answers each
//! `pbeam_cap:request/2` call through a grant signal. ERTS has no policy,
//! so `pbeam_cap:request/2` raises `error:undef` there. The pinned ERTS
//! run gives the results of the permitted part, and the policy modules of
//! `fixtures/policy` are the oracle of each grant and denial.

#[path = "../common/mod.rs"]
mod common;

use std::process::Command;

use common::pinned_beam;
use pbeam_runtime::{AllowAll, BeamProgram, Term};

const POLICY_PROBE: &[u8] = include_bytes!("../../fixtures/policy/policy_probe.beam");
const ERL_POLICY: &[u8] = include_bytes!("../../fixtures/policy/erl_policy.beam");
const ELIXIR_POLICY: &[u8] = include_bytes!("../../fixtures/policy/Elixir.ElixirPolicy.beam");

/// Return the `~w` text of a term of atoms, lists, and tuples.
///
/// # Examples
///
/// ```rust,ignore
/// assert_eq!(text(&Term::List(vec![])), "[]");
/// ```
fn text(term: &Term) -> String {
    let join = |terms: &[Term]| terms.iter().map(text).collect::<Vec<_>>().join(",");
    match term {
        Term::Atom(atom) => atom.name().to_owned(),
        Term::List(elements) => format!("[{}]", join(elements)),
        Term::Tuple(elements) => format!("{{{}}}", join(elements)),
        other => panic!("the probe returns no {other:?}"),
    }
}

/// Load the probe, both policy modules, and the pinned `erlang` and
/// `erts_internal`.
///
/// # Examples
///
/// ```rust,ignore
/// let program = program();
/// ```
fn program() -> BeamProgram {
    let erlang = pinned_beam("erlang");
    let erts_internal = pinned_beam("erts_internal");
    let modules = [
        &erlang[..],
        &erts_internal[..],
        POLICY_PROBE,
        ERL_POLICY,
        ELIXIR_POLICY,
    ];
    BeamProgram::load(&modules).unwrap()
}

/// Run `policy_probe:Function()` on PBeam with the policy module `policy`
/// and return the `~w` text of the result, or the error message of the
/// run.
///
/// # Examples
///
/// ```rust,ignore
/// assert_eq!(pbeam("idle", "erl_policy"), Ok(String::from("[denied,ok]")));
/// ```
fn pbeam(function: &str, policy: &str) -> Result<String, String> {
    program()
        .execute_with_policy_module("policy_probe", function, &[], policy, false)
        .map(|result| text(&result))
        .map_err(|error| error.to_string())
}

/// Run `policy_probe:Function()` on the pinned ERTS with one scheduler and
/// return the printed result.
///
/// # Examples
///
/// ```rust,ignore
/// assert_eq!(erts("initial"), "[pid,pid,string]");
/// ```
fn erts(function: &str) -> String {
    let output = Command::new("erl")
        .args([
            "+S",
            "1",
            "-noshell",
            "-pa",
            "fixtures/policy",
            "-eval",
            &format!("io:format(\"~w\", [policy_probe:{function}()]), halt()."),
        ])
        .output()
        .unwrap();
    assert!(
        output.status.success() && output.stderr.is_empty(),
        "{output:?}"
    );
    String::from_utf8(output.stdout).unwrap()
}

/// Return the result text of `count` copies of `result`, joined with
/// commas.
///
/// # Examples
///
/// ```rust,ignore
/// assert_eq!(repeat("denied", 2), "denied,denied");
/// ```
fn repeat(result: &str, count: usize) -> String {
    vec![result; count].join(",")
}

#[test]
fn gives_the_root_process_the_capabilities_of_initial() {
    // The root process can spawn only the modules of the probe, and it has
    // no env capability. On ERTS each call has its effect.
    assert_eq!(
        pbeam("initial", "erl_policy").unwrap(),
        "[pid,denied,denied]"
    );
    assert_eq!(erts("initial"), "[pid,pid,string]");
}

#[test]
fn grants_a_requested_capability_with_an_erlang_and_an_elixir_policy() {
    // The grant permits env read for HOME only.
    let granted = "{denied,ok,[string,denied]}";
    assert_eq!(pbeam("granted", "erl_policy").unwrap(), granted);
    assert_eq!(pbeam("granted", "Elixir.ElixirPolicy").unwrap(), granted);
    assert_eq!(erts("granted"), "{string,{error,undef},[string,string]}");
    // A run with a Rust policy has no policy process, so a request is
    // denied at once, and each other call keeps its ERTS result.
    let program = program();
    let host = program
        .execute_with_policy("policy_probe", "granted", &[], &AllowAll)
        .unwrap();
    assert_eq!(text(&host), "{string,denied,[string,string]}");
    // A run with no policy skips each check, so a request returns ok and
    // calls no policy code.
    let unchecked = program.execute("policy_probe", "granted", &[]).unwrap();
    assert_eq!(text(&unchecked), "{string,ok,[string,string]}");
}

#[test]
fn denies_each_value_that_is_not_a_grant_form() {
    let denied = repeat("denied", 13);
    assert_eq!(
        pbeam("forms", "erl_policy").unwrap(),
        format!("[{denied},ok]")
    );
}

#[test]
fn denies_an_exception_and_a_reduction_overrun_of_the_policy_code() {
    // The policy process answers each later request. Its own request is
    // denied, so the policy then grants.
    assert_eq!(
        pbeam("failures", "erl_policy").unwrap(),
        "[denied,denied,denied,ok,{error,badarg},{error,badarg}]"
    );
}

#[test]
fn denies_an_overrun_that_ends_in_a_receive_or_a_return() {
    // A call that waits in a receive at the end of each turn, and a call
    // that returns in the turn after its last switch out for the budget,
    // are denied when they use more than 100000 reductions.
    assert_eq!(
        pbeam("overrun", "erl_policy").unwrap(),
        "[ok,denied,denied]"
    );
}

#[test]
fn gives_the_requester_map_and_keeps_the_policy_process_state() {
    // The requester maps of a worker and of the root process are exact.
    // The crash of the worker, which is linked to the root process, does
    // not end the policy process.
    assert_eq!(pbeam("requester", "erl_policy").unwrap(), "{true,true,ok}");
}

#[test]
fn does_not_check_the_effects_of_the_policy_code_against_the_requester() {
    assert_eq!(pbeam("effect", "erl_policy").unwrap(), "[denied,ok,pid]");
}

#[test]
fn keeps_a_request_waiting_when_a_message_or_a_wake_arrives() {
    assert_eq!(pbeam("wait", "erl_policy").unwrap(), "{denied,hello}");
    assert_eq!(pbeam("idle", "erl_policy").unwrap(), "[denied,ok]");
}

#[test]
fn denies_each_request_after_the_policy_process_exits() {
    assert_eq!(
        pbeam("exited", "erl_policy").unwrap(),
        "[denied,denied,denied]"
    );
    // A link of the policy code ends the policy process while it waits.
    assert_eq!(pbeam("idle_exit", "erl_policy").unwrap(), "[denied,denied]");
}

#[test]
fn stops_the_run_when_initial_denies_the_root_process() {
    let denied = "the policy denied the root process";
    let cases = [
        ("bad_initial", "initial/1 returned [#{kind=>process}]"),
        ("atom_initial", "initial/1 returned all"),
        ("raise_initial", "initial/1 exited with {policy_bug,[]}"),
        ("loop_initial", "initial/1 used more than 100000 reductions"),
        ("exit_initial", "the policy process exited with killed"),
    ];
    for (function, cause) in cases {
        assert_eq!(
            pbeam(function, "erl_policy"),
            Err(format!("{denied}: {cause}")),
            "{function}"
        );
    }
    assert_eq!(
        pbeam("wait_initial", "erl_policy"),
        Err(String::from("the policy process waits in initial/1"))
    );
}

#[test]
fn needs_a_loaded_policy_module_with_both_callbacks() {
    assert_eq!(
        pbeam("granted", "no_policy"),
        Err(String::from("missing BEAM module no_policy"))
    );
    assert_eq!(
        pbeam("granted", "policy_probe"),
        Err(String::from("missing export policy_probe:grant/3"))
    );
}

#[test]
fn revokes_a_run_time_grant_and_its_downstream_copies_from_the_policy_process() {
    // The worker gets a delegable env grant and gives a copy to its child.
    // The policy revokes the copy of the worker, and the revocation goes on
    // to the child. A second revocation finds no copy and gives ok. On
    // ERTS the request and the revocation raise undef, and each read has
    // its effect.
    assert_eq!(
        pbeam("revoked", "erl_policy").unwrap(),
        "{ok,[string,string],[ok,ok],[denied,denied]}"
    );
    assert_eq!(
        erts("revoked"),
        "{{error,undef},[string,string],[{error,undef},{error,undef}],[string,string]}"
    );
    // The worker exits before the revocation; its released record takes
    // the revocation to its child.
    assert_eq!(
        pbeam("revoked_exited", "erl_policy").unwrap(),
        "{ok,string,ok,denied}"
    );
    assert_eq!(
        erts("revoked_exited"),
        "{{error,undef},string,{error,undef},string}"
    );
}

#[test]
fn revokes_a_launcher_copy_of_the_root_process_from_the_policy_process() {
    assert_eq!(
        pbeam("revoked_root", "erl_policy").unwrap(),
        "{pid,ok,denied}"
    );
    assert_eq!(erts("revoked_root"), "{pid,{error,undef},pid}");
}

#[test]
fn lets_only_the_policy_process_call_policy_revoke() {
    assert_eq!(
        pbeam("revoke_denied", "erl_policy").unwrap(),
        "[ok,denied,denied,{error,badarg},denied,{error,badarg},{error,badarg},ok,ok,string,pid]"
    );
    assert_eq!(
        erts("revoke_denied"),
        format!("[{},string,pid]", repeat("{error,undef}", 9))
    );
    // A run with no policy skips each check, so each call returns ok after
    // its argument checks and has no effect.
    let unchecked = program()
        .execute("policy_probe", "revoke_denied", &[])
        .unwrap();
    assert_eq!(
        text(&unchecked),
        "[ok,ok,ok,{error,badarg},ok,ok,ok,ok,ok,string,pid]"
    );
}

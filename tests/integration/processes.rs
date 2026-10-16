//! `erlang:processes/0` and `process_info(Pid, group_leader)`, which the
//! `application_master` of `kernel` calls when the kernel application stops
//! (application_master.erl:439-453).
#[path = "../common/mod.rs"]
mod common;

use std::process::Command;

use common::pinned_beam;
use pbeam_runtime::{BeamProgram, HostTerm, Term};

const PROCESSES_PROBE: &[u8] = include_bytes!("../../fixtures/processes/processes_probe.beam");
const PROCESSES_POLICY: &[u8] = include_bytes!("../../fixtures/processes/processes_policy.beam");

#[test]
fn lists_the_processes_and_gives_the_group_leader_as_pinned_erts() {
    // `processes/0` lists the caller and a live child in pid order, and not
    // a process that exited (erl_ptab.c:746-780). `process_info/2` gives
    // `{group_leader, Pid}` for the caller and for another process, and
    // `undefined` for a dead process (erl_bif_info.c:1384-1386, 2107-2112).
    let reference = Command::new("erl")
        .args([
            "-noshell",
            "-pa",
            "fixtures/processes",
            "-eval",
            "io:format(\"~w~n\", [processes_probe:main()]), halt().",
        ])
        .output()
        .unwrap();
    assert_eq!(
        (reference.stdout.as_slice(), reference.stderr.as_slice()),
        (
            &b"[true,true,false,true,true,true,undefined]\n"[..],
            &b""[..]
        )
    );

    let erlang = pinned_beam("erlang");
    let lists = pinned_beam("lists");
    let program = BeamProgram::load(&[&erlang, &lists, PROCESSES_PROBE]).unwrap();
    let atom = |name| Term::Atom(program.atom(name).unwrap());
    assert_eq!(
        program
            .execute("processes_probe", "main", &[])
            .unwrap()
            .term,
        Term::list_from(vec![
            atom("true"),
            atom("true"),
            atom("false"),
            atom("true"),
            atom("true"),
            atom("true"),
            atom("undefined"),
        ])
    );
}

#[test]
fn keeps_the_policy_process_out_of_the_list_of_a_runtime_process() {
    // A run with a policy module has the root process and the policy
    // process. The root process is not a boot process, so `processes/0`
    // gives it only its own pid: it cannot send a signal to the policy
    // process (section "Policy module, policy process, and policy table" of
    // docs/capability-security-migration.md). A root process in boot mode
    // is a boot process, so it also gets the policy process. A run without
    // a policy has only the root process.
    let erlang = pinned_beam("erlang");
    let program = BeamProgram::load(&[&erlang, PROCESSES_PROBE, PROCESSES_POLICY]).unwrap();
    let alone = |policy: Option<(&str, bool)>| {
        let result = match policy {
            Some((policy, boot)) => {
                program.execute_with_policy_module("processes_probe", "alone", &[], policy, boot)
            }
            None => program.execute("processes_probe", "alone", &[]),
        };
        result.unwrap().term
    };
    let atom = |name| Term::Atom(program.atom(name).unwrap());
    assert_eq!(alone(Some(("processes_policy", false))), atom("true"));
    assert_eq!(alone(Some(("processes_policy", true))), atom("false"));
    assert_eq!(alone(None), atom("true"));
}

#[test]
fn charges_and_traps_for_the_process_table_as_pinned_erts() {
    // `processes/0` reads the 1048576 indices of the table in chunks of
    // 1000, at 1 reduction for each 25 indices, and traps between chunks
    // (erl_ptab.c:42-43, 1006-1037): 41943 reductions, and 1 for each
    // `process_info/2` call. A child that exits before the call reads its
    // chunk comes last, from the deleted processes (erl_ptab.c:1046-1170).
    // A call with fewer than 40 reductions traps before its snapshot
    // (erl_ptab.c:766-768), so the child that exits first is not in it.
    let probe = "io:format(\"~w~n\", [[processes_probe:reductions() | \
                 [processes_probe:exited(L) || L <- [1000, 60, 20]]]]), halt().";
    let reference = Command::new("erl")
        .args([
            "+S",
            "1",
            "-noshell",
            "-pa",
            "fixtures/processes",
            "-eval",
            probe,
        ])
        .output()
        .unwrap();
    assert_eq!(
        (reference.stdout.as_slice(), reference.stderr.as_slice()),
        (
            &b"[41945,{true,false},{true,true},{false,false}]\n"[..],
            &b""[..]
        )
    );

    let erlang = pinned_beam("erlang");
    let lists = pinned_beam("lists");
    let program = BeamProgram::load(&[&erlang, &lists, PROCESSES_PROBE]).unwrap();
    let atom = |name| Term::Atom(program.atom(name).unwrap());
    let run = |function, arguments: &[HostTerm]| {
        program
            .execute("processes_probe", function, arguments)
            .unwrap()
            .term
    };
    assert_eq!(run("reductions", &[]), Term::Integer(41945));
    let exited = |left| run("exited", &[program.host_term(Term::Integer(left))]);
    let pair = |a, b| Term::tuple(vec![atom(a), atom(b)]);
    assert_eq!(exited(1000), pair("true", "false"));
    assert_eq!(exited(60), pair("true", "true"));
    assert_eq!(exited(20), pair("false", "false"));
}

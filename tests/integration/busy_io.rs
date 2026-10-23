//! The node poll thread on a node with more than one scheduler.

#[path = "../common/mod.rs"]
mod common;

use common::pinned_beam;
use pbeam_runtime::{BeamProgram, Term};

const BUSY_IO: &[u8] = include_bytes!("../../fixtures/poll/busy_io.beam");

/// The result of `busy_io:port_root/0` on the pinned ERTS with `erl +S 2`.
const REFERENCE: &str = "[{data,<<104,105,10>>},{exit_status,0}]\n";

/// Each root of two schedulers has a process that always can run and
/// waits for its spawn port. The poll thread wakes the scheduler that
/// serves each port through its inbox (`erl_process.c:3339-3372`).
#[test]
fn wakes_the_serving_scheduler_of_each_port() {
    let erlang = pinned_beam("erlang");
    let erts_internal = pinned_beam("erts_internal");
    let program =
        BeamProgram::load(&[&erlang, &erts_internal, BUSY_IO]).expect("the fixture loads");
    let atom = |name| Term::Atom(program.atom(name).expect("the fixture names the atom"));
    let expected = format!(
        "List([Tuple([{:?}, Binary([104, 105, 10])]), Tuple([{:?}, Integer(0)])])",
        atom("data"),
        atom("exit_status")
    );
    let results = program
        .execute_on_schedulers(&[("busy_io", "port_root"), ("busy_io", "port_root")])
        .expect("the two schedulers run");
    for result in results {
        assert_eq!(format!("{:?}", result.term), expected);
    }
}

/// The pinned ERTS gives the same result with two schedulers.
#[test]
fn the_pinned_erts_gives_the_same_result() {
    let output = std::process::Command::new("erl")
        .args(["+S", "2", "-noshell", "-pa", "fixtures/poll", "-eval"])
        .arg("io:format(\"~w~n\", [busy_io:port_root()]), halt().")
        .output()
        .expect("the pinned ERTS runs");
    assert!(output.status.success(), "{output:?}");
    assert_eq!(String::from_utf8_lossy(&output.stdout), REFERENCE);
}

const PORT_TASKS: &[u8] = include_bytes!("../../fixtures/poll/port_tasks.beam");

/// The result of `port_tasks:main/0` on the pinned ERTS with `erl +S 2`.
const PORT_TASKS_REFERENCE: &str = "{ok,true,{ok,<<104,105>>}}\n";

/// A process of one scheduler starts an accept on a listen socket, and
/// then a receive on the accepted socket, that the other scheduler serves
/// while that scheduler waits on its own fds. Each new fd wakes the waiting
/// scheduler, as `erts_poll_control` wakes the poller
/// (`erl_poll.c:1447-1448`); without the wake the request times out.
#[test]
fn wakes_the_idle_scheduler_of_a_new_fd() {
    let modules = ["erlang", "erts_internal", "lists", "prim_inet"].map(pinned_beam);
    let mut all: Vec<&[u8]> = modules.iter().map(Vec::as_slice).collect();
    all.push(PORT_TASKS);
    let program = BeamProgram::load(&all).expect("the fixture loads");
    let atom = |name| Term::Atom(program.atom(name).expect("the fixture names the atom"));
    let results = program
        .execute_on_schedulers(&[("port_tasks", "owner"), ("port_tasks", "holder")])
        .expect("the two schedulers run");
    let received = Term::tuple(vec![atom("ok"), Term::binary(b"hi".to_vec())]);
    assert_eq!(results[0].term, atom("ok"), "the owner");
    assert_eq!(
        results[1].term,
        Term::tuple(vec![atom("ok"), atom("true"), received]),
        "the holder"
    );
}

/// The pinned ERTS gives the same result with two schedulers.
#[test]
fn the_pinned_erts_gives_the_same_port_task_result() {
    let output = std::process::Command::new("erl")
        .args(["+S", "2", "-noshell", "-pa", "fixtures/poll", "-eval"])
        .arg("io:format(\"~w~n\", [port_tasks:main()]), halt().")
        .output()
        .expect("the pinned ERTS runs");
    assert!(output.status.success(), "{output:?}");
    assert_eq!(
        String::from_utf8_lossy(&output.stdout),
        PORT_TASKS_REFERENCE
    );
}

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

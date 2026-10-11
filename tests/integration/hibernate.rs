#[path = "../common/mod.rs"]
mod common;

use std::process::Command;

use common::pinned_beam;
use pbeam_runtime::{BeamProgram, Term};

const HIBERNATE_PROBE: &[u8] = include_bytes!("../../fixtures/hibernate/hibernate_probe.beam");

#[test]
fn runs_erlang_hibernate_as_the_pinned_erts_instruction() {
    // `i_hibernate` writes `ok` to `x0`, keeps the frame, and switches the
    // context (`jit/arm/instr_trace.cpp:222-238`,
    // `jit/arm/instr_call.cpp:30-36`). The process waits when its queue has
    // no message (`beam_common.c:1568-1602`), else it goes to the end of
    // the run queue (`erl_process.c:6695`).
    let erlang = pinned_beam("erlang");
    let program = BeamProgram::load(&[&erlang, HIBERNATE_PROBE]).unwrap();
    let atom = |name: &str| Term::Atom(program.atom(name).unwrap());
    let list = |names: &[&str]| Term::list_from(names.iter().map(|name| atom(name)));
    let run = |function| program.execute("hibernate_probe", function, &[]).unwrap();

    // A message in the queue before the call: the call returns at once.
    assert_eq!(
        run("queued"),
        Term::Tuple(vec![atom("ok"), atom("true"), list(&["early"])])
    );
    // No message in the queue: the process waits for the message of a
    // child.
    assert_eq!(
        run("later"),
        Term::Tuple(vec![atom("ok"), atom("true"), list(&["late"])])
    );
    // The `apply` instruction and a fun call run `hibernate() ->
    // erlang:hibernate().` in `erlang.beam` (`erlang.erl:3524-3527`).
    assert_eq!(
        run("apply"),
        Term::Tuple(vec![
            atom("ok"),
            atom("ok"),
            atom("ok"),
            list(&["late", "queued", "fun_late"]),
        ])
    );
    // `call_ext_last` waits first, and then returns from the frame
    // (`jit/arm/ops.tab:669-673`).
    assert_eq!(run("tail"), Term::Tuple(vec![atom("ok"), list(&["late"])]));
    // A process that starts in `erlang:hibernate/0` waits, and then exits
    // with reason `normal`.
    assert_eq!(
        run("bottom"),
        Term::Tuple(vec![
            atom("waiting"),
            Term::Tuple(vec![atom("down"), atom("normal")]),
        ])
    );
    // With a message in the queue, the child runs before the call returns.
    assert_eq!(
        run("order"),
        Term::Tuple(vec![atom("ok"), list(&["own", "child"])])
    );

    let reference = Command::new("erl")
        .args([
            "-noshell",
            "-pa",
            "fixtures/hibernate",
            "-eval",
            "io:format(\"~w~n\", [[hibernate_probe:F() || F <- [queued, later, apply, tail, bottom, order]]]), halt().",
        ])
        .output()
        .unwrap();
    assert!(reference.status.success());
    assert!(reference.stderr.is_empty());
    assert_eq!(
        reference.stdout,
        b"[{ok,true,[early]},{ok,true,[late]},{ok,ok,ok,[late,queued,fun_late]},{ok,[late]},{waiting,{down,normal}},{ok,[own,child]}]\n"
    );
}

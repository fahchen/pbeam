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
        run("queued").term,
        Term::tuple(vec![atom("ok"), atom("true"), list(&["early"])])
    );
    // No message in the queue: the process waits for the message of a
    // child.
    assert_eq!(
        run("later").term,
        Term::tuple(vec![atom("ok"), atom("true"), list(&["late"])])
    );
    // The `apply` instruction and a fun call run `hibernate() ->
    // erlang:hibernate().` in `erlang.beam` (`erlang.erl:3524-3527`).
    assert_eq!(
        run("apply").term,
        Term::tuple(vec![
            atom("ok"),
            atom("ok"),
            atom("ok"),
            list(&["late", "queued", "fun_late"]),
        ])
    );
    // `call_ext_last` waits first, and then returns from the frame
    // (`jit/arm/ops.tab:669-673`).
    assert_eq!(
        run("tail").term,
        Term::tuple(vec![atom("ok"), list(&["late"])])
    );
    // A process that starts in `erlang:hibernate/0` waits, and then exits
    // with reason `normal`.
    assert_eq!(
        run("bottom").term,
        Term::tuple(vec![
            atom("waiting"),
            Term::tuple(vec![atom("down"), atom("normal")]),
        ])
    );
    // With a message in the queue, the child runs before the call returns.
    assert_eq!(
        run("order").term,
        Term::tuple(vec![atom("ok"), list(&["own", "child"])])
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
    assert_eq!(reference.stderr, [] as [u8; 0]);
    assert_eq!(
        reference.stdout,
        b"[{ok,true,[early]},{ok,true,[late]},{ok,ok,ok,[late,queued,fun_late]},{ok,[late]},{waiting,{down,normal}},{ok,[own,child]}]\n"
    );
}

#[test]
fn runs_erlang_hibernate_3_as_the_pinned_erts_bif() {
    // `hibernate_3` checks its arguments, drops the stack and the catches,
    // and traps to `beam_run_process`, which runs `apply/3` with an empty
    // stack (`bif.c:1366-1427`). The process waits when its queue has no
    // message (`beam_common.c:1568-1602`).
    // The error handler asks the `init` server of `undef_init` to load a
    // module.
    let modules = ["erlang", "error_handler", "init", "lists", "proc_lib"].map(pinned_beam);
    let mut beams: Vec<&[u8]> = modules.iter().map(Vec::as_slice).collect();
    beams.push(include_bytes!("../../fixtures/undef/undef_init.beam"));
    beams.push(include_bytes!(
        "../../fixtures/hibernate/hibernate3_probe.beam"
    ));
    let program = BeamProgram::load(&beams).unwrap();
    let atom = |name: &str| Term::Atom(program.atom(name).unwrap());
    let int = Term::Integer;
    let pair = |a: Term, b: Term| Term::tuple(vec![a, b]);
    let run = |function| {
        program
            .execute("hibernate3_probe", function, &[])
            .unwrap()
            .term
    };
    let woke = |tag, message| Term::tuple(vec![atom(tag), atom(message), int(0)]);

    // A message in the queue before the call: the process does not wait.
    assert_eq!(run("queued"), pair(woke("queued", "early"), atom("normal")));
    // No message in the queue: the process waits for the next message.
    assert_eq!(
        run("later"),
        Term::tuple(vec![
            atom("waiting"),
            pair(atom("message_queue_len"), int(0)),
            woke("later", "wake"),
            atom("normal"),
        ])
    );
    // The `try` before the call does not catch the exit after the wake-up,
    // and the old frame does not continue.
    assert_eq!(run("dropped"), pair(atom("boom"), atom("none")));
    // The stack trace of `undef` has no frame from before the call.
    let undef = Term::tuple(vec![
        atom("hibernate3_probe"),
        atom("missing"),
        Term::list_from([int(1), int(2)]),
        Term::nil(),
    ]);
    assert_eq!(run("undef"), pair(atom("undef"), Term::list_from([undef])));
    // An exit signal reaches a hibernated process.
    assert_eq!(run("killed"), pair(atom("waiting"), atom("stop")));
    let error = |reason| pair(atom("error"), atom(reason));
    assert_eq!(
        run("errors"),
        Term::list_from([
            error("badarg"),
            error("badarg"),
            error("badarg"),
            error("badarg"),
            error("system_limit"),
            error("system_limit"),
            error("badarg"),
        ])
    );
    assert_eq!(run("wide"), pair(atom("undef"), int(256)));
    // The wake-up applies its MFA with no frame, so a raise after the
    // wake-up has no frame of `erlang:apply/3` in its stack trace
    // (`bif.c:1403-1427`).
    let probe = |function: &str, arguments: Term| {
        Term::tuple(vec![atom("hibernate3_probe"), atom(function), arguments])
    };
    let failed = probe("fail", Term::list_from([atom("n")]));
    assert_eq!(
        run("raised"),
        Term::list_from([
            pair(atom("boom"), Term::nil()),
            pair(pair(atom("nocatch"), atom("t")), Term::nil()),
            atom("x"),
            pair(atom("failed"), Term::list_from([failed.clone()])),
            pair(
                atom("failed"),
                Term::list_from([failed, probe("nested", int(1))])
            ),
        ])
    );
    // The `apply` instruction and a fun call reach the BIF through its
    // export.
    assert_eq!(
        run("applied"),
        Term::tuple(vec![
            woke("apply", "wake"),
            atom("normal"),
            woke("fun", "wake"),
            atom("normal"),
        ])
    );
    // A BIF as the MFA: the wake-up calls the BIF, not its BEAM stub.
    // A wake-up into `erlang:hibernate/3` hibernates again. A BIF that
    // raises ends the process, and `erlang:apply/3` applies its MFA.
    assert_eq!(
        run("bif"),
        Term::tuple(
            [
                "woke", "normal", "again", "normal", "stop", "applied", "normal"
            ]
            .map(atom)
            .to_vec()
        )
    );
    // `proc_lib:hibernate/3` wakes in `proc_lib:wake_up/3`.
    assert_eq!(
        run("proc_lib"),
        pair(woke("proc_lib", "wake"), atom("normal"))
    );
    // The idle path of `erts_code_purger` hibernates into its receive loop,
    // which serves each later request.
    let reply = pair(atom("reply"), atom("true"));
    assert_eq!(run("purger"), pair(reply.clone(), reply));
    // The wake-up and the start at `erlang:apply/3` apply the MFA as
    // `beam_run_process` does, so the `erlang:apply/3` body of
    // `erlang.beam` costs no reduction (`bif.c:1426-1427`,
    // `erl_process.c:12616-12624`).
    assert_eq!(run("reductions"), Term::list_from([2, 1, 1].map(int)));

    let reference = Command::new("erl")
        .args([
            "-noshell",
            "-pa",
            "fixtures/hibernate",
            "-eval",
            "logger:set_primary_config(level, none), io:format(\"~w~n\", [[hibernate3_probe:F() || F <- [queued, later, dropped, undef, killed, errors, wide, raised, applied, bif, proc_lib, purger, reductions]]]), halt().",
        ])
        .output()
        .unwrap();
    assert!(reference.status.success());
    assert_eq!(reference.stderr, [] as [u8; 0]);
    assert_eq!(
        reference.stdout,
        b"[{{queued,early,0},normal},{waiting,{message_queue_len,0},{later,wake,0},normal},{boom,none},{undef,[{hibernate3_probe,missing,[1,2],[]}]},{waiting,stop},[{error,badarg},{error,badarg},{error,badarg},{error,badarg},{error,system_limit},{error,system_limit},{error,badarg}],{undef,256},[{boom,[]},{{nocatch,t},[]},x,{failed,[{hibernate3_probe,fail,[n]}]},{failed,[{hibernate3_probe,fail,[n]},{hibernate3_probe,nested,1}]}],{{apply,wake,0},normal,{'fun',wake,0},normal},{woke,normal,again,normal,stop,applied,normal},{{proc_lib,wake,0},normal},{{reply,true},{reply,true}},[2,1,1]]\n"
    );
}

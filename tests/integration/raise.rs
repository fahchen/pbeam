use std::process::Command;

use pbeam_runtime::{BeamProgram, Term};

#[path = "../common/mod.rs"]
mod common;

use common::pinned_beam;

const RAISE_PROBE: &[u8] = include_bytes!("../../fixtures/raise/raise_probe.beam");

/// Evaluate `raise_probe:main()` on the pinned ERTS and return its `~w`
/// output. `-kernel logger_level none` stops the error report of the
/// process that `down/0` starts, as in the row "Undefined functions".
fn reference() -> String {
    let reference = Command::new("erl")
        .args([
            "-noshell",
            "-kernel",
            "logger_level",
            "none",
            "-pa",
            "fixtures/raise",
            "-eval",
            "io:format(\"~w~n\", [raise_probe:main()]), halt().",
        ])
        .output()
        .unwrap();
    assert!(
        reference.status.success() && reference.stderr.is_empty(),
        "{reference:?}"
    );
    String::from_utf8(reference.stdout).unwrap()
}

#[test]
fn raises_an_exception_again_with_raise_as_pinned_erts() {
    // No `catch` clause of `rethrow/1` matches the class, so the compiler
    // raises the exception again with `raise x2 x1`. ERTS `i_raise` takes the
    // class from the raw stack trace in `x2` and the reason from `x1`
    // (`emu/ops.tab:284-290`, `emu/instrs.tab:1236-1245`,
    // `beam_common.c:1089-1097`). `erlang:raise/3` with `[]` gives the stack
    // trace `[]` in ERTS and in PBeam, so the values are equal. The pinned
    // `erlang` module has `erlang:spawn_monitor/3`.
    let erlang = pinned_beam("erlang");
    let program = BeamProgram::load(&[RAISE_PROBE, &erlang]).unwrap();
    let atom = |name: &str| Term::Atom(program.atom(name).unwrap());
    let tuple = Term::tuple;
    let empty = || Term::nil();
    let reason = |class: &str| tuple(vec![atom("reason"), atom(class)]);
    let caught = |class: &str| tuple(vec![atom(class), reason(class), empty()]);
    let exit = |reason: Term| tuple(vec![atom("EXIT"), reason]);
    assert_eq!(
        program.execute("raise_probe", "main", &[]).unwrap().term,
        tuple(vec![
            Term::list_from(vec![
                tuple(vec![
                    caught("error"),
                    exit(tuple(vec![reason("error"), empty()])),
                ]),
                tuple(vec![caught("exit"), exit(reason("exit"))]),
                tuple(vec![caught("throw"), reason("throw")]),
            ]),
            // The stack trace keeps the frame of the missing function.
            tuple(vec![
                atom("undef"),
                tuple(vec![atom("raise_probe"), atom("missing"), empty(), empty()]),
            ]),
            // A throw with no catch becomes the error `{nocatch, Reason}`
            // (`beam_common.c:509-516`).
            tuple(vec![tuple(vec![atom("nocatch"), reason("throw")]), empty(),]),
        ])
    );
    assert_eq!(
        reference(),
        "{[{{error,{reason,error},[]},{'EXIT',{{reason,error},[]}}},\
         {{exit,{reason,exit},[]},{'EXIT',{reason,exit}}},\
         {{throw,{reason,throw},[]},{reason,throw}}],\
         {undef,{raise_probe,missing,[],[]}},\
         {{nocatch,{reason,throw}},[]}}\n"
    );
}

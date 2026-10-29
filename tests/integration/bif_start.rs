#[path = "../common/mod.rs"]
mod common;

use std::process::Command;

use common::pinned_beam;
use pbeam_runtime::{BeamProgram, Term};

const BIF_START_PROBE: &[u8] = include_bytes!("../../fixtures/bif_start/bif_start_probe.beam");

#[test]
fn runs_a_bif_as_the_initial_call_of_a_process_as_pinned_erts() {
    // A process starts with `i_apply_only` of its MFA
    // (`erl_process.c:12616`), which goes to the export entry of the MFA
    // (`beam_common.c:1559-1565`). The export of a BIF calls the BIF, and
    // not the `erlang:nif_error(undefined)` body of `erlang.beam`
    // (`emu/emu_load.c:704-713`, `jit/arm/ops.tab:917-927`).
    let erlang = pinned_beam("erlang");
    let program = BeamProgram::load(&[&erlang, BIF_START_PROBE]).unwrap();
    let atom = |name: &str| Term::Atom(program.atom(name).unwrap());
    let pair = |first, second| Term::tuple(vec![first, second]);
    let run = |function| {
        program
            .execute("bif_start_probe", function, &[])
            .unwrap()
            .term
    };
    // `throw/1`, `exit/1`, `error/1,2,3`, `abs/1`, and `apply/3` of
    // `error/1`. The stack trace of an initial call has no frame
    // (`apply_bif_error_adjustment`, `beam_common.c:1345-1365`).
    let x = || pair(atom("x"), Term::nil());
    assert_eq!(
        run("downs"),
        Term::list_from(vec![
            pair(pair(atom("nocatch"), atom("x")), Term::nil()),
            atom("x"),
            x(),
            x(),
            x(),
            atom("normal"),
            x(),
        ])
    );
    // `erlang:send/2` sends its message, and the process exits `normal`.
    assert_eq!(run("sent"), pair(atom("started"), atom("normal")));

    // The level `none` drops the error reports of the crashed processes,
    // which the logger writes after the result.
    let reference = Command::new("erl")
        .args([
            "-noshell",
            "-kernel",
            "logger_level",
            "none",
            "-pa",
            "fixtures/bif_start",
            "-eval",
            "io:format(\"~w~n~w~n\", [bif_start_probe:downs(), bif_start_probe:sent()]), halt().",
        ])
        .output()
        .unwrap();
    assert!(reference.status.success());
    assert_eq!(reference.stderr, [] as [u8; 0]);
    assert_eq!(
        String::from_utf8(reference.stdout).unwrap(),
        "[{{nocatch,x},[]},x,{x,[]},{x,[]},{x,[]},normal,{x,[]}]\n{started,normal}\n"
    );
}

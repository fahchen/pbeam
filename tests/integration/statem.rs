use std::process::Command;

use pbeam_runtime::{BeamProgram, Term};

#[path = "../common/mod.rs"]
mod common;

use common::pinned_beam;

const STATEM_PROBE: &[u8] = include_bytes!("../../fixtures/statem/statem_probe.beam");

#[test]
fn runs_a_gen_statem_server_entered_with_enter_loop() {
    // `gen_statem:call/3` has two `raw_raise` instructions (opcode 161),
    // which PBeam skips at load time. Without the skip, the load fails with
    // `unsupported BEAM opcode 161`. The enter-loop path does not run them.
    // `proc_lib`, `lists`, and `erlang` are not loaded: the path calls only
    // BIFs in those modules, and `lists:keyfind/3` runs from the BIF table.
    let gen_statem = pinned_beam("gen_statem");
    let gen_module = pinned_beam("gen");
    let program = BeamProgram::load(&[STATEM_PROBE, &gen_statem, &gen_module]).unwrap();

    // The server replies with its data twice and keeps `{1}` after the first
    // ping. On `stop`, `gen_statem:terminate/7` gets the stack trace `[]`
    // and runs `erlang:exit(normal)` through `apply_last`, so the `DOWN`
    // reason is `normal`.
    assert_eq!(
        program.execute("statem_probe", "main", &[]).unwrap(),
        Term::Tuple(vec![
            Term::Integer(1),
            Term::Tuple(vec![Term::Integer(1)]),
            Term::Atom(program.atom("normal").unwrap()),
        ])
    );

    let reference = Command::new("erl")
        .args([
            "-noshell",
            "-pa",
            "fixtures/statem",
            "-eval",
            "io:format(\"~p~n\", [statem_probe:main()]), halt().",
        ])
        .output()
        .unwrap();
    assert!(reference.status.success());
    assert!(reference.stderr.is_empty());
    assert_eq!(reference.stdout, b"{1,{1},normal}\n");
}

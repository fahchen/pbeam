use std::process::Command;

use pbeam_runtime::BeamProgram;

#[path = "../common/mod.rs"]
mod common;

use common::pinned_beam;

const PROBE: &[u8] = include_bytes!("../../fixtures/bif_trap_stack/bif_trap_stack_probe.beam");

/// The `%T` text of the exit reason of `bif_trap_stack_probe:uncaught()` on
/// the pinned ERTS. Each case is a BIF that traps on a long list and then
/// raises. The `lists` BIFs keep the rest of the list in the registers
/// (`erl_bif_lists.c:1129-1139`, `1191`, `1240-1243`), and the other BIFs
/// put the original arguments back (`ERTS_BIF_ERROR_TRAPPED1`,
/// `bif.h:242-249`), so the first frame is the frame of the BIF call with
/// its `error_info`.
const EXPECTED: &str = include_str!("../../fixtures/bif_trap_stack/bif_trap_stack_probe.expected");

#[test]
fn gives_the_stack_trace_of_a_bif_error_after_a_trap_as_pinned_erts() {
    let reference = Command::new("erl")
        .args([
            "-noshell",
            "-pa",
            "fixtures/bif_trap_stack",
            "-eval",
            "{'EXIT', R} = (catch bif_trap_stack_probe:uncaught()), io:format(\"~s~n\", [erts_internal:term_to_string(R, 100000)]), halt().",
        ])
        .output()
        .unwrap();
    assert!(
        reference.status.success() && reference.stderr.is_empty(),
        "{reference:?}"
    );
    assert_eq!(String::from_utf8(reference.stdout).unwrap(), EXPECTED);

    let erlang = pinned_beam("erlang");
    let lists = pinned_beam("lists");
    let program = BeamProgram::load(&[PROBE, &erlang, &lists]).unwrap();
    let error = program
        .execute("bif_trap_stack_probe", "uncaught", &[])
        .unwrap_err();
    assert_eq!(
        format!("{error}\n"),
        format!("root process exited with {EXPECTED}")
    );
}

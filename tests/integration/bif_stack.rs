//! Checks the stack trace of a BIF error on PBeam against the pinned ERTS.

use std::process::Command;

use pbeam_runtime::BeamProgram;

#[path = "../common/mod.rs"]
mod common;

use common::pinned_beam;

const BIF_STACK_PROBE: &[u8] = include_bytes!("../../fixtures/bif_stack/bif_stack_probe.beam");

/// The `%T` text of the exit reason of `bif_stack_probe:uncaught()` on the
/// pinned ERTS, as `erts_internal:term_to_string/2` (`bif.c:4156-4190`)
/// writes it. ERTS `save_stacktrace` puts the frame of the BIF with its
/// arguments and its `error_info` location first, then the function that
/// called the BIF, then the continuation pointers of the stack
/// (`beam_common.c:880-1055`). The cases cover a `bif` and a `gc_bif`
/// instruction with the fail label 0, `call_ext`, `call_ext_only`, and
/// `call_ext_last` of a BIF, the `apply` instruction, `apply/2`,
/// `apply/3`, a tail apply, a body and a tail call of a fun of a BIF, a
/// BIF as the initial call of a process, a BIF after a wake-up from
/// `erlang:hibernate/3`, `erlang:spawn_opt/2` with a bad option, and a BIF
/// that traps.
const EXPECTED: &str = include_str!("../../fixtures/bif_stack/bif_stack_probe.expected");

#[test]
fn builds_the_stack_trace_of_a_bif_error_as_pinned_erts() {
    // The run removes the default logger handler, so the error reports of
    // the exits of the spawned processes do not go to the output.
    let reference = Command::new("erl")
        .args([
            "-noshell",
            "-pa",
            "fixtures/bif_stack",
            "-eval",
            "logger:remove_handler(default), {'EXIT', R} = (catch bif_stack_probe:uncaught()), io:format(\"~s~n\", [erts_internal:term_to_string(R, 100000)]), halt().",
        ])
        .output()
        .unwrap();
    assert!(
        reference.status.success() && reference.stderr.is_empty(),
        "{reference:?}"
    );
    assert_eq!(String::from_utf8(reference.stdout).unwrap(), EXPECTED);

    // The root process exits with the result (row "Run end" of
    // `docs/erts-differences.md`).
    let erlang = pinned_beam("erlang");
    let lists = pinned_beam("lists");
    let program = BeamProgram::load(&[BIF_STACK_PROBE, &erlang, &lists]).unwrap();
    let error = program
        .execute("bif_stack_probe", "uncaught", &[])
        .unwrap_err();
    assert_eq!(
        format!("{error}\n"),
        format!("root process exited with {EXPECTED}")
    );
}

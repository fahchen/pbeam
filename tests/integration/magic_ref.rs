//! Checks the magic marker of each kind of reference on PBeam against the pinned ERTS.
#![expect(
    clippy::unwrap_used,
    reason = "test helpers report a failure by a panic"
)]

use std::process::Command;

use pbeam_runtime::BeamProgram;

#[path = "../common/mod.rs"]
mod common;

use common::pinned_beam;

const MAGIC_REF_PROBE: &[u8] = include_bytes!("../../fixtures/magic_ref/magic_ref_probe.beam");

/// The `%T` text of the exit reason of `magic_ref_probe:uncaught()` on the
/// pinned ERTS: for each reference, `is_reference/1` and the magic marker
/// bit of `N1` in the printed form and in `term_to_binary/2`
/// (`ERTS_REF1_MAGIC_MARKER_BIT__`, `erl_bif_unique.h:102-105`), the term
/// order of each pair (`erts_cmp`, `utils.c:2283-2286`), and the
/// `error_info` cause of `ets:lookup/2` on an atomics reference (`tid2tab`,
/// `erl_db.c:287-296`).
const EXPECTED: &str = include_str!("../../fixtures/magic_ref/magic_ref_probe.expected");

#[test]
fn marks_each_magic_reference_as_pinned_erts() {
    let reference = Command::new("erl")
        .args([
            "-noshell",
            "-pa",
            "fixtures/magic_ref",
            "-eval",
            "{'EXIT', R} = (catch magic_ref_probe:uncaught()), io:format(\"~s~n\", [erts_internal:term_to_string(R, 100000)]), halt().",
        ])
        .output()
        .unwrap();
    assert!(
        reference.status.success() && reference.stderr.is_empty(),
        "{reference:?}"
    );
    assert_eq!(String::from_utf8(reference.stdout).unwrap(), EXPECTED);

    let modules: Vec<Vec<u8>> = [
        "erlang",
        "ets",
        "atomics",
        "counters",
        "binary",
        "re",
        "prim_buffer",
        "lists",
        "maps",
        "proplists",
        "string",
        "unicode_util",
    ]
    .into_iter()
    .map(pinned_beam)
    .collect();
    let mut beams: Vec<&[u8]> = modules.iter().map(Vec::as_slice).collect();
    beams.push(MAGIC_REF_PROBE);
    let program = BeamProgram::load(&beams).unwrap();
    let error = program
        .execute("magic_ref_probe", "uncaught", &[])
        .unwrap_err();
    assert_eq!(
        format!("{error}\n"),
        format!("root process exited with {EXPECTED}")
    );
}

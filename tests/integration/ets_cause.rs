use std::process::Command;

use pbeam_runtime::BeamProgram;

#[path = "../common/mod.rs"]
mod common;

use common::pinned_beam;

const ETS_CAUSE_PROBE: &[u8] = include_bytes!("../../fixtures/ets/ets_cause_probe.beam");

/// The `%T` text of the exit reason of `ets_cause_probe:uncaught()` on the
/// pinned ERTS. Each case gives the reason, the BIF, the `error_info` map
/// of the BIF frame, and the text of `erl_stdlib_errors:format_error/2`.
/// ERTS adds `cause => Cause` when the BIF raises with `EXF_HAS_EXT_INFO`
/// (`beam_common.c:993-1002`), and the ETS BIFs set the cause in
/// `erl_db.c` (`EXI_*`, `erl_db.c:53-63`).
const EXPECTED: &str = include_str!("../../fixtures/ets/ets_cause_probe.expected");

#[test]
fn gives_the_error_info_cause_of_the_ets_bifs_as_pinned_erts() {
    let reference = Command::new("erl")
        .args([
            "-noshell",
            "-pa",
            "fixtures/ets",
            "-eval",
            "{'EXIT', R} = (catch ets_cause_probe:uncaught()), io:format(\"~s~n\", [erts_internal:term_to_string(R, 100000)]), halt().",
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
        "lists",
        "maps",
        "proplists",
        "erl_stdlib_errors",
        "io_lib",
        "unicode",
    ]
    .into_iter()
    .map(pinned_beam)
    .collect();
    let mut beams: Vec<&[u8]> = modules.iter().map(Vec::as_slice).collect();
    beams.push(ETS_CAUSE_PROBE);
    let program = BeamProgram::load(&beams).unwrap();
    let error = program
        .execute("ets_cause_probe", "uncaught", &[])
        .unwrap_err();
    assert_eq!(
        format!("{error}\n"),
        format!("root process exited with {EXPECTED}")
    );
}

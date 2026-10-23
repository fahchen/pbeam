use std::process::Command;

use pbeam_runtime::BeamProgram;

#[path = "../common/mod.rs"]
mod common;

use common::pinned_beam;

const BIF_CAUSE_PROBE: &[u8] = include_bytes!("../../fixtures/bif_cause/bif_cause_probe.beam");

/// The `%T` text of the exit reason of `bif_cause_probe:uncaught()` on the
/// pinned ERTS. Each case gives the reason, the BIF, the `error_info` map
/// of the BIF frame, and the text of `erl_erts_errors:format_error/2`.
/// ERTS adds `cause => Cause` when the BIF raises with `EXF_HAS_EXT_INFO`
/// (`beam_common.c:993-1002`), and the BIFs of `bif.c` set the cause in
/// `p->fvalue` (`bif.c:966`, `993`, `1902-1910`, `2721-2728`, `4396`, `5298`; `register.c:183-227`).
const EXPECTED: &str = include_str!("../../fixtures/bif_cause/bif_cause_probe.expected");

#[test]
fn gives_the_error_info_cause_of_the_erlang_bifs_of_bif_c_as_pinned_erts() {
    let reference = Command::new("erl")
        .args([
            "-noshell",
            "-pa",
            "fixtures/bif_cause",
            "-eval",
            "{'EXIT', R} = (catch bif_cause_probe:uncaught()), io:format(\"~s~n\", [erts_internal:term_to_string(R, 100000)]), halt().",
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
        "erl_erts_errors",
        "lists",
        "maps",
        "proplists",
        "io_lib_format",
        "io_lib",
        "unicode",
    ]
    .into_iter()
    .map(pinned_beam)
    .collect();
    let mut beams: Vec<&[u8]> = modules.iter().map(Vec::as_slice).collect();
    beams.push(BIF_CAUSE_PROBE);
    let program = BeamProgram::load(&beams).unwrap();
    let error = program
        .execute("bif_cause_probe", "uncaught", &[])
        .unwrap_err();
    assert_eq!(
        format!("{error}\n"),
        format!("root process exited with {EXPECTED}")
    );
}

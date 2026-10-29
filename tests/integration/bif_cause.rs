//! Checks the `error_info` cause of the BIF errors on PBeam against the pinned ERTS.

use std::process::Command;

use pbeam_runtime::BeamProgram;

#[path = "../common/mod.rs"]
mod common;

use common::pinned_beam;

const BIF_CAUSE_PROBE: &[u8] = include_bytes!("../../fixtures/bif_cause/bif_cause_probe.beam");
const TIMER_CAUSE_PROBE: &[u8] = include_bytes!("../../fixtures/bif_cause/timer_cause_probe.beam");
const PORT_CAUSE_PROBE: &[u8] = include_bytes!("../../fixtures/bif_cause/port_cause_probe.beam");

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

/// The `%T` text of the exit reason of `timer_cause_probe:uncaught()` on the
/// pinned ERTS. `erlang:send_after/3,4` and `erlang:start_timer/3,4` set the
/// cause `badopt` or `time` in `BIF_P->fvalue` (`erl_hl_timer.c:2573-2597`,
/// `2612-2636`). `erlang:cancel_timer/1,2` and
/// `erlang:read_timer/1` give no cause.
const TIMER_EXPECTED: &str = include_str!("../../fixtures/bif_cause/timer_cause_probe.expected");

#[test]
fn gives_the_error_info_cause_of_the_timer_bifs_as_pinned_erts() {
    let reference = Command::new("erl")
        .args([
            "-noshell",
            "-pa",
            "fixtures/bif_cause",
            "-eval",
            "{'EXIT', R} = (catch timer_cause_probe:uncaught()), io:format(\"~s~n\", [erts_internal:term_to_string(R, 100000)]), halt().",
        ])
        .output()
        .unwrap();
    assert!(
        reference.status.success() && reference.stderr.is_empty(),
        "{reference:?}"
    );
    assert_eq!(String::from_utf8(reference.stdout).unwrap(), TIMER_EXPECTED);

    let modules: Vec<Vec<u8>> = ["erlang", "erl_erts_errors", "lists", "maps", "proplists"]
        .into_iter()
        .map(pinned_beam)
        .collect();
    let mut beams: Vec<&[u8]> = modules.iter().map(Vec::as_slice).collect();
    beams.push(TIMER_CAUSE_PROBE);
    let program = BeamProgram::load(&beams).unwrap();
    let error = program
        .execute("timer_cause_probe", "uncaught", &[])
        .unwrap_err();
    assert_eq!(
        format!("{error}\n"),
        format!("root process exited with {TIMER_EXPECTED}")
    );
}

/// The `%T` text of the exit reason of `port_cause_probe:uncaught()` on the
/// pinned ERTS. An option error of `re:compile/2` and `re:run/3`
/// (`erl_bif_re.c:908-911`, `1501-1503`), a bad mode of `os:set_signal/2`
/// (`erl_bif_os.c:146-150`), a bad type of `erlang:decode_packet/3`
/// (`erl_bif_port.c:1508-1510`), and a bad option of `binary_to_term/2`
/// (`external.c:2191-2193`) set the cause `badopt`. The other errors give
/// no cause.
const PORT_EXPECTED: &str = include_str!("../../fixtures/bif_cause/port_cause_probe.expected");

#[test]
fn gives_the_error_info_cause_of_the_re_os_port_and_external_bifs_as_pinned_erts() {
    let reference = Command::new("erl")
        .args([
            "-noshell",
            "-pa",
            "fixtures/bif_cause",
            "-eval",
            "{'EXIT', R} = (catch port_cause_probe:uncaught()), io:format(\"~s~n\", [erts_internal:term_to_string(R, 100000)]), halt().",
        ])
        .output()
        .unwrap();
    assert!(
        reference.status.success() && reference.stderr.is_empty(),
        "{reference:?}"
    );
    assert_eq!(String::from_utf8(reference.stdout).unwrap(), PORT_EXPECTED);

    let modules: Vec<Vec<u8>> = [
        "erlang",
        "erl_erts_errors",
        "erl_stdlib_errors",
        "erl_kernel_errors",
        "re",
        "os",
        "lists",
        "maps",
        "proplists",
        "unicode",
    ]
    .into_iter()
    .map(pinned_beam)
    .collect();
    let mut beams: Vec<&[u8]> = modules.iter().map(Vec::as_slice).collect();
    beams.push(PORT_CAUSE_PROBE);
    let program = BeamProgram::load(&beams).unwrap();
    let error = program
        .execute("port_cause_probe", "uncaught", &[])
        .unwrap_err();
    assert_eq!(
        format!("{error}\n"),
        format!("root process exited with {PORT_EXPECTED}")
    );
}

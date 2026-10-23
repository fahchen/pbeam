use std::process::Command;

use pbeam_runtime::{BeamProgram, Term};

#[path = "../common/mod.rs"]
mod common;

const NET_PROBE: &[u8] = include_bytes!("../../fixtures/net/net_probe.beam");

/// Load the probe with the pinned `prim_net`, whose `on_load/0` loads the
/// static `net` NIF library, the pinned `net`, and the modules of the probe.
fn program() -> BeamProgram {
    let modules: Vec<Vec<u8>> = ["prim_net", "net", "maps", "lists"]
        .into_iter()
        .map(common::pinned_beam)
        .collect();
    let mut beams: Vec<&[u8]> = modules.iter().map(Vec::as_slice).collect();
    beams.push(NET_PROBE);
    BeamProgram::load(&beams).unwrap()
}

#[test]
fn gives_the_info_the_command_and_the_host_name_as_the_pinned_erts() {
    let output = Command::new("erl")
        .args([
            "-noshell",
            "-pa",
            "fixtures/net",
            "-eval",
            "io:format(\"~s\", [binary:encode_hex(net_probe:text())]), halt().",
        ])
        .output()
        .unwrap();
    assert!(
        output.status.success() && output.stderr.is_empty(),
        "{output:?}"
    );
    let term = program().execute("net_probe", "text", &[]).unwrap().term;
    let Term::Binary(text) = &term else {
        panic!("the probe gives a binary");
    };
    let hex: String = text
        .bytes()
        .unwrap()
        .iter()
        .map(|byte| format!("{byte:02X}"))
        .collect();
    assert_eq!(hex.as_bytes(), output.stdout);
}

#[test]
fn stops_the_run_at_the_debug_trace() {
    // ERTS writes a debug trace of each later `net` call; PBeam has no such
    // trace (row "NIF stubs" of `docs/erts-differences.md`).
    let error = program().execute("net_probe", "debug_on", &[]).unwrap_err();
    assert_eq!(
        error.to_string(),
        "prim_net:nif_command/1 debug trace is not supported"
    );
}

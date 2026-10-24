use std::process::Command;

use pbeam_runtime::{BeamProgram, Term};

#[path = "../common/mod.rs"]
mod common;

const NET_PROBE: &[u8] = include_bytes!("../../fixtures/net/net_probe.beam");

/// Load the probe with the pinned `prim_net`, whose `on_load/0` loads the
/// static `net` NIF library, the pinned `net`, the pinned `prim_socket` and
/// `socket_registry`, whose socket address encoding and protocol table
/// `prim_net` reads, and the modules of the probe.
fn program() -> BeamProgram {
    let modules: Vec<Vec<u8>> = [
        "socket_registry",
        "prim_socket",
        "prim_net",
        "net",
        "maps",
        "lists",
        "binary",
    ]
    .into_iter()
    .map(common::pinned_beam)
    .collect();
    let mut beams: Vec<&[u8]> = modules.iter().map(Vec::as_slice).collect();
    beams.push(NET_PROBE);
    BeamProgram::load(&beams).unwrap()
}

#[test]
fn gives_the_info_the_command_the_host_name_the_lookups_and_the_interfaces_as_the_pinned_erts() {
    let output = Command::new("erl")
        .args([
            "-noshell",
            "-pa",
            "fixtures/net",
            "-eval",
            "io:format(\"~s\", [net_probe:text()]), halt().",
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
    let text = String::from_utf8(text.bytes().unwrap().to_vec()).unwrap();
    let reference = String::from_utf8(output.stdout).unwrap();
    assert_eq!(text.lines().count(), reference.lines().count());
    for (index, (line, expected)) in text.lines().zip(reference.lines()).enumerate() {
        assert_eq!(line, expected, "case {index}");
    }
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

#[test]
#[cfg(target_os = "linux")]
fn gives_the_interface_addresses_as_the_pinned_erts() {
    let output = Command::new("erl")
        .args([
            "-noshell",
            "-pa",
            "fixtures/net",
            "-eval",
            "io:format(\"~s\", [net_probe:ifaddrs_text()]), halt().",
        ])
        .output()
        .unwrap();
    assert!(
        output.status.success() && output.stderr.is_empty(),
        "{output:?}"
    );
    let term = program()
        .execute("net_probe", "ifaddrs_text", &[])
        .unwrap()
        .term;
    let Term::Binary(text) = &term else {
        panic!("the probe gives a binary");
    };
    let text = String::from_utf8(text.bytes().unwrap().to_vec()).unwrap();
    let reference = String::from_utf8(output.stdout).unwrap();
    assert_eq!(text.lines().count(), 4);
    for (index, (line, expected)) in text.lines().zip(reference.lines()).enumerate() {
        assert_eq!(line, expected, "case {index}");
    }
}

#[test]
#[cfg(target_os = "linux")]
fn stops_the_run_at_a_network_namespace() {
    // PBeam does not change the network namespace of a thread (row "NIF
    // stubs" of `docs/erts-differences.md`).
    let error = program()
        .execute("net_probe", "ifaddrs_netns", &[])
        .unwrap_err();
    assert_eq!(
        error.to_string(),
        "prim_net:nif_getifaddrs/1 network namespace is not supported"
    );
}

#[test]
#[cfg(not(target_os = "linux"))]
fn stops_the_run_at_the_interface_addresses_of_another_host() {
    // The safe `getifaddrs` of `nix` drops the link address bytes past
    // `sdl_data` and each address of an unknown family (row "NIF stubs" of
    // `docs/erts-differences.md`).
    let error = program()
        .execute("net_probe", "ifaddrs_netns", &[])
        .unwrap_err();
    assert_eq!(
        error.to_string(),
        "prim_net:nif_getifaddrs/1 is supported only on Linux"
    );
}

#[test]
fn stops_the_run_at_the_debug_trace_of_the_interface_addresses() {
    let error = program()
        .execute("net_probe", "ifaddrs_debug", &[])
        .unwrap_err();
    assert_eq!(
        error.to_string(),
        "prim_net:nif_getifaddrs/1 debug trace is not supported"
    );
}

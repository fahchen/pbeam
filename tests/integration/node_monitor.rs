//! `erlang:process_flag/2` with `async_dist`, `monitor_nodes`, and
//! `{monitor_nodes, Options}` (`bif.c:2032-2044`, `2202-2227`;
//! `erts_monitor_nodes`, `dist.c:6731-6845`), and `erlang:nodes/0,1,2`
//! (`dist.c:6240-6476`) on a node that is not alive.

use std::process::Command;

use pbeam_runtime::{BeamProgram, Term};

#[path = "../common/mod.rs"]
mod common;

use common::pinned_beam;

const PROBE: &[u8] = include_bytes!("../../fixtures/node_monitor/node_monitor_probe.beam");

#[test]
fn runs_the_node_monitor_flags_and_nodes_as_pinned_erts() {
    // `node_monitor_probe:main/0` compares each result with the value of
    // the pinned ERTS and gives `ok` when all are equal. The probe calls
    // `net_kernel:monitor_nodes/1,2` of the pinned `net_kernel.beam`
    // (`net_kernel.erl:477-481`, `652-673`), which call `maps` and
    // `lists`.
    let modules = ["erlang", "net_kernel", "maps", "lists"].map(pinned_beam);
    let mut beams: Vec<&[u8]> = modules.iter().map(Vec::as_slice).collect();
    beams.push(PROBE);
    let program = BeamProgram::load(&beams).unwrap();
    assert_eq!(
        program
            .execute("node_monitor_probe", "main", &[])
            .unwrap()
            .term,
        Term::Atom(program.atom("ok").unwrap())
    );

    let reference = Command::new("erl")
        .args([
            "-noshell",
            "-pa",
            "fixtures/node_monitor",
            "-eval",
            "io:format(\"~p~n\", [node_monitor_probe:main()]), halt().",
        ])
        .output()
        .unwrap();
    assert!(reference.status.success(), "{reference:?}");
    assert!(reference.stderr.is_empty(), "{reference:?}");
    assert_eq!(reference.stdout, b"ok\n");
}

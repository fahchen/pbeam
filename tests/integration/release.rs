//! The release of a large graph of task
//! [#559](https://github.com/fahchen/pbeam/issues/559), row 11 of
//! `docs/term-performance-design-codex-v2.md`.
//!
//! The fixture builds the graph in a live process on each path that the row
//! names: a process that exits with the graph in its process dictionary, a
//! graph that a send shares between a sender that exits and the receiver,
//! and a graph that stays in a child process when the root process ends the
//! run. Each call must give the pinned ERTS result, so no path loses a value
//! or a root.
//!
//! The graph of this test has 100000 cells, which keeps the check gate
//! short. `mise run bench-release` releases a graph of 1000000 cells through
//! the same paths.

use std::process::Command;

use pbeam_runtime::{BeamProgram, Term};

#[path = "../common/mod.rs"]
mod common;

use common::pinned_beam;

const RELEASE_PROBE: &[u8] = include_bytes!("../../fixtures/release/release_probe.beam");

/// The number of cells of the graph of each call of the fixture.
const CELLS: i64 = 100_000;

#[test]
fn releases_a_large_graph_on_each_path_as_pinned_erts_does() {
    // `erlang:spawn_opt/4` runs from the pinned `erlang.beam`, and the other
    // calls are BIFs and instructions.
    let erlang = pinned_beam("erlang");
    let program = BeamProgram::load(&[&erlang, RELEASE_PROBE]).unwrap();
    let cells = Term::Integer(CELLS);
    let ok = Term::Atom(program.atom("ok").unwrap());
    assert_eq!(
        program
            .execute("release_probe", "exits_with", std::slice::from_ref(&cells))
            .unwrap(),
        ok,
        "the exit of a process that owns the graph keeps the result"
    );
    assert_eq!(
        program
            .execute("release_probe", "shared", std::slice::from_ref(&cells))
            .unwrap(),
        Term::Integer(CELLS),
        "the shared graph keeps every value after the sender exits"
    );
    assert_eq!(
        program
            .execute("release_probe", "shutdown", std::slice::from_ref(&cells))
            .unwrap(),
        ok,
        "the run ends while a child owns the graph"
    );

    let reference = Command::new("erl")
        .args([
            "-noshell",
            "-pa",
            "fixtures/release",
            "-eval",
            &format!(
                "io:format(\"~p~n\", [{{release_probe:exits_with({CELLS}), \
                 release_probe:shared({CELLS}), release_probe:shutdown({CELLS})}}]), halt()."
            ),
        ])
        .output()
        .unwrap();
    assert!(reference.status.success());
    assert!(reference.stderr.is_empty());
    assert_eq!(reference.stdout, b"{ok,100000,ok}\n");
}

//! The resources of the node on two scheduler threads. A zlib stream that
//! one root opens resolves on the scheduler of the other root, as an ERTS
//! resource does (`enif_get_resource`, `erl_nif.c`).

#[path = "../common/mod.rs"]
mod common;

use common::pinned_beam;
use pbeam_runtime::{BeamProgram, Term};

/// The fixture with the two roots. `reference/0` runs them on the pinned
/// ERTS.
const NODE_RESOURCES: &[u8] = include_bytes!("../../fixtures/multischeduler/node_resources.beam");

/// The number of runs, to exercise different interleavings of the threads.
const RUNS: usize = 10;

/// The root `zlib_give` gives the control of its stream to the root of the
/// other scheduler. A use after that raises `not_on_controlling_process`,
/// and the other root deflates with the stream.
#[test]
fn a_zlib_stream_resolves_on_another_scheduler() {
    let zlib = pinned_beam("zlib");
    let program = BeamProgram::load(&[&zlib, NODE_RESOURCES]).expect("the fixture loads");
    let atom = |name| Term::Atom(program.atom(name).expect("the fixture names the atom"));
    for _ in 0..RUNS {
        let results = program
            .execute_on_schedulers(&[
                ("node_resources", "zlib_give"),
                ("node_resources", "zlib_use"),
            ])
            .expect("the two schedulers run");
        assert_eq!(results[0].term, atom("not_on_controlling_process"));
        assert_eq!(results[1].term, atom("true"));
    }
}

/// The two roots give the same results on the pinned ERTS with two
/// schedulers.
#[test]
fn the_pinned_erts_gives_the_same_results() {
    let directory =
        std::env::temp_dir().join(format!("pbeam-node-resources-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&directory);
    std::fs::create_dir_all(&directory).expect("the directory of the reference run");
    std::fs::write(directory.join("node_resources.beam"), NODE_RESOURCES)
        .expect("the fixture file");
    let output = std::process::Command::new("erl")
        .args(["+S", "2", "-noshell"])
        .args([
            "-eval",
            "io:format(\"~w~n\", [node_resources:reference()]), halt().",
        ])
        .arg("-pa")
        .arg(&directory)
        .output()
        .expect("the pinned ERTS runs");
    std::fs::remove_dir_all(&directory).expect("the reference directory leaves");
    assert!(output.status.success(), "{output:?}");
    assert_eq!(
        String::from_utf8_lossy(&output.stdout),
        "{not_on_controlling_process,true}\n"
    );
}

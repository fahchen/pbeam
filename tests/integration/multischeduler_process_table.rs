//! The process table of the node, read from two scheduler threads. Each
//! pair of roots of `process_table` runs on two threads, and each root
//! returns `ok` when its result is the result of the pinned ERTS.

#[path = "../common/mod.rs"]
mod common;

use common::pinned_beam;
use pbeam_runtime::{BeamProgram, Term};

/// The fixture with the pairs of roots. `reference/0` runs each pair on the
/// pinned ERTS.
const PROCESS_TABLE: &[u8] = include_bytes!("../../fixtures/multischeduler/process_table.beam");

/// The number of runs of each pair, to exercise different interleavings of
/// the two threads.
const RUNS: usize = 10;

/// Run the roots `first` and `second` on two scheduler threads `RUNS` times,
/// and check that each root returns `ok`.
fn run_pair(first: &str, second: &str) {
    let (erlang, erts_internal, lists) = (
        pinned_beam("erlang"),
        pinned_beam("erts_internal"),
        pinned_beam("lists"),
    );
    let program = BeamProgram::load(&[&erlang, &erts_internal, &lists, PROCESS_TABLE])
        .expect("the fixture loads");
    let ok = Term::Atom(program.atom("ok").expect("the fixture names ok"));
    for _ in 0..RUNS {
        let results = program
            .execute_on_schedulers(&[("process_table", first), ("process_table", second)])
            .expect("the two schedulers run");
        assert_eq!(results[0].term, ok, "the root {first}");
        assert_eq!(results[1].term, ok, "the root {second}");
    }
}

/// `erlang:processes/0` gives the processes of both schedulers, each
/// scheduler reads the same list, and the process iterator gives the same
/// pids in the same order.
#[test]
fn lists_the_processes_of_the_node_from_each_scheduler() {
    run_pair("list_a", "list_b");
}

/// After the exit of a process of the other scheduler, the process list,
/// the process iterator, and `is_process_alive/1` do not find it on either
/// scheduler. The monitor of the watcher reaches the process before the
/// kill of the other root, so the `DOWN` reason is `killed`.
#[test]
fn finds_no_exited_process_on_either_scheduler() {
    run_pair("gone_a", "gone_b");
}

/// Each pair gives `ok` for each root on the pinned ERTS.
#[test]
fn the_pinned_erts_gives_the_same_results() {
    let directory =
        std::env::temp_dir().join(format!("pbeam-multischeduler-ptab-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&directory);
    std::fs::create_dir_all(&directory).expect("the directory of the reference run");
    std::fs::write(directory.join("process_table.beam"), PROCESS_TABLE).expect("the fixture file");
    let output = std::process::Command::new("erl")
        .args(["+S", "2", "-noshell"])
        .args([
            "-eval",
            "io:format(\"~w~n\", [process_table:reference()]), halt().",
        ])
        .arg("-pa")
        .arg(&directory)
        .output()
        .expect("the pinned ERTS runs");
    std::fs::remove_dir_all(&directory).expect("the reference directory leaves");
    assert!(output.status.success(), "{output:?}");
    assert_eq!(
        String::from_utf8_lossy(&output.stdout),
        "[{ok,ok},{ok,ok}]\n"
    );
}

//! Group-leader, is-alive, and process-info requests between processes of
//! two scheduler threads. Each pair of roots of `info_requests` runs on two
//! threads, and each root returns `ok` when its result is the result of the
//! pinned ERTS.
#![expect(
    clippy::unwrap_used,
    reason = "test helpers report a failure by a panic"
)]

#[path = "../common/mod.rs"]
mod common;

use common::pinned_beam;
use pbeam_runtime::{BeamProgram, Term};

/// The fixture with the pairs of roots. `reference/0` runs each pair on the
/// pinned ERTS.
const INFO_REQUESTS: &[u8] = include_bytes!("../../fixtures/multischeduler/info_requests.beam");

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
    let program = BeamProgram::load(&[&erlang, &erts_internal, &lists, INFO_REQUESTS])
        .expect("the fixture loads");
    let ok = Term::Atom(program.atom("ok").expect("the fixture names ok"));
    for _ in 0..RUNS {
        let results = program
            .execute_on_schedulers(&[("info_requests", first), ("info_requests", second)])
            .expect("the two schedulers run");
        assert_eq!(results[0].term, ok, "the root {first}");
        assert_eq!(results[1].term, ok, "the root {second}");
    }
}

/// A read of the group leader of a remote process gives its group leader,
/// before and after a group-leader request changes it. After the exit of the
/// process, the request raises `badarg` and the read gives `undefined`.
#[test]
fn reads_and_changes_the_group_leader_of_a_remote_process() {
    run_pair("leader_a", "leader_b");
}

/// `is_process_alive/1` gives `true` for a live remote process, `false`
/// after an exit signal `kill` of the caller, and `false` after its `DOWN`.
#[test]
fn answers_the_liveness_of_a_remote_process_after_the_earlier_signals() {
    run_pair("alive_a", "alive_b");
}

/// `process_info/2` gives each supported item of a live remote process, and
/// the message count includes the messages of the caller before the read.
/// Each item of a dead process gives `undefined`.
#[test]
fn reads_the_items_of_a_remote_process() {
    run_pair("info_a", "info_b");
}

/// Each pair gives `ok` for each root on the pinned ERTS.
#[test]
fn the_pinned_erts_gives_the_same_results() {
    let directory =
        std::env::temp_dir().join(format!("pbeam-multischeduler-info-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&directory);
    std::fs::create_dir_all(&directory).expect("the directory of the reference run");
    std::fs::write(directory.join("info_requests.beam"), INFO_REQUESTS).expect("the fixture file");
    let output = std::process::Command::new("erl")
        .args(["+S", "2", "-noshell"])
        .args([
            "-eval",
            "io:format(\"~w~n\", [info_requests:reference()]), halt().",
        ])
        .arg("-pa")
        .arg(&directory)
        .output()
        .expect("the pinned ERTS runs");
    std::fs::remove_dir_all(&directory).expect("the reference directory leaves");
    assert!(output.status.success(), "{output:?}");
    assert_eq!(
        String::from_utf8_lossy(&output.stdout),
        "[{ok,ok},{ok,ok},{ok,ok}]\n"
    );
}

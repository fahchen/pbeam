//! Work stealing between two scheduler threads. The root `busy` of
//! `work_stealing` starts its workers on its own scheduler, and the root
//! `idle` only waits, so each process that runs on the second scheduler
//! moved there in a steal. Each worker records
//! `erlang:system_info(scheduler_id)` after each yield, so the result of
//! `busy` names each scheduler that ran a worker: a result with scheduler 2
//! proves a steal.

#[path = "../common/mod.rs"]
mod common;

use common::pinned_beam;
use pbeam_runtime::{BeamProgram, Term};

/// The fixture with the two roots. `reference/0` runs them on the pinned
/// ERTS.
const WORK_STEALING: &[u8] = include_bytes!("../../fixtures/multischeduler/work_stealing.beam");

/// The number of runs, to exercise different interleavings of the threads.
const RUNS: usize = 20;

/// Each run moves workers to the idle scheduler: the workers ran on both
/// schedulers. The ticks of each worker arrive in order, each ack reaches
/// its worker, each worker exits `normal` after its last tick, and the
/// spinner that the root kills while it yields exits `killed`.
#[test]
fn an_idle_scheduler_takes_runnable_workers() {
    let (erlang, lists, maps) = (
        pinned_beam("erlang"),
        pinned_beam("lists"),
        pinned_beam("maps"),
    );
    let program =
        BeamProgram::load(&[&erlang, &lists, &maps, WORK_STEALING]).expect("the fixture loads");
    let atom = |name| Term::Atom(program.atom(name).expect("the fixture names the atom"));
    let expected = Term::tuple(vec![
        atom("ok"),
        Term::list_from([Term::Integer(1), Term::Integer(2)]),
    ]);
    for _ in 0..RUNS {
        let results = program
            .execute_on_schedulers(&[("work_stealing", "busy"), ("work_stealing", "idle")])
            .expect("the two schedulers run");
        assert_eq!(results[0].term, expected);
        assert_eq!(results[1].term, atom("ok"));
    }
}

/// The two roots give `ok` on the pinned ERTS with two schedulers.
#[test]
fn the_pinned_erts_gives_the_same_results() {
    let directory =
        std::env::temp_dir().join(format!("pbeam-work-stealing-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&directory);
    std::fs::create_dir_all(&directory).expect("the directory of the reference run");
    std::fs::write(directory.join("work_stealing.beam"), WORK_STEALING).expect("the fixture file");
    let output = std::process::Command::new("erl")
        .args(["+S", "2", "-noshell"])
        .args([
            "-eval",
            "io:format(\"~w~n\", [work_stealing:reference()]), halt().",
        ])
        .arg("-pa")
        .arg(&directory)
        .output()
        .expect("the pinned ERTS runs");
    std::fs::remove_dir_all(&directory).expect("the reference directory leaves");
    assert!(output.status.success(), "{output:?}");
    assert_eq!(String::from_utf8_lossy(&output.stdout), "[{ok,ok}]\n");
}

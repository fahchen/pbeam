//! Persistent terms of a node with more than one scheduler thread. Each
//! root of `persistent_terms` returns `ok` when its result is the result
//! of the pinned ERTS.

#[path = "../common/mod.rs"]
mod common;

use common::pinned_beam;
use pbeam_runtime::{BeamProgram, Term};

/// The fixture with the roots. `pair_reference/0` runs each pair of roots
/// on the pinned ERTS, and `one_reference/0` runs the probes of one
/// scheduler.
const PERSISTENT_TERMS: &[u8] =
    include_bytes!("../../fixtures/multischeduler/persistent_terms.beam");

/// The number of runs of each pair, to exercise different interleavings of
/// the two threads.
const RUNS: usize = 10;

/// Load the fixture with the pinned modules that it calls.
fn program() -> BeamProgram {
    let beams = ["erlang", "lists"].map(pinned_beam);
    let mut modules: Vec<&[u8]> = beams.iter().map(Vec::as_slice).collect();
    modules.push(PERSISTENT_TERMS);
    BeamProgram::load(&modules).expect("the fixture loads")
}

/// Run the roots `first` and `second` on two scheduler threads `RUNS` times,
/// and check that each root returns `ok`.
fn run_pair(first: &str, second: &str) {
    let program = program();
    let ok = Term::Atom(program.atom("ok").expect("the fixture names ok"));
    for _ in 0..RUNS {
        let results = program
            .execute_on_schedulers(&[("persistent_terms", first), ("persistent_terms", second)])
            .expect("the two schedulers run");
        assert_eq!(results[0].term, ok, "the root {first}");
        assert_eq!(results[1].term, ok, "the root {second}");
    }
}

/// A root reads the term that the root of the other scheduler puts.
#[test]
fn reads_a_term_of_the_other_scheduler() {
    run_pair("writer", "reader");
}

/// A root sees the erase of a term by the root of the other scheduler.
#[test]
fn sees_an_erase_of_the_other_scheduler() {
    run_pair("eraser", "erase_reader");
}

/// A `get/0` that traps on one scheduler gives a consistent snapshot while
/// the root of the other scheduler puts keys, and both roots end.
#[test]
fn reads_consistent_snapshots_during_puts_of_the_other_scheduler() {
    run_pair("snapshot_writer", "snapshot_reader");
}

/// On one scheduler, another process runs at least two turns while
/// `persistent_term:put/2` waits for thread progress, and a put waits while
/// `persistent_term:get/0` of another process traps, as with `erl +S 1`.
#[test]
fn runs_other_processes_while_a_put_waits() {
    let program = program();
    let ok = Term::Atom(program.atom("ok").expect("the fixture names ok"));
    for root in ["turns", "trapped_get", "trapped_info"] {
        let result = program
            .execute("persistent_terms", root, &[])
            .expect("the run ends");
        assert_eq!(result.term, ok, "the root {root}");
    }
}

/// Run `expression` on the pinned ERTS with `schedulers` scheduler threads
/// and return its output.
fn erts(schedulers: &str, expression: &str) -> String {
    let directory = std::env::temp_dir().join(format!(
        "pbeam-multischeduler-persistent-terms-{}-{schedulers}",
        std::process::id()
    ));
    let _ = std::fs::remove_dir_all(&directory);
    std::fs::create_dir_all(&directory).expect("the directory of the reference run");
    std::fs::write(directory.join("persistent_terms.beam"), PERSISTENT_TERMS)
        .expect("the fixture file");
    let output = std::process::Command::new("erl")
        .args(["+S", schedulers, "-noshell", "-eval", expression])
        .arg("-pa")
        .arg(&directory)
        .output()
        .expect("the pinned ERTS runs");
    std::fs::remove_dir_all(&directory).expect("the reference directory leaves");
    assert!(output.status.success(), "{output:?}");
    String::from_utf8_lossy(&output.stdout).into_owned()
}

/// The pinned ERTS gives `ok` for each root with one and two schedulers.
#[test]
fn the_pinned_erts_gives_the_same_results() {
    let expression = "io:format(\"~w ~w~n\", [persistent_terms:pair_reference(), \
                      persistent_terms:one_reference()]), halt().";
    for schedulers in ["1", "2"] {
        assert_eq!(
            erts(schedulers, expression),
            "[{ok,ok},{ok,ok},{ok,ok}] {ok,ok,ok}\n",
            "erl +S {schedulers}"
        );
    }
}

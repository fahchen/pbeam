//! ETS tables that processes of two scheduler threads share. Each pair of
//! roots of `ets_tables` runs on two threads, and each root returns `ok`
//! when its result is the result of the pinned ERTS.

#[path = "../common/mod.rs"]
mod common;

use common::pinned_beam;
use pbeam_runtime::{BeamProgram, Term};

/// The fixture with the pairs of roots. `reference/0` runs each pair on the
/// pinned ERTS.
const ETS_TABLES: &[u8] = include_bytes!("../../fixtures/multischeduler/ets_tables.beam");

/// The number of runs of each pair, to exercise different interleavings of
/// the two threads.
const RUNS: usize = 10;

/// Run the roots `first` and `second` on two scheduler threads `RUNS` times,
/// and check that each root returns `ok`.
fn run_pair(first: &str, second: &str) {
    let beams = ["erlang", "erts_internal", "ets"].map(pinned_beam);
    let mut modules: Vec<&[u8]> = beams.iter().map(Vec::as_slice).collect();
    modules.push(ETS_TABLES);
    let program = BeamProgram::load(&modules).expect("the fixture loads");
    let ok = Term::Atom(program.atom("ok").expect("the fixture names ok"));
    for _ in 0..RUNS {
        let results = program
            .execute_on_schedulers(&[("ets_tables", first), ("ets_tables", second)])
            .expect("the two schedulers run");
        assert_eq!(results[0].term, ok, "the root {first}");
        assert_eq!(results[1].term, ok, "the root {second}");
    }
}

/// A process of the other scheduler reads and writes the `public`,
/// `protected`, and `private` tables of the owner as ERTS allows it, and
/// the exit of the owner deletes them. A table of a run with scheduler
/// threads reports its `write_concurrency` options.
#[test]
fn shares_the_tables_of_an_owner_on_the_other_scheduler() {
    run_pair("owner", "user");
}

/// The delete of a table that a process of the other scheduler fixed
/// leaves a fixation cleanup for that process.
#[test]
fn deletes_a_table_that_a_process_of_the_other_scheduler_fixed() {
    run_pair("fixer", "deleter");
}

/// Each pair gives `ok` for each root on the pinned ERTS.
#[test]
fn the_pinned_erts_gives_the_same_results() {
    let directory =
        std::env::temp_dir().join(format!("pbeam-multischeduler-ets-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&directory);
    std::fs::create_dir_all(&directory).expect("the directory of the reference run");
    std::fs::write(directory.join("ets_tables.beam"), ETS_TABLES).expect("the fixture file");
    let output = std::process::Command::new("erl")
        .args(["+S", "2", "-noshell"])
        .args([
            "-eval",
            "io:format(\"~w~n\", [ets_tables:reference()]), halt().",
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

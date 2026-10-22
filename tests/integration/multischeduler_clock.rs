//! The clocks of two scheduler threads. The roots of `clock_signals` run on
//! two threads, exchange `erlang:monotonic_time/0,1`,
//! `erlang:time_offset/0,1`, and the time support settings, and each root
//! returns `ok` when the values agree as on `erl +S 2`.

#[path = "../common/mod.rs"]
mod common;

use common::pinned_beam;
use pbeam_runtime::{BeamProgram, Term};

/// The fixture with the pair of roots. `reference/0` runs the pair on the
/// pinned ERTS.
const CLOCK_SIGNALS: &[u8] = include_bytes!("../../fixtures/multischeduler/clock_signals.beam");

/// The time of a reading on one thread is at or after the time of the
/// reading of the other thread that it received, and the time support
/// settings are the same, before and after the first time correction check
/// of the node.
#[test]
fn reads_one_clock_of_the_node_on_two_schedulers() {
    let beams = ["erlang", "erts_internal"].map(pinned_beam);
    let mut modules: Vec<&[u8]> = beams.iter().map(Vec::as_slice).collect();
    modules.push(CLOCK_SIGNALS);
    let program = BeamProgram::load(&modules).expect("the fixture loads");
    let ok = Term::Atom(program.atom("ok").expect("the fixture names ok"));
    let results = program
        .execute_on_schedulers(&[("clock_signals", "writer"), ("clock_signals", "reader")])
        .expect("the two schedulers run");
    assert_eq!(results[0].term, ok, "the writer");
    assert_eq!(results[1].term, ok, "the reader");
}

/// The pair gives `ok` for each root on the pinned ERTS.
#[test]
fn the_pinned_erts_gives_the_same_results() {
    let directory =
        std::env::temp_dir().join(format!("pbeam-multischeduler-clock-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&directory);
    std::fs::create_dir_all(&directory).expect("the directory of the reference run");
    std::fs::write(directory.join("clock_signals.beam"), CLOCK_SIGNALS).expect("the fixture file");
    let output = std::process::Command::new("erl")
        .args(["+S", "2", "-noshell"])
        .args([
            "-eval",
            "io:format(\"~w~n\", [clock_signals:reference()]), halt().",
        ])
        .arg("-pa")
        .arg(&directory)
        .output()
        .expect("the pinned ERTS runs");
    std::fs::remove_dir_all(&directory).expect("the reference directory leaves");
    assert!(output.status.success(), "{output:?}");
    assert_eq!(String::from_utf8_lossy(&output.stdout), "{ok,ok}\n");
}

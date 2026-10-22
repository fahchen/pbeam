//! Link, unlink, and exit signals between processes of two scheduler
//! threads. Each pair of roots of `link_signals` runs on two threads, and
//! each root returns `ok` when its result is the result of the pinned ERTS.

#[path = "../common/mod.rs"]
mod common;

use common::pinned_beam;
use pbeam_runtime::{BeamProgram, Term};

/// The fixture with the pairs of roots. `reference/0` runs each pair on the
/// pinned ERTS.
const LINK_SIGNALS: &[u8] = include_bytes!("../../fixtures/multischeduler/link_signals.beam");

/// The number of runs of each pair, to exercise different interleavings of
/// the two threads.
const RUNS: usize = 10;

/// Run the roots `first` and `second` on two scheduler threads `RUNS` times,
/// and check that each root returns `ok`.
fn run_pair(first: &str, second: &str) {
    let (erlang, lists) = (pinned_beam("erlang"), pinned_beam("lists"));
    let program = BeamProgram::load(&[&erlang, &lists, LINK_SIGNALS]).expect("the fixture loads");
    let ok = Term::Atom(program.atom("ok").expect("the fixture names ok"));
    for _ in 0..RUNS {
        let results = program
            .execute_on_schedulers(&[("link_signals", first), ("link_signals", second)])
            .expect("the two schedulers run");
        assert_eq!(results[0].term, ok, "the root {first}");
        assert_eq!(results[1].term, ok, "the root {second}");
    }
}

/// The messages and exit signals of one sender arrive in send order, and a
/// link or unlink signal takes effect before the next message of its sender.
#[test]
fn keeps_the_order_of_messages_and_link_signals_of_one_sender() {
    run_pair("order_send", "order_recv");
}

/// A link to a process that exited raises `noproc`, or with `trap_exit`
/// gives an `EXIT` message with `noproc`. An unlink and an exit signal to it
/// return `true` and change nothing.
#[test]
fn answers_signals_to_a_remote_process_that_exited() {
    run_pair("dead_a", "dead_b");
}

/// Sends, links, unlinks, and exit signals that race with the exit of a
/// remote process give a result that the pinned ERTS can give.
#[test]
fn a_signal_that_races_with_a_remote_exit_gets_the_erts_answer() {
    run_pair("race_a", "race_b");
}

/// A remote process that does not trap exits ignores the exit signal
/// `normal` and ends at `kill`. A link before a message that ends the
/// remote process gives its exit reason.
#[test]
fn applies_exit_signals_to_a_remote_process() {
    run_pair("exit_a", "exit_b");
}

/// Each pair gives `ok` for each root on the pinned ERTS.
#[test]
fn the_pinned_erts_gives_the_same_results() {
    let directory =
        std::env::temp_dir().join(format!("pbeam-multischeduler-links-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&directory);
    std::fs::create_dir_all(&directory).expect("the directory of the reference run");
    std::fs::write(directory.join("link_signals.beam"), LINK_SIGNALS).expect("the fixture file");
    let output = std::process::Command::new("erl")
        .args(["+S", "2", "-noshell"])
        .args([
            "-eval",
            "io:format(\"~w~n\", [link_signals:reference()]), halt().",
        ])
        .arg("-pa")
        .arg(&directory)
        .output()
        .expect("the pinned ERTS runs");
    std::fs::remove_dir_all(&directory).expect("the reference directory leaves");
    assert!(output.status.success(), "{output:?}");
    assert_eq!(
        String::from_utf8_lossy(&output.stdout),
        "[{ok,ok},{ok,ok},{ok,ok},{ok,ok}]\n"
    );
}

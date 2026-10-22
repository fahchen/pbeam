//! Monitor, demonitor, and `DOWN` signals between processes of two scheduler
//! threads, and `demonitor/2` with `flush`. Each pair of roots of
//! `monitor_signals` runs on two threads, and each root returns `ok` when its
//! result is the result of the pinned ERTS.

#[path = "../common/mod.rs"]
mod common;

use common::pinned_beam;
use pbeam_runtime::{BeamProgram, Term};

/// The fixture with the pairs of roots. `reference/0` runs each pair on the
/// pinned ERTS.
const MONITOR_SIGNALS: &[u8] = include_bytes!("../../fixtures/multischeduler/monitor_signals.beam");

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
    let program = BeamProgram::load(&[&erlang, &erts_internal, &lists, MONITOR_SIGNALS])
        .expect("the fixture loads");
    let ok = Term::Atom(program.atom("ok").expect("the fixture names ok"));
    for _ in 0..RUNS {
        let results = program
            .execute_on_schedulers(&[("monitor_signals", first), ("monitor_signals", second)])
            .expect("the two schedulers run");
        assert_eq!(results[0].term, ok, "the root {first}");
        assert_eq!(results[1].term, ok, "the root {second}");
    }
}

/// The messages of a remote target arrive in send order before its `DOWN`
/// messages. Each monitor gives one `DOWN`, a `tag` option gives its tag,
/// and a monitor that ended before the exit gives none.
#[test]
fn keeps_the_order_of_messages_and_down_of_one_target() {
    run_pair("order_watch", "order_owner");
}

/// A monitor of a remote process that exited gives one `DOWN` with
/// `noproc`, and a monitor by the name of a remote process gives the name in
/// its `DOWN`.
#[test]
fn answers_a_monitor_of_a_remote_process_that_exited() {
    run_pair("dead_watch", "dead_owner");
}

/// `demonitor/2` with `flush` removes the `DOWN` message of its monitor
/// only, and a demonitor that races with the remote exit leaves no `DOWN`.
#[test]
fn flushes_only_the_down_of_the_monitor() {
    run_pair("flush_watch", "flush_owner");
}

/// A monitor that races with the exit of a remote process gives one `DOWN`
/// with the exit reason or `noproc`.
#[test]
fn a_monitor_that_races_with_a_remote_exit_gets_one_down() {
    run_pair("race_watch", "race_owner");
}

/// A monitor signal that the watcher sends before its message to the owner
/// of the target is in the queue of the target before the owner can kill
/// the target when it gets the message, so each `DOWN` has `killed`.
#[test]
fn a_monitor_before_a_message_reaches_the_target_before_a_later_kill() {
    run_pair("kill_watch", "kill_owner");
}

/// A monitor signal waits in the pending slot while the watcher sends to
/// other remote processes. A delivery to its target, a second monitor
/// signal, or the switch out sends it, and each `DOWN` has the exit reason.
#[test]
fn a_pending_monitor_goes_at_the_erts_points() {
    run_pair("slot_watch", "slot_owner");
}

/// Each pair gives `ok` for each root on the pinned ERTS.
#[test]
fn the_pinned_erts_gives_the_same_results() {
    let directory = std::env::temp_dir().join(format!(
        "pbeam-multischeduler-monitors-{}",
        std::process::id()
    ));
    let _ = std::fs::remove_dir_all(&directory);
    std::fs::create_dir_all(&directory).expect("the directory of the reference run");
    std::fs::write(directory.join("monitor_signals.beam"), MONITOR_SIGNALS)
        .expect("the fixture file");
    let output = std::process::Command::new("erl")
        .args(["+S", "2", "-noshell"])
        .args([
            "-eval",
            "io:format(\"~w~n\", [monitor_signals:reference()]), halt().",
        ])
        .arg("-pa")
        .arg(&directory)
        .output()
        .expect("the pinned ERTS runs");
    std::fs::remove_dir_all(&directory).expect("the reference directory leaves");
    assert!(output.status.success(), "{output:?}");
    assert_eq!(
        String::from_utf8_lossy(&output.stdout),
        "[{ok,ok},{ok,ok},{ok,ok},{ok,ok},{ok,ok},{ok,ok}]\n"
    );
}

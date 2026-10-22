//! Alias messages between processes of two scheduler threads. Each pair of
//! roots of `alias_signals` runs on two threads, and each root returns `ok`
//! when its result is the result of the pinned ERTS.

#[path = "../common/mod.rs"]
mod common;

use common::pinned_beam;
use pbeam_runtime::{BeamProgram, Term};

/// The fixture with the pairs of roots. `reference/0` runs each pair on the
/// pinned ERTS.
const ALIAS_SIGNALS: &[u8] = include_bytes!("../../fixtures/multischeduler/alias_signals.beam");

/// The number of runs of each pair, to exercise different interleavings of
/// the two threads.
const RUNS: usize = 10;

/// Run the roots `first` and `second` on two scheduler threads `RUNS` times,
/// and check that each root returns `ok`.
fn run_pair(first: &str, second: &str) {
    let beams = ["erlang", "erts_internal", "lists", "gen"].map(pinned_beam);
    let mut modules: Vec<&[u8]> = beams.iter().map(Vec::as_slice).collect();
    modules.push(ALIAS_SIGNALS);
    let program = BeamProgram::load(&modules).expect("the fixture loads");
    let ok = Term::Atom(program.atom("ok").expect("the fixture names ok"));
    for _ in 0..RUNS {
        let results = program
            .execute_on_schedulers(&[("alias_signals", first), ("alias_signals", second)])
            .expect("the two schedulers run");
        assert_eq!(results[0].term, ok, "the root {first}");
        assert_eq!(results[1].term, ok, "the root {second}");
    }
}

/// `gen:call/3` to a process of the other scheduler gets each reply through
/// the alias of its monitor.
#[test]
fn routes_the_reply_of_a_gen_call() {
    run_pair("call_client", "call_server");
}

/// A `reply` alias takes one message, an `explicit_unalias` alias takes
/// each message until `unalias/1`, and a send after it, or after the exit
/// of the owner, drops the message.
#[test]
fn routes_and_drops_messages_to_an_alias_of_the_other_scheduler() {
    run_pair("alias_client", "alias_owner");
}

/// A monitor with an `explicit_unalias` alias of a remote process gets the
/// reply, the alias stays after the `DOWN`, and `unalias/1` ends it.
#[test]
fn keeps_a_monitor_alias_after_the_down() {
    run_pair("monitor_client", "monitor_server");
}

/// Each pair gives `ok` for each root on the pinned ERTS.
#[test]
fn the_pinned_erts_gives_the_same_results() {
    let directory = std::env::temp_dir().join(format!(
        "pbeam-multischeduler-aliases-{}",
        std::process::id()
    ));
    let _ = std::fs::remove_dir_all(&directory);
    std::fs::create_dir_all(&directory).expect("the directory of the reference run");
    std::fs::write(directory.join("alias_signals.beam"), ALIAS_SIGNALS).expect("the fixture file");
    let output = std::process::Command::new("erl")
        .args(["+S", "2", "-noshell"])
        .args([
            "-eval",
            "io:format(\"~w~n\", [alias_signals:reference()]), halt().",
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

//! Two scheduler threads share one node. A root on each thread exchanges
//! messages: the ping root sends a burst to the pong root, and the pong root
//! reports whether the burst arrived in the order of the sender.

use pbeam_runtime::{BeamProgram, Term};

/// The fixture with the two roots. `ping/0` sends `COUNT` messages to the
/// process registered as `pong`, and `pong/0` records the next expected
/// sequence number. `reference/0` runs the same exchange on the pinned ERTS.
const PING_PONG: &[u8] = include_bytes!("../../fixtures/multischeduler/ping_pong.beam");

/// Run the two roots on two scheduler threads and check the result of each
/// root.
///
/// The channel of one sender and one receiver keeps its order, so the burst
/// arrives in order. The test repeats the run to exercise different
/// interleavings of the two threads.
#[test]
fn runs_a_cross_thread_ping_pong_on_two_schedulers() {
    let program = BeamProgram::load(&[PING_PONG]).expect("the fixture loads");
    let true_atom = program.atom("true").expect("the fixture names true");
    let ok_atom = program.atom("ok").expect("the fixture names ok");
    let expected_ping = Term::tuple(vec![Term::Integer(100), Term::Atom(true_atom)]);
    for _ in 0..10 {
        let results = program
            .execute_on_schedulers(&[("ping_pong", "ping"), ("ping_pong", "pong")])
            .expect("the two schedulers run");
        assert_eq!(results.len(), 2, "one result for each root");
        assert_eq!(results[0].term, expected_ping, "the ping root");
        assert_eq!(
            results[1].term,
            Term::Atom(ok_atom.clone()),
            "the pong root"
        );
    }
}

/// The same exchange on the pinned ERTS returns the same results.
#[test]
fn the_pinned_erts_gives_the_same_result() {
    let directory = std::env::temp_dir().join(format!(
        "pbeam-multischeduler-{}-reference",
        std::process::id()
    ));
    let _ = std::fs::remove_dir_all(&directory);
    std::fs::create_dir_all(&directory).expect("the directory of the reference run");
    let beam = directory.join("ping_pong.beam");
    std::fs::write(&beam, PING_PONG).expect("the fixture file");
    let output = std::process::Command::new("erl")
        .args(["+S", "2", "-noshell"])
        .args([
            "-eval",
            "io:format(\"~p~n\", [ping_pong:reference()]), halt().",
        ])
        .arg("-pa")
        .arg(&directory)
        .output()
        .expect("the pinned ERTS runs");
    std::fs::remove_dir_all(&directory).expect("the reference directory leaves");
    assert!(output.status.success(), "{output:?}");
    assert_eq!(String::from_utf8_lossy(&output.stdout), "{{100,true},ok}\n");
}

/// A root that exits with an error gives that error.
#[test]
fn reports_a_root_that_exits_with_an_error() {
    let program = BeamProgram::load(&[PING_PONG]).expect("the fixture loads");
    let error = program
        .execute_on_schedulers(&[("ping_pong", "boom")])
        .expect_err("the root exits with an error");
    assert!(
        error.to_string().contains("root process exited with {boom"),
        "{error}"
    );
}

/// A process of another scheduler answers its registered name and its
/// liveness from the tables of the node.
#[test]
fn reads_the_name_and_liveness_of_a_remote_process() {
    let program = BeamProgram::load(&[PING_PONG]).expect("the fixture loads");
    let ok_atom = program.atom("ok").expect("the fixture names ok");
    let results = program
        .execute_on_schedulers(&[("ping_pong", "info_watch"), ("ping_pong", "info_hold")])
        .expect("the two schedulers run");
    assert_eq!(results[0].term, Term::Atom(ok_atom.clone()), "the watcher");
    assert_eq!(results[1].term, Term::Atom(ok_atom), "the holder");
}

/// An item of a remote process that the runtime cannot read stops the run
/// with a clear error.
#[test]
fn rejects_an_unsupported_item_of_a_remote_process() {
    let program = BeamProgram::load(&[PING_PONG]).expect("the fixture loads");
    let error = program
        .execute_on_schedulers(&[("ping_pong", "info_item"), ("ping_pong", "info_hold")])
        .expect_err("the item is unsupported");
    assert!(
        error
            .to_string()
            .contains("unsupported erlang:process_info/2"),
        "{error}"
    );
}

/// The DOWN of the last other process reaches a root that waits without a
/// timer before the runtime reports an impossible wait.
#[test]
fn a_down_resumes_a_root_that_waits_for_it() {
    let program = BeamProgram::load(&[PING_PONG]).expect("the fixture loads");
    let ok_atom = program.atom("ok").expect("the fixture names ok");
    let results = program
        .execute_on_schedulers(&[("ping_pong", "down_wait"), ("ping_pong", "down_hold")])
        .expect("the two schedulers run");
    assert_eq!(results[0].term, Term::Atom(ok_atom.clone()), "the watcher");
    assert_eq!(results[1].term, Term::Atom(ok_atom), "the holder");
}

/// The whole state of a remote process is not readable yet, and the run
/// stops with a clear error.
#[test]
fn rejects_the_whole_state_of_a_remote_process() {
    let program = BeamProgram::load(&[PING_PONG]).expect("the fixture loads");
    let error = program
        .execute_on_schedulers(&[("ping_pong", "info_all"), ("ping_pong", "info_hold")])
        .expect_err("the whole state is unsupported");
    assert!(
        error
            .to_string()
            .contains("unsupported erlang:process_info/1"),
        "{error}"
    );
}

/// An atom that guest code creates stops a run with scheduler threads with
/// a clear error, because the copies of the atom table can differ.
#[test]
fn rejects_an_atom_that_guest_code_creates() {
    let program = BeamProgram::load(&[PING_PONG]).expect("the fixture loads");
    let error = program
        .execute_on_schedulers(&[("ping_pong", "atom_create"), ("ping_pong", "info_hold")])
        .expect_err("the atom creation is unsupported");
    assert!(
        error
            .to_string()
            .contains("cannot create the atom new_scheduler_atom_"),
        "{error}"
    );
}

/// A failing root decides the error of the run even when another root waits
/// and the abort stops it.
#[test]
fn a_failed_root_reports_its_error_before_a_stopped_wait() {
    let program = BeamProgram::load(&[PING_PONG]).expect("the fixture loads");
    let error = program
        .execute_on_schedulers(&[("ping_pong", "wait"), ("ping_pong", "boom")])
        .expect_err("the boom root fails");
    assert!(
        error.to_string().contains("root process exited with {boom"),
        "{error}"
    );
}

/// A root that waits for a message that no process can send gives the error
/// of an impossible wait.
#[test]
fn reports_a_root_that_waits_for_a_message_that_cannot_arrive() {
    let program = BeamProgram::load(&[PING_PONG]).expect("the fixture loads");
    let error = program
        .execute_on_schedulers(&[("ping_pong", "wait")])
        .expect_err("the root waits");
    assert!(
        error
            .to_string()
            .contains("root process waits for a message that cannot arrive"),
        "{error}"
    );
}

/// A run without a root gives an error.
#[test]
fn rejects_an_empty_root_list() {
    let program = BeamProgram::load(&[PING_PONG]).expect("the fixture loads");
    let error = program
        .execute_on_schedulers(&[])
        .expect_err("an empty root list is an error");
    assert!(error.to_string().contains("at least one root"), "{error}");
}

/// Two roots of different scheduler threads register one name. One root gets
/// the name and the other root gets false.
#[test]
fn one_of_two_schedulers_registers_a_shared_name() {
    let program = BeamProgram::load(&[PING_PONG]).expect("the fixture loads");
    let true_atom = program.atom("true").expect("the fixture names true");
    let false_atom = program.atom("false").expect("the fixture names false");
    let results = program
        .execute_on_schedulers(&[
            ("ping_pong", "conflict_try"),
            ("ping_pong", "conflict_hold"),
        ])
        .expect("the two schedulers run");
    assert_eq!(
        results[0].term,
        Term::Atom(false_atom),
        "the second root has the name"
    );
    assert_eq!(
        results[1].term,
        Term::Atom(true_atom),
        "the first root has the name"
    );
}

/// A root that fails aborts the run, and the run reports its error instead
/// of the result of the root that the abort stopped.
#[test]
fn a_missing_entry_stops_the_other_scheduler() {
    let program = BeamProgram::load(&[PING_PONG]).expect("the fixture loads");
    let error = program
        .execute_on_schedulers(&[("ping_pong", "missing"), ("ping_pong", "pong")])
        .expect_err("the missing entry is an error");
    assert!(error.to_string().contains("missing"), "{error}");
}

/// A monitor of a process of another scheduler reaches it when the watcher
/// waits, and the DOWN message keeps the real exit reason.
#[test]
fn a_pending_monitor_reaches_a_remote_target() {
    let program = BeamProgram::load(&[PING_PONG]).expect("the fixture loads");
    let true_atom = program.atom("true").expect("the fixture names true");
    let ok_atom = program.atom("ok").expect("the fixture names ok");
    let results = program
        .execute_on_schedulers(&[
            ("ping_pong", "monitor_watch"),
            ("ping_pong", "monitor_hold"),
        ])
        .expect("the two schedulers run");
    assert_eq!(results[0].term, Term::Atom(true_atom), "the DOWN reason");
    assert_eq!(results[1].term, Term::Atom(ok_atom), "the holder");
}

/// An unregister from another scheduler clears the name of the owner, and
/// the owner can register the name again.
#[test]
fn unregistering_a_remote_name_clears_the_owner() {
    let program = BeamProgram::load(&[PING_PONG]).expect("the fixture loads");
    let true_atom = program.atom("true").expect("the fixture names true");
    let results = program
        .execute_on_schedulers(&[("ping_pong", "unreg_hold"), ("ping_pong", "unreg_taker")])
        .expect("the two schedulers run");
    assert_eq!(results[0].term, Term::Atom(true_atom.clone()), "the holder");
    assert_eq!(results[1].term, Term::Atom(true_atom), "the taker");
}

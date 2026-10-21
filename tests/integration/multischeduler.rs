//! Two scheduler threads share one node. A root on each thread exchanges
//! messages: the ping root sends a burst to the pong root, and the pong root
//! reports whether the burst arrived in the order of the sender.

#[path = "../common/mod.rs"]
mod common;

use common::pinned_beam;
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
    // The liveness check routes through `erts_internal:is_process_alive/1`.
    let erts_internal = pinned_beam("erts_internal");
    let program = BeamProgram::load(&[&erts_internal, PING_PONG]).expect("the fixture loads");
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

/// A check of a process that the caller killed on another scheduler
/// answers false, after the exit signal that the caller sent.
#[test]
fn a_killed_remote_process_is_not_alive() {
    let erts_internal = pinned_beam("erts_internal");
    let program = BeamProgram::load(&[&erts_internal, PING_PONG]).expect("the fixture loads");
    let ok_atom = program.atom("ok").expect("the fixture names ok");
    let results = program
        .execute_on_schedulers(&[("ping_pong", "kill_watch"), ("ping_pong", "kill_hold")])
        .expect("the two schedulers run");
    assert_eq!(results[0].term, Term::Atom(ok_atom.clone()), "the watcher");
    assert_eq!(results[1].term, Term::Atom(ok_atom), "the holder");
}

/// A liveness check of a local child that another scheduler killed answers
/// false, also when the kill waited in the inbox at the call.
#[test]
fn a_killed_local_child_is_not_alive() {
    let erts_internal = pinned_beam("erts_internal");
    let program = BeamProgram::load(&[&erts_internal, PING_PONG]).expect("the fixture loads");
    let false_atom = program.atom("false").expect("the fixture names false");
    let ok_atom = program.atom("ok").expect("the fixture names ok");
    let results = program
        .execute_on_schedulers(&[("ping_pong", "alive_wait"), ("ping_pong", "alive_send")])
        .expect("the two schedulers run");
    assert_eq!(results[0].term, Term::Atom(false_atom), "the watcher");
    assert_eq!(results[1].term, Term::Atom(ok_atom), "the killer");
}

/// A port of another scheduler works as a term: its text and its encoded
/// form need no port service.
#[test]
fn uses_a_port_of_another_scheduler_as_a_term() {
    let erlang = pinned_beam("erlang");
    let program = BeamProgram::load(&[&erlang, PING_PONG]).expect("the fixture loads");
    let ok_atom = program.atom("ok").expect("the fixture names ok");
    let results = program
        .execute_on_schedulers(&[("ping_pong", "port_term"), ("ping_pong", "port_term_give")])
        .expect("the two schedulers run");
    assert_eq!(results[0].term, Term::Atom(ok_atom.clone()), "the user");
    assert_eq!(results[1].term, Term::Atom(ok_atom), "the giver");
}

/// A use of a port of another scheduler fails with a clear error.
#[test]
fn rejects_the_use_of_a_port_of_another_scheduler() {
    let erlang = pinned_beam("erlang");
    let program = BeamProgram::load(&[&erlang, PING_PONG]).expect("the fixture loads");
    let error = program
        .execute_on_schedulers(&[("ping_pong", "port_use"), ("ping_pong", "port_give")])
        .expect_err("the port use is unsupported");
    assert!(
        error
            .to_string()
            .contains("unsupported: a port of another scheduler"),
        "{error}"
    );
}

/// A transfer of a port to a process of another scheduler fails with a
/// clear error.
#[test]
fn rejects_a_port_transfer_across_schedulers() {
    let erlang = pinned_beam("erlang");
    let program = BeamProgram::load(&[&erlang, PING_PONG]).expect("the fixture loads");
    let error = program
        .execute_on_schedulers(&[
            ("ping_pong", "port_transfer"),
            ("ping_pong", "port_transfer_hold"),
        ])
        .expect_err("the port transfer is unsupported");
    assert!(
        error
            .to_string()
            .contains("unsupported: erlang:port_connect/2 to a process of another scheduler"),
        "{error}"
    );
}

/// The port list of a run with scheduler threads is not readable yet, and
/// the run stops with a clear error.
#[test]
fn rejects_the_port_list_of_a_scheduler_run() {
    let program = BeamProgram::load(&[PING_PONG]).expect("the fixture loads");
    let error = program
        .execute_on_schedulers(&[("ping_pong", "ports_list"), ("ping_pong", "info_hold")])
        .expect_err("the port list is unsupported");
    assert!(
        error.to_string().contains("unsupported: erlang:ports/0"),
        "{error}"
    );
}

/// A BIF timer for a process of another scheduler fails with a clear
/// error.
#[test]
fn rejects_a_timer_for_another_scheduler() {
    let program = BeamProgram::load(&[PING_PONG]).expect("the fixture loads");
    let error = program
        .execute_on_schedulers(&[("ping_pong", "timer_remote"), ("ping_pong", "timer_hold")])
        .expect_err("the timer is unsupported");
    assert!(
        error
            .to_string()
            .contains("unsupported: a BIF timer to a process of another scheduler"),
        "{error}"
    );
}

/// A read of an idle local child takes the direct path.
#[test]
fn reads_the_queue_length_of_an_idle_child() {
    let program = BeamProgram::load(&[PING_PONG]).expect("the fixture loads");
    let ok_atom = program.atom("ok").expect("the fixture names ok");
    let results = program
        .execute_on_schedulers(&[("ping_pong", "info_idle"), ("ping_pong", "monitor_hold")])
        .expect("the two schedulers run");
    assert_eq!(results[0].term, Term::Atom(ok_atom.clone()), "the reader");
    assert_eq!(results[1].term, Term::Atom(ok_atom), "the holder");
}

/// A read of another local process counts a message that another scheduler
#[test]
fn rejects_a_compiled_binary_pattern_of_another_scheduler() {
    let program = BeamProgram::load(&[PING_PONG]).expect("the fixture loads");
    let error = program
        .execute_on_schedulers(&[
            ("ping_pong", "pattern_match"),
            ("ping_pong", "pattern_give"),
        ])
        .expect_err("the pattern is unsupported");
    assert!(
        error
            .to_string()
            .contains("unsupported: a compiled binary pattern of another scheduler"),
        "{error}"
    );
}

/// A group leader request routes to a process of another scheduler, and
/// its reply comes back to the caller.
#[test]
fn routes_a_group_leader_request() {
    let erlang = pinned_beam("erlang");
    let erts_internal = pinned_beam("erts_internal");
    let program =
        BeamProgram::load(&[&erlang, &erts_internal, PING_PONG]).expect("the fixture loads");
    let ok_atom = program.atom("ok").expect("the fixture names ok");
    let results = program
        .execute_on_schedulers(&[
            ("ping_pong", "group_leader_remote"),
            ("ping_pong", "leader_hold"),
        ])
        .expect("the two schedulers run");
    assert_eq!(results[0].term, Term::Atom(ok_atom.clone()), "the caller");
    assert_eq!(results[1].term, Term::Atom(ok_atom), "the leader");
}

/// A code delete in a run with scheduler threads fails with a clear error.
#[test]
fn rejects_a_code_delete() {
    let program = BeamProgram::load(&[PING_PONG]).expect("the fixture loads");
    let error = program
        .execute_on_schedulers(&[
            ("ping_pong", "delete_module"),
            ("ping_pong", "monitor_hold"),
        ])
        .expect_err("the code delete is unsupported");
    assert!(
        error
            .to_string()
            .contains("unsupported: erlang:delete_module/1 in a run of execute_on_schedulers"),
        "{error}"
    );
}

/// A compiled expression of another scheduler fails with a clear error.
#[test]
fn rejects_a_compiled_expression_of_another_scheduler() {
    let re = pinned_beam("re");
    let prim_tty = pinned_beam("prim_tty");
    let program = BeamProgram::load(&[PING_PONG, &prim_tty, &re]).expect("the fixture loads");
    let error = program
        .execute_on_schedulers(&[("ping_pong", "regex_run"), ("ping_pong", "regex_give")])
        .expect_err("the expression is unsupported");
    assert!(
        error
            .to_string()
            .contains("unsupported: a compiled regular expression of another scheduler"),
        "{error}"
    );
}

/// A change of a node flag that stays with a scheduler fails with a clear
/// error.
#[test]
fn rejects_a_scheduler_local_system_flag() {
    let program = BeamProgram::load(&[PING_PONG]).expect("the fixture loads");
    let error = program
        .execute_on_schedulers(&[
            ("ping_pong", "system_flag_depth"),
            ("ping_pong", "monitor_hold"),
        ])
        .expect_err("the flag is unsupported");
    assert!(
        error
            .to_string()
            .contains("unsupported: erlang:system_flag/2 backtrace_depth"),
        "{error}"
    );
}

/// A code load in a run with scheduler threads fails with a clear error.
#[test]
fn rejects_a_code_load() {
    let program = BeamProgram::load(&[PING_PONG]).expect("the fixture loads");
    let error = program
        .execute_on_schedulers(&[
            ("ping_pong", "finish_loading"),
            ("ping_pong", "monitor_hold"),
        ])
        .expect_err("the code load is unsupported");
    assert!(
        error
            .to_string()
            .contains("unsupported: erlang:finish_loading/1 in a run of execute_on_schedulers"),
        "{error}"
    );
}

/// A BIF timer of a shared run fires, and its entry leaves the node.
#[test]
fn fires_a_local_bif_timer() {
    let program = BeamProgram::load(&[PING_PONG]).expect("the fixture loads");
    let ok_atom = program.atom("ok").expect("the fixture names ok");
    let results = program
        .execute_on_schedulers(&[("ping_pong", "timer_fire"), ("ping_pong", "monitor_hold")])
        .expect("the two schedulers run");
    assert_eq!(results[0].term, Term::Atom(ok_atom.clone()), "the timer");
    assert_eq!(results[1].term, Term::Atom(ok_atom), "the holder");
}

/// A split with a compiled pattern of another scheduler fails with a clear
/// error.
#[test]
fn rejects_a_split_with_a_pattern_of_another_scheduler() {
    let program = BeamProgram::load(&[PING_PONG]).expect("the fixture loads");
    let error = program
        .execute_on_schedulers(&[
            ("ping_pong", "pattern_split"),
            ("ping_pong", "pattern_split_give"),
        ])
        .expect_err("the pattern is unsupported");
    assert!(
        error
            .to_string()
            .contains("unsupported: a compiled binary pattern of another scheduler"),
        "{error}"
    );
}

/// A matches call with a compiled pattern of another scheduler fails with a
/// clear error.
#[test]
fn rejects_a_matches_call_with_a_pattern_of_another_scheduler() {
    let program = BeamProgram::load(&[PING_PONG]).expect("the fixture loads");
    let error = program
        .execute_on_schedulers(&[
            ("ping_pong", "pattern_matches"),
            ("ping_pong", "pattern_matches_give"),
        ])
        .expect_err("the pattern is unsupported");
    assert!(
        error
            .to_string()
            .contains("unsupported: a compiled binary pattern of another scheduler"),
        "{error}"
    );
}

/// A four-argument BIF timer of a shared run fires.
#[test]
fn fires_a_local_four_argument_bif_timer() {
    let program = BeamProgram::load(&[PING_PONG]).expect("the fixture loads");
    let ok_atom = program.atom("ok").expect("the fixture names ok");
    let results = program
        .execute_on_schedulers(&[("ping_pong", "timer_fire4"), ("ping_pong", "monitor_hold")])
        .expect("the two schedulers run");
    assert_eq!(results[0].term, Term::Atom(ok_atom.clone()), "the timer");
    assert_eq!(results[1].term, Term::Atom(ok_atom), "the holder");
}

/// A named four-argument BIF timer fails with a clear error.
#[test]
fn rejects_a_named_four_argument_timer() {
    let program = BeamProgram::load(&[PING_PONG]).expect("the fixture loads");
    let error = program
        .execute_on_schedulers(&[("ping_pong", "timer_named4"), ("ping_pong", "monitor_hold")])
        .expect_err("the named timer is unsupported");
    assert!(
        error
            .to_string()
            .contains("unsupported: a named BIF timer in a run of execute_on_schedulers"),
        "{error}"
    );
}

/// A four-argument BIF timer for another scheduler fails with a clear
/// error.
#[test]
fn rejects_a_four_argument_timer_for_another_scheduler() {
    let program = BeamProgram::load(&[PING_PONG]).expect("the fixture loads");
    let error = program
        .execute_on_schedulers(&[("ping_pong", "timer_remote4"), ("ping_pong", "timer_hold")])
        .expect_err("the timer is unsupported");
    assert!(
        error
            .to_string()
            .contains("unsupported: a BIF timer to a process of another scheduler"),
        "{error}"
    );
}

/// A cancel of a local BIF timer leaves its node entry.
#[test]
fn cancels_a_local_bif_timer() {
    let program = BeamProgram::load(&[PING_PONG]).expect("the fixture loads");
    let ok_atom = program.atom("ok").expect("the fixture names ok");
    let results = program
        .execute_on_schedulers(&[("ping_pong", "timer_cancel"), ("ping_pong", "monitor_hold")])
        .expect("the two schedulers run");
    assert_eq!(results[0].term, Term::Atom(ok_atom.clone()), "the cancel");
    assert_eq!(results[1].term, Term::Atom(ok_atom), "the holder");
}

/// The exit of a process cancels its BIF timer and leaves its node entry.
#[test]
fn cancels_a_bif_timer_at_an_exit() {
    let program = BeamProgram::load(&[PING_PONG]).expect("the fixture loads");
    let ok_atom = program.atom("ok").expect("the fixture names ok");
    let results = program
        .execute_on_schedulers(&[
            ("ping_pong", "timer_child_exit"),
            ("ping_pong", "monitor_hold"),
        ])
        .expect("the two schedulers run");
    assert_eq!(
        results[0].term,
        Term::Atom(ok_atom.clone()),
        "the child timer"
    );
    assert_eq!(results[1].term, Term::Atom(ok_atom), "the holder");
}

/// A cancel of a timer of another scheduler fails with a clear error.
#[test]
fn rejects_a_cancel_of_a_timer_of_another_scheduler() {
    let program = BeamProgram::load(&[PING_PONG]).expect("the fixture loads");
    let error = program
        .execute_on_schedulers(&[
            ("ping_pong", "timer_cancel_use"),
            ("ping_pong", "timer_cancel_give"),
        ])
        .expect_err("the timer cancel is unsupported");
    assert!(
        error
            .to_string()
            .contains("unsupported: a BIF timer of another scheduler"),
        "{error}"
    );
}

/// A link to a port of another scheduler fails with a clear error.
#[test]
fn rejects_a_link_to_a_port_of_another_scheduler() {
    let erlang = pinned_beam("erlang");
    let program = BeamProgram::load(&[&erlang, PING_PONG]).expect("the fixture loads");
    let error = program
        .execute_on_schedulers(&[
            ("ping_pong", "port_link_use"),
            ("ping_pong", "port_link_give"),
        ])
        .expect_err("the port link is unsupported");
    assert!(
        error
            .to_string()
            .contains("unsupported: a port of another scheduler"),
        "{error}"
    );
}

/// A monitor of a port of another scheduler fails with a clear error.
#[test]
fn rejects_a_monitor_of_a_port_of_another_scheduler() {
    let erlang = pinned_beam("erlang");
    let program = BeamProgram::load(&[&erlang, PING_PONG]).expect("the fixture loads");
    let error = program
        .execute_on_schedulers(&[
            ("ping_pong", "port_monitor_use"),
            ("ping_pong", "port_monitor_give"),
        ])
        .expect_err("the port monitor is unsupported");
    assert!(
        error
            .to_string()
            .contains("unsupported: a port of another scheduler"),
        "{error}"
    );
}

/// A named BIF timer in a run with scheduler threads fails with a clear
/// error.
#[test]
fn rejects_a_named_timer() {
    let program = BeamProgram::load(&[PING_PONG]).expect("the fixture loads");
    let error = program
        .execute_on_schedulers(&[("ping_pong", "timer_named"), ("ping_pong", "monitor_hold")])
        .expect_err("the named timer is unsupported");
    assert!(
        error
            .to_string()
            .contains("unsupported: a named BIF timer in a run of execute_on_schedulers"),
        "{error}"
    );
}

/// A read of a timer of another scheduler fails with a clear error.
#[test]
fn rejects_a_read_of_a_timer_of_another_scheduler() {
    let program = BeamProgram::load(&[PING_PONG]).expect("the fixture loads");
    let error = program
        .execute_on_schedulers(&[("ping_pong", "timer_read"), ("ping_pong", "timer_give")])
        .expect_err("the timer read is unsupported");
    assert!(
        error
            .to_string()
            .contains("unsupported: a BIF timer of another scheduler"),
        "{error}"
    );
}

/// An alias in a run with scheduler threads fails with a clear error.
#[test]
fn rejects_an_alias() {
    let erlang = pinned_beam("erlang");
    let program = BeamProgram::load(&[&erlang, PING_PONG]).expect("the fixture loads");
    let error = program
        .execute_on_schedulers(&[("ping_pong", "alias_create"), ("ping_pong", "monitor_hold")])
        .expect_err("the alias is unsupported");
    assert!(
        error
            .to_string()
            .contains("unsupported: erlang:alias/0,1 in a run of execute_on_schedulers"),
        "{error}"
    );
}

/// An alias from a monitor in a run with scheduler threads fails with a
/// clear error.
#[test]
fn rejects_an_alias_from_a_monitor() {
    let program = BeamProgram::load(&[PING_PONG]).expect("the fixture loads");
    let error = program
        .execute_on_schedulers(&[("ping_pong", "monitor_alias"), ("ping_pong", "alias_hold")])
        .expect_err("the monitor alias is unsupported");
    assert!(
        error
            .to_string()
            .contains("unsupported: an alias from a monitor in a run of execute_on_schedulers"),
        "{error}"
    );
}

/// An atomic array in a run with scheduler threads fails with a clear
/// error.
#[test]
fn rejects_an_atomic_array() {
    let erts_internal = pinned_beam("erts_internal");
    let program = BeamProgram::load(&[&erts_internal, PING_PONG]).expect("the fixture loads");
    let error = program
        .execute_on_schedulers(&[("ping_pong", "atomics_new"), ("ping_pong", "info_hold")])
        .expect_err("atomic arrays are unsupported");
    assert!(
        error
            .to_string()
            .contains("unsupported: erts_internal:atomics_new/2 in a run of execute_on_schedulers"),
        "{error}"
    );
}

/// A persistent term use in a run with scheduler threads fails with a
/// clear error.
#[test]
fn rejects_a_persistent_term() {
    let program = BeamProgram::load(&[PING_PONG]).expect("the fixture loads");
    let error = program
        .execute_on_schedulers(&[("ping_pong", "persistent_put"), ("ping_pong", "info_hold")])
        .expect_err("persistent_term is unsupported");
    assert!(
        error
            .to_string()
            .contains("unsupported: persistent_term:put/2 in a run of execute_on_schedulers"),
        "{error}"
    );
}

/// An ETS call in a run with scheduler threads fails with a clear error.
#[test]
fn rejects_an_ets_table() {
    let program = BeamProgram::load(&[PING_PONG]).expect("the fixture loads");
    let error = program
        .execute_on_schedulers(&[("ping_pong", "ets_new"), ("ping_pong", "info_hold")])
        .expect_err("ets is unsupported");
    assert!(
        error
            .to_string()
            .contains("unsupported: ets:new/2 in a run of execute_on_schedulers"),
        "{error}"
    );
}

/// A term with a new atom stops a run with scheduler threads with the
/// clear unsupported error, not with `badarg`.
#[test]
fn rejects_an_atom_in_a_decoded_term() {
    let program = BeamProgram::load(&[PING_PONG]).expect("the fixture loads");
    let error = program
        .execute_on_schedulers(&[("ping_pong", "decode_atom"), ("ping_pong", "info_hold")])
        .expect_err("the atom creation is unsupported");
    assert!(
        error
            .to_string()
            .contains("cannot create the atom review_runtime_atom at runtime"),
        "{error}"
    );
}

/// A kill that the caller sent before a read of the registered name of the
/// target is handled first, so the read reports a dead target.
#[test]
fn a_remote_name_read_follows_the_kill_of_the_caller() {
    let erts_internal = pinned_beam("erts_internal");
    let program = BeamProgram::load(&[&erts_internal, PING_PONG]).expect("the fixture loads");
    let ok_atom = program.atom("ok").expect("the fixture names ok");
    let results = program
        .execute_on_schedulers(&[
            ("ping_pong", "name_after_kill"),
            ("ping_pong", "name_kill_hold"),
        ])
        .expect("the two schedulers run");
    assert_eq!(results[0].term, Term::Atom(ok_atom.clone()), "the reader");
    assert_eq!(results[1].term, Term::Atom(ok_atom), "the holder");
}

/// The scheduler wall time request gets one reply for each scheduler.
#[test]
fn disables_the_scheduler_wall_time_for_each_scheduler() {
    let erts_internal = pinned_beam("erts_internal");
    let program = BeamProgram::load(&[&erts_internal, PING_PONG]).expect("the fixture loads");
    let false_atom = program.atom("false").expect("the fixture names false");
    let ok_atom = program.atom("ok").expect("the fixture names ok");
    let results = program
        .execute_on_schedulers(&[("ping_pong", "wall_time"), ("ping_pong", "monitor_hold")])
        .expect("the two schedulers run");
    assert_eq!(results[0].term, Term::Atom(false_atom), "the old state");
    assert_eq!(results[1].term, Term::Atom(ok_atom), "the holder");
}

/// The run reports the number of its schedulers.
#[test]
fn reports_the_number_of_schedulers() {
    let program = BeamProgram::load(&[PING_PONG]).expect("the fixture loads");
    let ok_atom = program.atom("ok").expect("the fixture names ok");
    let results = program
        .execute_on_schedulers(&[
            ("ping_pong", "scheduler_count"),
            ("ping_pong", "monitor_hold"),
        ])
        .expect("the two schedulers run");
    assert_eq!(results[0].term, Term::Integer(2), "the scheduler count");
    assert_eq!(results[1].term, Term::Atom(ok_atom), "the holder");
}

/// The ports of two schedulers have different identifiers.
#[test]
fn ports_of_two_schedulers_differ() {
    // The pinned `erlang` module holds `open_port/2`.
    let erlang = pinned_beam("erlang");
    let program = BeamProgram::load(&[&erlang, PING_PONG]).expect("the fixture loads");
    let ok_atom = program.atom("ok").expect("the fixture names ok");
    let results = program
        .execute_on_schedulers(&[("ping_pong", "port_send"), ("ping_pong", "port_other")])
        .expect("the two schedulers run");
    assert_eq!(results[0].term, Term::Atom(ok_atom.clone()), "the sender");
    assert_eq!(results[1].term, Term::Atom(ok_atom), "the receiver");
}

/// A root that waits for the output of a port without a receive timer is
/// not an impossible wait, because the port can send a message.
#[test]
fn waits_for_a_port_without_a_receive_timer() {
    // The pinned `erlang` module holds `open_port/2`.
    let erlang = pinned_beam("erlang");
    let program = BeamProgram::load(&[&erlang, PING_PONG]).expect("the fixture loads");
    let ok_atom = program.atom("ok").expect("the fixture names ok");
    let results = program
        .execute_on_schedulers(&[("ping_pong", "port_wait")])
        .expect("the port sends its output");
    assert_eq!(results[0].term, Term::Atom(ok_atom), "the root");
}

/// A term that `term_to_binary/2` seeds with the local node hash decodes on
/// another scheduler, because the node holds one hash.
#[test]
fn a_local_seeded_term_decodes_on_another_scheduler() {
    let program = BeamProgram::load(&[PING_PONG]).expect("the fixture loads");
    let ok_atom = program.atom("ok").expect("the fixture names ok");
    let results = program
        .execute_on_schedulers(&[("ping_pong", "term_wait"), ("ping_pong", "term_send")])
        .expect("the two schedulers run");
    assert_eq!(results[0].term, Term::Atom(ok_atom.clone()), "the receiver");
    assert_eq!(results[1].term, Term::Atom(ok_atom), "the sender");
}

/// The schedulers agree on the run clock: a later read is not below an
/// earlier value from another scheduler, and the time offset is the same.
#[test]
fn the_schedulers_agree_on_the_run_clock() {
    let program = BeamProgram::load(&[PING_PONG]).expect("the fixture loads");
    let ok_atom = program.atom("ok").expect("the fixture names ok");
    for _ in 0..5 {
        let results = program
            .execute_on_schedulers(&[("ping_pong", "stamp_wait"), ("ping_pong", "stamp_send")])
            .expect("the two schedulers run");
        assert_eq!(results[0].term, Term::Atom(ok_atom.clone()), "the receiver");
        assert_eq!(results[1].term, Term::Atom(ok_atom.clone()), "the sender");
    }
}

/// A send to a registered name reaches its process on another scheduler.
#[test]
fn sends_to_a_registered_name_across_schedulers() {
    let program = BeamProgram::load(&[PING_PONG]).expect("the fixture loads");
    let ok_atom = program.atom("ok").expect("the fixture names ok");
    let results = program
        .execute_on_schedulers(&[("ping_pong", "name_wait"), ("ping_pong", "name_send")])
        .expect("the two schedulers run");
    assert_eq!(results[0].term, Term::Atom(ok_atom.clone()), "the receiver");
    assert_eq!(results[1].term, Term::Atom(ok_atom), "the sender");
}

/// The send to a registered name returns the same result on the pinned
/// ERTS.
#[test]
fn the_pinned_erts_sends_to_the_same_registered_name() {
    let directory =
        std::env::temp_dir().join(format!("pbeam-multischeduler-{}-name", std::process::id()));
    let _ = std::fs::remove_dir_all(&directory);
    std::fs::create_dir_all(&directory).expect("the directory of the reference run");
    let beam = directory.join("ping_pong.beam");
    std::fs::write(&beam, PING_PONG).expect("the fixture file");
    let output = std::process::Command::new("erl")
        .args(["+S", "2", "-noshell"])
        .args([
            "-eval",
            "io:format(\"~p~n\", [ping_pong:reference_name()]), halt().",
        ])
        .arg("-pa")
        .arg(&directory)
        .output()
        .expect("the pinned ERTS runs");
    std::fs::remove_dir_all(&directory).expect("the reference directory leaves");
    assert!(output.status.success(), "{output:?}");
    assert_eq!(String::from_utf8_lossy(&output.stdout), "ok\n");
}

/// A message that arrives while its receiver runs is visible to a receive
/// with no wait, also when the sender registers a name after the message.
#[test]
fn a_running_receive_reads_a_message_from_an_inbox() {
    let program = BeamProgram::load(&[PING_PONG]).expect("the fixture loads");
    let ready_atom = program.atom("ready").expect("the fixture names ready");
    let ok_atom = program.atom("ok").expect("the fixture names ok");
    for _ in 0..20 {
        let results = program
            .execute_on_schedulers(&[("ping_pong", "drain_wait"), ("ping_pong", "drain_hold")])
            .expect("the two schedulers run");
        assert_eq!(
            results[0].term,
            Term::Atom(ready_atom.clone()),
            "the receiver"
        );
        assert_eq!(results[1].term, Term::Atom(ok_atom.clone()), "the sender");
    }
}

/// A read of the caller counts a message that another scheduler sent, also
/// when the caller did not schedule since the send.
#[test]
fn a_running_caller_read_counts_a_message_from_an_inbox() {
    let program = BeamProgram::load(&[PING_PONG]).expect("the fixture loads");
    let ok_atom = program.atom("ok").expect("the fixture names ok");
    for _ in 0..10 {
        let results = program
            .execute_on_schedulers(&[("ping_pong", "len_wait"), ("ping_pong", "len_hold")])
            .expect("the two schedulers run");
        assert_eq!(results[0].term, Term::Integer(1), "the queued message");
        assert_eq!(results[1].term, Term::Atom(ok_atom.clone()), "the sender");
    }
}

/// The process list of a run with scheduler threads is not readable yet,
/// and the run stops with a clear error.
#[test]
fn rejects_the_process_list_of_a_scheduler_run() {
    let program = BeamProgram::load(&[PING_PONG]).expect("the fixture loads");
    let error = program
        .execute_on_schedulers(&[("ping_pong", "process_list"), ("ping_pong", "info_hold")])
        .expect_err("the process list is unsupported");
    assert!(
        error
            .to_string()
            .contains("unsupported: erlang:processes/0"),
        "{error}"
    );
}

/// A read of another local process counts a message that another scheduler
/// sent, also when its scheduler did not schedule since the send.
#[test]
fn a_running_read_counts_a_message_for_a_local_child() {
    // A target with a message that it did not fetch gets a process-info
    // request, and the caller waits for the reply.
    let erts_internal = pinned_beam("erts_internal");
    let program = BeamProgram::load(&[&erts_internal, PING_PONG]).expect("the fixture loads");
    let ok_atom = program.atom("ok").expect("the fixture names ok");
    for _ in 0..10 {
        let results = program
            .execute_on_schedulers(&[("ping_pong", "child_wait"), ("ping_pong", "child_send")])
            .expect("the two schedulers run");
        assert_eq!(results[0].term, Term::Integer(1), "the queued message");
        assert_eq!(results[1].term, Term::Atom(ok_atom.clone()), "the sender");
    }
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

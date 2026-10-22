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

/// The fixture whose two roots load code on one scheduler and call it on
/// the other.
const CODE_LOADING: &[u8] = include_bytes!("../../fixtures/multischeduler/code_loading.beam");

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

/// Two schedulers that create one new atom get one atom, as the one atom
/// table of ERTS gives (`erts_atom_put_index`, `atom.c:231`). The pinned
/// ERTS returns `pbeam_shared_atom` and
/// `{true, pbeam_shared_atom, "pbeam_shared_atom"}`.
#[test]
fn two_schedulers_share_a_new_atom() {
    let program = BeamProgram::load(&[PING_PONG]).expect("the fixture loads");
    let results = program
        .execute_on_schedulers(&[("ping_pong", "atom_send"), ("ping_pong", "atom_receive")])
        .expect("the two schedulers run");
    let Term::Atom(sent) = &results[0].term else {
        panic!("the sender returns an atom: {:?}", results[0]);
    };
    assert_eq!(results[0].atoms().name_of(sent), "pbeam_shared_atom");
    let Term::Tuple(received) = &results[1].term else {
        panic!("the receiver returns a tuple: {:?}", results[1]);
    };
    assert_eq!(
        received[0],
        Term::Atom(program.atom("true").expect("the fixture names true"))
    );
    assert_eq!(received[1], results[0].term);
}

/// A new atom of `list_to_atom/1` and of `binary_to_term/1` on two
/// schedulers. The pinned ERTS returns `review_runtime_atom` and an atom
/// `new_scheduler_atom_<N>`.
#[test]
fn creates_an_atom_on_each_scheduler() {
    let program = BeamProgram::load(&[PING_PONG]).expect("the fixture loads");
    let results = program
        .execute_on_schedulers(&[("ping_pong", "decode_atom"), ("ping_pong", "atom_create")])
        .expect("the two schedulers run");
    let names: Vec<&str> = results
        .iter()
        .map(|result| match &result.term {
            Term::Atom(atom) => result.atoms().name_of(atom),
            other => panic!("each root returns an atom: {other:?}"),
        })
        .collect();
    assert_eq!(names[0], "review_runtime_atom");
    assert!(names[1].starts_with("new_scheduler_atom_"), "{}", names[1]);
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

/// A module that one scheduler loads runs on the other scheduler. A load of
/// a new version after `erlang:delete_module/1` leaves a process of the
/// other scheduler in the old code, and a later call runs the new code, as
/// `erl +S 2` gives (`beam_bif_load.c:381-507`, `2412-2455`).
#[test]
fn loads_code_on_one_scheduler_and_runs_it_on_another() {
    let (erlang, zlib) = (pinned_beam("erlang"), pinned_beam("zlib"));
    let program = BeamProgram::load(&[&erlang, &zlib, CODE_LOADING]).expect("the fixture loads");
    let atom = |name: &str| Term::Atom(program.atom(name).expect("the fixture names the atom"));
    let pair = |first: Term, second: Term| Term::tuple(vec![first, second]);
    let loader = pair(pair(atom("error"), atom("not_purged")), Term::Integer(2));
    let caller = Term::tuple(vec![
        pair(atom("true"), Term::Integer(1)),
        pair(atom("old"), Term::Integer(1)),
        Term::Integer(2),
    ]);
    for _ in 0..10 {
        let results = program
            .execute_on_schedulers(&[("code_loading", "loader"), ("code_loading", "caller")])
            .expect("the two schedulers run");
        assert_eq!(results[0].term, loader, "the loader");
        assert_eq!(results[1].term, caller, "the caller");
    }
    let directory = std::env::temp_dir().join(format!(
        "pbeam-multischeduler-{}-code-loading",
        std::process::id()
    ));
    let _ = std::fs::remove_dir_all(&directory);
    std::fs::create_dir_all(&directory).expect("the directory of the reference run");
    std::fs::write(directory.join("code_loading.beam"), CODE_LOADING).expect("the fixture file");
    let output = std::process::Command::new("erl")
        .args(["+S", "2", "-noshell"])
        .args([
            "-eval",
            "io:format(\"~p~n\", [code_loading:reference()]), halt().",
        ])
        .arg("-pa")
        .arg(&directory)
        .output()
        .expect("the pinned ERTS runs");
    std::fs::remove_dir_all(&directory).expect("the reference directory leaves");
    assert!(output.status.success(), "{output:?}");
    assert_eq!(
        String::from_utf8_lossy(&output.stdout),
        "{{{error,not_purged},2},{{true,1},{old,1},2}}\n"
    );
}

/// An external fun and `apply/3` on one scheduler see the module that the
/// other scheduler loaded, as `erl +S 2` gives `{{true,1},ok}` for the
/// two roots.
#[test]
fn applies_a_module_that_another_scheduler_loaded() {
    let (erlang, zlib) = (pinned_beam("erlang"), pinned_beam("zlib"));
    let program = BeamProgram::load(&[&erlang, &zlib, CODE_LOADING]).expect("the fixture loads");
    let atom = |name: &str| Term::Atom(program.atom(name).expect("the fixture names the atom"));
    for _ in 0..10 {
        let results = program
            .execute_on_schedulers(&[
                ("code_loading", "apply_loaded"),
                ("code_loading", "load_tell"),
            ])
            .expect("the two schedulers run");
        assert_eq!(
            results[0].term,
            Term::tuple(vec![atom("true"), Term::Integer(1)])
        );
        assert_eq!(results[1].term, atom("ok"));
    }
}

/// A purge in a run with scheduler threads fails with a clear error: it
/// needs `check_process_code` on the processes of each scheduler.
#[test]
fn rejects_a_purge() {
    let program = BeamProgram::load(&[CODE_LOADING]).expect("the fixture loads");
    let error = program
        .execute_on_schedulers(&[("code_loading", "purge"), ("code_loading", "purge")])
        .expect_err("the purge is unsupported");
    assert!(
        error.to_string().contains(
            "unsupported: erts_internal:purge_module/2 in a run of execute_on_schedulers"
        ),
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

/// The impossible wait is a decision of the node: two roots of two
/// schedulers that wait without a timer give the error, although each
/// scheduler has a live process that another scheduler could reach.
#[test]
fn reports_two_roots_that_wait_on_two_schedulers() {
    let program = BeamProgram::load(&[PING_PONG]).expect("the fixture loads");
    let error = program
        .execute_on_schedulers(&[("ping_pong", "wait"), ("ping_pong", "wait")])
        .expect_err("both roots wait");
    assert_eq!(
        error.to_string(),
        "root process waits for a message that cannot arrive"
    );
}

/// A root and its child that both wait without a timer give the error.
#[test]
fn reports_a_root_that_waits_with_a_waiting_child() {
    // The pinned `erlang` module holds `spawn/1`.
    let erlang = pinned_beam("erlang");
    let program = BeamProgram::load(&[&erlang, PING_PONG]).expect("the fixture loads");
    let error = program
        .execute_on_schedulers(&[("ping_pong", "stuck_child")])
        .expect_err("the root and its child wait");
    assert_eq!(
        error.to_string(),
        "root process waits for a message that cannot arrive"
    );
}

/// Run `relay_wait/0` on one scheduler and `sender` on the other, and check
/// that the waiting root gets the message of the sender. While the sender
/// uses its source, the scheduler of the waiting root is idle, so the
/// source alone prevents the error.
fn relays_through(sender: &str) {
    // The pinned `erlang` module holds `open_port/2`.
    let erlang = pinned_beam("erlang");
    let program = BeamProgram::load(&[&erlang, PING_PONG]).expect("the fixture loads");
    let ok_atom = Term::Atom(program.atom("ok").expect("the fixture names ok"));
    let results = program
        .execute_on_schedulers(&[("ping_pong", "relay_wait"), ("ping_pong", sender)])
        .expect("the waiting root gets its message");
    let results: Vec<Term> = results.into_iter().map(|result| result.term).collect();
    assert_eq!(results, [ok_atom.clone(), ok_atom], "{sender}");
}

/// A process that can run on another scheduler prevents the error.
#[test]
fn a_runnable_process_of_another_scheduler_prevents_the_error() {
    relays_through("busy_send");
}

/// A receive timer of another scheduler prevents the error.
#[test]
fn a_timer_of_another_scheduler_prevents_the_error() {
    relays_through("timer_send");
}

/// A port of another scheduler that waits for output prevents the error.
#[test]
fn a_port_of_another_scheduler_prevents_the_error() {
    relays_through("port_relay");
}

/// Two roots that bounce one message, each with a wait without a timer, do
/// not give the error while the message is in flight. The test repeats the
/// run to exercise different interleavings of the two threads.
#[test]
fn a_delivery_in_flight_prevents_the_error() {
    let program = BeamProgram::load(&[PING_PONG]).expect("the fixture loads");
    let ok_atom = Term::Atom(program.atom("ok").expect("the fixture names ok"));
    for _ in 0..10 {
        let results = program
            .execute_on_schedulers(&[("ping_pong", "bounce_ping"), ("ping_pong", "bounce_pong")])
            .expect("the message is always in flight or handled");
        let results: Vec<Term> = results.into_iter().map(|result| result.term).collect();
        assert_eq!(results, [ok_atom.clone(), ok_atom.clone()]);
    }
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

/// Run the two roots `first` and `second` on two schedulers, and assert
/// that their results are the terms of `expected`. `expected` gets a
/// function that makes an atom of the fixture.
fn timer_pair(
    first: &str,
    second: &str,
    expected: impl FnOnce(&dyn Fn(&str) -> Term) -> [Term; 2],
) {
    let program = BeamProgram::load(&[PING_PONG]).expect("the fixture loads");
    let results = program
        .execute_on_schedulers(&[("ping_pong", first), ("ping_pong", second)])
        .expect("the two schedulers run");
    let atom = |name: &str| Term::Atom(program.atom(name).expect("the fixture names the atom"));
    let expected = expected(&atom);
    assert_eq!(results[0].term, expected[0], "{first}");
    assert_eq!(results[1].term, expected[1], "{second}");
}

/// A BIF timer to a process of another scheduler sends its message there,
/// as `erl +S 2` gives `[ok, ok]` (`erl_hl_timer.c:1804-1871`).
#[test]
fn fires_a_timer_to_a_process_of_another_scheduler() {
    timer_pair("timer_remote", "timer_hold", |a| [a("ok"), a("ok")]);
    timer_pair("timer_remote4", "timer_hold", |a| [a("ok"), a("ok")]);
}

/// A named BIF timer sends to the name on another scheduler, and a timer
/// to a name that no process has stays active, as `erl +S 2` gives
/// `[true, ok]`.
#[test]
fn fires_a_named_timer() {
    timer_pair("timer_named", "timer_hold", |a| [a("true"), a("ok")]);
    timer_pair("timer_named4", "timer_hold", |a| [a("true"), a("ok")]);
}

/// A read of a timer of another scheduler gives the time left and leaves
/// the timer active, as `erl +S 2` gives `[true, true]`
/// (`erl_hl_timer.c:2236-2339`).
#[test]
fn reads_a_timer_of_another_scheduler() {
    timer_pair("timer_read", "timer_give", |a| [a("true"), a("true")]);
}

/// A cancel of a timer of another scheduler gives the time left, then
/// `false`, and an async cancel sends its result, as `erl +S 2` gives
/// `[{true,false,true}, {false,false}]`.
#[test]
fn cancels_a_timer_of_another_scheduler() {
    timer_pair("timer_cancel_use", "timer_cancel_give", |a| {
        [
            Term::tuple(vec![a("true"), a("false"), a("true")]),
            Term::tuple(vec![a("false"), a("false")]),
        ]
    });
}

/// The exit of a receiver of another scheduler cancels the timer at its
/// owner, as `erl +S 2` gives `[false, ok]`.
#[test]
fn cancels_a_timer_of_another_scheduler_at_the_receiver_exit() {
    timer_pair("timer_exit_use", "timer_exit_hold", |a| {
        [a("false"), a("ok")]
    });
}

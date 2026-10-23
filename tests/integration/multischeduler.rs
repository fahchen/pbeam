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

/// The fixture that makes local funs of two versions of `fun_target`.
const FUN_ENTRIES: &[u8] = include_bytes!("../../fixtures/multischeduler/fun_entries.beam");

/// The fixture whose two roots purge code on one scheduler while a process
/// of the other scheduler runs it.
const CODE_PURGE: &[u8] = include_bytes!("../../fixtures/multischeduler/code_purge.beam");

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

/// Each port BIF on a spawn port of another scheduler gives the result of
/// the pinned ERTS on `erl +S 2`: `{true, "/bin/cat", true, <<"hi">>,
/// badarg, normal, false, undefined}` and `ok`.
#[test]
fn uses_a_port_of_another_scheduler() {
    let erlang = pinned_beam("erlang");
    let lists = pinned_beam("lists");
    let program = BeamProgram::load(&[&erlang, &lists, PING_PONG]).expect("the fixture loads");
    let atom = |name| Term::Atom(program.atom(name).expect("the fixture names the atom"));
    let results = program
        .execute_on_schedulers(&[("ping_pong", "port_use"), ("ping_pong", "port_give")])
        .expect("the two schedulers run");
    let expected = Term::tuple(vec![
        atom("true"),
        Term::list_from(b"/bin/cat".iter().map(|&byte| Term::Integer(byte.into()))),
        atom("true"),
        Term::binary(b"hi".to_vec()),
        atom("badarg"),
        atom("normal"),
        atom("false"),
        atom("undefined"),
    ]);
    assert_eq!(results[0].term, expected, "the user");
    assert_eq!(results[1].term, atom("ok"), "the giver");
}

/// A transfer of a tcp_inet port to a process of another scheduler fails
/// with a clear error.
#[test]
fn rejects_an_inet_port_transfer_across_schedulers() {
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
            .contains("unsupported: an inet port of another scheduler"),
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

/// A compiled binary pattern of another scheduler matches as on `erl +S 2`.
#[test]
fn matches_with_a_compiled_binary_pattern_of_another_scheduler() {
    let program = BeamProgram::load(&[PING_PONG]).expect("the fixture loads");
    let ok_atom = program.atom("ok").expect("the fixture names ok");
    let results = program
        .execute_on_schedulers(&[
            ("ping_pong", "pattern_match"),
            ("ping_pong", "pattern_give"),
        ])
        .expect("the two schedulers run");
    assert_eq!(
        results[0].term,
        Term::tuple(vec![Term::Integer(0.into()), Term::Integer(3.into())]),
        "the user"
    );
    assert_eq!(results[1].term, Term::Atom(ok_atom), "the giver");
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

/// A compiled expression of another scheduler runs as on `erl +S 2`.
#[test]
fn runs_a_compiled_expression_of_another_scheduler() {
    let program = {
        let re = pinned_beam("re");
        let prim_tty = pinned_beam("prim_tty");
        BeamProgram::load(&[PING_PONG, &prim_tty, &re])
    }
    .expect("the fixture loads");
    let ok_atom = program.atom("ok").expect("the fixture names ok");
    let results = program
        .execute_on_schedulers(&[("ping_pong", "regex_run"), ("ping_pong", "regex_give")])
        .expect("the two schedulers run");
    assert_eq!(
        results[0].term,
        Term::tuple(vec![
            match_atom(&program),
            Term::list_from([Term::tuple(vec![
                Term::Integer(0.into()),
                Term::Integer(3.into())
            ])])
        ]),
        "the user"
    );
    assert_eq!(results[1].term, Term::Atom(ok_atom), "the giver");
}

/// A compiled expression and a compiled pattern in a persistent term of
/// another scheduler run as on `erl +S 2`.
#[test]
fn runs_compiled_patterns_of_a_persistent_term_of_another_scheduler() {
    let program = {
        let re = pinned_beam("re");
        let prim_tty = pinned_beam("prim_tty");
        BeamProgram::load(&[PING_PONG, &prim_tty, &re])
    }
    .expect("the fixture loads");
    let ok_atom = program.atom("ok").expect("the fixture names ok");
    let results = program
        .execute_on_schedulers(&[
            ("ping_pong", "term_patterns_use"),
            ("ping_pong", "term_patterns_give"),
        ])
        .expect("the two schedulers run");
    assert_eq!(
        results[0].term,
        Term::tuple(vec![
            Term::tuple(vec![
                match_atom(&program),
                Term::list_from([Term::binary(b"bc".to_vec()), Term::binary(b"c".to_vec())])
            ]),
            Term::tuple(vec![Term::Integer(1.into()), Term::Integer(2.into())])
        ]),
        "the user"
    );
    assert_eq!(results[1].term, Term::Atom(ok_atom), "the giver");
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

/// A failed `-on_load` function on one scheduler purges its code while a
/// process of the other scheduler runs it: `erts_code_purger` asks each
/// process with `check_process_code`, and kills the process that runs the
/// code (`erts_code_purger.erl:166-182`, `342-344`). Then
/// `erts_code_purger:purge/1` and `soft_purge/1` of a module with no old
/// code give `{false, false}` and `true` (`beam_bif_load.c:2100-2189`). The
/// pinned ERTS gives the same result with `erl +S 2`.
#[test]
fn purges_code_that_a_process_of_another_scheduler_runs() {
    let beams = ["erlang", "zlib", "erts_code_purger"].map(pinned_beam);
    let mut modules: Vec<&[u8]> = beams.iter().map(Vec::as_slice).collect();
    modules.push(CODE_PURGE);
    let program = BeamProgram::load(&modules).expect("the fixture loads");
    let atom = |name: &str| Term::Atom(program.atom(name).expect("the fixture names the atom"));
    let no_purge = Term::tuple(vec![atom("false"), atom("false")]);
    let loader = Term::tuple(vec![
        atom("error"),
        atom("false"),
        no_purge.clone(),
        atom("true"),
        no_purge,
        atom("true"),
    ]);
    let holder = Term::tuple(vec![atom("down"), atom("killed")]);
    for _ in 0..10 {
        let results = program
            .execute_on_schedulers(&[("code_purge", "loader"), ("code_purge", "holder")])
            .expect("the two schedulers run");
        assert_eq!(results[0].term, loader, "the loader");
        assert_eq!(results[1].term, holder, "the holder");
    }
    let directory = std::env::temp_dir().join(format!(
        "pbeam-multischeduler-{}-code-purge",
        std::process::id()
    ));
    let _ = std::fs::remove_dir_all(&directory);
    std::fs::create_dir_all(&directory).expect("the directory of the reference run");
    std::fs::write(directory.join("code_purge.beam"), CODE_PURGE).expect("the fixture file");
    let output = std::process::Command::new("erl")
        .args(["+S", "2", "-noshell"])
        .args([
            "-eval",
            "io:format(\"~p~n\", [code_purge:reference()]), halt().",
        ])
        .arg("-pa")
        .arg(&directory)
        .output()
        .expect("the pinned ERTS runs");
    std::fs::remove_dir_all(&directory).expect("the reference directory leaves");
    assert!(output.status.success(), "{output:?}");
    assert_eq!(
        String::from_utf8_lossy(&output.stdout),
        "{{error,false,{false,false},true,{false,false},true},{down,killed}}\n"
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

/// A split with a compiled pattern of another scheduler gives the result of
/// `erl +S 2`.
#[test]
fn splits_with_a_pattern_of_another_scheduler() {
    let program = BeamProgram::load(&[PING_PONG]).expect("the fixture loads");
    let ok_atom = program.atom("ok").expect("the fixture names ok");
    let results = program
        .execute_on_schedulers(&[
            ("ping_pong", "pattern_split"),
            ("ping_pong", "pattern_split_give"),
        ])
        .expect("the two schedulers run");
    assert_eq!(
        results[0].term,
        Term::list_from([Term::binary(b"a".to_vec()), Term::binary(b"b".to_vec())]),
        "the user"
    );
    assert_eq!(results[1].term, Term::Atom(ok_atom), "the giver");
}

/// A matches call with a compiled pattern of another scheduler gives the
/// result of `erl +S 2`.
#[test]
fn matches_all_with_a_pattern_of_another_scheduler() {
    let program = BeamProgram::load(&[PING_PONG]).expect("the fixture loads");
    let ok_atom = program.atom("ok").expect("the fixture names ok");
    let results = program
        .execute_on_schedulers(&[
            ("ping_pong", "pattern_matches"),
            ("ping_pong", "pattern_matches_give"),
        ])
        .expect("the two schedulers run");
    assert_eq!(
        results[0].term,
        Term::list_from([Term::tuple(vec![
            Term::Integer(1.into()),
            Term::Integer(1.into())
        ])]),
        "the user"
    );
    assert_eq!(results[1].term, Term::Atom(ok_atom), "the giver");
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

/// A link to a tcp_inet port of another scheduler fails with a clear error.
#[test]
fn rejects_a_link_to_an_inet_port_of_another_scheduler() {
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
            .contains("unsupported: an inet port of another scheduler"),
        "{error}"
    );
}

/// Two roots on two schedulers add to one counter array, one
/// `write_concurrency` counter array, and one unsigned atomic array. The
/// pinned ERTS on `erl +S 10`, with the roots on the schedulers 1 and 2,
/// gives `{8, 8, 18446744073709551615}` and `ok`.
#[test]
fn shares_an_atomic_array_between_schedulers() {
    let program = BeamProgram::load(&[
        &pinned_beam("erts_internal"),
        &pinned_beam("counters"),
        &pinned_beam("atomics"),
        PING_PONG,
    ])
    .expect("the fixture loads");
    let ok_atom = program.atom("ok").expect("the fixture names ok");
    let results = program
        .execute_on_schedulers(&[
            ("ping_pong", "counters_hold"),
            ("ping_pong", "counters_add"),
        ])
        .expect("the two schedulers run");
    assert_eq!(
        results[0].term,
        Term::tuple(vec![
            Term::Integer(8),
            Term::Integer(8),
            Term::BigInt(std::sync::Arc::new(num_bigint::BigInt::from(u64::MAX))),
        ]),
        "the holder"
    );
    assert_eq!(results[1].term, Term::Atom(ok_atom), "the adder");
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

/// Return the atom `match` of the program.
fn match_atom(program: &BeamProgram) -> Term {
    Term::Atom(program.atom("match").expect("the program names match"))
}

/// The MD5 of version 1 and version 2 of `fun_target`, which `erl +S 2`
/// gives for `erlang:fun_info(F, new_uniq)`.
const FUN_TARGET_MD5: [[u8; 16]; 2] = [
    [
        85, 226, 106, 132, 50, 37, 76, 123, 119, 49, 23, 113, 186, 185, 144, 210,
    ],
    [
        220, 232, 137, 3, 146, 143, 59, 198, 32, 142, 210, 51, 8, 0, 84, 90,
    ],
];

/// Return the `fun_info` items of `fun_entries:info/1` for a fun of
/// version `version` of `fun_target`, with the `pid` item `pid`.
fn fun_entry_info(program: &BeamProgram, version: usize, pid: &Term) -> Term {
    let atom = |name: &str| Term::Atom(program.atom(name).expect("the fixture names the atom"));
    let pair = |name: &str, value: Term| Term::tuple(vec![atom(name), value]);
    let uniq = [45_028_180, 115_819_592][version - 1];
    Term::list_from([
        pair("index", Term::Integer(0)),
        pair("new_index", Term::Integer(0)),
        pair("uniq", Term::Integer(uniq)),
        pair(
            "new_uniq",
            Term::binary(FUN_TARGET_MD5[version - 1].to_vec()),
        ),
        pair("pid", pid.clone()),
    ])
}

/// Return the `pid` item of the first `fun_info` list of `info`, and check
/// that it is `<0.0.0>`, the pid of the first process (`erl_bif_info.c:3943-3946`).
fn first_process(info: &Term) -> Term {
    let (items, _) = info.list_elements().expect("a list of items");
    let pid = items[4].tuple_elements().expect("an item")[1].clone();
    assert_eq!(format!("{pid:?}"), "Pid(ProcessId(0))");
    pid
}

/// The loader keeps the module MD5 and the old index of each `FunT` entry
/// (`jit/asm_load.c:1218-1223`, `beam_file.c:1751-1830`), so `erlang:fun_info/2` gives `index`,
/// `new_uniq`, and `pid` as `erl +S 2` does, with one scheduler and with two
/// (`erl_bif_info.c:3943-3967`). A fun of another version is not equal, and
/// two funs of one entry are (`utils.c:2157-2177`).
#[test]
fn gives_the_fun_entry_items_of_two_module_versions_as_pinned_erts() {
    let modules = ["erlang", "zlib", "erts_code_purger"].map(pinned_beam);
    let program = BeamProgram::load(&[&modules[0], &modules[1], &modules[2], FUN_ENTRIES])
        .expect("the fixture loads");
    let atom = |name: &str| Term::Atom(program.atom(name).expect("the fixture names the atom"));
    let single = program
        .execute("fun_entries", "single", &[])
        .expect("the single run");
    let (items, _) = single.term.list_elements().expect("a list");
    let pid = first_process(&items[0]);
    let expected_single = Term::list_from([
        fun_entry_info(&program, 1, &pid),
        fun_entry_info(&program, 2, &pid),
        atom("false"),
        atom("true"),
        Term::tuple(vec![Term::Integer(2), atom("a")]),
    ]);
    assert_eq!(single.term, expected_single, "one scheduler");
    for _ in 0..10 {
        let results = program
            .execute_on_schedulers(&[("fun_entries", "loader"), ("fun_entries", "maker")])
            .expect("the two schedulers run");
        let pid = first_process(&results[1].term);
        let loader = Term::tuple(vec![
            fun_entry_info(&program, 1, &pid),
            atom("true"),
            Term::tuple(vec![Term::Integer(1), atom("a")]),
        ]);
        assert_eq!(results[0].term, loader, "the loader");
        assert_eq!(
            results[1].term,
            fun_entry_info(&program, 1, &pid),
            "the maker"
        );
    }
    let directory = std::env::temp_dir().join(format!(
        "pbeam-multischeduler-{}-fun-entries",
        std::process::id()
    ));
    let _ = std::fs::remove_dir_all(&directory);
    std::fs::create_dir_all(&directory).expect("the directory of the reference run");
    std::fs::write(directory.join("fun_entries.beam"), FUN_ENTRIES).expect("the fixture file");
    let output = std::process::Command::new("erl")
        .args(["+S", "2", "-noshell"])
        .args([
            "-eval",
            "io:format(\"~w~n\", [fun_entries:reference()]), halt().",
        ])
        .arg("-pa")
        .arg(&directory)
        .output()
        .expect("the pinned ERTS runs");
    std::fs::remove_dir_all(&directory).expect("the reference directory leaves");
    assert!(output.status.success(), "{output:?}");
    let info = |version: usize| {
        let uniq = [45_028_180, 115_819_592][version - 1];
        let md5 = FUN_TARGET_MD5[version - 1]
            .map(|byte| byte.to_string())
            .join(",");
        format!(
            "[{{index,0}},{{new_index,0}},{{uniq,{uniq}}},{{new_uniq,<<{md5}>>}},{{pid,<0.0.0>}}]"
        )
    };
    assert_eq!(
        String::from_utf8_lossy(&output.stdout),
        format!(
            "{{[{},{},false,true,{{2,a}}],{{{},true,{{1,a}}}},{}}}\n",
            info(1),
            info(2),
            info(1),
            info(1)
        )
    );
}

use super::{
    CONTEXT_REDS, ExitPhase, LinkId, MonitorOptions, MonitorTarget, RELEASE_BATCH, RELEASE_CHUNK,
    Release, Scheduler, StartFlags, Turn, node::Node,
};
use crate::runtime::capability::{AllowAll, Capability, CapabilityKind, Denied, Policy};
use crate::runtime::host::input::{Readiness, wait_readable};
use crate::runtime::{
    Job,
    dirty::{Output, Work},
    host::output::{IovecWrite, IovecWritten},
};
use crate::term::{AtomTable, test_atom};
use crate::{
    runtime::ets::{Access, Table, TableType},
    runtime::process::{
        Monitor, Origin, OriginMonitor, PendingBif, ProcessState, ReceiveTimer, StackSlot, Start,
        TimerReference,
        tests::{frame, test_scheduler, test_scheduler_without_checks},
    },
    runtime::signal::Signal,
    runtime::timer::{Receiver, Timer},
    term::{Atom, Fun, Lambda, Map, ProcessId, Reference, Term},
};
use std::{
    io::{PipeReader, PipeWriter, Read, Write},
    os::fd::AsRawFd,
    sync::Arc,
    time::Duration,
};

/// Return the options of `erlang:monitor/2`.
fn no_options() -> MonitorOptions {
    MonitorOptions {
        alias: None,
        tag: atom("DOWN"),
        priority: false,
    }
}

fn atom(name: &str) -> Term {
    test_atom(name)
}

fn messages(scheduler: &Scheduler, id: u64) -> Vec<Term> {
    scheduler.processes[&ProcessId(id)].mailbox.messages()
}

/// Return the initial capabilities of the default policy.
fn allow_all() -> Vec<Capability> {
    AllowAll.initial()
}

fn current(scheduler: &mut Scheduler) -> ProcessId {
    scheduler.current_process_mut().id
}

#[test]
fn adds_the_reductions_of_an_ended_turn_and_starts_the_next() {
    // A BIF that runs several turns in one call adds each ended turn to the
    // process total and starts the next with a full budget, as the switch
    // out of the `erlang:re_run_trap/3` trap does (`erl_process.c:9678-9700`).
    let mut scheduler = test_scheduler();
    scheduler.start(frame(), vec![], allow_all()).unwrap();
    assert_eq!(scheduler.schedule_next().unwrap(), Turn::Run);
    scheduler.current_process_mut().fcalls = 3_000;
    scheduler.account_turn();
    assert_eq!(scheduler.current_process().fcalls, CONTEXT_REDS);
    assert_eq!(scheduler.current_process().reductions, 1_000);
    scheduler.current_process_mut().fcalls = 500;
    scheduler.account_turn();
    assert_eq!(scheduler.current_process().fcalls, CONTEXT_REDS);
    assert_eq!(scheduler.current_process().reductions, 4_500);
}

#[test]
fn keeps_the_current_process_in_the_table_and_out_of_the_run_queue() {
    let mut scheduler = test_scheduler();
    scheduler.start(frame(), vec![], allow_all()).unwrap();
    assert_eq!(scheduler.schedule_next().unwrap(), Turn::Run);
    assert_eq!(current(&mut scheduler), ProcessId(0));
    assert!(scheduler.processes.contains_key(&ProcessId(0)));
    let (child, monitor) = scheduler
        .spawn(
            Start::Entry(frame()),
            vec![],
            None,
            false,
            StartFlags::default(),
            None,
        )
        .unwrap()
        .unwrap();
    assert_eq!((child, monitor), (ProcessId(1), None));

    scheduler.park_current();
    assert_eq!(scheduler.schedule_next().unwrap(), Turn::Run);
    assert_eq!(current(&mut scheduler), ProcessId(1));
    assert!(scheduler.runnable.ids().is_empty());
    scheduler.deliver(ProcessId(0), atom("ping"));
    scheduler.deliver(ProcessId(1), atom("self"));
    scheduler.deliver(ProcessId(9), atom("lost"));

    assert_eq!(scheduler.runnable.ids(), vec![ProcessId(0)]);
    assert_eq!(messages(&scheduler, 0), [atom("ping")]);
    assert_eq!(messages(&scheduler, 1), [atom("self")]);
    scheduler.yield_current();
    assert_eq!(scheduler.schedule_next().unwrap(), Turn::Run);
    scheduler.park_current();
    assert_eq!(scheduler.schedule_next().unwrap(), Turn::Run);
    scheduler.park_current();
    assert_eq!(scheduler.schedule_next().unwrap(), Turn::Idle);
}

#[test]
fn makes_no_capability_copies_in_a_run_that_skips_the_checks() {
    let kinds = [
        CapabilityKind::Process,
        CapabilityKind::Terminal,
        CapabilityKind::Port,
        CapabilityKind::Env,
        CapabilityKind::Signal,
        CapabilityKind::Runtime,
        CapabilityKind::PersistentTerm,
        CapabilityKind::Code,
    ];
    // With the checks, a root process with no capability cannot spawn.
    let mut checked = test_scheduler();
    assert!(!checked.skips_checks());
    checked.start(frame(), vec![], Vec::new()).unwrap();
    checked.schedule_next().unwrap();
    let spawn = checked.spawn(
        Start::Entry(frame()),
        vec![],
        Some(Term::static_atom("DOWN")),
        true,
        StartFlags::default(),
        None,
    );
    assert_eq!(spawn.unwrap().err(), Some(Denied));
    assert_eq!(
        checked.revoke_given(ProcessId(9), CapabilityKind::Env),
        Err(Denied)
    );
    // Without the checks, it spawns with a monitor and a link, the child
    // gets no copy, and a revocation has no effect.
    let mut scheduler = test_scheduler_without_checks();
    assert!(scheduler.skips_checks());
    scheduler.start(frame(), vec![], Vec::new()).unwrap();
    scheduler.schedule_next().unwrap();
    let (child, monitor) = scheduler
        .spawn(
            Start::Entry(frame()),
            vec![],
            Some(Term::static_atom("DOWN")),
            true,
            StartFlags::default(),
            None,
        )
        .unwrap()
        .unwrap();
    assert_eq!(child, ProcessId(1));
    assert!(monitor.is_some());
    assert_eq!(scheduler.revoke_given(child, CapabilityKind::Env), Ok(()));
    for id in [ProcessId(0), child] {
        let security = &scheduler.processes[&id].security;
        assert!(kinds.iter().all(|&kind| security.ids(kind).is_empty()));
    }
}

#[test]
fn switches_out_to_the_end_of_the_run_queue_with_a_new_budget() {
    let mut scheduler = test_scheduler();
    scheduler.start(frame(), vec![], allow_all()).unwrap();
    assert_eq!(scheduler.schedule_next().unwrap(), Turn::Run);
    scheduler
        .spawn(
            Start::Entry(frame()),
            vec![],
            None,
            false,
            StartFlags::default(),
            None,
        )
        .unwrap()
        .unwrap();
    scheduler.current_process_mut().fcalls = -1;
    scheduler.yield_current();
    assert_eq!(scheduler.runnable.ids(), vec![ProcessId(1), ProcessId(0)]);
    assert_eq!(scheduler.schedule_next().unwrap(), Turn::Run);
    assert_eq!(current(&mut scheduler), ProcessId(1));
    scheduler.yield_current();
    assert_eq!(scheduler.schedule_next().unwrap(), Turn::Run);
    assert_eq!(scheduler.current_process_mut().fcalls, CONTEXT_REDS);
}

/// Return the result of the root process of a run with one scheduler, which
/// the node collects.
fn root_result(scheduler: &Scheduler) -> Option<Result<Term, Term>> {
    scheduler.node.take_root_results().pop().flatten()
}

#[test]
fn sends_down_and_exit_signals_and_records_the_root_result() {
    let mut scheduler = test_scheduler();
    scheduler.start(frame(), vec![], allow_all()).unwrap();
    scheduler.schedule_next().unwrap();
    scheduler.current_process_mut().trap_exit = true;
    let (monitored, monitor) = scheduler
        .spawn(
            Start::Entry(frame()),
            vec![],
            Some(Term::static_atom("DOWN")),
            false,
            StartFlags::default(),
            None,
        )
        .unwrap()
        .unwrap();
    let (linked, _) = scheduler
        .spawn(
            Start::Entry(frame()),
            vec![],
            None,
            true,
            StartFlags::default(),
            None,
        )
        .unwrap()
        .unwrap();
    scheduler.park_current();

    scheduler.schedule_next().unwrap();
    scheduler.finish_current(atom("normal"), None).unwrap();
    scheduler.schedule_next().unwrap();
    scheduler.finish_current(atom("boom"), None).unwrap();

    // The root process waits, so it handles both signals at schedule-in,
    // and the messages give it Erlang work.
    assert_eq!(scheduler.schedule_next().unwrap(), Turn::Run);
    assert_eq!(current(&mut scheduler), ProcessId(0));
    assert!(scheduler.current_process_mut().links.is_empty());
    assert_eq!(
        messages(&scheduler, 0),
        [
            Term::tuple(vec![
                atom("DOWN"),
                Term::Reference(monitor.unwrap()),
                atom("process"),
                Term::Pid(monitored),
                atom("normal"),
            ]),
            Term::tuple(vec![atom("EXIT"), Term::Pid(linked), atom("boom")]),
        ]
    );
    scheduler
        .finish_current(atom("normal"), Some(atom("ok")))
        .unwrap();
    assert!(scheduler.processes.is_empty());
    assert_eq!(root_result(&scheduler), Some(Ok(atom("ok"))));
}

#[test]
fn ends_the_run_only_after_the_exit_of_the_root_process_completes() {
    // With 100 or fewer reductions left, the exit switches out before it
    // leaves the process table. With 150 or fewer, it switches out before
    // its links (erl_process.c:14315-14316, 14541-14543). In both cases the
    // root result is recorded, but the run continues until the next turn
    // of the root process sends its link exit signal.
    for fcalls in [100, 150] {
        let mut scheduler = test_scheduler();
        scheduler.start(frame(), vec![], allow_all()).unwrap();
        scheduler.schedule_next().unwrap();
        let (child, _) = scheduler
            .spawn(
                Start::Entry(frame()),
                vec![],
                None,
                true,
                StartFlags::default(),
                None,
            )
            .unwrap()
            .unwrap();
        scheduler.current_process_mut().fcalls = fcalls;
        scheduler
            .finish_current(atom("normal"), Some(atom("ok")))
            .unwrap();
        assert!(!scheduler.node.is_shutdown());
        let signals =
            |scheduler: &Scheduler| scheduler.processes[&child].mailbox.has_control_signals();
        assert!(!signals(&scheduler));

        assert_eq!(scheduler.schedule_next().unwrap(), Turn::Run);
        assert_eq!(current(&mut scheduler), child);
        scheduler.yield_current();
        assert!(!scheduler.node.is_shutdown());
        assert_eq!(scheduler.schedule_next().unwrap(), Turn::Done);
        assert!(scheduler.node.is_shutdown());
        assert!(signals(&scheduler));
        assert_eq!(root_result(&scheduler), Some(Ok(atom("ok"))));
    }
}

#[test]
fn links_and_unlinks_processes_and_ignores_normal_exit_without_trap_exit() {
    let mut scheduler = test_scheduler();
    scheduler.start(frame(), vec![], allow_all()).unwrap();
    scheduler.schedule_next().unwrap();
    let (child, _) = scheduler
        .spawn(
            Start::Entry(frame()),
            vec![],
            None,
            false,
            StartFlags::default(),
            None,
        )
        .unwrap()
        .unwrap();
    assert!(scheduler.link(child));
    assert!(scheduler.link(child));
    assert!(scheduler.link(ProcessId(0)));
    assert!(!scheduler.link(ProcessId(99)));
    scheduler.unlink(child);
    let unlinking = scheduler
        .current_process_mut()
        .links
        .get(&LinkId::Process(child));
    assert!(unlinking.is_some_and(|&id| id != 0));
    scheduler.unlink(child);
    scheduler.unlink(ProcessId(99));
    scheduler.unlink(ProcessId(0));
    assert!(scheduler.link(child));
    assert_eq!(
        scheduler
            .current_process_mut()
            .links
            .get(&LinkId::Process(child)),
        Some(&0)
    );
    scheduler.park_current();

    // The child handles the link, the unlink, and the link again.
    assert_eq!(scheduler.schedule_next().unwrap(), Turn::Run);
    assert_eq!(
        scheduler
            .current_process_mut()
            .links
            .get(&LinkId::Process(ProcessId(0))),
        Some(&0)
    );
    scheduler.finish_current(atom("normal"), None).unwrap();

    // The root process handles the acknowledgment of the old unlink,
    // which leaves the new link, and then the normal exit signal, which
    // removes the link. It still waits.
    assert_eq!(scheduler.schedule_next().unwrap(), Turn::Done);
    assert_eq!(scheduler.schedule_next().unwrap(), Turn::Idle);
    let root = &scheduler.processes[&ProcessId(0)];
    assert!(root.links.is_empty());
    assert!(root.mailbox.messages().is_empty());
}

#[test]
fn ends_a_linked_process_and_rejects_counter_overflow() {
    let mut scheduler = test_scheduler();
    scheduler.start(frame(), vec![], allow_all()).unwrap();
    scheduler.schedule_next().unwrap();
    scheduler
        .spawn(
            Start::Entry(frame()),
            vec![],
            None,
            true,
            StartFlags::default(),
            None,
        )
        .unwrap()
        .unwrap();
    scheduler.park_current();
    scheduler.schedule_next().unwrap();
    scheduler.finish_current(atom("boom"), None).unwrap();
    // The link exit signal ends the root process, which does not trap
    // exits, with the reason of the child.
    assert_eq!(scheduler.schedule_next().unwrap(), Turn::Done);
    assert_eq!(root_result(&scheduler), Some(Err(atom("boom"))));

    let mut scheduler = test_scheduler();
    scheduler.start(frame(), vec![], allow_all()).unwrap();
    scheduler.schedule_next().unwrap();
    scheduler
        .finish_current(atom("boom"), Some(atom("ok")))
        .unwrap();
    assert_eq!(root_result(&scheduler), Some(Err(atom("boom"))));

    let mut scheduler = Scheduler {
        node: Node::exhausted(),
        ..test_scheduler()
    };
    // The root process has the fixed pid 0, so the overflow comes at the
    // first spawn.
    scheduler.start(frame(), vec![], allow_all()).unwrap();
    scheduler.schedule_next().unwrap();
    let spawn = scheduler.spawn(
        Start::Entry(frame()),
        vec![],
        None,
        false,
        StartFlags::default(),
        None,
    );
    assert!(spawn.is_err());
    assert!(scheduler.make_ref().is_err());
}

#[test]
fn sends_the_pending_monitor_signal_before_a_demonitor_signal() {
    // `proc_queue_signal` sends the pending monitor signal first when the
    // next signal goes to the same target (`erl_proc_sig_queue.c:1146-1170`).
    let mut scheduler = test_scheduler();
    scheduler.start(frame(), vec![], allow_all()).unwrap();
    scheduler.schedule_next().unwrap();
    let (child, _) = scheduler
        .spawn(
            Start::Entry(frame()),
            vec![],
            None,
            false,
            StartFlags::default(),
            None,
        )
        .unwrap()
        .unwrap();
    let reference = scheduler
        .monitor(MonitorTarget::Process(child), no_options())
        .unwrap();
    assert!(!scheduler.processes[&child].mailbox.has_incoming());
    assert!(scheduler.demonitor(reference));
    assert!(!scheduler.demonitor(reference));
    scheduler.park_current();

    assert_eq!(scheduler.schedule_next().unwrap(), Turn::Run);
    assert!(scheduler.current_process_mut().monitored_by.is_empty());
}

#[test]
fn creates_no_monitor_of_the_caller_and_sends_noproc_for_a_missing_target() {
    // ERTS 29.1.1: `monitor(process, self())` creates no monitor
    // (`bif.c:781-784`). A port name gives the noproc signal at once, and a
    // pid that no process has gives it when the pending signal goes, at the
    // switch out.
    let mut scheduler = test_scheduler();
    scheduler.start(frame(), vec![], allow_all()).unwrap();
    scheduler.schedule_next().unwrap();
    let own = scheduler
        .monitor(MonitorTarget::Process(ProcessId(0)), no_options())
        .unwrap();
    assert!(scheduler.processes[&ProcessId(0)].monitors.is_empty());
    assert!(!scheduler.demonitor(own));
    let gone = scheduler
        .monitor(MonitorTarget::Process(ProcessId(9)), no_options())
        .unwrap();
    let name = Atom::from_static("undefined").unwrap();
    let port = scheduler
        .monitor(MonitorTarget::PortName(name), no_options())
        .unwrap();
    scheduler.park_current();
    assert_eq!(scheduler.schedule_next().unwrap(), Turn::Run);
    let named = Term::tuple(vec![atom("undefined"), atom("nonode@nohost")]);
    assert_eq!(
        messages(&scheduler, 0),
        [
            Term::tuple(vec![
                atom("DOWN"),
                Term::Reference(port),
                atom("port"),
                named,
                atom("noproc"),
            ]),
            Term::tuple(vec![
                atom("DOWN"),
                Term::Reference(gone),
                atom("process"),
                Term::Pid(ProcessId(9)),
                atom("noproc"),
            ]),
        ]
    );
}

#[test]
fn frees_the_name_of_a_process_when_it_exits() {
    // ERTS 29.1.1: after a named process exits, register(Name, self())
    // in another process is true.
    let mut scheduler = test_scheduler();
    scheduler.start(frame(), vec![], allow_all()).unwrap();
    scheduler.schedule_next().unwrap();
    let (child, _) = scheduler
        .spawn(
            Start::Entry(frame()),
            vec![],
            None,
            false,
            StartFlags::default(),
            None,
        )
        .unwrap()
        .unwrap();
    let name = Term::Atom(scheduler.atoms.put("pbeam_probe_reg").unwrap());
    assert_eq!(scheduler.whereis("pbeam_probe_reg"), None);
    assert!(scheduler.register(&name, &Term::Pid(child)));
    assert!(!scheduler.register(&name, &Term::Pid(ProcessId(0))));
    assert_eq!(scheduler.whereis("pbeam_probe_reg"), Some(child));
    scheduler.park_current();

    scheduler.schedule_next().unwrap();
    scheduler.finish_current(atom("normal"), None).unwrap();
    assert_eq!(scheduler.whereis("pbeam_probe_reg"), None);
    assert!(scheduler.register(&name, &Term::Pid(ProcessId(0))));
    assert_eq!(scheduler.whereis("pbeam_probe_reg"), Some(ProcessId(0)));
    assert_eq!(
        scheduler.processes[&ProcessId(0)]
            .registered_name
            .clone()
            .map(Term::Atom),
        Some(name)
    );
}

#[test]
fn deletes_the_tables_of_a_process_when_it_exits() {
    // ERTS 29.1.1: after the owner of a named table exits, ets:new/2
    // with the same name in the monitor process is valid when the DOWN
    // message arrives (erl_process.c:14360-14368).
    let mut scheduler = test_scheduler();
    scheduler.start(frame(), vec![], allow_all()).unwrap();
    scheduler.schedule_next().unwrap();
    let (child, monitor) = scheduler
        .spawn(
            Start::Entry(frame()),
            vec![],
            Some(Term::static_atom("DOWN")),
            false,
            StartFlags::default(),
            None,
        )
        .unwrap()
        .unwrap();
    scheduler.park_current();

    scheduler.schedule_next().unwrap();
    let table = scheduler.make_ref().unwrap();
    scheduler.ets.exclusive().create(
        table,
        Some("ets_probe_owned"),
        Table::new(child, Access::Protected, 1, TableType::Set),
    );
    assert!(scheduler.ets.exclusive().name_in_use("ets_probe_owned"));
    scheduler.finish_current(atom("normal"), None).unwrap();

    assert!(!scheduler.ets.exclusive().name_in_use("ets_probe_owned"));
    assert_eq!(scheduler.schedule_next().unwrap(), Turn::Run);
    assert_eq!(
        messages(&scheduler, 0),
        [Term::tuple(vec![
            atom("DOWN"),
            Term::Reference(monitor.unwrap()),
            atom("process"),
            Term::Pid(child),
            atom("normal"),
        ])]
    );
}

#[test]
fn resumes_after_wait_timeout_when_the_timer_expires_after_a_message() {
    // ERTS 29.1.1: a process that a message woke and whose receive timer
    // then expires stays in the run queue once, and it continues at the
    // instruction after `wait_timeout` with `F_TIMO`
    // (`erl_hl_timer.c:692-706`, `erl_process.c:10451-10458`).
    let mut scheduler = test_scheduler();
    scheduler.start(frame(), vec![], allow_all()).unwrap();
    scheduler.schedule_next().unwrap();
    assert!(scheduler.set_receive_timer(1_000_000_000, &Term::Integer(100), 7));
    scheduler.park_current();
    scheduler.deliver(ProcessId(0), atom("early"));
    scheduler.expire_timers(1_099_999_999);
    assert!(matches!(
        scheduler.processes[&ProcessId(0)].receive_timer,
        ReceiveTimer::Set { resume: 7, .. }
    ));
    scheduler.expire_timers(1_100_000_000);
    assert_eq!(scheduler.runnable.ids(), vec![ProcessId(0)]);
    assert_eq!(
        scheduler.processes[&ProcessId(0)].receive_timer,
        ReceiveTimer::Expired { resume: 7 }
    );

    assert_eq!(scheduler.schedule_next().unwrap(), Turn::Run);
    let process = scheduler.current_process_mut();
    assert_eq!(process.frames[0].next_instruction, 7);
    assert_eq!(process.receive_timer, ReceiveTimer::Timo);
    scheduler.cancel_receive_timer();
    assert_eq!(
        scheduler.current_process_mut().receive_timer,
        ReceiveTimer::None
    );
}

#[test]
fn wakes_a_waiting_process_and_stops_receive_timers_at_cancel_and_exit() {
    let mut scheduler = test_scheduler();
    scheduler.start(frame(), vec![], allow_all()).unwrap();
    scheduler.schedule_next().unwrap();
    assert!(scheduler.set_receive_timer(1_000_000_000, &Term::Integer(1), 3));
    scheduler.park_current();
    scheduler.expire_timers(1_001_000_000);
    assert_eq!(scheduler.schedule_next().unwrap(), Turn::Run);

    assert!(scheduler.set_receive_timer(1_000_000_000, &Term::Integer(1), 3));
    scheduler.cancel_receive_timer();
    assert!(!scheduler.timers.active());
    assert!(scheduler.set_receive_timer(1_000_000_000, &Term::Integer(1), 3));
    scheduler.finish_current(atom("normal"), None).unwrap();
    assert!(!scheduler.timers.active());
}

#[test]
fn continues_the_exit_timer_walk_at_its_next_timer() {
    // Each turn cancels the timers that its reductions pay for and keeps
    // the position of the next timer (erl_rbtree.h:1392-1396). A turn that
    // walked the tree again from its start would cancel no new timer for
    // the reductions of each timer that an earlier turn cancelled.
    let mut scheduler = test_scheduler();
    scheduler.start(frame(), vec![], allow_all()).unwrap();
    scheduler.schedule_next().unwrap();
    let mut mirror = crate::runtime::process::MonitorLinkTree::default();
    for _ in 0..60 {
        let reference = scheduler.make_ref().unwrap();
        scheduler.timers.start(
            1100,
            Timer::Bif {
                reference,
                receiver: Receiver::Process(ProcessId(0)),
                message: atom("timeout"),
            },
        );
        scheduler
            .node
            .register_timer(reference, 0, 1100, Some(ProcessId(0)), false);
        mirror.insert(TimerReference(reference), ());
    }
    // The tree of the node has the shape of the mirror, which has the same
    // inserts.
    let order: Vec<TimerReference> = mirror.post_order().map(|(&t, _)| t).collect();
    let process = scheduler.current_process_mut();
    // Check that the exit cancelled the first `done` timers in post-order,
    // and keeps the position of the next timer.
    let check = |scheduler: &mut Scheduler, done: usize| {
        let process = &scheduler.processes[&ProcessId(0)];
        let mut walk = process.bif_timers.post_order();
        walk.by_ref().take(done).for_each(drop);
        assert_eq!(process.continue_exit, Some(walk.position()), "{done}");
        for timer in &order[..done] {
            assert_eq!(scheduler.timers.cancel_bif(timer.0), None, "{done}");
        }
    };
    // 100 reductions for the exit and 80 for each of two timers.
    process.fcalls = 260;
    scheduler.finish_current(atom("normal"), None).unwrap();
    check(&mut scheduler, 2);
    // A new turn has 4000 reductions, which pay for 50 timers.
    assert_eq!(scheduler.schedule_next().unwrap(), Turn::Done);
    check(&mut scheduler, 52);
    for timer in &order[52..] {
        assert!(scheduler.timers.cancel_bif(timer.0).is_some());
    }
    assert_eq!(scheduler.schedule_next().unwrap(), Turn::Done);
    assert!(scheduler.processes.is_empty());
}

#[test]
fn rejects_receive_timeouts_as_erts_set_proc_timer_term() {
    // `erts_set_proc_timer_term` (`erl_hl_timer.c:2786-2805`) accepts an
    // integer from 0 to 4294967295 whose end is not after
    // `ERTS_MONOTONIC_END`.
    let mut scheduler = test_scheduler();
    scheduler.start(frame(), vec![], allow_all()).unwrap();
    scheduler.schedule_next().unwrap();
    for timeout in [Term::Integer(-1), Term::Integer(4_294_967_296), atom("foo")] {
        assert!(!scheduler.set_receive_timer(1_000_000_000, &timeout, 0));
    }
    let end = 9_223_372_036_000_000_000;
    assert!(!scheduler.set_receive_timer(end, &Term::Integer(1), 0));
    assert!(scheduler.set_receive_timer(end - 1_000_000, &Term::Integer(1), 0));
    assert_eq!(
        scheduler.current_process_mut().receive_timer,
        // Start number 0 is the time correction check timer.
        ReceiveTimer::Set {
            key: (9_223_372_036_000, 1),
            resume: 0
        }
    );
}

/// Return a write of `bytes` to `fd` in one group.
fn dirty_write(fd: i32, bytes: &[u8]) -> IovecWrite {
    IovecWrite {
        fd,
        groups: vec![vec![bytes.to_vec()]],
        bad_group: false,
    }
}

/// Start the dirty IO call `write` of the current process, as a call of
/// `prim_tty:write_nif/2` does.
fn start_write(scheduler: &mut Scheduler, write: IovecWrite) {
    let entry = ("prim_tty".to_owned(), "write_nif".to_owned(), Vec::new());
    scheduler
        .start_dirty_call(Job::IoBound, Work::Write(write), entry)
        .unwrap();
}

/// Wait until a dirty IO call completes, then take the completions.
fn wait_completion(scheduler: &mut Scheduler) {
    assert!(
        scheduler
            .wait_events(Some(Duration::from_secs(10)))
            .unwrap()
    );
    scheduler.complete_dirty_calls();
}

#[test]
fn gives_the_invariant_error_of_a_dirty_call_with_the_job_normal() {
    // A `Normal` job has no dirty pool, so the switch out fails and the
    // process does not record a call.
    let mut scheduler = test_scheduler();
    scheduler.start(frame(), vec![], allow_all()).unwrap();
    assert_eq!(scheduler.schedule_next().unwrap(), Turn::Run);
    let entry = ("m".to_owned(), "f".to_owned(), Vec::new());
    let write = dirty_write(-1, b"");
    let error = scheduler
        .start_dirty_call(Job::Normal, Work::Write(write), entry)
        .unwrap_err();
    assert_eq!(
        error.to_string(),
        "invariant error: a NIF with the job Normal switches out to a dirty pool"
    );
    assert!(scheduler.processes[&ProcessId(0)].dirty_call.is_none());
    assert!(!scheduler.dirty.running());
}

#[test]
fn resumes_a_dirty_write_once_when_it_completes_before_the_next_poll() {
    // The scheduler records the call on the process before the dirty IO
    // thread gets it. The completion is in the inbox before the scheduler
    // takes it; the scheduler resumes the process once.
    let (mut reader, writer) = std::io::pipe().unwrap();
    let mut scheduler = test_scheduler();
    scheduler.start(frame(), vec![], allow_all()).unwrap();
    assert_eq!(scheduler.schedule_next().unwrap(), Turn::Run);
    start_write(&mut scheduler, dirty_write(writer.as_raw_fd(), b"hi\n"));
    let process = &scheduler.processes[&ProcessId(0)];
    assert_eq!(
        process
            .dirty_call
            .as_ref()
            .map(|call| (call.call, call.output.is_none())),
        Some((0, true))
    );
    assert!(!process.in_run_queue && !process.waiting);
    assert_eq!(scheduler.schedule_next().unwrap(), Turn::Idle);
    let mut bytes = [0; 3];
    reader.read_exact(&mut bytes).unwrap();
    assert_eq!(&bytes, b"hi\n");
    wait_completion(&mut scheduler);
    assert_eq!(scheduler.runnable.ids(), [ProcessId(0)]);
    assert_eq!(
        scheduler.processes[&ProcessId(0)]
            .dirty_call
            .as_ref()
            .and_then(|call| match &call.output {
                Some(Output::Written(written)) => Some(written),
                _ => None,
            }),
        Some(&IovecWritten::All)
    );
    // The inbox is empty, and the wait does not wait for the call.
    scheduler.complete_dirty_calls();
    assert_eq!(scheduler.runnable.ids(), [ProcessId(0)]);
    assert!(!scheduler.wait_events(None).unwrap());
    assert_eq!(scheduler.schedule_next().unwrap(), Turn::Run);
}

#[test]
fn runs_other_processes_while_a_dirty_write_waits_and_resumes_it_after() {
    // The completion comes after the scheduler took the inbox and ran
    // another process. A message to the waiting process does not put it
    // in the run queue, and it gets the message after the call.
    let (reader, writer) = std::io::pipe().unwrap();
    let fd = writer.as_raw_fd();
    // SAFETY: `fcntl` changes only the status flags of an open fd.
    let flags = unsafe { libc::fcntl(fd, libc::F_GETFL) };
    // SAFETY: as above.
    unsafe { libc::fcntl(fd, libc::F_SETFL, flags | libc::O_NONBLOCK) };
    while (&writer).write(&[0; 4096]).is_ok() {}
    // SAFETY: as above.
    unsafe { libc::fcntl(fd, libc::F_SETFL, flags) };
    let mut scheduler = test_scheduler();
    scheduler.start(frame(), vec![], allow_all()).unwrap();
    assert_eq!(scheduler.schedule_next().unwrap(), Turn::Run);
    scheduler
        .spawn(
            Start::Entry(frame()),
            vec![],
            None,
            false,
            StartFlags::default(),
            None,
        )
        .unwrap()
        .unwrap();
    start_write(&mut scheduler, dirty_write(fd, b"later"));
    scheduler.complete_dirty_calls();
    assert_eq!(scheduler.schedule_next().unwrap(), Turn::Run);
    assert_eq!(current(&mut scheduler), ProcessId(1));
    scheduler.deliver(ProcessId(0), atom("ping"));
    scheduler.park_current();
    assert_eq!(scheduler.schedule_next().unwrap(), Turn::Idle);
    assert!(
        scheduler
            .wait_events(Some(Duration::from_millis(20)))
            .unwrap()
    );
    scheduler.complete_dirty_calls();
    assert!(scheduler.runnable.ids().is_empty());
    let reading = std::thread::spawn(move || {
        let mut bytes = Vec::new();
        (&reader).read_to_end(&mut bytes).unwrap();
        bytes
    });
    wait_completion(&mut scheduler);
    drop(writer);
    assert!(reading.join().unwrap().ends_with(b"later"));
    assert_eq!(scheduler.runnable.ids(), [ProcessId(0)]);
    assert_eq!(scheduler.schedule_next().unwrap(), Turn::Run);
    assert_eq!(current(&mut scheduler), ProcessId(0));
    assert_eq!(messages(&scheduler, 0), [atom("ping")]);
}

/// Return a pipe whose write end holds as many bytes as the pipe takes, so
/// a blocking write to it waits until the test reads the pipe.
fn full_pipe() -> (PipeReader, PipeWriter) {
    let (reader, writer) = std::io::pipe().unwrap();
    let fd = writer.as_raw_fd();
    // SAFETY: `fcntl` changes only the status flags of an open fd.
    let flags = unsafe { libc::fcntl(fd, libc::F_GETFL) };
    // SAFETY: as above.
    unsafe { libc::fcntl(fd, libc::F_SETFL, flags | libc::O_NONBLOCK) };
    while (&writer).write(&[0; 4096]).is_ok() {}
    while (&writer).write(&[0]).is_ok() {}
    // SAFETY: as above.
    unsafe { libc::fcntl(fd, libc::F_SETFL, flags) };
    (reader, writer)
}

/// Read `reader` until the end of file in a new thread, so each write to
/// the pipe completes.
fn drain(reader: PipeReader) -> std::thread::JoinHandle<Vec<u8>> {
    std::thread::spawn(move || {
        let mut bytes = Vec::new();
        (&reader).read_to_end(&mut bytes).unwrap();
        bytes
    })
}

/// Take the completions until no dirty IO call runs.
fn wait_all_completions(scheduler: &mut Scheduler) {
    while scheduler.dirty.running() {
        wait_completion(scheduler);
    }
}

/// Return the exit signal `kill` of `erlang:exit/2` from process 0.
fn kill() -> Signal {
    Signal::Exit {
        from: LinkId::Process(ProcessId(0)),
        reason: atom("kill"),
        linked: false,
        normal_kills: false,
        alias: None,
    }
}

/// Start the root process 0, which spawns the processes `1..=count` with a
/// monitor and parks. Each spawned process then starts a dirty write of
/// `bytes` to the fd that `fd` gives for its number. Return the monitor
/// references.
fn start_dirty_writers(
    scheduler: &mut Scheduler,
    count: u64,
    fd: impl Fn(u64) -> i32,
    bytes: &[u8],
) -> Vec<Reference> {
    scheduler.start(frame(), vec![], allow_all()).unwrap();
    assert_eq!(scheduler.schedule_next().unwrap(), Turn::Run);
    let references = (1..=count)
        .map(|_| {
            let (_, reference) = scheduler
                .spawn(
                    Start::Entry(frame()),
                    vec![],
                    Some(Term::static_atom("DOWN")),
                    false,
                    StartFlags::default(),
                    None,
                )
                .unwrap()
                .unwrap();
            reference.unwrap()
        })
        .collect();
    scheduler.park_current();
    for id in 1..=count {
        assert_eq!(scheduler.schedule_next().unwrap(), Turn::Run);
        assert_eq!(current(scheduler), ProcessId(id));
        start_write(scheduler, dirty_write(fd(id), bytes));
    }
    references
}

/// Return the `DOWN` message of the monitor `reference` on process `id`
/// with `reason`.
fn down(reference: Reference, id: u64, reason: &str) -> Term {
    Term::tuple(vec![
        atom("DOWN"),
        Term::Reference(reference),
        atom("process"),
        Term::Pid(ProcessId(id)),
        atom(reason),
    ])
}

/// The bytes of a write that waits: a pipe holds 64 KiB.
const WAITING_WRITE: usize = 1 << 20;

/// Wait until `reader` has bytes: the dirty IO thread started the write to
/// the empty pipe.
fn wait_write_started(reader: &PipeReader) {
    assert_eq!(
        wait_readable(&[reader.as_raw_fd()], Some(Duration::from_secs(10))),
        Readiness::Ready(vec![reader.as_raw_fd()])
    );
}

#[test]
fn handles_the_signals_of_a_dirty_writer_and_drops_its_late_completion() {
    // ERTS 29.1.1 handles the signals of a process that runs dirty with
    // `erts_dirty_process_signal_handler`, and a normal scheduler runs the
    // exit of a killed process while its dirty NIF runs. The completion of
    // the write then arrives after the exit, and the scheduler drops it.
    let (reader, writer) = std::io::pipe().unwrap();
    let mut scheduler = test_scheduler();
    let bytes = vec![b'w'; WAITING_WRITE];
    let references = start_dirty_writers(&mut scheduler, 1, |_| writer.as_raw_fd(), &bytes);
    wait_write_started(&reader);
    assert_eq!(scheduler.schedule_next().unwrap(), Turn::Idle);
    // A message does not put the writer in the run queue.
    scheduler.deliver(ProcessId(1), atom("ping"));
    assert!(scheduler.runnable.ids().is_empty());
    // A signal does: the turn handles it, and the writer does not run.
    scheduler
        .send_signal(Some(ProcessId(0)), ProcessId(1), kill())
        .unwrap();
    assert_eq!(scheduler.runnable.ids(), [ProcessId(1)]);
    let reductions = scheduler.processes[&ProcessId(1)].reductions;
    assert_eq!(scheduler.schedule_next().unwrap(), Turn::Done);
    let writer_state = &scheduler.processes[&ProcessId(1)];
    assert_eq!(writer_state.exiting, Some(atom("killed")));
    assert!(writer_state.dirty_call.is_none());
    assert_eq!(writer_state.reductions, reductions);
    // The exit runs in the next turn, while the write waits.
    assert_eq!(scheduler.runnable.ids(), [ProcessId(1)]);
    assert_eq!(scheduler.schedule_next().unwrap(), Turn::Done);
    assert!(!scheduler.processes.contains_key(&ProcessId(1)));
    assert_eq!(scheduler.schedule_next().unwrap(), Turn::Run);
    assert_eq!(current(&mut scheduler), ProcessId(0));
    assert_eq!(messages(&scheduler, 0), [down(references[0], 1, "killed")]);
    scheduler.park_current();
    // The write continues and completes; its completion does not revive
    // the writer or wake another process.
    assert!(scheduler.dirty.running());
    let reading = drain(reader);
    wait_all_completions(&mut scheduler);
    drop(writer);
    assert_eq!(reading.join().unwrap(), bytes);
    assert!(scheduler.runnable.ids().is_empty());
    assert!(!scheduler.processes.contains_key(&ProcessId(1)));
    assert_eq!(scheduler.schedule_next().unwrap(), Turn::Idle);
}

#[test]
fn drops_the_completion_that_arrives_before_the_exit_of_a_killed_dirty_writer() {
    // The writer handles the kill while its write waits, and the write
    // completes before the exit turn of the writer. The completion does not
    // resume the exiting writer, and the writer exits without its result.
    let (reader, writer) = std::io::pipe().unwrap();
    let mut scheduler = test_scheduler();
    let bytes = vec![b'w'; WAITING_WRITE];
    let references = start_dirty_writers(&mut scheduler, 1, |_| writer.as_raw_fd(), &bytes);
    wait_write_started(&reader);
    scheduler
        .send_signal(Some(ProcessId(0)), ProcessId(1), kill())
        .unwrap();
    assert_eq!(scheduler.schedule_next().unwrap(), Turn::Done);
    let reading = drain(reader);
    wait_all_completions(&mut scheduler);
    drop(writer);
    assert_eq!(reading.join().unwrap(), bytes);
    assert_eq!(scheduler.runnable.ids(), [ProcessId(1)]);
    assert!(scheduler.processes[&ProcessId(1)].dirty_call.is_none());
    assert_eq!(scheduler.schedule_next().unwrap(), Turn::Done);
    assert!(!scheduler.processes.contains_key(&ProcessId(1)));
    assert_eq!(scheduler.schedule_next().unwrap(), Turn::Run);
    assert_eq!(messages(&scheduler, 0), [down(references[0], 1, "killed")]);
}

#[test]
fn exits_without_the_result_of_a_dirty_write_that_completed_before_a_kill() {
    // The completion puts the writer in the run queue with its result. A
    // kill then arrives before its turn: the turn handles the kill at
    // schedule-in, and the writer exits without its result.
    let (mut reader, writer) = std::io::pipe().unwrap();
    let mut scheduler = test_scheduler();
    let references = start_dirty_writers(&mut scheduler, 1, |_| writer.as_raw_fd(), b"done");
    let mut bytes = [0; 4];
    reader.read_exact(&mut bytes).unwrap();
    wait_all_completions(&mut scheduler);
    assert_eq!(scheduler.runnable.ids(), [ProcessId(1)]);
    scheduler
        .send_signal(Some(ProcessId(0)), ProcessId(1), kill())
        .unwrap();
    assert_eq!(scheduler.runnable.ids(), [ProcessId(1)]);
    assert_eq!(scheduler.schedule_next().unwrap(), Turn::Done);
    assert!(!scheduler.processes.contains_key(&ProcessId(1)));
    assert_eq!(scheduler.schedule_next().unwrap(), Turn::Run);
    assert_eq!(messages(&scheduler, 0), [down(references[0], 1, "killed")]);
}

#[test]
fn does_not_run_the_queued_dirty_write_of_a_killed_writer() {
    // Ten writes wait for a full pipe on the ten dirty IO threads, and the
    // write of process 11 waits in the queue. The kill of process 11 drops
    // its claim when its turn handles the kill, so no thread runs its
    // write, also when the threads take it before its exit turn. ERTS
    // 29.1.1: a dirty IO scheduler does not call the NIF of an exiting
    // process (erl_process.c:10252-10258).
    let (reader, writer) = full_pipe();
    let (queued_reader, queued_writer) = std::io::pipe().unwrap();
    let mut scheduler = test_scheduler();
    let fd = |id| {
        if id == 11 {
            queued_writer.as_raw_fd()
        } else {
            writer.as_raw_fd()
        }
    };
    start_dirty_writers(&mut scheduler, 11, fd, b"w");
    scheduler
        .send_signal(Some(ProcessId(0)), ProcessId(11), kill())
        .unwrap();
    assert_eq!(scheduler.schedule_next().unwrap(), Turn::Done);
    let reading = drain(reader);
    wait_all_completions(&mut scheduler);
    drop(writer);
    assert!(reading.join().unwrap().ends_with(b"wwwwwwwwww"));
    assert_eq!(
        wait_readable(&[queued_reader.as_raw_fd()], Some(Duration::ZERO)),
        Readiness::Pending
    );
    let mut runnable = scheduler.runnable.ids();
    runnable.sort();
    assert_eq!(runnable, (1..=11).map(ProcessId).collect::<Vec<_>>());
}

#[test]
fn sends_a_message_that_shares_the_term_of_the_sender() {
    // Owner decision R3 of #450: a send shares a term, and the receiver
    // cannot see the difference (`docs/copy-on-write-terms.md`). ERTS
    // copies the message to the heap of the receiver and expands shared
    // subterms (`erl_message.c:834-861`), and a send to the process itself
    // with the message queue on the heap does not copy (`:824-826`). PBeam
    // reports no value that depends on the copy (row "Message copy" of
    // docs/erts-differences.md).
    let mut scheduler = test_scheduler();
    scheduler.start(frame(), vec![], allow_all()).unwrap();
    let message = Term::tuple(vec![Term::Integer(1), Term::tuple(vec![Term::Integer(2)])]);
    // The interpreter copies the operand of `erlang:send/2` first, and the
    // copy is shallow. `Scheduler::send_message` is the delivery step that
    // `deliver_to` of `erlang:send/2` calls for a local pid.
    scheduler.send_message(ProcessId(0), ProcessId(0), message.clone());
    let received = messages(&scheduler, 0);
    assert_eq!(received.len(), 1);
    assert!(message.shares_storage_with(&received[0]));
    // A term of another kind does not share its storage.
    assert!(!Term::Integer(1).shares_storage_with(&Term::Integer(1)));
}

/// Return a term of one integer, for a test that tells the terms of the
/// release work list apart.
///
/// # Examples
///
/// ```rust,ignore
/// assert_eq!(marker(1), Term::tuple(vec![Term::Integer(1)]));
/// ```
fn marker(number: i64) -> Term {
    Term::tuple(vec![Term::Integer(number)])
}

/// Release each item of the release work list of `scheduler`, and check that
/// the list ends empty. A test uses it to finish a bounded release.
///
/// # Examples
///
/// ```rust,ignore
/// release_all(&mut test_scheduler());
/// ```
fn release_all(scheduler: &mut Scheduler) {
    while !scheduler.release.is_empty() {
        scheduler.drain_release(RELEASE_BATCH);
    }
}

#[test]
fn moves_the_containers_of_an_exiting_process_to_the_release_work_list() {
    // Row 11 of docs/term-performance-design-codex-v2.md: the exit moves the
    // term-holding containers of a process to the release work list as
    // containers, so the exit step walks no dictionary entry, mailbox node,
    // frame, or monitor. Each container reaches the list.
    let mut scheduler = test_scheduler();
    scheduler
        .start(frame(), vec![Term::Integer(1)], allow_all())
        .unwrap();
    let id = ProcessId(0);
    {
        let process = scheduler.processes.get_mut(&id).unwrap();
        process.frames[0].stack = vec![StackSlot::Term(marker(2))];
        process.mailbox.push(marker(3));
        process.dictionary.put(Term::Integer(4), marker(5));
        process.pending_bif = Some(PendingBif {
            module: "erlang",
            function: "length",
            arguments: vec![marker(6)],
            tail: false,
        });
        process.length_trap = Some(vec![marker(7)]);
        process.monitors.insert(
            Reference(1),
            Origin {
                monitor: Some(OriginMonitor {
                    kind: "process",
                    target: None,
                    item: marker(8),
                    tag: marker(9),
                }),
                alias: None,
            },
        );
    }
    assert_eq!(scheduler.schedule_next().unwrap(), Turn::Run);
    scheduler.finish_current(atom("normal"), None).unwrap();
    let items: Vec<&Release> = scheduler.release.iter().collect();
    assert_eq!(items.len(), 8, "the exit moves eight containers");
    assert!(matches!(items[0], Release::Terms(_)));
    assert!(matches!(items[1], Release::Terms(_)));
    assert!(matches!(items[2], Release::Term(reason) if *reason == atom("normal")));
    assert!(matches!(items[3], Release::Registers(_)));
    assert!(matches!(items[4], Release::Frames(_)));
    assert!(matches!(items[5], Release::Mailbox(_)));
    assert!(matches!(items[6], Release::Dictionary(_)));
    assert!(matches!(items[7], Release::Monitors(_)));
    release_all(&mut scheduler);
    assert!(scheduler.release.is_empty());
}

#[test]
fn releases_the_terms_of_every_container_of_an_exiting_process() {
    // The drain releases the terms of each container that the exit moved to
    // the release work list. The test keeps one holder of each term, so a
    // value that the release lost shows as a changed value, and the run ends
    // with no item on the list.
    let mut scheduler = test_scheduler();
    scheduler.start(frame(), vec![], allow_all()).unwrap();
    let id = ProcessId(0);
    let register = marker(1);
    let stack = marker(2);
    let message = marker(3);
    let dictionary = marker(4);
    let pending = marker(5);
    let length_trap = marker(6);
    let item = marker(7);
    let tag = marker(8);
    {
        let process = scheduler.processes.get_mut(&id).unwrap();
        process.registers = vec![Some(register.clone())];
        process.frames[0].stack = vec![
            StackSlot::Empty,
            StackSlot::Catch(1),
            StackSlot::Term(stack.clone()),
        ];
        process.mailbox.push(message.clone());
        process.dictionary.put(Term::Integer(9), dictionary.clone());
        process.pending_bif = Some(PendingBif {
            module: "erlang",
            function: "length",
            arguments: vec![pending.clone()],
            tail: false,
        });
        process.length_trap = Some(vec![length_trap.clone()]);
        process.monitors.insert(
            Reference(1),
            Origin {
                monitor: Some(OriginMonitor {
                    kind: "process",
                    target: None,
                    item: item.clone(),
                    tag: tag.clone(),
                }),
                alias: None,
            },
        );
        // An alias without a monitor holds no term, so the reader skips it.
        process.monitors.insert(
            Reference(2),
            Origin {
                monitor: None,
                alias: None,
            },
        );
    }
    assert_eq!(scheduler.schedule_next().unwrap(), Turn::Run);
    scheduler.finish_current(atom("normal"), None).unwrap();
    release_all(&mut scheduler);
    assert!(
        scheduler.release.is_empty(),
        "the drain releases every container"
    );
    for (term, expected) in [
        (register, 1),
        (stack, 2),
        (message, 3),
        (dictionary, 4),
        (pending, 5),
        (length_trap, 6),
        (item, 7),
        (tag, 8),
    ] {
        assert_eq!(term, marker(expected), "the value waits for the release");
    }
}

#[test]
fn releases_the_terms_of_an_exiting_process_at_each_later_step() {
    // A step releases one batch of the release work, also when no process
    // runs, and that step reports `Done` and not `Idle`, so that the run loop
    // keeps stepping and the release completes. The X registers hold the 512
    // terms of the start arguments.
    let mut scheduler = test_scheduler();
    let terms: Vec<Term> = (0..2 * RELEASE_BATCH as i64).map(Term::Integer).collect();
    scheduler.start(frame(), terms, allow_all()).unwrap();
    assert_eq!(scheduler.schedule_next().unwrap(), Turn::Run);
    scheduler.finish_current(atom("normal"), None).unwrap();
    assert!(matches!(
        scheduler.release.iter().last(),
        Some(Release::Monitors(_))
    ));
    assert_eq!(scheduler.schedule_next().unwrap(), Turn::Done);
    assert!(
        !scheduler.release.is_empty(),
        "one batch of the registers is left"
    );
    release_all(&mut scheduler);
    assert!(scheduler.release.is_empty());
    assert_eq!(scheduler.schedule_next().unwrap(), Turn::Idle);
}

#[test]
fn keeps_the_shared_child_of_a_released_graph_alive() {
    // The release work list owns each child root. A child that another holder
    // still owns keeps its value across the release of the graph that the
    // process held.
    let mut scheduler = test_scheduler();
    let shared = marker(11);
    let held = shared.clone();
    let graph = Term::list_from([shared.clone(), shared.clone()]);
    drop(shared);
    scheduler.start(frame(), vec![graph], allow_all()).unwrap();
    assert_eq!(scheduler.schedule_next().unwrap(), Turn::Run);
    scheduler.finish_current(atom("normal"), None).unwrap();
    release_all(&mut scheduler);
    assert_eq!(held, marker(11), "the shared child keeps its value");
}

#[test]
fn moves_the_pairs_of_a_map_in_batches() {
    // The pairs of a map move through a drain that one step advances by at
    // most one batch, so the map of 200 keys waits on the list after a step.
    let mut scheduler = test_scheduler();
    let map = Term::Map(
        Map::from_pairs(
            (1..=200)
                .map(|key| (Term::Integer(key), Term::Integer(key)))
                .collect(),
            &crate::term::test_atom_table(),
        )
        .unwrap(),
    );
    scheduler.start(frame(), vec![map], allow_all()).unwrap();
    assert_eq!(scheduler.schedule_next().unwrap(), Turn::Run);
    scheduler.finish_current(atom("normal"), None).unwrap();
    scheduler.drain_release(RELEASE_BATCH);
    assert!(
        scheduler
            .release
            .iter()
            .any(|item| matches!(item, Release::Map(_))),
        "one step leaves part of the map on the list"
    );
    release_all(&mut scheduler);
    assert!(scheduler.release.is_empty());
}

#[test]
fn releases_a_deep_chain_through_the_batches_without_a_stack_overflow() {
    // The walk of the release work list uses no Rust recursion, so the
    // release of a deep chain does not overflow the stack.
    let mut scheduler = test_scheduler();
    let deep = (0..100_000).fold(Term::Nil, |inner, _| Term::tuple(vec![inner]));
    scheduler.start(frame(), vec![deep], allow_all()).unwrap();
    assert_eq!(scheduler.schedule_next().unwrap(), Turn::Run);
    scheduler.finish_current(atom("normal"), None).unwrap();
    release_all(&mut scheduler);
    assert!(scheduler.release.is_empty());
}

#[test]
fn drops_a_live_process_that_holds_a_deep_chain_without_a_stack_overflow() {
    // The shutdown of a run releases the state of a live process through the
    // iterative drop walk of `Term::drop`, which does not recurse on a deep
    // term. No process waits to run at the shutdown.
    let mut scheduler = test_scheduler();
    let deep = (0..100_000).fold(Term::Nil, |inner, _| Term::tuple(vec![inner]));
    scheduler.start(frame(), vec![deep], allow_all()).unwrap();
    drop(scheduler);
}

#[test]
fn moves_the_array_of_a_wide_tuple_in_batches() {
    // One node holds more direct children than one batch. The exit moves the
    // term to the release work list without taking its array, one step takes
    // the array as one group and moves at most one batch of the group, and
    // the rest waits for a later step. A step that moved the whole array
    // would leave no group.
    let mut scheduler = test_scheduler();
    let width = 4 * RELEASE_BATCH;
    let wide = Term::tuple((0..width as i64).map(Term::Integer).collect());
    scheduler.start(frame(), vec![wide], allow_all()).unwrap();
    assert_eq!(scheduler.schedule_next().unwrap(), Turn::Run);
    scheduler.finish_current(atom("normal"), None).unwrap();
    assert!(
        scheduler
            .release
            .iter()
            .all(|item| !matches!(item, Release::Terms(_))),
        "the exit takes no element of the array"
    );
    scheduler.drain_release(RELEASE_BATCH);
    let group = scheduler
        .release
        .iter()
        .find_map(|item| match item {
            Release::Terms(group) => Some(group.len()),
            _ => None,
        })
        .expect("the array of the tuple waits as one group");
    assert!(
        group > width - RELEASE_BATCH,
        "one step moves at most one batch of the group: {group} of {width} wait"
    );
    release_all(&mut scheduler);
    assert!(scheduler.release.is_empty());
}

#[test]
fn moves_the_free_variables_of_a_fun_in_batches() {
    // The free variables of a fun are one group, as the elements of a tuple
    // are, so one step moves at most one batch of them.
    let mut scheduler = test_scheduler();
    let module = Atom::from_static("lists").unwrap();
    let width = 4 * RELEASE_BATCH;
    let free: Vec<Term> = (0..width as i64).map(Term::Integer).collect();
    let fun = Term::Fun(Fun::new(Lambda::new(module, 0, 7, 53, 1), free));
    scheduler.start(frame(), vec![fun], allow_all()).unwrap();
    assert_eq!(scheduler.schedule_next().unwrap(), Turn::Run);
    scheduler.finish_current(atom("normal"), None).unwrap();
    scheduler.drain_release(RELEASE_BATCH);
    let group = scheduler
        .release
        .iter()
        .find_map(|item| match item {
            Release::Terms(group) => Some(group.len()),
            _ => None,
        })
        .expect("the free variables wait as one group");
    assert!(
        group > width - RELEASE_BATCH,
        "one step moves at most one batch of the group: {group} of {width} wait"
    );
    release_all(&mut scheduler);
    assert!(scheduler.release.is_empty());
}

#[test]
fn releases_the_empty_containers_of_a_process_that_has_no_frame() {
    // A process that an undefined start made has no frame, so its release
    // work holds an empty frame list and an empty monitor tree, and the drain
    // drops them.
    let mut scheduler = test_scheduler();
    let module = Atom::from_static("erlang").unwrap();
    let function = Atom::from_static("ok").unwrap();
    let process = ProcessState::new(
        ProcessId(0),
        Start::Undefined { module, function },
        Vec::new(),
    );
    scheduler.insert(process);
    assert_eq!(scheduler.schedule_next().unwrap(), Turn::Run);
    scheduler.finish_current(atom("normal"), None).unwrap();
    assert!(matches!(
        scheduler.release.iter().last(),
        Some(Release::Monitors(_))
    ));
    release_all(&mut scheduler);
    assert!(scheduler.release.is_empty());
}

#[test]
fn moves_the_slots_of_one_frame_in_batches() {
    // One frame holds more Y registers than one batch. The exit moves the
    // frame to the list as a frame, one step takes its slots as one group and
    // moves at most one batch of them, and the rest waits for a later step.
    let mut scheduler = test_scheduler();
    scheduler.start(frame(), Vec::new(), allow_all()).unwrap();
    let id = ProcessId(0);
    {
        let process = scheduler.processes.get_mut(&id).unwrap();
        process.frames[0].stack = (0..4 * RELEASE_BATCH)
            .map(|number| StackSlot::Term(marker(number as i64)))
            .collect();
    }
    assert_eq!(scheduler.schedule_next().unwrap(), Turn::Run);
    scheduler.finish_current(atom("normal"), None).unwrap();
    scheduler.drain_release(RELEASE_BATCH);
    assert!(
        scheduler
            .release
            .iter()
            .any(|item| matches!(item, Release::Slots(_))),
        "one step leaves part of the Y registers on the list"
    );
    release_all(&mut scheduler);
    assert!(scheduler.release.is_empty());
}

#[test]
fn moves_the_monitors_of_an_exiting_process_in_batches() {
    // The monitor tree of an exited process goes to the release work list as
    // a tree, and one step takes at most one batch of its values.
    let mut scheduler = test_scheduler();
    scheduler.start(frame(), Vec::new(), allow_all()).unwrap();
    let id = ProcessId(0);
    {
        let process = scheduler.processes.get_mut(&id).unwrap();
        for reference in 1..=2 {
            process.monitors.insert(
                Reference(reference),
                Origin {
                    monitor: Some(OriginMonitor {
                        kind: "process",
                        target: None,
                        item: marker(reference as i64),
                        tag: marker(reference as i64),
                    }),
                    alias: None,
                },
            );
        }
    }
    assert_eq!(scheduler.schedule_next().unwrap(), Turn::Run);
    scheduler.finish_current(atom("normal"), None).unwrap();
    scheduler.drain_release(1);
    assert!(
        scheduler
            .release
            .iter()
            .any(|item| matches!(item, Release::Monitors(_))),
        "one step of one unit leaves a monitor on the list"
    );
    release_all(&mut scheduler);
    assert!(scheduler.release.is_empty());
}

#[test]
fn moves_the_terms_of_a_discarded_signal_to_the_release_work_list() {
    // The exit takes each queued signal out of the mailbox and ignores it.
    // The terms of that signal go to the release work list before the signal
    // drops, so an ignored signal that owns the last root of a large graph
    // does not release that graph in the exit step.
    let mut scheduler = test_scheduler();
    let expected = Term::list_from((0..4 * RELEASE_BATCH as i64).map(Term::Integer));
    scheduler.start(frame(), Vec::new(), allow_all()).unwrap();
    assert_eq!(scheduler.schedule_next().unwrap(), Turn::Run);
    // The signal arrives after the process took its turn, so the exit sees it
    // in the mailbox and discards it.
    scheduler
        .processes
        .get_mut(&ProcessId(0))
        .unwrap()
        .mailbox
        .push_signal(Signal::Exit {
            from: LinkId::Process(ProcessId(1)),
            reason: expected.clone(),
            linked: false,
            normal_kills: false,
            alias: None,
        });
    scheduler.finish_current(atom("normal"), None).unwrap();
    assert!(
        scheduler
            .release
            .iter()
            .any(|item| matches!(item, Release::Term(term) if *term == expected)),
        "the exit moves the reason of the discarded signal to the list"
    );
    release_all(&mut scheduler);
    assert!(scheduler.release.is_empty());
}

#[test]
fn keeps_the_pending_work_of_a_skewed_chain_in_chunks() {
    // A chain whose every node holds one integer beside the rest of the chain
    // leaves one pending item on the release work list for each level that
    // the drain passed, so the list grows past one chunk. The list allocates
    // one chunk at a time and copies no item that a step of the drain holds.
    let mut scheduler = test_scheduler();
    let chain = (0..4 * RELEASE_CHUNK).fold(Term::Nil, |inner, _| {
        Term::tuple(vec![Term::Integer(1), inner])
    });
    scheduler.start(frame(), vec![chain], allow_all()).unwrap();
    assert_eq!(scheduler.schedule_next().unwrap(), Turn::Run);
    scheduler.finish_current(atom("normal"), None).unwrap();
    let mut longest = 0;
    while !scheduler.release.is_empty() {
        longest = longest.max(scheduler.release.len());
        scheduler.drain_release(RELEASE_BATCH);
    }
    assert!(
        longest > RELEASE_CHUNK,
        "the pending siblings fill more than one chunk: {longest}"
    );
}

#[test]
fn reports_the_live_bytes_of_the_scheduler_state() {
    let mut scheduler = test_scheduler();
    assert_eq!(scheduler.live_bytes(), 0);
    scheduler
        .start(
            frame(),
            vec![Term::tuple(vec![Term::Integer(1)])],
            Vec::new(),
        )
        .unwrap();
    assert!(scheduler.live_bytes() > 0);
}

#[test]
fn counts_the_exit_reason_of_a_freed_process() {
    let mut scheduler = test_scheduler();
    scheduler.start(frame(), vec![], Vec::new()).unwrap();
    let mut process = scheduler.processes.remove(&ProcessId(0)).unwrap();
    process.exiting = Some(Term::tuple(vec![Term::Integer(1)]));
    scheduler
        .freed
        .insert(ProcessId(0), (process, ExitPhase::Links(None)));
    let with_reason = scheduler.live_bytes();
    let (mut process, _) = scheduler.freed.remove(&ProcessId(0)).unwrap();
    process.exiting = None;
    scheduler
        .freed
        .insert(ProcessId(0), (process, ExitPhase::Links(None)));
    assert!(with_reason > scheduler.live_bytes());
}

#[test]
fn counts_the_persistent_terms_the_ets_objects_and_the_timer_messages() {
    let mut scheduler = test_scheduler();
    let before = scheduler.live_bytes();
    {
        let mut store = scheduler.node().persistent_terms();
        let _ = std::sync::Arc::make_mut(&mut store.terms)
            .put(&atom("key"), &Term::tuple(vec![Term::Integer(1)]));
    }
    assert!(scheduler.live_bytes() > before);
    let key = Term::Atom(scheduler.atoms.put("key").unwrap());
    let mut table = Table::new(ProcessId(1), Access::Public, 1, TableType::Set);
    table.insert(
        &scheduler.atoms,
        vec![key, Term::tuple(vec![Term::Integer(2)])],
    );
    scheduler.ets.exclusive().create(Reference(1), None, table);
    assert!(scheduler.live_bytes() > before);
    scheduler.timers.start(
        1100,
        Timer::Bif {
            reference: Reference(2),
            receiver: Receiver::Process(ProcessId(1)),
            message: Term::tuple(vec![Term::Integer(3)]),
        },
    );
    assert!(scheduler.live_bytes() > before);
}

#[test]
fn counts_the_bytes_of_the_release_work_list() {
    // The live-byte walk of a run must count the graph that the release work
    // list holds, for each kind of item of that list: a term, an element
    // array, a map walk, the X registers, the Y slots of a frame, the
    // dictionary, the mailbox, and the monitors.
    let mut scheduler = test_scheduler();
    let id = ProcessId(0);
    let map = Map::from_pairs(
        (0..4 * RELEASE_BATCH as i64)
            .map(|key| (Term::Integer(key), atom("v")))
            .collect(),
        &crate::term::test_atom_table(),
    )
    .unwrap();
    scheduler
        .start(
            frame(),
            vec![Term::tuple(vec![marker(1)]), Term::Map(map)],
            allow_all(),
        )
        .unwrap();
    {
        let process = scheduler.processes.get_mut(&id).unwrap();
        // The frame holds more Y slots than one batch, so the group of slots
        // waits on the list after one step.
        process.frames[0].stack = (0..4 * RELEASE_BATCH)
            .map(|number| StackSlot::Term(marker(number as i64)))
            .collect();
        process.mailbox.push(marker(3));
        process.dictionary.put(Term::Integer(4), marker(5));
        process.pending_bif = Some(PendingBif {
            module: "erlang",
            function: "length",
            arguments: vec![marker(8)],
            tail: false,
        });
        process.length_trap = Some(vec![marker(9)]);
        process.monitors.insert(
            Reference(1),
            Origin {
                monitor: Some(OriginMonitor {
                    kind: "process",
                    target: None,
                    item: marker(6),
                    tag: marker(7),
                }),
                alias: None,
            },
        );
    }
    assert_eq!(scheduler.schedule_next().unwrap(), Turn::Run);
    scheduler.finish_current(atom("normal"), None).unwrap();
    // Every item of the list waits here, so the walk reads each kind of item.
    assert!(
        scheduler.live_bytes() > 0,
        "the walk counts the items of the release work list"
    );
    // One step spends its budget on the group of Y slots of the frame, which
    // holds more slots than the batch, so the group waits on the list.
    scheduler.drain_release(RELEASE_BATCH);
    assert!(
        scheduler
            .release
            .iter()
            .any(|item| matches!(item, Release::Slots(_))),
        "the step leaves the group of Y slots on the list"
    );
    assert!(
        scheduler.live_bytes() > 0,
        "the walk counts the group of Y slots of the release work list"
    );
    // A later step takes the form of the map into a walk, which needs several
    // steps of its own.
    for _ in 0..64 {
        scheduler.drain_release(RELEASE_BATCH);
        if scheduler
            .release
            .iter()
            .any(|item| matches!(item, Release::Map(_)))
        {
            break;
        }
    }
    assert!(
        scheduler
            .release
            .iter()
            .any(|item| matches!(item, Release::Map(_))),
        "a step leaves the map walk on the list"
    );
    assert!(
        scheduler.live_bytes() > 0,
        "the walk counts the map walk of the release work list"
    );
    release_all(&mut scheduler);
    assert!(scheduler.release.is_empty());
}

#[test]
fn counts_the_arena_of_a_mailbox_across_the_drain_steps() {
    // The bounded drain removes nodes from the arena of the mailbox and
    // leaves the queue links that point at them, so a walk of the queue
    // cannot run after a partial drain. The walk of a release reads the
    // arena instead. The mailbox holds more than one batch of entries, so
    // the drain needs more than one step.
    let mut scheduler = test_scheduler();
    let messages = 4 * RELEASE_BATCH;
    scheduler.start(frame(), Vec::new(), allow_all()).unwrap();
    assert_eq!(scheduler.schedule_next().unwrap(), Turn::Run);
    // The entries arrive while the process holds its turn, so the handling
    // of the step that took it from the run queue does not consume them.
    {
        let process = scheduler.processes.get_mut(&ProcessId(0)).unwrap();
        for number in 0..messages as i64 {
            process.mailbox.push(marker(number));
        }
        process.mailbox.push_signal(Signal::Exit {
            from: LinkId::Process(ProcessId(1)),
            reason: marker(0),
            linked: false,
            normal_kills: false,
            alias: None,
        });
        process.mailbox.reserve_marker();
    }
    scheduler.finish_current(atom("normal"), None).unwrap();
    // One batch moves part of the mailbox, so the arena of the item that
    // waits holds nodes whose queue links point at the removed nodes.
    scheduler.drain_release(RELEASE_BATCH);
    assert!(
        !scheduler.release.is_empty(),
        "the drain needs a later step for the rest of the mailbox"
    );
    assert!(
        scheduler.live_bytes() > 0,
        "the walk counts the arena of the mailbox that waits"
    );
    release_all(&mut scheduler);
    assert!(scheduler.release.is_empty());
    assert_eq!(
        scheduler.live_bytes(),
        0,
        "the walk releases the whole mailbox"
    );
}

#[test]
fn counts_the_storage_of_the_containers_across_the_drain_steps() {
    // The walk counts the storage of each container that the release work
    // list holds, and not only the terms that the container reaches. The
    // element array of a wide tuple stays allocated while the drain moves its
    // elements in batches, so its bytes stay in the count until the last
    // batch releases the array.
    let mut scheduler = test_scheduler();
    let width = 4 * RELEASE_BATCH;
    let array = width * std::mem::size_of::<Term>();
    let tuple = Term::tuple((0..width as i64).map(Term::Integer).collect());
    scheduler
        .start(frame(), vec![Term::Integer(0)], allow_all())
        .unwrap();
    scheduler
        .processes
        .get_mut(&ProcessId(0))
        .unwrap()
        .registers[0] = Some(tuple);
    assert_eq!(scheduler.schedule_next().unwrap(), Turn::Run);
    scheduler.finish_current(atom("normal"), None).unwrap();
    // One step moves the element array to a group of terms, which keeps the
    // capacity of that array.
    scheduler.drain_release(RELEASE_BATCH);
    assert!(
        scheduler
            .release
            .iter()
            .any(|item| matches!(item, Release::Terms(_))),
        "the step leaves the group of the array on the list"
    );
    let with_array = scheduler.live_bytes();
    release_all(&mut scheduler);
    let without_array = scheduler.live_bytes();
    assert_eq!(
        with_array - without_array,
        array,
        "the walk counts exactly the element array of the group"
    );
}

#[test]
fn counts_the_terms_of_a_map_drain_that_is_the_only_item() {
    // The walk of a drain queues the keys and the values of its leaves. When
    // the drain is the only item of the release work list, no later term call
    // flushes that queue, so the walk must flush it before it reads its
    // number, and it must hold the whole graph of each value.
    let width = 4 * RELEASE_BATCH;
    let mut direct = crate::term::LiveBytes::new();
    direct.term(&Term::list_from((0..width as i64).map(Term::Integer)));
    let expected = direct.bytes();
    assert!(expected > 0, "a list of cells holds live bytes");
    let mut map = Map::from_pairs(
        vec![(
            Term::Integer(0),
            Term::list_from((0..width as i64).map(Term::Integer)),
        )],
        &crate::term::test_atom_table(),
    )
    .unwrap();
    let drain = crate::term::MapDrain::take(&mut map).expect("the only holder takes the form");
    let mut scheduler = test_scheduler();
    scheduler.release.push(Release::Map(drain));
    assert!(
        scheduler.live_bytes() >= expected,
        "the walk counts the list that the value of the map holds"
    );
}

#[test]
fn removes_the_accept_of_a_process_that_exits_with_its_port_monitor_signal_pending() {
    use crate::runtime::port::{
        Driver,
        inet::{Acceptor, Inet, SocketType},
    };
    // At exit, a monitor signal that the process did not handle runs the
    // monitor (erl_proc_sig_queue.c:7347-7355; inet_drv.c:12732-12771).
    let mut scheduler = test_scheduler();
    scheduler.start(frame(), vec![], allow_all()).unwrap();
    scheduler.schedule_next().unwrap();
    let (child, _) = scheduler
        .spawn(
            Start::Entry(frame()),
            vec![],
            None,
            false,
            StartFlags::default(),
            None,
        )
        .unwrap()
        .unwrap();
    let mut tcp = Inet::new(SocketType::Stream);
    tcp.acceptors.push_back(Acceptor {
        id: 0,
        caller: child,
        monitor: Reference(7),
        timer: None,
    });
    let listen = scheduler
        .open_port(b"tcp_inet".to_vec(), Driver::Inet(tcp))
        .unwrap();
    scheduler.park_current();
    scheduler.schedule_next().unwrap();
    assert_eq!(current(&mut scheduler), child);
    let signal = Signal::PortMonitor {
        port: listen,
        reference: Reference(7),
    };
    scheduler.send_signal(None, child, signal).unwrap();
    scheduler.finish_current(atom("normal"), None).unwrap();
    let acceptors = scheduler
        .node
        .with_port(listen, |port| port.inet_mut().unwrap().acceptors.len())
        .unwrap();
    assert_eq!(acceptors, 0);
}

#[test]
fn removes_a_reply_demonitor_port_monitor_from_the_port_at_the_alias_message() {
    use super::{AliasMode, signals::HandlingPoint};
    use crate::runtime::port::{
        Driver,
        inet::{Inet, SocketType},
    };
    // The first message through a `reply_demonitor` alias removes its port
    // monitor from the port too (erl_proc_sig_queue.c:6101-6107).
    let mut scheduler = test_scheduler();
    scheduler.start(frame(), vec![], allow_all()).unwrap();
    scheduler.schedule_next().unwrap();
    let id = current(&mut scheduler);
    let tcp = Inet::new(SocketType::Stream);
    let port = scheduler
        .open_port(b"tcp_inet".to_vec(), Driver::Inet(tcp))
        .unwrap();
    let options = MonitorOptions {
        alias: Some(AliasMode::ReplyDemonitor),
        ..no_options()
    };
    let alias = scheduler
        .monitor(MonitorTarget::Port(port), options)
        .unwrap();
    let signal = Signal::AliasMessage {
        alias,
        message: atom("reply"),
    };
    scheduler.send_signal(None, id, signal).unwrap();
    scheduler
        .handle_signals(id, 100, 100, HandlingPoint::ScheduleIn)
        .unwrap();
    assert!(scheduler.processes[&id].monitors.get(&alias).is_none());
    let monitors = scheduler.node.with_port(port, |port| port.monitors.len());
    assert_eq!(monitors, Some(0));
}

#[test]
fn sends_the_messages_of_an_active_socket_to_the_owner_and_ends_the_port() {
    use super::signals::HandlingPoint;
    use crate::runtime::port::{
        Driver,
        inet::{Event, Inet, SocketType},
    };
    use socket2::{Domain, Socket, Type};
    // `tcp_error_message`, `tcp_closed_message`, and `driver_exit`
    // (inet_drv.c:4058-4095, 12870-12877; io.c:7228-7249).
    let mut scheduler = test_scheduler();
    scheduler.start(frame(), vec![], allow_all()).unwrap();
    scheduler.schedule_next().unwrap();
    let id = current(&mut scheduler);
    let (local, _peer) = Socket::pair(Domain::UNIX, Type::STREAM, None).unwrap();
    let tcp = Inet::new(SocketType::Stream).copy(local);
    let port = scheduler
        .open_port(b"tcp_inet".to_vec(), Driver::Inet(tcp))
        .unwrap();
    let events = vec![
        Event::Error("etimedout".to_owned()),
        Event::Closed,
        Event::Exit("etimedout".to_owned()),
    ];
    scheduler.deliver_events(port, events).unwrap();
    let name = |text| Term::Atom(scheduler.atoms.put(text).unwrap());
    let (tcp_error, etimedout, tcp_closed) =
        (name("tcp_error"), name("etimedout"), name("tcp_closed"));
    let error = Term::tuple(vec![tcp_error, Term::Port(port), etimedout]);
    let closed = Term::tuple(vec![tcp_closed, Term::Port(port)]);
    assert_eq!(messages(&scheduler, id.0), [error, closed]);
    assert!(scheduler.node.ports().get(port).is_none());
    scheduler
        .handle_signals(id, 100, 100, HandlingPoint::ScheduleIn)
        .unwrap();
    assert!(scheduler.processes[&id].links.is_empty());
}

#[test]
fn unlinks_the_new_owner_after_its_queued_link_signal_at_a_driver_exit() {
    use super::signals::HandlingPoint;
    use crate::runtime::port::{
        Driver,
        inet::{Event, Inet, SocketType},
    };
    use socket2::{Domain, Socket, Type};
    // `erlang:port_connect/2` queues the link signal (io.c:2355-2380), and
    // `driver_exit` then queues the unlink signal of `unlink_proc`
    // (io.c:2889-2909, 7228-7249). The new owner handles them in order and
    // keeps no link to the closed port.
    let mut scheduler = test_scheduler();
    scheduler.start(frame(), vec![], allow_all()).unwrap();
    scheduler.schedule_next().unwrap();
    let (child, _) = scheduler
        .spawn(
            Start::Entry(frame()),
            vec![],
            None,
            false,
            StartFlags::default(),
            None,
        )
        .unwrap()
        .unwrap();
    let (local, _peer) = Socket::pair(Domain::UNIX, Type::STREAM, None).unwrap();
    let tcp = Inet::new(SocketType::Stream).copy(local);
    let port = scheduler
        .open_port(b"tcp_inet".to_vec(), Driver::Inet(tcp))
        .unwrap();
    assert!(scheduler.connect_port(port, child));
    let events = vec![Event::Exit("normal".to_owned())];
    scheduler.deliver_events(port, events).unwrap();
    scheduler
        .handle_signals(child, 100, 100, HandlingPoint::ScheduleIn)
        .unwrap();
    let links = &scheduler.processes[&child].links;
    assert!(links.get(&LinkId::Port(port)).is_none());
}

#[test]
fn drops_the_link_exit_signal_of_a_process_that_the_port_does_not_link() {
    use super::signals::HandlingPoint;
    use crate::runtime::port::{
        Driver,
        inet::{Inet, SocketType},
    };
    use socket2::{Domain, Socket, Type};
    // `erlang:port_connect/2` queues the link signal (io.c:2355-2380). The
    // new owner runs `link/1` and `unlink/1` with the port before it
    // handles the signal, so only its own side of the link stays. The port
    // then drops the link exit signal of the owner (io.c:2187-2190).
    let mut scheduler = test_scheduler();
    scheduler.start(frame(), vec![], allow_all()).unwrap();
    scheduler.schedule_next().unwrap();
    let (child, _) = scheduler
        .spawn(
            Start::Entry(frame()),
            vec![],
            None,
            false,
            StartFlags::default(),
            None,
        )
        .unwrap()
        .unwrap();
    let (local, _peer) = Socket::pair(Domain::UNIX, Type::STREAM, None).unwrap();
    let tcp = Inet::new(SocketType::Stream).copy(local);
    let port = scheduler
        .open_port(b"tcp_inet".to_vec(), Driver::Inet(tcp))
        .unwrap();
    assert!(scheduler.connect_port(port, child));
    let removed = scheduler
        .node
        .with_port(port, |port| port.links.remove(&child))
        .unwrap();
    assert!(removed.is_some());
    scheduler
        .handle_signals(child, 100, 100, HandlingPoint::ScheduleIn)
        .unwrap();
    let links = &scheduler.processes[&child].links;
    assert_eq!(links.get(&LinkId::Port(port)), Some(&0));
    let killed = Term::static_atom("killed");
    scheduler.port_link_exit(port, child, &killed);
    assert!(scheduler.node.ports().get(port).is_some());
}

#[test]
fn stops_a_break_key_with_no_data_at_a_second_break() {
    // `sys_get_key` retries a read with no data, and a second break stops
    // the run while the read waits (`sys.c:1051-1071`, `611-624`).
    crate::runtime::host::signal::mark_second_break();
    assert_eq!(Scheduler::break_key(|| Err(libc::EAGAIN)), 0);
    assert!(!crate::runtime::host::signal::take_second_break());
}

#[test]
fn reads_a_break_key_after_a_failed_menu_wait() {
    // `sys_get_key` gives 0 when its `select` fails, so the menu reads fd 0,
    // where the read fails and the run stops (`sys.c:1051-1071`).
    assert_eq!(
        Scheduler::menu_ready(Readiness::Invalid),
        vec![libc::STDIN_FILENO]
    );
    assert!(Scheduler::menu_ready(Readiness::Pending).is_empty());
    assert_eq!(Scheduler::menu_ready(Readiness::Ready(vec![7])), vec![7]);
}

#[test]
fn reads_a_break_key_after_a_read_with_no_data() {
    // `sys_get_key` retries a read that returns EAGAIN, and it gives 0 for
    // the end of the input, a NUL byte, and a read error (`sys.c:1051-1071`).
    let mut reads = [Err(libc::EAGAIN), Ok(vec![b'a'])].into_iter();
    assert_eq!(Scheduler::break_key(|| reads.next().unwrap()), b'a');
    assert_eq!(Scheduler::break_key(|| Ok(Vec::new())), 0);
    assert_eq!(Scheduler::break_key(|| Ok(vec![0])), 0);
    assert_eq!(Scheduler::break_key(|| Err(libc::EBADF)), 0);
}

#[test]
fn a_signal_to_a_dead_process_answers_a_monitor() {
    let mut scheduler = test_scheduler();
    scheduler.start(frame(), vec![], allow_all()).unwrap();
    let root = ProcessId(0);
    let reference = Reference::ordinary(0).unwrap();
    scheduler.dead_target_signal(
        ProcessId(99),
        Signal::Monitor {
            watcher: root,
            reference,
        },
    );
    assert!(
        scheduler
            .process(root)
            .unwrap()
            .mailbox
            .has_control_signals()
    );
}

#[test]
fn a_signal_to_a_dead_process_answers_an_unlink() {
    use super::signals::HandlingPoint;
    let mut scheduler = test_scheduler();
    scheduler.start(frame(), vec![], allow_all()).unwrap();
    let root = ProcessId(0);
    let target = ProcessId(99);
    let link = LinkId::Process(target);
    scheduler
        .processes
        .get_mut(&root)
        .unwrap()
        .links
        .insert(link, 7);
    scheduler.dead_target_signal(target, Signal::Unlink { from: root, id: 7 });
    scheduler
        .handle_signals(root, 100, 100, HandlingPoint::ScheduleIn)
        .unwrap();
    assert_eq!(scheduler.processes[&root].links.get(&link), None);
}

#[test]
fn a_signal_server_is_an_external_source() {
    use super::node::Node;
    let (node, mut readers) = Node::new(1).unwrap();
    let mut scheduler = Scheduler::with_shared(
        Arc::new(AtomTable::new()),
        Arc::clone(&node),
        0,
        readers.remove(0),
    );
    scheduler.start(frame(), vec![], allow_all()).unwrap();
    let root = ProcessId(0);
    assert!(!scheduler.has_external_sources());
    assert!(node.name_register("erl_signal_server", root));
    assert!(scheduler.has_external_sources());
}

#[test]
fn a_remote_signal_flushes_the_pending_monitor_first() {
    use super::node::{Delivery, Node};
    let (node, mut readers) = Node::new(2).unwrap();
    let mut scheduler = Scheduler::with_shared(
        Arc::new(AtomTable::new()),
        Arc::clone(&node),
        0,
        readers.remove(0),
    );
    scheduler.start(frame(), vec![], allow_all()).unwrap();
    let root = ProcessId(0);
    let target = ProcessId(77);
    node.register(target, 1);
    let reference = Reference::ordinary(0).unwrap();
    scheduler.pending_monitor = Some((
        target,
        Monitor {
            watcher: root,
            reference,
        },
    ));
    scheduler.send_signal_to_owner(
        1,
        Some(root),
        target,
        Signal::ProcessInfo {
            requester: root,
            reference,
            item: atom("registered_name"),
        },
    );
    let mut deliveries = Vec::new();
    node.take(1, &mut deliveries);
    assert!(matches!(
        deliveries.as_slice(),
        [
            Delivery::Signal {
                signal: Signal::Monitor { .. },
                ..
            },
            Delivery::Signal {
                signal: Signal::ProcessInfo { .. },
                ..
            }
        ]
    ));
}

#[test]
fn a_remote_target_adds_and_removes_the_monitor_of_a_watcher() {
    // The target of another scheduler adds the target monitor when it
    // handles the monitor signal (`erl_proc_sig_queue.c:6544-6562`), and
    // removes it when it handles the demonitor signal
    // (`erl_proc_sig_queue.c:6564-6633`). Both signals go through the inbox
    // of its owner. The watcher removes its own monitor first, so only the
    // target shows that the demonitor signal arrived.
    use super::HandlingPoint;
    use crate::runtime::process::TargetMonitor;
    let (node, mut readers) = Node::new(2).unwrap();
    let mut owner = Scheduler::with_shared(
        Arc::new(AtomTable::new()),
        Arc::clone(&node),
        1,
        readers.remove(1),
    );
    let mut scheduler = Scheduler::with_shared(
        Arc::new(AtomTable::new()),
        Arc::clone(&node),
        0,
        readers.remove(0),
    );
    let (target, watcher) = (ProcessId(0), ProcessId(1));
    owner
        .start_with_id(target, 1, frame(), vec![], allow_all())
        .unwrap();
    scheduler
        .start_with_id(watcher, 0, frame(), vec![], allow_all())
        .unwrap();
    scheduler.schedule_next().unwrap();
    let reference = scheduler
        .monitor(MonitorTarget::Process(target), no_options())
        .unwrap();
    scheduler.send_pending_monitor();
    assert!(owner.drain_inbox());
    owner
        .handle_signals(target, 100, 100, HandlingPoint::ScheduleIn)
        .unwrap();
    let watchers = |owner: &Scheduler| -> Vec<ProcessId> {
        owner.processes[&target]
            .monitored_by
            .iter()
            .filter_map(|monitor| match monitor {
                TargetMonitor::Process(monitor) => Some(monitor.watcher),
                _ => None,
            })
            .collect()
    };
    assert_eq!(watchers(&owner), [watcher]);
    assert!(scheduler.demonitor(reference));
    assert!(owner.drain_inbox());
    owner
        .handle_signals(target, 100, 100, HandlingPoint::ScheduleIn)
        .unwrap();
    assert!(watchers(&owner).is_empty());
}

#[test]
fn a_host_signal_notification_reaches_a_remote_signal_server() {
    use super::node::Node;
    let (node, mut readers) = Node::new(2).unwrap();
    let mut owner = Scheduler::with_shared(
        Arc::new(AtomTable::new()),
        Arc::clone(&node),
        1,
        readers.remove(1),
    );
    let mut scheduler = Scheduler::with_shared(
        Arc::new(AtomTable::new()),
        Arc::clone(&node),
        0,
        readers.remove(0),
    );
    owner.start(frame(), vec![], allow_all()).unwrap();
    let server = ProcessId(0);
    assert!(node.name_register("erl_signal_server", server));
    scheduler.notify_signals(&[libc::SIGUSR1]).unwrap();
    owner.drain_inbox();
    assert_eq!(
        messages(&owner, 0),
        vec![Term::tuple(vec![
            Term::static_atom("notify"),
            Term::static_atom("sigusr1")
        ])]
    );
    // A message of the system sets no self-signal mark on the receiver.
    assert!(!owner.processes[&server].self_signals);
}

#[test]
fn a_flushed_port_leaves_the_shared_port_table() {
    use super::node::Node;
    use crate::runtime::port::{
        Driver,
        inet::{Inet, SocketType},
    };
    let (node, mut readers) = Node::new(1).unwrap();
    let mut scheduler = Scheduler::with_shared(
        Arc::new(AtomTable::new()),
        Arc::clone(&node),
        0,
        readers.remove(0),
    );
    scheduler.start(frame(), vec![], allow_all()).unwrap();
    scheduler.schedule_next().unwrap();
    let tcp = Inet::new(SocketType::Stream);
    let port = scheduler
        .open_port(b"tcp_inet".to_vec(), Driver::Inet(tcp))
        .unwrap();
    assert!(node.ports().get(port).is_some());
    scheduler.node.with_port(port, |port| port.closing = true);
    scheduler.tcp_flushed(port);
    assert!(node.ports().get(port).is_none());
    assert!(scheduler.node.ports().get(port).is_none());
}

#[test]
fn changes_nothing_on_a_port_that_closed() {
    use crate::runtime::port::{
        Driver,
        inet::{Inet, SocketType},
    };
    let (node, mut readers) = Node::new(1).unwrap();
    let mut scheduler = Scheduler::with_shared(
        Arc::new(AtomTable::new()),
        Arc::clone(&node),
        0,
        readers.remove(0),
    );
    scheduler.start(frame(), vec![], allow_all()).unwrap();
    scheduler.schedule_next().unwrap();
    let owner = current(&mut scheduler);
    let tcp = Inet::new(SocketType::Stream);
    let port = scheduler
        .open_port(b"tcp_inet".to_vec(), Driver::Inet(tcp))
        .unwrap();
    // A handle that a scheduler took before the close sees the closed port
    // under the port lock.
    let cell = node.port(port).unwrap();
    scheduler.close_port(port, atom("normal"));
    assert!(crate::runtime::port::lock(&cell).is_none());
    assert!(node.with_port(port, |_| ()).is_none());
    // Each operation of a closed port, as after a close of another
    // scheduler in between, changes nothing and does not panic.
    scheduler.close_port(port, atom("normal"));
    scheduler.driver_exit(port, atom("normal"));
    assert!(!scheduler.connect_port(port, owner));
    scheduler.tcp_flushed(port);
    assert!(node.ports().get(port).is_none());
}

#[test]
fn a_delivery_to_an_exiting_process_changes_nothing() {
    let mut scheduler = test_scheduler();
    scheduler.start(frame(), vec![], allow_all()).unwrap();
    let root = ProcessId(0);
    scheduler.processes.get_mut(&root).unwrap().exiting = Some(atom("normal"));
    scheduler.deliver(root, atom("lost"));
    assert!(messages(&scheduler, 0).is_empty());
}

#[test]
fn a_delivery_to_a_missing_process_changes_nothing() {
    let mut scheduler = test_scheduler();
    scheduler.start(frame(), vec![], allow_all()).unwrap();
    scheduler.deliver(ProcessId(99), atom("lost"));
    assert!(scheduler.process(ProcessId(99)).is_none());
}

#[test]
fn a_signal_to_a_dead_process_drops_an_unlink_ack() {
    let mut scheduler = test_scheduler();
    scheduler.start(frame(), vec![], allow_all()).unwrap();
    let root = ProcessId(0);
    scheduler.dead_target_signal(ProcessId(99), Signal::UnlinkAck { from: root, id: 1 });
    assert!(messages(&scheduler, 0).is_empty());
}

#[test]
fn a_delivery_to_a_dead_target_answers_its_request() {
    use super::node::Node;
    let (node, mut readers) = Node::new(1).unwrap();
    let mut scheduler = Scheduler::with_shared(
        Arc::new(AtomTable::new()),
        Arc::clone(&node),
        0,
        readers.remove(0),
    );
    scheduler.start(frame(), vec![], allow_all()).unwrap();
    let root = ProcessId(0);
    let reference = Reference::ordinary(3).unwrap();
    node.send_signal(
        0,
        ProcessId(99),
        Some(root),
        Signal::IsAlive {
            requester: root,
            reference,
        },
        None,
    );
    scheduler.drain_inbox();
    assert_eq!(
        messages(&scheduler, 0),
        vec![Term::tuple(vec![
            Term::Reference(reference),
            Term::static_atom("false")
        ])]
    );
}

#[test]
fn a_signal_to_a_dead_process_answers_a_group_leader_and_an_is_alive() {
    let mut scheduler = test_scheduler();
    scheduler.start(frame(), vec![], allow_all()).unwrap();
    let root = ProcessId(0);
    let leader_reference = Reference::ordinary(0).unwrap();
    let alive_reference = Reference::ordinary(1).unwrap();
    scheduler.dead_target_signal(
        ProcessId(99),
        Signal::GroupLeader {
            leader: ProcessId(98),
            requester: root,
            reference: leader_reference,
        },
    );
    scheduler.dead_target_signal(
        ProcessId(99),
        Signal::IsAlive {
            requester: root,
            reference: alive_reference,
        },
    );
    assert_eq!(
        messages(&scheduler, 0),
        vec![
            Term::tuple(vec![
                Term::Reference(leader_reference),
                Term::static_atom("badarg")
            ]),
            Term::tuple(vec![
                Term::Reference(alive_reference),
                Term::static_atom("false")
            ]),
        ]
    );
}

#[test]
fn a_signal_to_a_dead_process_answers_a_process_info() {
    let mut scheduler = test_scheduler();
    scheduler.start(frame(), vec![], allow_all()).unwrap();
    let root = ProcessId(0);
    let reference = Reference::ordinary(2).unwrap();
    scheduler.dead_target_signal(
        ProcessId(99),
        Signal::ProcessInfo {
            requester: root,
            reference,
            item: atom("registered_name"),
        },
    );
    assert_eq!(
        messages(&scheduler, 0),
        vec![Term::tuple(vec![
            Term::Reference(reference),
            Term::static_atom("undefined")
        ])]
    );
}

#[test]
fn a_signal_to_a_dead_process_answers_a_link() {
    let mut scheduler = test_scheduler();
    scheduler.start(frame(), vec![], allow_all()).unwrap();
    let root = ProcessId(0);
    scheduler.dead_target_signal(ProcessId(99), Signal::Link { from: root });
    assert!(
        scheduler
            .process(root)
            .unwrap()
            .mailbox
            .has_control_signals()
    );
}

#[test]
fn a_scheduler_registers_a_name_for_a_remote_process() {
    use super::node::Node;
    let (node, mut readers) = Node::new(2).unwrap();
    // Make the two names before the scheduler starts.
    let atoms = AtomTable::new();
    let first = atoms.put("pbeam_remote_name").unwrap();
    let second = atoms.put("pbeam_remote_other").unwrap();
    let mut scheduler =
        Scheduler::with_shared(Arc::new(atoms), Arc::clone(&node), 0, readers.remove(0));
    let remote = ProcessId(7);
    node.register(remote, 1);
    assert!(scheduler.register(&Term::Atom(first.clone()), &Term::Pid(remote)));
    assert_eq!(scheduler.registered_name_of(remote), Some(first.clone()));
    // One process has one name.
    assert!(!scheduler.register(&Term::Atom(second), &Term::Pid(remote)));
    assert!(scheduler.unregister(&Term::Atom(first)));
    assert_eq!(scheduler.registered_name_of(remote), None);
}

#[test]
fn answers_a_check_process_code_request_of_another_scheduler() {
    use super::signals::HandlingPoint;
    // The scheduler of the target runs the check when it drains its inbox:
    // a live process with no frame in old code and a process that is gone
    // give `false` (`erl_process.c:11425-11433`). A process with a signal to
    // handle gets the request behind it, and replies when it handles it, or
    // with `false` at its exit (`erl_process.c:11459-11489`).
    let (node, mut readers) = Node::new(1).unwrap();
    let mut scheduler = Scheduler::with_shared(
        Arc::new(AtomTable::new()),
        Arc::clone(&node),
        0,
        readers.remove(0),
    );
    scheduler.start(frame(), vec![], allow_all()).unwrap();
    scheduler.schedule_next().unwrap();
    let root = ProcessId(0);
    let mut spawn = |id: u64| {
        let child = ProcessId(id);
        scheduler.insert(ProcessState::new(child, Start::Entry(frame()), vec![]));
        let link = Signal::Link { from: root };
        scheduler
            .send_signal_local(Some(root), child, link)
            .unwrap();
        child
    };
    let (handling, exiting) = (spawn(1), spawn(2));
    for (target, tag) in [(root, 1), (ProcessId(99), 2), (handling, 3), (exiting, 4)] {
        let module = Atom::from_static("erlang").unwrap();
        node.check_process_code(0, target, root, Term::Integer(tag), module);
    }
    scheduler.drain_inbox();
    scheduler
        .handle_signals(handling, 100, 100, HandlingPoint::ScheduleIn)
        .unwrap();
    let mut process = scheduler.processes.remove(&exiting).unwrap();
    process.mailbox.fetch();
    assert!(scheduler.handle_exit_signals(&mut process, 100).unwrap());
    let reply = |tag: i64| {
        Term::tuple(vec![
            atom("check_process_code"),
            Term::Integer(tag),
            atom("false"),
        ])
    };
    let replies: Vec<Term> = messages(&scheduler, 0)
        .into_iter()
        .filter(|message| matches!(message, Term::Tuple(_)))
        .collect();
    assert_eq!(replies, vec![reply(1), reply(2), reply(3), reply(4)]);
}

use super::{CONTEXT_REDS, LinkId, MonitorOptions, MonitorTarget, Scheduler, Turn};
use crate::term::test_atom;
use crate::{
    runtime::ets::{Access, Table},
    runtime::process::{ReceiveTimer, Start, TimerReference, tests::frame},
    runtime::timer::{Receiver, Timer},
    term::{Atom, AtomTable, ProcessId, Term},
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

fn current(scheduler: &mut Scheduler) -> ProcessId {
    scheduler.current_process_mut().id
}

#[test]
fn keeps_the_current_process_in_the_table_and_out_of_the_run_queue() {
    let mut scheduler = Scheduler::new(AtomTable::new());
    scheduler.start(frame(), vec![]).unwrap();
    assert_eq!(scheduler.schedule_next().unwrap(), Turn::Run);
    assert_eq!(current(&mut scheduler), ProcessId(0));
    assert!(scheduler.processes.contains_key(&ProcessId(0)));
    let (child, monitor) = scheduler
        .spawn(Start::Entry(frame()), vec![], false, false, false)
        .unwrap();
    assert_eq!((child, monitor), (ProcessId(1), None));

    scheduler.park_current();
    assert_eq!(scheduler.schedule_next().unwrap(), Turn::Run);
    assert_eq!(current(&mut scheduler), ProcessId(1));
    assert!(scheduler.runnable.is_empty());
    scheduler.deliver(ProcessId(0), atom("ping"));
    scheduler.deliver(ProcessId(1), atom("self"));
    scheduler.deliver(ProcessId(9), atom("lost"));

    assert_eq!(Vec::from(scheduler.runnable.clone()), vec![ProcessId(0)]);
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
fn switches_out_to_the_end_of_the_run_queue_with_a_new_budget() {
    let mut scheduler = Scheduler::new(AtomTable::new());
    scheduler.start(frame(), vec![]).unwrap();
    assert_eq!(scheduler.schedule_next().unwrap(), Turn::Run);
    scheduler
        .spawn(Start::Entry(frame()), vec![], false, false, false)
        .unwrap();
    scheduler.current_process_mut().fcalls = -1;
    scheduler.yield_current();
    assert_eq!(
        Vec::from(scheduler.runnable.clone()),
        vec![ProcessId(1), ProcessId(0)]
    );
    assert_eq!(scheduler.schedule_next().unwrap(), Turn::Run);
    assert_eq!(current(&mut scheduler), ProcessId(1));
    scheduler.yield_current();
    assert_eq!(scheduler.schedule_next().unwrap(), Turn::Run);
    assert_eq!(scheduler.current_process_mut().fcalls, CONTEXT_REDS);
}

#[test]
fn sends_down_and_exit_signals_and_records_the_root_result() {
    let mut scheduler = Scheduler::new(AtomTable::new());
    scheduler.start(frame(), vec![]).unwrap();
    scheduler.schedule_next().unwrap();
    scheduler.current_process_mut().trap_exit = true;
    let (monitored, monitor) = scheduler
        .spawn(Start::Entry(frame()), vec![], true, false, false)
        .unwrap();
    let (linked, _) = scheduler
        .spawn(Start::Entry(frame()), vec![], false, true, false)
        .unwrap();
    scheduler.park_current();

    scheduler.schedule_next().unwrap();
    scheduler.finish_current(atom("normal"), None);
    scheduler.schedule_next().unwrap();
    scheduler.finish_current(atom("boom"), None);

    // The root process waits, so it handles both signals at schedule-in,
    // and the messages give it Erlang work.
    assert_eq!(scheduler.schedule_next().unwrap(), Turn::Run);
    assert_eq!(current(&mut scheduler), ProcessId(0));
    assert!(scheduler.current_process_mut().links.is_empty());
    assert_eq!(
        messages(&scheduler, 0),
        [
            Term::Tuple(vec![
                atom("DOWN"),
                Term::Reference(monitor.unwrap()),
                atom("process"),
                Term::Pid(monitored),
                atom("normal"),
            ]),
            Term::Tuple(vec![atom("EXIT"), Term::Pid(linked), atom("boom")]),
        ]
    );
    scheduler.finish_current(atom("normal"), Some(atom("ok")));
    assert!(scheduler.processes.is_empty());
    assert_eq!(scheduler.into_root_result(), Some(Ok(atom("ok"))));
}

#[test]
fn ends_the_run_only_after_the_exit_of_the_root_process_completes() {
    // With 100 or fewer reductions left, the exit switches out before it
    // leaves the process table. With 150 or fewer, it switches out before
    // its links (erl_process.c:14315-14316, 14541-14543). In both cases the
    // root result is recorded, but the run continues until the next turn
    // of the root process sends its link exit signal.
    for fcalls in [100, 150] {
        let mut scheduler = Scheduler::new(AtomTable::new());
        scheduler.start(frame(), vec![]).unwrap();
        scheduler.schedule_next().unwrap();
        let (child, _) = scheduler
            .spawn(Start::Entry(frame()), vec![], false, true, false)
            .unwrap();
        scheduler.current_process_mut().fcalls = fcalls;
        scheduler.finish_current(atom("normal"), Some(atom("ok")));
        assert!(!scheduler.root_exited());
        let signals =
            |scheduler: &Scheduler| scheduler.processes[&child].mailbox.has_control_signals();
        assert!(!signals(&scheduler));

        assert_eq!(scheduler.schedule_next().unwrap(), Turn::Run);
        assert_eq!(current(&mut scheduler), child);
        scheduler.yield_current();
        assert!(!scheduler.root_exited());
        assert_eq!(scheduler.schedule_next().unwrap(), Turn::Done);
        assert!(scheduler.root_exited());
        assert!(signals(&scheduler));
        assert_eq!(scheduler.into_root_result(), Some(Ok(atom("ok"))));
    }
}

#[test]
fn links_and_unlinks_processes_and_ignores_normal_exit_without_trap_exit() {
    let mut scheduler = Scheduler::new(AtomTable::new());
    scheduler.start(frame(), vec![]).unwrap();
    scheduler.schedule_next().unwrap();
    let (child, _) = scheduler
        .spawn(Start::Entry(frame()), vec![], false, false, false)
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
    scheduler.finish_current(atom("normal"), None);

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
    let mut scheduler = Scheduler::new(AtomTable::new());
    scheduler.start(frame(), vec![]).unwrap();
    scheduler.schedule_next().unwrap();
    scheduler
        .spawn(Start::Entry(frame()), vec![], false, true, false)
        .unwrap();
    scheduler.park_current();
    scheduler.schedule_next().unwrap();
    scheduler.finish_current(atom("boom"), None);
    // The link exit signal ends the root process, which does not trap
    // exits, with the reason of the child.
    assert_eq!(scheduler.schedule_next().unwrap(), Turn::Done);
    assert_eq!(scheduler.into_root_result(), Some(Err(atom("boom"))));

    let mut scheduler = Scheduler::new(AtomTable::new());
    scheduler.start(frame(), vec![]).unwrap();
    scheduler.schedule_next().unwrap();
    scheduler.finish_current(atom("boom"), Some(atom("ok")));
    assert_eq!(scheduler.into_root_result(), Some(Err(atom("boom"))));

    let mut scheduler = Scheduler {
        next_process_id: u64::MAX,
        next_reference: u64::MAX,
        ..Scheduler::new(AtomTable::new())
    };
    assert!(scheduler.start(frame(), vec![]).is_err());
    assert!(scheduler.make_ref().is_err());
}

#[test]
fn sends_the_pending_monitor_signal_before_a_demonitor_signal() {
    // `proc_queue_signal` sends the pending monitor signal first when the
    // next signal goes to the same target (`erl_proc_sig_queue.c:1146-1170`).
    let mut scheduler = Scheduler::new(AtomTable::new());
    scheduler.start(frame(), vec![]).unwrap();
    scheduler.schedule_next().unwrap();
    let (child, _) = scheduler
        .spawn(Start::Entry(frame()), vec![], false, false, false)
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
    let mut scheduler = Scheduler::new(AtomTable::new());
    scheduler.start(frame(), vec![]).unwrap();
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
    let named = Term::Tuple(vec![atom("undefined"), atom("nonode@nohost")]);
    assert_eq!(
        messages(&scheduler, 0),
        [
            Term::Tuple(vec![
                atom("DOWN"),
                Term::Reference(port),
                atom("port"),
                named,
                atom("noproc"),
            ]),
            Term::Tuple(vec![
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
    let mut scheduler = Scheduler::new(AtomTable::new());
    scheduler.start(frame(), vec![]).unwrap();
    scheduler.schedule_next().unwrap();
    let (child, _) = scheduler
        .spawn(Start::Entry(frame()), vec![], false, false, false)
        .unwrap();
    let name = atom("pbeam_probe_reg");
    assert_eq!(scheduler.whereis("pbeam_probe_reg"), None);
    assert!(scheduler.register(&name, &Term::Pid(child)));
    assert!(!scheduler.register(&name, &Term::Pid(ProcessId(0))));
    assert_eq!(scheduler.whereis("pbeam_probe_reg"), Some(child));
    scheduler.park_current();

    scheduler.schedule_next().unwrap();
    scheduler.finish_current(atom("normal"), None);
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
    let mut scheduler = Scheduler::new(AtomTable::new());
    scheduler.start(frame(), vec![]).unwrap();
    scheduler.schedule_next().unwrap();
    let (child, monitor) = scheduler
        .spawn(Start::Entry(frame()), vec![], true, false, false)
        .unwrap();
    scheduler.park_current();

    scheduler.schedule_next().unwrap();
    let table = scheduler.make_ref().unwrap();
    scheduler.ets.create(
        table,
        Some("ets_probe_owned"),
        Table::new(child, Access::Protected, 1),
    );
    assert!(scheduler.ets.name_in_use("ets_probe_owned"));
    scheduler.finish_current(atom("normal"), None);

    assert!(!scheduler.ets.name_in_use("ets_probe_owned"));
    assert_eq!(scheduler.schedule_next().unwrap(), Turn::Run);
    assert_eq!(
        messages(&scheduler, 0),
        [Term::Tuple(vec![
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
    let mut scheduler = Scheduler::new(AtomTable::new());
    scheduler.start(frame(), vec![]).unwrap();
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
    assert_eq!(Vec::from(scheduler.runnable.clone()), vec![ProcessId(0)]);
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
    let mut scheduler = Scheduler::new(AtomTable::new());
    scheduler.start(frame(), vec![]).unwrap();
    scheduler.schedule_next().unwrap();
    assert!(scheduler.set_receive_timer(1_000_000_000, &Term::Integer(1), 3));
    scheduler.park_current();
    scheduler.expire_timers(1_001_000_000);
    assert_eq!(scheduler.schedule_next().unwrap(), Turn::Run);

    assert!(scheduler.set_receive_timer(1_000_000_000, &Term::Integer(1), 3));
    scheduler.cancel_receive_timer();
    assert_eq!(scheduler.timers.wait_time(1_000_000_000), None);
    assert!(scheduler.set_receive_timer(1_000_000_000, &Term::Integer(1), 3));
    scheduler.finish_current(atom("normal"), None);
    assert_eq!(scheduler.timers.wait_time(1_000_000_000), None);
}

#[test]
fn continues_the_exit_timer_walk_at_its_next_timer() {
    // Each turn cancels the timers that its reductions pay for and keeps
    // the position of the next timer (erl_rbtree.h:1392-1396). A turn that
    // walked the tree again from its start would cancel no new timer for
    // the reductions of each timer that an earlier turn cancelled.
    let mut scheduler = Scheduler::new(AtomTable::new());
    scheduler.start(frame(), vec![]).unwrap();
    scheduler.schedule_next().unwrap();
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
        let process = scheduler.current_process_mut();
        process.bif_timers.insert(TimerReference(reference), ());
    }
    let process = scheduler.current_process_mut();
    let order: Vec<TimerReference> = process.bif_timers.post_order().map(|(&t, _)| t).collect();
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
    scheduler.finish_current(atom("normal"), None);
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
    let mut scheduler = Scheduler::new(AtomTable::new());
    scheduler.start(frame(), vec![]).unwrap();
    scheduler.schedule_next().unwrap();
    for timeout in [Term::Integer(-1), Term::Integer(4_294_967_296), atom("foo")] {
        assert!(!scheduler.set_receive_timer(1_000_000_000, &timeout, 0));
    }
    let end = 9_223_372_036_000_000_000;
    assert!(!scheduler.set_receive_timer(end, &Term::Integer(1), 0));
    assert!(scheduler.set_receive_timer(end - 1_000_000, &Term::Integer(1), 0));
    assert_eq!(
        scheduler.current_process_mut().receive_timer,
        ReceiveTimer::Set {
            key: (9_223_372_036_000, 0),
            resume: 0
        }
    );
}

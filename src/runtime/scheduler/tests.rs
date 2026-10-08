use super::{MonitorOptions, MonitorTarget, Scheduler};
use crate::term::test_atom;
use crate::{
    runtime::ets::{Access, Table},
    runtime::process::{ReceiveTimer, Start, tests::frame},
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

fn first_message(scheduler: &Scheduler, id: u64, index: usize) -> Term {
    scheduler.processes[&ProcessId(id)].mailbox.messages()[index].clone()
}

#[test]
fn keeps_the_current_process_in_the_table_and_out_of_the_run_queue() {
    let mut scheduler = Scheduler::new(AtomTable::new());
    scheduler.start(frame(), vec![]).unwrap();
    assert_eq!(scheduler.schedule_next(), Some(ProcessId(0)));
    assert!(scheduler.processes.contains_key(&ProcessId(0)));
    let (child, monitor) = scheduler
        .spawn(Start::Entry(frame()), vec![], false, false, false)
        .unwrap();
    assert_eq!((child, monitor), (ProcessId(1), None));

    scheduler.park_current();
    assert_eq!(scheduler.schedule_next(), Some(ProcessId(1)));
    assert!(scheduler.runnable.is_empty());
    scheduler.deliver(ProcessId(0), atom("ping"));
    scheduler.deliver(ProcessId(1), atom("self"));
    scheduler.deliver(ProcessId(9), atom("lost"));

    assert_eq!(Vec::from(scheduler.runnable.clone()), vec![ProcessId(0)]);
    assert_eq!(first_message(&scheduler, 0, 0), atom("ping"));
    assert_eq!(first_message(&scheduler, 1, 0), atom("self"));
}

#[test]
fn sends_down_and_exit_messages_and_records_the_root_result() {
    let mut scheduler = Scheduler::new(AtomTable::new());
    scheduler.start(frame(), vec![]).unwrap();
    scheduler.schedule_next();
    scheduler.current_process_mut().trap_exit = true;
    let (monitored, monitor) = scheduler
        .spawn(Start::Entry(frame()), vec![], true, false, false)
        .unwrap();
    let (linked, _) = scheduler
        .spawn(Start::Entry(frame()), vec![], false, true, false)
        .unwrap();
    scheduler.park_current();

    scheduler.schedule_next();
    scheduler.finish_current(atom("normal"), None).unwrap();
    scheduler.schedule_next();
    scheduler.finish_current(atom("boom"), None).unwrap();

    assert_eq!(scheduler.schedule_next(), Some(ProcessId(0)));
    assert!(scheduler.current_process_mut().links.is_empty());
    assert_eq!(
        [
            first_message(&scheduler, 0, 0),
            first_message(&scheduler, 0, 1)
        ],
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
    scheduler
        .finish_current(atom("normal"), Some(atom("ok")))
        .unwrap();
    assert!(scheduler.processes.is_empty());
    assert_eq!(scheduler.into_root_result(), Some(Ok(atom("ok"))));
}

#[test]
fn links_and_unlinks_processes_and_ignores_normal_exit_without_trap_exit() {
    let mut scheduler = Scheduler::new(AtomTable::new());
    scheduler.start(frame(), vec![]).unwrap();
    scheduler.schedule_next();
    let (child, _) = scheduler
        .spawn(Start::Entry(frame()), vec![], false, false, false)
        .unwrap();
    assert_eq!(scheduler.link(child).unwrap(), atom("true"));
    assert!(scheduler.link(ProcessId(99)).is_err());
    scheduler.unlink(child);
    assert!(scheduler.current_process_mut().links.is_empty());
    assert!(scheduler.processes[&child].links.is_empty());
    scheduler.unlink(ProcessId(99));
    scheduler.unlink(ProcessId(0));
    assert_eq!(scheduler.link(child).unwrap(), atom("true"));
    scheduler.park_current();

    scheduler.schedule_next();
    assert!(
        scheduler
            .current_process_mut()
            .links
            .contains(&ProcessId(0))
    );
    scheduler.finish_current(atom("normal"), None).unwrap();

    assert_eq!(scheduler.schedule_next(), None);
    let root = &scheduler.processes[&ProcessId(0)];
    assert!(root.links.is_empty());
    assert!(root.mailbox.messages().is_empty());
}

#[test]
fn rejects_unsupported_exit_signals_and_counter_overflow() {
    let mut scheduler = Scheduler::new(AtomTable::new());
    scheduler.start(frame(), vec![]).unwrap();
    scheduler.schedule_next();
    scheduler
        .spawn(Start::Entry(frame()), vec![], false, true, false)
        .unwrap();
    scheduler.park_current();
    scheduler.schedule_next();
    assert!(scheduler.finish_current(atom("boom"), None).is_err());

    let mut scheduler = Scheduler::new(AtomTable::new());
    scheduler.start(frame(), vec![]).unwrap();
    scheduler.schedule_next();
    scheduler
        .finish_current(atom("boom"), Some(atom("ok")))
        .unwrap();
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
fn removes_active_monitors_and_sets_group_leaders() {
    let mut scheduler = Scheduler::new(AtomTable::new());
    scheduler.start(frame(), vec![]).unwrap();
    scheduler.schedule_next();
    let (child, _) = scheduler
        .spawn(Start::Entry(frame()), vec![], false, false, false)
        .unwrap();
    assert_eq!(scheduler.processes[&child].group_leader, ProcessId(0));
    assert!(scheduler.set_group_leader(child, child));
    assert_eq!(scheduler.processes[&child].group_leader, child);
    assert!(!scheduler.set_group_leader(ProcessId(99), child));

    let reference = scheduler
        .monitor(MonitorTarget::Process(child), no_options())
        .unwrap();
    assert_eq!(scheduler.processes[&child].monitored_by.len(), 1);
    assert!(scheduler.demonitor(reference));
    assert!(scheduler.processes[&child].monitored_by.is_empty());
    assert!(!scheduler.demonitor(reference));
}

#[test]
fn creates_no_monitor_of_the_caller_and_sends_noproc_for_a_missing_target() {
    // ERTS 29.1.1: `monitor(process, self())` creates no monitor
    // (`bif.c:781-784`), and a missing target gets the noproc message.
    let mut scheduler = Scheduler::new(AtomTable::new());
    scheduler.start(frame(), vec![]).unwrap();
    scheduler.schedule_next();
    let own = scheduler
        .monitor(MonitorTarget::Process(ProcessId(0)), no_options())
        .unwrap();
    assert!(scheduler.processes[&ProcessId(0)].monitored_by.is_empty());
    assert!(!scheduler.demonitor(own));
    let gone = scheduler
        .monitor(MonitorTarget::Process(ProcessId(9)), no_options())
        .unwrap();
    let name = Atom::from_static("undefined").unwrap();
    let port = scheduler
        .monitor(MonitorTarget::PortName(name), no_options())
        .unwrap();
    let named = Term::Tuple(vec![atom("undefined"), atom("nonode@nohost")]);
    assert_eq!(
        scheduler.processes[&ProcessId(0)].mailbox.messages(),
        [
            Term::Tuple(vec![
                atom("DOWN"),
                Term::Reference(gone),
                atom("process"),
                Term::Pid(ProcessId(9)),
                atom("noproc"),
            ]),
            Term::Tuple(vec![
                atom("DOWN"),
                Term::Reference(port),
                atom("port"),
                named,
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
    scheduler.schedule_next();
    let (child, _) = scheduler
        .spawn(Start::Entry(frame()), vec![], false, false, false)
        .unwrap();
    let name = atom("pbeam_probe_reg");
    assert_eq!(scheduler.whereis("pbeam_probe_reg"), None);
    assert!(scheduler.register(&name, &Term::Pid(child)));
    assert!(!scheduler.register(&name, &Term::Pid(ProcessId(0))));
    assert_eq!(scheduler.whereis("pbeam_probe_reg"), Some(child));
    scheduler.park_current();

    scheduler.schedule_next();
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
    let mut scheduler = Scheduler::new(AtomTable::new());
    scheduler.start(frame(), vec![]).unwrap();
    scheduler.schedule_next();
    let (child, monitor) = scheduler
        .spawn(Start::Entry(frame()), vec![], true, false, false)
        .unwrap();
    scheduler.park_current();

    scheduler.schedule_next();
    let table = scheduler.make_ref().unwrap();
    scheduler.ets.create(
        table,
        Some("ets_probe_owned"),
        Table::new(child, Access::Protected, 1),
    );
    assert!(scheduler.ets.name_in_use("ets_probe_owned"));
    scheduler.finish_current(atom("normal"), None).unwrap();

    assert!(!scheduler.ets.name_in_use("ets_probe_owned"));
    assert_eq!(
        first_message(&scheduler, 0, 0),
        Term::Tuple(vec![
            atom("DOWN"),
            Term::Reference(monitor.unwrap()),
            atom("process"),
            Term::Pid(child),
            atom("normal"),
        ])
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
    scheduler.schedule_next();
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

    assert_eq!(scheduler.schedule_next(), Some(ProcessId(0)));
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
    scheduler.schedule_next();
    assert!(scheduler.set_receive_timer(1_000_000_000, &Term::Integer(1), 3));
    scheduler.park_current();
    scheduler.expire_timers(1_001_000_000);
    assert_eq!(scheduler.schedule_next(), Some(ProcessId(0)));

    assert!(scheduler.set_receive_timer(1_000_000_000, &Term::Integer(1), 3));
    scheduler.cancel_receive_timer();
    assert_eq!(scheduler.timers.wait_time(1_000_000_000), None);
    assert!(scheduler.set_receive_timer(1_000_000_000, &Term::Integer(1), 3));
    scheduler.finish_current(atom("normal"), None).unwrap();
    assert_eq!(scheduler.timers.wait_time(1_000_000_000), None);
}

#[test]
fn rejects_receive_timeouts_as_erts_set_proc_timer_term() {
    // `erts_set_proc_timer_term` (`erl_hl_timer.c:2786-2805`) accepts an
    // integer from 0 to 4294967295 whose end is not after
    // `ERTS_MONOTONIC_END`.
    let mut scheduler = Scheduler::new(AtomTable::new());
    scheduler.start(frame(), vec![]).unwrap();
    scheduler.schedule_next();
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

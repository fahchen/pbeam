//! Work stealing between the schedulers of a node: the choice of the
//! processes that move, the owner in the process table, and the order of
//! the deliveries around a move.

use std::sync::Arc;

use super::super::{Delivery, Scheduler, node::Node, run_queue::RunQueue};
use crate::runtime::dirty::Claim;
use crate::runtime::process::{
    DirtyCall, Priority, ProcessState, ReceiveTimer, Start, tests::frame,
};
use crate::runtime::signal::Signal;
use crate::term::{AtomTable, ProcessId, Term};

/// Return the two schedulers of a node with two schedulers.
fn two_schedulers() -> (Arc<Node>, Scheduler, Scheduler) {
    let (node, mut readers) = Node::new(2).unwrap();
    let first = Scheduler::with_shared(
        Arc::new(AtomTable::new()),
        Arc::clone(&node),
        0,
        readers.remove(0),
    );
    let second = Scheduler::with_shared(
        Arc::new(AtomTable::new()),
        Arc::clone(&node),
        1,
        readers.remove(0),
    );
    (node, first, second)
}

/// Put the runnable processes `ids` into the scheduler `scheduler`, in this
/// order of its run queue.
fn insert(scheduler: &mut Scheduler, ids: &[u64]) {
    for &id in ids {
        scheduler.insert(ProcessState::new(
            ProcessId(id),
            Start::Entry(frame()),
            vec![],
        ));
    }
}

/// Return the state of the process `id` of the scheduler `scheduler`.
fn process(scheduler: &mut Scheduler, id: u64) -> &mut ProcessState {
    scheduler.processes.get_mut(&ProcessId(id)).unwrap()
}

/// Return the pid of each delivery of the inbox of `index`, with `P` for a
/// process, `M` for a message, `S` for a signal, and `C` with 0 for a timer
/// cancel.
fn inbox(node: &Node, index: usize) -> Vec<(char, u64)> {
    let mut deliveries = Vec::new();
    node.take(index, &mut deliveries);
    deliveries
        .iter()
        .map(|delivery| match delivery {
            Delivery::Process { process, .. } => ('P', process.id.0),
            Delivery::Message { target, .. } => ('M', target.0),
            Delivery::Signal { target, .. } => ('S', target.0),
            Delivery::Cleanup { target } => ('C', target.0),
            Delivery::CancelTimer { .. } => ('T', 0),
            Delivery::CheckProcessCode { target, .. } => ('K', target.0),
        })
        .collect()
}

/// A scheduler with four runnable processes gives two of them, from the
/// head of its queue, to the hungry scheduler, and skips a pinned process.
/// The owner in the process table moves with the state, and the thief runs
/// the processes in their order.
#[test]
fn gives_half_of_the_queue_to_a_hungry_scheduler() {
    let (node, mut victim, mut thief) = two_schedulers();
    insert(&mut victim, &[1, 2, 3, 4]);
    victim.processes.get_mut(&ProcessId(1)).unwrap().pinned = true;
    node.set_hungry(1, true);
    victim.serve_steal();
    assert_eq!(node.owner(ProcessId(2)), Some(1));
    assert_eq!(node.owner(ProcessId(3)), Some(1));
    assert_eq!(node.owner(ProcessId(1)), Some(0));
    assert_eq!(node.owner(ProcessId(4)), Some(0));
    assert!(!victim.processes.contains_key(&ProcessId(2)));
    assert_eq!(victim.runnable.ids(), vec![ProcessId(1), ProcessId(4)]);
    assert!(!node.has_hungry(0), "the steal cleared the mark");
    assert!(thief.drain_inbox());
    assert_eq!(thief.runnable.ids(), vec![ProcessId(2), ProcessId(3)]);
    assert!(thief.processes.contains_key(&ProcessId(3)));
}

/// A scheduler with one runnable process keeps it, and the mark of the
/// hungry scheduler stays.
#[test]
fn keeps_the_only_runnable_process() {
    let (node, mut victim, _thief) = two_schedulers();
    insert(&mut victim, &[1]);
    node.set_hungry(1, true);
    victim.serve_steal();
    assert_eq!(node.owner(ProcessId(1)), Some(0));
    assert!(node.has_hungry(0));
}

/// Without a hungry scheduler, no process moves.
#[test]
fn gives_nothing_without_a_hungry_scheduler() {
    let (node, mut victim, _thief) = two_schedulers();
    insert(&mut victim, &[1, 2]);
    victim.serve_steal();
    assert_eq!(victim.runnable.ids(), vec![ProcessId(1), ProcessId(2)]);
    assert!(inbox(&node, 1).is_empty());
}

/// A process that waits, exits, waits for a receive timer, used a
/// scheduler service, is a system process, or is in
/// a dirty call stays with its scheduler, also when the thief is hungry.
#[test]
fn keeps_each_process_that_cannot_move() {
    let (node, mut victim, _thief) = two_schedulers();
    insert(&mut victim, &[1, 2, 3, 4, 5, 6, 7]);
    process(&mut victim, 1).waiting = true;
    process(&mut victim, 2).exiting = Some(Term::static_atom("kill"));
    process(&mut victim, 3).receive_timer = ReceiveTimer::Set {
        key: (1100, 0),
        resume: 7,
    };
    process(&mut victim, 4).pinned = true;
    process(&mut victim, 5).pinned = true;
    victim.mark_system(ProcessId(6));
    // The dirty call starts after the process is in the run queue, as a
    // process in a dirty call that has signals to handle is.
    process(&mut victim, 7).dirty_call = Some(DirtyCall {
        call: 0,
        entry: (String::new(), String::new(), Vec::new()),
        output: None,
        claim: Claim::default(),
    });
    node.set_hungry(1, true);
    victim.serve_steal();
    assert!(inbox(&node, 1).is_empty());
    assert_eq!(victim.runnable.len(), 7);
    assert!(node.has_hungry(0), "a steal with no process keeps the mark");
}

/// A process that uses a scheduler service in its turn is pinned at its
/// switch out. A borrow outside the turn of a process pins no process.
#[test]
fn a_borrow_of_a_scheduler_service_pins_the_process() {
    let (_node, mut scheduler, _thief) = two_schedulers();
    insert(&mut scheduler, &[1, 2]);
    scheduler.resources.put(0u8);
    let _ = scheduler.resources.borrow_mut::<u8>();
    assert_eq!(scheduler.schedule_next().unwrap(), super::super::Turn::Run);
    scheduler.yield_current();
    assert!(!scheduler.processes[&ProcessId(1)].pinned);
    assert_eq!(scheduler.schedule_next().unwrap(), super::super::Turn::Run);
    let _ = scheduler.resources.borrow_mut::<u8>();
    scheduler.yield_current();
    assert!(scheduler.processes[&ProcessId(2)].pinned);
}

/// The deliveries for a process that wait in the inbox of its old
/// scheduler follow it, in their order, after the process, and the
/// deliveries for other processes stay. A sender that read the old owner
/// before the move gives its next delivery to the new owner, after them.
#[test]
fn deliveries_in_flight_follow_the_process_in_order() {
    let (node, mut victim, _thief) = two_schedulers();
    insert(&mut victim, &[1, 2]);
    let sender = Some(ProcessId(9));
    node.send_message(0, sender, Term::Integer(1), ProcessId(1), None);
    node.send_message(0, sender, Term::Integer(2), ProcessId(2), None);
    node.send_signal(
        0,
        ProcessId(1),
        sender,
        Signal::Unlink {
            from: ProcessId(9),
            id: 1,
        },
        None,
    );
    node.schedule_cleanup(0, ProcessId(1));
    let module = crate::term::Atom::from_static("erlang").unwrap();
    node.check_process_code(0, ProcessId(1), ProcessId(9), Term::Nil, module);
    let process = victim.processes.remove(&ProcessId(1)).unwrap();
    node.transfer(0, 1, Box::new(process), None);
    node.send_message(0, sender, Term::Integer(3), ProcessId(1), None);
    assert_eq!(node.owner(ProcessId(1)), Some(1));
    assert_eq!(inbox(&node, 0), vec![('M', 2)]);
    assert_eq!(
        inbox(&node, 1),
        vec![('P', 1), ('M', 1), ('S', 1), ('C', 1), ('K', 1), ('M', 1)]
    );
}

/// A delivery for a process that left the table goes to the scheduler that
/// the sender found, which handles it as a delivery to a dead process.
#[test]
fn a_delivery_for_a_dead_process_goes_to_the_scheduler_of_the_sender() {
    let (node, _first, _second) = two_schedulers();
    node.send_signal(
        1,
        ProcessId(5),
        None,
        Signal::Unlink {
            from: ProcessId(9),
            id: 1,
        },
        None,
    );
    assert_eq!(inbox(&node, 1), vec![('S', 5)]);
}

/// The hungry scheduler after the giver in the cyclic order of the indices
/// gets the work. Without a hungry scheduler, the giver gives nothing.
#[test]
fn gives_work_to_the_next_hungry_scheduler_in_cyclic_order() {
    let (node, _readers) = Node::new(3).unwrap();
    node.register(ProcessId(1), 2);
    node.set_hungry(0, true);
    node.set_hungry(1, true);
    let process = ProcessState::new(ProcessId(1), Start::Entry(frame()), vec![]);
    assert_eq!(node.give_work(2, || vec![(process, None)]), 1);
    assert_eq!(node.owner(ProcessId(1)), Some(0));
    assert_eq!(inbox(&node, 0), vec![('P', 1)]);
    assert!(node.has_hungry(2), "scheduler 1 waits");
    assert_eq!(node.give_work(2, Vec::new), 0);
    assert!(node.has_hungry(2), "a give with no process keeps the mark");
    node.set_hungry(1, false);
    assert!(!node.has_hungry(2));
    assert_eq!(
        node.give_work(2, || unreachable!("no scheduler is hungry")),
        0
    );
}

/// A steal takes from the queue of the highest priority that gives a
/// process, at most half of it and at most 100 processes. The `normal` queue holds the `low`
/// processes.
#[test]
fn a_steal_takes_half_of_the_highest_queue() {
    let mut queue = RunQueue::default();
    assert!(queue.steal(|_| true).is_empty());
    queue.push(ProcessId(1), Priority::Low);
    queue.push(ProcessId(2), Priority::Normal);
    queue.push(ProcessId(3), Priority::High);
    // The high queue runs next and gives 1 / 2, so the search goes on to
    // the normal queue, which gives (2 + 1) / 2.
    assert_eq!(queue.steal(|_| true), vec![(ProcessId(1), Some(8))]);
    assert_eq!(queue.pop(), Some(ProcessId(3)));
    assert_eq!(queue.pop(), Some(ProcessId(2)));
    queue.push(ProcessId(4), Priority::Normal);
    queue.push(ProcessId(5), Priority::Normal);
    assert_eq!(queue.steal(|_| true), vec![(ProcessId(4), None)]);
    assert_eq!(queue.pop(), Some(ProcessId(5)));
    for id in 0..250 {
        queue.push(ProcessId(id), Priority::Max);
    }
    assert_eq!(queue.steal(|_| true).len(), 100);
    assert_eq!(queue.len(), 150);
}

/// A stolen `low` process keeps the count that it has left, and one with no
/// count runs next.
#[test]
fn a_stolen_low_process_keeps_its_count() {
    let mut queue = RunQueue::default();
    queue.push_stolen(ProcessId(1), Priority::Low, Some(2));
    queue.push_stolen(ProcessId(2), Priority::Normal, None);
    assert_eq!(queue.pop(), Some(ProcessId(2)), "the count of 1 skips it");
    queue.push_stolen(ProcessId(3), Priority::Low, None);
    assert_eq!(queue.pop(), Some(ProcessId(1)));
    assert_eq!(queue.pop(), Some(ProcessId(3)));
}

/// The first stolen process gives no count, because it runs next on the
/// receiver, and each other `low` process gives the count that it has left.
#[test]
fn the_first_stolen_process_gives_no_count() {
    let (node, mut victim, _thief) = two_schedulers();
    for id in 1..=4 {
        let mut state = ProcessState::new(ProcessId(id), Start::Entry(frame()), vec![]);
        state.priority = Priority::Low;
        victim.insert(state);
    }
    node.set_hungry(1, true);
    victim.serve_steal();
    let mut deliveries = Vec::new();
    node.take(1, &mut deliveries);
    let counts: Vec<_> = deliveries
        .iter()
        .map(|delivery| match delivery {
            Delivery::Process { process, low } => (process.id, *low),
            _ => unreachable!("only processes move"),
        })
        .collect();
    assert_eq!(counts, vec![(ProcessId(1), None), (ProcessId(2), Some(8))]);
}

/// A pinned `high` process that the scheduler runs next gives no process,
/// so the steal takes from the `normal` queue: the idle scheduler gets one
/// of the two `normal` processes, as ERTS goes on to the next queue
/// (`erl_process.c:4489-4592`).
#[test]
fn a_queue_that_gives_nothing_lets_the_steal_take_a_lower_queue() {
    let (node, mut victim, _thief) = two_schedulers();
    let mut high = ProcessState::new(ProcessId(1), Start::Entry(frame()), vec![]);
    high.priority = Priority::High;
    high.pinned = true;
    victim.insert(high);
    insert(&mut victim, &[2, 3]);
    node.set_hungry(1, true);
    victim.serve_steal();
    assert_eq!(node.owner(ProcessId(2)), Some(1));
    assert_eq!(victim.runnable.ids(), vec![ProcessId(1), ProcessId(3)]);
    assert_eq!(inbox(&node, 1), vec![('P', 2)]);
}

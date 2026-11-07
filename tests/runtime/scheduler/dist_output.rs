//! Distribution port queue states that cannot be forced by a BEAM fixture.

use std::{cell::RefCell, sync::Arc};

use socket2::{Domain, Socket, Type};

use super::{Controller, Scheduler};
use crate::runtime::scheduler::{MonitorOptions, MonitorTarget};
use crate::{
    runtime::{
        port::{
            Driver,
            inet::{Inet, SocketType},
        },
        process::{
            MonitorKey, ProcessState, RemotePid, SpawnLink, SpawnReply, Start, tests::frame,
        },
        scheduler::node::Node,
    },
    term::{AtomTable, Binary, ProcessId, Reference, Term},
};

thread_local! {
    static AFTER_IDLE_SEND_CHECK: RefCell<Option<Box<dyn FnOnce()>>> = RefCell::new(None);
    static AFTER_CONNECTED_SIGNAL_CHECK: RefCell<Option<Box<dyn FnOnce()>>> = RefCell::new(None);
}

/// Run the test-only interleaving at the send's idle-check boundary.
pub(super) fn after_idle_send_check() {
    AFTER_IDLE_SEND_CHECK.with(|hook| {
        if let Some(hook) = hook.borrow_mut().take() {
            hook();
        }
    });
}

/// Run the test-only transition between a signal's checks.
pub(super) fn after_connected_signal_check() {
    AFTER_CONNECTED_SIGNAL_CHECK.with(|hook| {
        if let Some(hook) = hook.borrow_mut().take() {
            hook();
        }
    });
}

fn scheduler() -> Scheduler {
    let (node, mut readers) = Node::new(1).unwrap();
    let mut scheduler =
        Scheduler::with_shared(Arc::new(AtomTable::new()), node, 0, readers.remove(0));
    scheduler.insert(ProcessState::new(
        ProcessId(0),
        Start::Entry(frame()),
        vec![],
    ));
    scheduler.schedule_next().unwrap();
    scheduler
}

fn scheduler_with_kernel() -> Scheduler {
    let mut scheduler = scheduler();
    scheduler.insert(ProcessState::new(
        ProcessId(1),
        Start::Entry(frame()),
        vec![],
    ));
    let node = Arc::clone(&scheduler.node);
    assert!(node.name_register("net_kernel", ProcessId(1)));
    let local = scheduler.atoms.put("coverage_local@nohost").unwrap();
    assert!(node.set_this_node(&scheduler.atoms, ProcessId(1), &local, 4));
    scheduler
}

#[test]
fn rejects_auto_connect_when_the_connection_changes_before_its_lock() {
    // A BEAM fixture cannot pause between the caller's state check and
    // dist_auto_connect's entry lock. Model the legal connected, shutdown,
    // and exiting-owner transitions directly at that boundary (dist.c:5713-5736).
    let mut scheduler = scheduler_with_kernel();
    let name = scheduler.atoms.put("coverage_connected@nohost").unwrap();
    {
        let mut dist = scheduler.node.dist();
        dist.insert(&name, Reference(8), 0);
        dist.new_connection(&name);
        assert!(
            dist.connect(&name, Controller::Process(ProcessId(0)), 0)
                .is_some()
        );
    }
    let mut queued = false;
    assert_eq!(
        scheduler
            .dist_auto_connect(&name, |_, _| queued = true)
            .unwrap(),
        None
    );
    assert!(!queued);

    let mut scheduler = scheduler_with_kernel();
    let name = scheduler.atoms.put("coverage_shutdown@nohost").unwrap();
    let node = Arc::clone(&scheduler.node);
    node.begin_net_exits(&Term::static_atom("net_kernel_terminated"));
    let _ = node.net_exits(ProcessId(1), &scheduler.atoms);
    assert!(node.is_going_down());
    assert_eq!(
        scheduler
            .dist_auto_connect(&name, |_, _| queued = true)
            .unwrap(),
        None
    );
    assert!(!queued);

    let mut scheduler = scheduler_with_kernel();
    let name = scheduler.atoms.put("coverage_exiting@nohost").unwrap();
    scheduler.node.set_exiting(ProcessId(1));
    assert_eq!(
        scheduler
            .dist_auto_connect(&name, |_, _| queued = true)
            .unwrap(),
        None
    );
    assert!(!queued);
}

#[test]
fn spawn_joins_a_new_pending_connection_after_the_connected_check() {
    // A BEAM fixture cannot pause between the first connection check and
    // the output lock. Force that transition here. Pinned OTP returns
    // {true,none,29}: a reference, no early error, and SPAWN_REQUEST
    // (dist.c:3247-3255, 6081-6120).
    let mut scheduler = scheduler_with_kernel();
    let node = scheduler.atoms.put("spawn_race@nohost").unwrap();
    let shared = Arc::clone(&scheduler.node);
    {
        let mut dist = shared.dist();
        dist.insert(&node, Reference(8), 0);
        dist.new_connection(&node);
        assert!(
            dist.connect(&node, Controller::Process(ProcessId(0)), 0)
                .is_some()
        );
    }
    let other = Arc::clone(&shared);
    let other_name = node.clone();
    AFTER_CONNECTED_SIGNAL_CHECK.with(|hook| {
        *hook.borrow_mut() = Some(Box::new(move || {
            let mut dist = other.dist();
            assert!(
                dist.controller_exits(Controller::Process(ProcessId(0)))
                    .is_some()
            );
            dist.new_connection(&other_name);
        }));
    });
    let module = scheduler.atoms.put("erlang").unwrap();
    let function = scheduler.atoms.put("node").unwrap();
    let reply = SpawnReply {
        no_smsg: false,
        no_emsg: false,
    };
    let (reference, monitored) = scheduler
        .dist_spawn_request(
            &node,
            &module,
            &function,
            0,
            &Term::nil(),
            Term::nil(),
            None,
            &Term::static_atom("spawn_reply"),
            reply,
            SpawnLink::No,
        )
        .unwrap();
    assert!(!monitored);
    let key = MonitorKey::Local(reference);
    assert!(scheduler.current_process().monitors.get(&key).is_some());
    assert_eq!(
        scheduler.current_process().mailbox.messages(),
        Vec::<Term>::new()
    );
    let mut dist = shared.dist();
    assert!(
        dist.connect(&node, Controller::Process(ProcessId(0)), 1 << 32)
            .is_some()
    );
    scheduler
        .dist_flush_pending(&mut dist, &node, 1 << 32)
        .unwrap();
    let packet = dist
        .dequeue(&node)
        .expect("the spawn request remains queued");
    let ended = dist
        .controller_exits(Controller::Process(ProcessId(0)))
        .unwrap();
    assert_eq!(ended.monitors.len(), 1);
    assert_eq!(ended.monitors[0].key, key);
    drop(dist);
    let bytes: Vec<_> = packet.elements.iter().flat_map(Binary::to_bytes).collect();
    assert_eq!(bytes[0], 112);
    let (control, _) = crate::runtime::interpreter::bif::decode_dist(&scheduler, &bytes[1..], None)
        .unwrap()
        .unwrap();
    let Term::Tuple(ref fields) = control else {
        panic!("control is a tuple");
    };
    assert_eq!(fields[0].small_integer(), Some(29));
}

#[test]
fn exit_link_and_monitor_join_a_new_pending_connection() {
    // A compiled fixture cannot pause between the connected check and the
    // output lock. Force a disconnect and a new pending connection there.
    // Pinned OTP emits [8,1,19] for exit, link, and monitor on a pending
    // connection (dist.c:3247-3255, bif.c:342-350, 851-857, 1812-1823).
    let mut operations = Vec::new();
    for operation in 0..3 {
        let mut scheduler = scheduler_with_kernel();
        let node = scheduler.atoms.put("signal_race@nohost").unwrap();
        let remote = RemotePid {
            node: node.index(),
            creation: 1,
            number: 1,
            serial: 0,
        };
        let target = remote.term();
        let shared = Arc::clone(&scheduler.node);
        {
            let mut dist = shared.dist();
            dist.insert(&node, Reference(8), 0);
            dist.new_connection(&node);
            assert!(
                dist.connect(&node, Controller::Process(ProcessId(0)), 0)
                    .is_some()
            );
        }
        let other = Arc::clone(&shared);
        let other_name = node.clone();
        AFTER_CONNECTED_SIGNAL_CHECK.with(|hook| {
            *hook.borrow_mut() = Some(Box::new(move || {
                let mut dist = other.dist();
                assert!(
                    dist.controller_exits(Controller::Process(ProcessId(0)))
                        .is_some()
                );
                dist.new_connection(&other_name);
            }));
        });
        match operation {
            0 => {
                let reason = Term::Atom(scheduler.atoms.put("boom").unwrap());
                assert!(
                    scheduler
                        .dist_exit(&node, &target, &reason, super::SendPriority::Normal)
                        .unwrap()
                );
            }
            1 => scheduler.dist_link(&target).unwrap(),
            _ => {
                let options = MonitorOptions {
                    alias: None,
                    tag: Term::static_atom("DOWN"),
                    priority: false,
                };
                scheduler
                    .monitor(&MonitorTarget::RemoteProcess(remote), options)
                    .unwrap();
            }
        }
        let mut dist = shared.dist();
        assert!(
            dist.connect(&node, Controller::Process(ProcessId(0)), 0x28)
                .is_some()
        );
        scheduler
            .dist_flush_pending(&mut dist, &node, 0x28)
            .unwrap();
        let packet = dist.dequeue(&node).expect("the pending signal is retained");
        let ended = dist
            .controller_exits(Controller::Process(ProcessId(0)))
            .unwrap();
        assert_eq!(ended.links.len(), usize::from(operation == 1));
        assert_eq!(ended.monitors.len(), usize::from(operation == 2));
        drop(dist);
        let bytes: Vec<_> = packet.elements.iter().flat_map(Binary::to_bytes).collect();
        assert_eq!(bytes[0], 112);
        let (control, _) =
            crate::runtime::interpreter::bif::decode_dist(&scheduler, &bytes[1..], None)
                .unwrap()
                .unwrap();
        let Term::Tuple(ref fields) = control else {
            panic!("control is a tuple");
        };
        operations.push(fields[0].small_integer().unwrap());
    }
    assert_eq!(operations, [8, 1, 19]);
}

#[test]
fn noconnect_send_joins_a_connection_started_after_the_idle_check() {
    // A BEAM fixture cannot pause a scheduler between the idle check and
    // the fallback. Force that transition here. Pinned OTP returns
    // {noconnect,ok,hello} for idle, pending, and the delivered message
    // (dist.c:3247-3271, bif.c:2391-2417).
    let mut scheduler = scheduler();
    let node = scheduler.atoms.put("ncrace_peer@nohost").unwrap();
    let destination = Term::tuple(vec![
        Term::Atom(scheduler.atoms.put("sink").unwrap()),
        Term::Atom(node.clone()),
    ]);
    let message = Term::Atom(scheduler.atoms.put("hello").unwrap());
    let node_ref = Arc::clone(&scheduler.node);
    let local = scheduler.atoms.put("ncrace_local@nohost").unwrap();
    assert!(node_ref.set_this_node(&scheduler.atoms, ProcessId(0), &local, 4));
    node_ref.dist().insert(&node, Reference(8), 0);
    assert!(
        !scheduler
            .dist_send(
                &node,
                &destination,
                &message,
                super::SendPriority::Normal,
                super::ConnectMode::NoConnect
            )
            .unwrap()
    );
    // Force the other scheduler's transition before this send takes its
    // fallback lock; a compiled BEAM fixture cannot pause at this boundary.
    let other_node = Arc::clone(&node_ref);
    let other_name = node.clone();
    AFTER_IDLE_SEND_CHECK.with(|hook| {
        *hook.borrow_mut() = Some(Box::new(move || {
            other_node.dist().new_connection(&other_name);
        }));
    });
    assert!(
        scheduler
            .dist_send(
                &node,
                &destination,
                &message,
                super::SendPriority::Normal,
                super::ConnectMode::NoConnect
            )
            .unwrap()
    );
    let mut dist = node_ref.dist();
    assert!(
        dist.connect(&node, Controller::Process(ProcessId(0)), 0)
            .is_some()
    );
    scheduler.dist_flush_pending(&mut dist, &node, 0).unwrap();
    let packet = dist.dequeue(&node).unwrap();
    drop(dist);
    let bytes: Vec<_> = packet.elements.iter().flat_map(Binary::to_bytes).collect();
    assert_eq!(bytes[0], 112);
    let (_, used) = crate::runtime::interpreter::bif::decode_dist(&scheduler, &bytes[1..], None)
        .unwrap()
        .unwrap();
    let (delivered, _) =
        crate::runtime::interpreter::bif::decode_dist(&scheduler, &bytes[1 + used..], None)
            .unwrap()
            .unwrap();
    assert_eq!(delivered, message);
}

#[test]
fn drops_a_queued_drain_after_the_controller_is_detached() {
    // Another scheduler can detach a connection while this scheduler waits
    // for its port lock. A BEAM fixture cannot force that interleaving.
    let mut scheduler = scheduler();
    let port = scheduler
        .open_port(
            b"tcp_inet".to_vec(),
            Driver::Inet(Inet::new(SocketType::Stream)),
        )
        .unwrap();
    let node = scheduler.atoms.put("detached_controller@host").unwrap();
    {
        let mut dist = scheduler.node.dist();
        dist.insert(&node, Reference(8), 0);
        dist.new_connection(&node);
        scheduler.node.add_connection();
        let controller = Controller::Port(port);
        assert!(dist.connect(&node, controller, 0).is_some());
        assert!(dist.controller_exits(controller).is_some());
        assert!(dist.connected(&node).is_none());
    }
    scheduler.dist_port_send(port).unwrap();
    assert!(scheduler.node.port(port).is_some());
}

#[test]
fn closes_a_distribution_port_when_the_write_fails() {
    // The peer closes after the packet queues but before the port writes it.
    // The BEAM fixture has no way to hold the port at that boundary.
    let mut scheduler = scheduler();
    let (socket, peer) = Socket::pair(Domain::UNIX, Type::STREAM, None).unwrap();
    let mut inet = Inet::new(SocketType::Stream).copy(socket);
    inet.control(0x03f1_a307, &[22, 0, 0, 0, 1], ProcessId(0))
        .unwrap();
    let port = scheduler
        .open_port(b"tcp_inet".to_vec(), Driver::Inet(inet))
        .unwrap();
    let node = scheduler.atoms.put("write_failure@host").unwrap();
    {
        let mut dist = scheduler.node.dist();
        dist.insert(&node, Reference(7), 0);
        dist.new_connection(&node);
        scheduler.node.add_connection();
        assert!(dist.connect(&node, Controller::Port(port), 0).is_some());
        dist.enqueue(&node, vec![Binary::from(vec![1])]);
    }
    drop(peer);
    scheduler.dist_port_send(port).unwrap();
    assert!(scheduler.node.dist().connected(&node).is_none());
}

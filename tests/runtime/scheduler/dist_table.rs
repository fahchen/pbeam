use super::{Controller, DistTable};
use crate::runtime::process::RemotePid;
use crate::term::{AtomTable, PortId, ProcessId, Reference, Term};

#[test]
fn returns_the_links_of_the_connection_that_ended() {
    // erts_do_net_exits detaches the `links` list of a dist entry and sends
    // the link exit signal `noconnection` to each local process of it
    // (dist.c:1021-1117, 341-345).
    let atoms = AtomTable::new();
    let a = atoms.put("a@h").unwrap();
    let remote = RemotePid {
        node: a.index(),
        creation: 1,
        number: 2,
        serial: 0,
    };
    let mut table = DistTable::default();
    table.insert(&a, Reference(7), 0);
    table.new_connection(&a);
    let controller = Controller::Process(ProcessId(1));
    assert!(table.connect(&a, controller, 0).is_some());
    assert!(table.insert_link(ProcessId(1), remote, 1));
    assert!(table.insert_link(ProcessId(2), remote, 1));
    let ended = table
        .controller_exits(controller)
        .expect("the connection has links");
    assert_eq!(ended.connection, 1);
    assert_eq!(
        ended.links,
        vec![(ProcessId(2), remote), (ProcessId(1), remote)]
    );
    // A second exit has no controller, and the links are gone.
    assert!(table.controller_exits(controller).is_none());
}

#[test]
fn adds_a_link_only_to_its_own_connected_connection() {
    // `erts_link_dist_insert` fails when the connection is down, and a
    // duplicate LINK deletes its own dist link, so the connection keeps one
    // link of a pair (dist.c:2246-2258, erl_proc_sig_queue.c:6635-6663).
    let atoms = AtomTable::new();
    let a = atoms.put("a@h").unwrap();
    let remote = RemotePid {
        node: a.index(),
        creation: 1,
        number: 2,
        serial: 0,
    };
    let mut table = DistTable::default();
    table.insert(&a, Reference(7), 0);
    table.new_connection(&a);
    // The entry is pending, so it has no connection yet.
    assert!(!table.insert_link(ProcessId(1), remote, 1));
    let controller = Controller::Process(ProcessId(1));
    assert!(table.connect(&a, controller, 0).is_some());
    assert!(table.insert_link(ProcessId(1), remote, 1));
    // A link of another connection and another process do not belong here.
    assert!(!table.insert_link(ProcessId(2), remote, 2));
    assert!(!table.insert_link(ProcessId(1), remote, 2));
    // The duplicate keeps the single link at its position.
    assert!(table.insert_link(ProcessId(1), remote, 1));
    let ended = table.controller_exits(controller).unwrap();
    assert_eq!(ended.links, vec![(ProcessId(1), remote)]);
    // The entry is idle now.
    assert!(!table.insert_link(ProcessId(1), remote, 1));
}

#[test]
fn returns_the_links_of_each_connection_at_the_exit_of_net_kernel() {
    // The exit of `net_kernel` makes each pending entry idle and detaches
    // the `links` list of each connected entry (dist.c:926-1001).
    let atoms = AtomTable::new();
    let [a, b] = ["a@h", "b@h"].map(|name| atoms.put(name).unwrap());
    let mut table = DistTable::default();
    for (index, name) in [&a, &b].into_iter().enumerate() {
        table.insert(name, Reference(u64::try_from(index).unwrap()), 0);
        table.new_connection(name);
    }
    assert!(
        table
            .connect(&a, Controller::Process(ProcessId(1)), 0)
            .is_some()
    );
    assert!(
        table
            .connect(&b, Controller::Process(ProcessId(2)), 0)
            .is_some()
    );
    let remote = |name: &crate::term::Atom, number| RemotePid {
        node: name.index(),
        creation: 1,
        number,
        serial: 0,
    };
    assert!(table.insert_link(ProcessId(3), remote(&a, 4), 1));
    assert!(table.insert_link(ProcessId(5), remote(&b, 6), 1));
    let (ended, controllers) = table.net_kernel_exits();
    assert_eq!(controllers.len(), 2);
    assert!(ended.iter().all(|connection| connection.connection == 1));
    let links: Vec<_> = ended
        .iter()
        .map(|connection| connection.links.clone())
        .collect();
    assert_eq!(links.len(), 2);
    assert!(links.contains(&vec![(ProcessId(3), remote(&a, 4))]));
    assert!(links.contains(&vec![(ProcessId(5), remote(&b, 6))]));
}

#[test]
fn wraps_the_connection_id_at_24_bits() {
    // ERTS masks the connection id with ERTS_DIST_CON_ID_MASK
    // (external.h:137, erl_node_tables.c:162, 716).
    let atoms = AtomTable::new();
    let a = atoms.put("a@h").unwrap();
    let mut table = DistTable::default();
    table.insert(&a, Reference(7), 0xff_ff_ff_ff);
    assert_eq!(table.connection_id(&a), 0x00ff_ffff);
    table.new_connection(&a);
    assert_eq!(table.connection_id(&a), 0);
    // A pending entry keeps its connection id.
    table.new_connection(&a);
    assert_eq!(table.connection_id(&a), 0);
}

#[test]
fn keeps_the_low_32_bits_of_the_connection_id_of_a_handle() {
    // `unsigned_val` of a small integer and the `Uint32` of
    // erts_dhandle_to_dist_entry (erl_node_tables.c:393).
    let atoms = AtomTable::new();
    let a = atoms.put("a@h").unwrap();
    let mut table = DistTable::default();
    table.insert(&a, Reference(7), 0);
    let handle = |id: i64| Term::tuple(vec![Term::Integer(id), Term::Reference(Reference(7))]);
    assert_eq!(table.lookup(&handle(-1)), Some((a.clone(), u32::MAX)));
    assert_eq!(table.lookup(&handle(1 << 32 | 5)), Some((a.clone(), 5)));
    assert_eq!(table.lookup(&handle(1 << 60)), None);
    assert_eq!(
        table.lookup(&Term::tuple(vec![
            Term::Integer(0),
            Term::Reference(Reference(8))
        ])),
        None
    );
}

#[test]
fn orders_the_nodes_by_state_and_by_the_time_of_each_change() {
    // erlang:nodes/1,2 gives the pending, the idle, the hidden, and the
    // visible entries, each from the oldest change (dist.c:6356-6411).
    let atoms = AtomTable::new();
    let [nonode, local, a, b, c, d] =
        ["nonode@nohost", "l@h", "a@h", "b@h", "c@h", "d@h"].map(|name| atoms.put(name).unwrap());
    let mut table = DistTable::default();
    for (index, name) in [&a, &b, &c, &d].into_iter().enumerate() {
        table.insert(name, Reference(u64::try_from(index).unwrap()), 0);
        table.new_connection(name);
    }
    table.set_local(&nonode, Reference(10), &local, Reference(11), 0);
    assert!(
        table
            .connect(&d, Controller::Process(ProcessId(1)), 1)
            .is_some()
    );
    assert!(table.connect(&c, Controller::Port(PortId(2)), 0).is_some());
    assert!(
        table
            .connect(&b, Controller::Process(ProcessId(3)), 0)
            .is_some()
    );
    table.controller_exits(Controller::Port(PortId(2)));
    let nodes = table.nodes();
    let names: Vec<_> = nodes
        .iter()
        .map(|info| (info.name.clone(), info.kind))
        .collect();
    assert_eq!(
        names,
        vec![
            (a.clone(), "known"),
            (nonode, "known"),
            (local, "known"),
            (c, "known"),
            (b, "hidden"),
            (d, "visible"),
        ]
    );
    assert_eq!(nodes[0].connection_id, None);
    assert_eq!(nodes[4].connection_id, Some(1));
    // The exit of net_kernel gives each controller and makes the pending
    // entry `a` idle (dist.c:926-1001).
    let (ended, mut controllers) = table.net_kernel_exits();
    assert!(ended.is_empty());
    controllers.sort_by_key(|controller| match controller {
        Controller::Process(id) => id.0,
        Controller::Port(_) => u64::MAX,
    });
    assert_eq!(
        controllers,
        vec![
            Controller::Process(ProcessId(1)),
            Controller::Process(ProcessId(3))
        ]
    );
    assert_eq!(table.nodes()[0].kind, "known");
}

use super::DistTable;
use crate::term::{AtomTable, ProcessId, Reference, Term};

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
    assert!(table.connect(&d, ProcessId(1), 1).is_some());
    assert!(table.connect(&c, ProcessId(2), 0).is_some());
    assert!(table.connect(&b, ProcessId(3), 0).is_some());
    table.controller_exits(ProcessId(2));
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
    let mut controllers = table.net_kernel_exits();
    controllers.sort_by_key(|id| id.0);
    assert_eq!(controllers, vec![ProcessId(1), ProcessId(3)]);
    assert_eq!(table.nodes()[0].kind, "known");
}

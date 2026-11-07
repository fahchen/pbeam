//! The dist entries of the connections of a node, as the dist table of ERTS
//! keeps them (`erl_node_tables.c:150-371`, `630-792`).
//!
//! A dist entry has the name of the other node, a connection state, and a
//! connection id. `erts_internal:new_connection/1` makes an idle entry
//! pending, `erts_internal:create_dist_channel/3` makes a pending entry
//! connected with a process as its distribution controller, and
//! `erts_internal:abort_pending_connection/2` and the exit of the controller
//! make the entry idle again. A dist handle `{ConnId, MagicRef}` names an
//! entry and one of its connections (`erl_node_tables.c:381-416`).
//!
//! The dist table also holds send-only entries and their expiry timers
//! ([`super::known_nodes`]). PBeam keeps each connection entry until the
//! run ends, where ERTS deletes an entry that no term refers to after
//! `node_tab_delete_delay` (`erl_node_tables.c:429-537`): see row
//! "Distribution" of `docs/erts-differences.md`.

use std::{
    collections::{BTreeMap, HashMap, VecDeque},
    sync::{Arc, Mutex},
};

use crate::{
    runtime::process::{MonitorKey, RemotePid},
    term::{Atom, Binary, PortId, ProcessId, Reference, Term},
};

/// The mask of a connection id (`ERTS_DIST_CON_ID_MASK`, `external.h:137`).
const CONNECTION_ID_MASK: u32 = 0x00ff_ffff;

/// `DFLAG_PUBLISHED` (`dist.h:30`): the other node is a visible node.
const PUBLISHED: u64 = 0x01;

/// The bytes that the queue size of a connection counts for each packet
/// besides its data on a 64-bit host: `sizeof(ErtsDistOutputBuf)` and
/// `sizeof(ErlIOVec)` (`size_obuf`, `dist.c:1255-1270`).
const PACKET_OVERHEAD: u64 = 64;

/// The bytes that the queue size of a connection counts for each element
/// of the iovec of a packet on a 64-bit host: `sizeof(SysIOVec)` and
/// `sizeof(ErlDrvBinary *)` (`size_obuf`, `dist.c:1267-1268`). The iovec
/// has an empty first element (`dist.c:4670-4672`).
const ELEMENT_OVERHEAD: u64 = 24;

/// The distribution controller of a connection (`dep->cid`): a process or
/// a port.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(in crate::runtime) enum Controller {
    /// A process controller, such as the one of `inet_epmd_socket`.
    Process(ProcessId),
    /// A port controller, such as the `tcp_inet` port of `inet_tcp_dist`.
    Port(PortId),
}

/// The state of a dist entry (`ERTS_DE_STATE_*`, `erl_node_tables.h:74-77`).
/// ERTS also has the state `EXITING` between the exit of a controller and
/// the idle state; PBeam makes the entry idle in one step at the exit.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum State {
    /// No connection.
    Idle,
    /// A connection setup runs.
    Pending,
    /// The connection is up.
    Connected,
}

/// The dist entry of one other node.
#[derive(Debug)]
struct Entry {
    /// Serialize event enqueue with connection transitions. Take this lock
    /// before the port and dist locks; mailbox locks are leaves.
    event_lock: Arc<Mutex<()>>,
    /// The name of the other node.
    name: Atom,
    /// The magic reference of the entry, which each dist handle of the
    /// entry holds (`ErtsDistEntry2Bin`, `erl_node_tables.c:404`).
    handle: Reference,
    /// The connection state.
    state: State,
    /// The id of the current or the last connection.
    connection_id: u32,
    /// The distribution flags of the connection (`dep->dflags`).
    flags: u64,
    /// The peer's cache entries that this connection has sent.
    atom_cache_out: HashMap<u16, Atom>,
    /// The cache entries that the peer has sent on this connection.
    atom_cache_in: HashMap<u16, Atom>,
    /// Incomplete fragment sequences of this connection.
    fragments: HashMap<u64, DistFragment>,
    /// The distribution controller of a connected entry (`dep->cid`).
    controller: Option<Controller>,
    /// The process that gives the data of the connection to
    /// `erlang:dist_ctrl_put_data/2` (`dep->input_handler`). The exit of the
    /// controller clears it (`dist.c:1074`), and an aborted connection had
    /// none.
    input_handler: Option<ProcessId>,
    /// The count of `erlang:dist_ctrl_put_data/2` calls (`dep->in`).
    input: u64,
    /// The option `get_size` of `erlang:dist_ctrl_set_opt/3`
    /// (`ERTS_DIST_CTRL_OPT_GET_SIZE`).
    get_size: bool,
    /// The packets that wait for `erlang:dist_ctrl_get_data/1`, each as the
    /// binaries of its iovec (`dep->out_queue`).
    output: VecDeque<Vec<Binary>>,
    /// Signals held until the connection flags are known.
    pending_signals: VecDeque<PendingSignal>,
    /// The queue size of the packets in `output` (`dep->total_qsize`, see
    /// [`PACKET_OVERHEAD`]).
    queue_size: u64,
    /// The count of the packets that the controller took (`dep->out`).
    out: u64,
    /// Whether `erlang:dist_ctrl_get_data_notification/1` asked for the
    /// `dist_data` message of the next packet (`dep->notify`).
    notify: bool,
    /// The order of the entry in its list of idle, pending, or connected
    /// entries: ERTS puts an entry that changes its state at the head of
    /// the list of the new state (`erl_node_tables.c:216-227`, `678-684`,
    /// `722-728`, `773-790`).
    order: u64,
    /// The local process and the remote pid of each link of the connection,
    /// from the newest link to the oldest, as the `links` list of
    /// `ErtsMonLnkDist` (`dist.c:480-549`, `erl_monitor_link.h:651-668`).
    links: Vec<(ProcessId, RemotePid)>,
    /// Each monitor of a process of another node that the connection
    /// carries, from the oldest monitor to the newest, as the `monitors`
    /// list of `ErtsMonLnkDist` (`dist.c:392-397`,
    /// `erl_monitor_link.h:651-668`, `1534-1565`). A local process holds
    /// the monitor or a process of the other node holds it on the local
    /// process (`dist.c:2387-2400`, `bif.c:853`).
    monitors: Vec<DistMonitor>,
    /// Counts of node monitors held by each local process on this connection.
    node_monitors: HashMap<ProcessId, u64>,
}

/// A signal queued while the connection flags are not yet known.
#[derive(Debug)]
pub(super) enum PendingSignal {
    Send(ProcessId, Term, Term, super::dist_output::SendPriority),
    Monitor(ProcessId, Term),
    Link(ProcessId, Term),
    Spawn(ProcessId, Term, Term),
}

/// An incomplete distribution message and its own atom translation table.
#[derive(Debug)]
pub(super) struct DistFragment {
    pub(super) next: u64,
    pub(super) bytes: Vec<u8>,
    pub(super) atoms: Vec<Atom>,
}

/// One monitor of a connection: the local process, the pid or the name of
/// the other node, the reference of the monitor, and the side of the local
/// process (`ERTS_MON_TYPE_DIST_PROC`, `dist.c:2387-2400`, `bif.c:853`,
/// `875-894`).
///
/// # Examples
///
/// ```rust,ignore
/// let monitor = DistMonitor { process, remote: Some(remote), key, side: MonitorSide::Watcher };
/// ```
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(in crate::runtime) struct DistMonitor {
    /// The local process of the monitor.
    pub(in crate::runtime) process: ProcessId,
    /// The pid of the other node: the watched process of a monitor that the
    /// local process holds, or the watcher of a monitor that the other node
    /// holds on the local process. `None` for a monitor of a registered
    /// name, where the packets name the atom (`bif.c:875-894`).
    pub(in crate::runtime) remote: Option<RemotePid>,
    /// The reference of the monitor, which its watcher made.
    pub(in crate::runtime) key: MonitorKey,
    /// The side of the local process in the monitor.
    pub(in crate::runtime) side: MonitorSide,
}

/// The side of the local process in a monitor of a process of another node
/// (`dist.c:2387-2400`, `bif.c:853`).
///
/// # Examples
///
/// ```rust,ignore
/// assert_eq!(MonitorSide::Watcher.is_watcher(), true);
/// ```
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(in crate::runtime) enum MonitorSide {
    /// The local process holds the monitor.
    Watcher,
    /// A process of the other node holds the monitor on the local process.
    Watched,
}

/// The links and the monitors of a connection that ended, as the detached
/// `mld` list of ERTS (`dist.c:1065-1066`, `480-549`): the local process and
/// the pid of the other node of each link, from the newest link to the
/// oldest, as the head of the list (`erl_monitor_link.h:651-668`,
/// `2438-2442`), and then the monitors in the same order.
#[derive(Debug)]
pub(in crate::runtime) struct EndedConnection {
    /// The id of the connection that ended.
    pub(in crate::runtime) connection: u32,
    /// The links of the connection.
    pub(in crate::runtime) links: Vec<(ProcessId, RemotePid)>,
    /// The monitors of the connection.
    pub(in crate::runtime) monitors: Vec<DistMonitor>,
    /// Node name and the counts of the detached node monitors.
    pub(in crate::runtime) node_monitors: (Atom, HashMap<ProcessId, u64>),
    /// Node-event monitor snapshot, node type, and connection identifier.
    pub(in crate::runtime) node_events: (bool, Vec<(ProcessId, u8, u64)>),
}

/// The node type and the connection id of a node that `erlang:nodes/1,2`
/// gives (`ErtsNodeInfo`, `dist.c:6356-6399`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub(in crate::runtime) struct NodeInfo {
    /// The node name.
    pub(in crate::runtime) name: Atom,
    /// `visible`, `hidden`, or `known` (a pending or an idle entry).
    pub(in crate::runtime) kind: &'static str,
    /// The connection id, or `None` for a pending connection.
    pub(in crate::runtime) connection_id: Option<u32>,
}

/// The dist entries of a node, by the atom index of the node name.
#[derive(Debug)]
pub(in crate::runtime) struct DistTable {
    /// Each entry by the atom index of its node name.
    entries: HashMap<u32, Entry>,
    /// The atom index of the entry of each magic reference.
    handles: HashMap<Reference, u32>,
    /// Counts of node event monitors by process and option bits.
    nodes_monitors: Vec<(ProcessId, u8, u64)>,
    /// The shared not-connected list, from oldest to newest.
    idle: BTreeMap<u64, Atom>,
    /// Expiry timers for send-only entries in the not-connected list.
    known: super::known_nodes::KnownNodes,
    /// The order number of the next entry that changes its list. The
    /// entry of `nonode@nohost` has the order 0 (see [`Self::set_local`]).
    next_order: u64,
}

impl Default for DistTable {
    fn default() -> Self {
        DistTable {
            entries: HashMap::new(),
            handles: HashMap::new(),
            nodes_monitors: Vec::new(),
            idle: BTreeMap::new(),
            known: super::known_nodes::KnownNodes::default(),
            next_order: 1,
        }
    }
}

impl DistTable {
    /// Expire send-only entries whose timers ended by `now`.
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// table.expire_known(now);
    /// ```
    pub(in crate::runtime) fn expire_known(&mut self, now: i64) {
        for order in self.known.expire(now) {
            self.idle.remove(&order);
        }
    }

    /// Add or refresh a send-only entry in the shared not-connected list.
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// table.send_known(&name, now);
    /// ```
    pub(in crate::runtime) fn send_known(&mut self, name: &Atom, now: i64) {
        self.expire_known(now);
        if !self.entries.contains_key(&name.index())
            && self.known.send_ordered(name, self.next_order, now)
        {
            self.idle.insert(self.next_order, name.clone());
            self.next_order += 1;
        }
    }

    /// Make the idle entry of `name` when it has no entry, as
    /// `erts_find_or_insert_dist_entry` does (`erl_node_tables.c:352-371`):
    /// `handle` is its magic reference, and the low 24 bits of `start` its
    /// first connection id. ERTS takes `start` from the monotonic time
    /// (`erl_node_tables.c:157-184`).
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// table.insert(&a, Reference(7), 0x0100_0005);
    /// assert_eq!(table.connection_id(&a), 5);
    /// ```
    pub(in crate::runtime) fn insert(&mut self, name: &Atom, handle: Reference, start: u32) {
        if self.entries.contains_key(&name.index()) {
            return;
        }
        let order = if let Some(order) = self.known.take(name) {
            order
        } else {
            let order = self.next_order;
            self.next_order += 1;
            self.idle.insert(order, name.clone());
            order
        };
        self.handles.insert(handle, name.index());
        self.entries.insert(
            name.index(),
            Entry {
                event_lock: Arc::new(Mutex::new(())),
                name: name.clone(),
                handle,
                state: State::Idle,
                connection_id: start & CONNECTION_ID_MASK,
                flags: 0,
                atom_cache_out: HashMap::new(),
                atom_cache_in: HashMap::new(),
                fragments: HashMap::new(),
                controller: None,
                input_handler: None,
                input: 0,
                get_size: false,
                output: VecDeque::new(),
                pending_signals: VecDeque::new(),
                queue_size: 0,
                out: 0,
                notify: false,
                order,
                links: Vec::new(),
                monitors: Vec::new(),
                node_monitors: HashMap::new(),
            },
        );
    }

    /// Make the entries of `erlang:setnode/2`, which gives the local node
    /// the name `name`: the entry of `nonode@nohost`, which ERTS makes at
    /// the start as the first entry (`erl_node_tables.c:1118-1127`), and
    /// the entry of `name` (`erl_node_tables.c:1042-1056`). ERTS keeps the entry of the local node in its
    /// list of idle entries, and `erlang:nodes/1,2` skips it
    /// (`dist.c:6384-6392`).
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// table.set_local(&nonode, Reference(1), &a, Reference(2), 0);
    /// assert_eq!(table.nodes()[0].name, nonode);
    /// ```
    pub(in crate::runtime) fn set_local(
        &mut self,
        nonode: &Atom,
        nonode_handle: Reference,
        name: &Atom,
        handle: Reference,
        start: u32,
    ) {
        if !self.entries.contains_key(&nonode.index()) {
            self.insert(nonode, nonode_handle, start);
            let old_order = self.entry_mut(nonode).order;
            self.idle.remove(&old_order);
            self.entry_mut(nonode).order = 0;
            self.idle.insert(0, nonode.clone());
        }
        self.insert(name, handle, start);
    }

    /// Make the entry of `name` pending with the next connection id when it
    /// is idle, as `erts_internal_new_connection_1` does
    /// (`dist.c:5605-5620`, `erl_node_tables.c:690-731`).
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// table.insert(&a, Reference(7), 0);
    /// table.new_connection(&a);
    /// table.new_connection(&a);
    /// assert_eq!(table.connection_id(&a), 1);
    /// ```
    pub(in crate::runtime) fn new_connection(&mut self, name: &Atom) {
        let order = self.next_order;
        let entry = self.entry_mut(name);
        if entry.state == State::Idle {
            let old_order = entry.order;
            entry.state = State::Pending;
            entry.connection_id = (entry.connection_id + 1) & CONNECTION_ID_MASK;
            entry.order = order;
            self.idle.remove(&old_order);
            self.next_order += 1;
        }
    }

    /// Return the node name and the connection id of the dist handle
    /// `handle`, or `None` when `handle` is not `{ConnId, MagicRef}` with
    /// the magic reference of an entry (`erts_dhandle_to_dist_entry`,
    /// `erl_node_tables.c:381-398`). The connection id is the unsigned
    /// value of a small integer (`unsigned_val`), so its low 32 bits count.
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// let (id, handle) = table.new_connection(&a, Reference(7), 0);
    /// let term = Term::tuple(vec![Term::Integer(id.into()), Term::Reference(handle)]);
    /// assert_eq!(table.lookup(&term), Some((a, id)));
    /// assert_eq!(table.lookup(&Term::Integer(1)), None);
    /// ```
    pub(in crate::runtime) fn lookup(&self, handle: &Term) -> Option<(Atom, u32)> {
        let Term::Tuple(tuple) = handle else {
            return None;
        };
        let [id, Term::Reference(reference)] = tuple.as_ref() else {
            return None;
        };
        // ERTS keeps the low 32 bits of the word (`Uint32 *conn_id`).
        let id = u32::try_from(id.small_integer()? & i64::from(u32::MAX))
            .expect("the low 32 bits fit in u32");
        let index = self.handles.get(reference)?;
        Some((self.entries[index].name.clone(), id))
    }

    /// Return the dist handle term of `connection_id` and the entry of
    /// `name` (`erts_build_dhandle`, `erl_node_tables.c:400-416`).
    fn dhandle(&self, name: &Atom, connection_id: u32) -> Term {
        Term::tuple(vec![
            Term::Integer(i64::from(connection_id)),
            Term::Reference(self.entries[&name.index()].handle),
        ])
    }

    /// Return the dist handle of the current connection of `name`, for the
    /// result of `erts_internal:new_connection/1`.
    pub(in crate::runtime) fn current_dhandle(&self, name: &Atom) -> Term {
        self.dhandle(name, self.entries[&name.index()].connection_id)
    }

    /// Make the pending entry of `name` idle when its connection id is
    /// `connection_id`, and return whether the entry was connected and the
    /// detached connection state, as
    /// `abort_pending_connection` does (`dist.c:5627-5691`). An entry in
    /// another state or of another connection does not change.
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// let (id, _) = table.new_connection(&a, Reference(7), 0);
    /// assert!(!table.abort_pending(&a, id).0);
    /// assert_eq!(table.new_connection(&a, Reference(7), 0).0, id + 1);
    /// ```
    pub(in crate::runtime) fn abort_pending(
        &mut self,
        name: &Atom,
        connection_id: u32,
    ) -> (bool, Option<EndedConnection>) {
        let order = self.next_order;
        let entry = self.entry_mut(name);
        if entry.connection_id != connection_id {
            return (false, None);
        }
        match entry.state {
            State::Pending => {
                let ended = EndedConnection {
                    connection: connection_id,
                    links: std::mem::take(&mut entry.links),
                    monitors: std::mem::take(&mut entry.monitors),
                    node_monitors: (name.clone(), std::mem::take(&mut entry.node_monitors)),
                    node_events: (false, Vec::new()),
                };
                entry.set_idle(order);
                self.idle.insert(order, name.clone());
                self.next_order += 1;
                (false, Some(ended))
            }
            State::Connected => (true, None),
            State::Idle => (false, None),
        }
    }

    /// Make the pending entry of `name` connected with the controller
    /// `controller` and the flags `flags`, and return the dist handle of the
    /// connection, as `setup_connection_distctrl` and
    /// `erts_set_dist_entry_connected` do (`dist.c:5463-5525`,
    /// `erl_node_tables.c:733-792`). Return `None` when the entry is not
    /// pending or `controller` already controls an entry. A pending entry
    /// has no controller (`dist.c:5487-5488`). The connection starts with the input count 0 and with
    /// `controller` as the input handler.
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// table.new_connection(&a, Reference(7), 0);
    /// assert!(table.connect(&a, ProcessId(5), 0).is_some());
    /// assert!(table.connect(&a, ProcessId(5), 0).is_none());
    /// ```
    pub(in crate::runtime) fn connect(
        &mut self,
        name: &Atom,
        controller: Controller,
        flags: u64,
    ) -> Option<Term> {
        if self.controlled_by(controller).is_some() {
            return None;
        }
        let order = self.next_order;
        let entry = self.entry_mut(name);
        if entry.state != State::Pending {
            return None;
        }
        entry.state = State::Connected;
        entry.flags = flags;
        entry.controller = Some(controller);
        entry.input_handler = match controller {
            Controller::Process(process) => Some(process),
            Controller::Port(_) => None,
        };
        entry.input = 0;
        // erl_node_tables.c:771.
        entry.out = 0;
        entry.order = order;
        let connection_id = entry.connection_id;
        self.next_order += 1;
        Some(self.dhandle(name, connection_id))
    }

    /// Return the event lock of an existing dist entry.
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// let event_lock = table.event_lock(&name);
    /// ```
    pub(in crate::runtime) fn event_lock(&self, name: &Atom) -> Arc<Mutex<()>> {
        Arc::clone(&self.entries[&name.index()].event_lock)
    }

    /// Return the name of the entry that `controller` controls
    /// (`ERTS_PROC_GET_DIST_ENTRY`, `ERTS_PRTSD_DIST_ENTRY`).
    pub(in crate::runtime) fn controlled_by(&self, controller: Controller) -> Option<Atom> {
        self.entries
            .values()
            .find(|entry| entry.controller == Some(controller))
            .map(|entry| entry.name.clone())
    }

    /// Return the connection id of the entry of `name`.
    pub(in crate::runtime) fn connection_id(&self, name: &Atom) -> u32 {
        self.entries[&name.index()].connection_id
    }

    /// Return the option `get_size` of the entry of `name`.
    pub(in crate::runtime) fn get_size(&self, name: &Atom) -> bool {
        self.entries[&name.index()].get_size
    }

    /// Return the option `get_size` of the entry of `name` to change it.
    pub(in crate::runtime) fn get_size_mut(&mut self, name: &Atom) -> &mut bool {
        &mut self.entry_mut(name).get_size
    }

    /// Return whether a port controls the entry of `name`.
    pub(in crate::runtime) fn port_controlled(&self, name: &Atom) -> bool {
        matches!(
            self.entries[&name.index()].controller,
            Some(Controller::Port(_))
        )
    }

    /// Return the output cache of the connected entry.
    pub(super) fn atom_cache_out(&mut self, name: &Atom) -> &mut HashMap<u16, Atom> {
        &mut self.entry_mut(name).atom_cache_out
    }

    /// Return the input cache of the named connection when it is current.
    pub(super) fn atom_cache_in(
        &mut self,
        name: &Atom,
        connection: u32,
    ) -> Option<&mut HashMap<u16, Atom>> {
        let entry = self.entries.get_mut(&name.index())?;
        (entry.state == State::Connected && entry.connection_id == connection)
            .then_some(&mut entry.atom_cache_in)
    }

    /// Return the sequences of the current connection.
    pub(super) fn fragments_in(
        &mut self,
        name: &Atom,
        connection: u32,
    ) -> Option<&mut HashMap<u64, DistFragment>> {
        let entry = self.entries.get_mut(&name.index())?;
        (entry.state == State::Connected && entry.connection_id == connection)
            .then_some(&mut entry.fragments)
    }

    /// Return the distribution flags of the entry of `name` (`dep->dflags`).
    pub(in crate::runtime) fn flags(&self, name: &Atom) -> u64 {
        self.entries[&name.index()].flags
    }

    /// Return the input handler of the entry of `name`.
    pub(in crate::runtime) fn input_handler(&self, name: &Atom) -> Option<ProcessId> {
        self.entries[&name.index()].input_handler
    }

    /// Set the input handler of the entry of `name`.
    pub(in crate::runtime) fn set_input_handler(&mut self, name: &Atom, handler: ProcessId) {
        self.entry_mut(name).input_handler = Some(handler);
    }

    /// Add one to the input count of the entry of `name`.
    pub(in crate::runtime) fn count_input(&mut self, name: &Atom) {
        self.entry_mut(name).input += 1;
    }

    /// Return the input count of the entry of `name`.
    pub(in crate::runtime) fn input(&self, name: &Atom) -> u64 {
        self.entries[&name.index()].input
    }

    /// Register a node monitor and return its connected entry id.
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// assert_eq!(table.monitor_node(&name, process), Some(1));
    /// ```
    pub(in crate::runtime) fn monitor_node(
        &mut self,
        name: &Atom,
        process: ProcessId,
    ) -> Option<u32> {
        let entry = self.entries.get_mut(&name.index())?;
        if entry.state != State::Connected {
            return None;
        }
        *entry.node_monitors.entry(process).or_default() += 1;
        Some(entry.connection_id)
    }

    /// Remove one node monitor from its connection when it is still active.
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// table.unmonitor_node(&name, process, 1);
    /// ```
    pub(in crate::runtime) fn unmonitor_node(
        &mut self,
        name: &Atom,
        process: ProcessId,
        connection: u32,
    ) {
        let entry = self.entry_mut(name);
        if entry.connection_id != connection {
            return;
        }
        if let Some(count) = entry.node_monitors.get_mut(&process) {
            *count -= 1;
            if *count == 0 {
                entry.node_monitors.remove(&process);
            }
        }
    }

    /// Set or clear a process node-event monitor and return its old count.
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// assert_eq!(table.monitor_nodes(process, 0, true), 0);
    /// ```
    #[expect(
        clippy::fn_params_excessive_bools,
        reason = "the process flag takes a boolean"
    )]
    pub(in crate::runtime) fn monitor_nodes(
        &mut self,
        process: ProcessId,
        key: u8,
        enabled: bool,
    ) -> u64 {
        let position = self
            .nodes_monitors
            .iter()
            .position(|&(owner, options, _)| owner == process && options == key);
        if enabled {
            if let Some(position) = position {
                let count = &mut self.nodes_monitors[position].2;
                let old = *count;
                *count += 1;
                old
            } else {
                self.nodes_monitors.push((process, key, 1));
                0
            }
        } else if let Some(position) = position {
            self.nodes_monitors.remove(position).2
        } else {
            0
        }
    }

    /// Snapshot node-event monitors under the distribution lock.
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// let watchers = table.nodes_monitors();
    /// ```
    pub(in crate::runtime) fn nodes_monitors(&self) -> Vec<(ProcessId, u8, u64)> {
        self.nodes_monitors.clone()
    }

    /// Return the current count of a node-event monitor.
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// assert_eq!(table.nodes_monitor_count(process, 0), 0);
    /// ```
    pub(in crate::runtime) fn nodes_monitor_count(&self, process: ProcessId, key: u8) -> u64 {
        self.nodes_monitors
            .iter()
            .find(|&&(owner, options, _)| owner == process && options == key)
            .map_or(0, |entry| entry.2)
    }

    /// Remove each node monitor held by an exiting process.
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// table.remove_node_monitors(process);
    /// ```
    pub(in crate::runtime) fn remove_node_monitors(&mut self, process: ProcessId) {
        self.nodes_monitors
            .retain(|(owner, _, _)| *owner != process);
        for entry in self.entries.values_mut() {
            entry.node_monitors.remove(&process);
        }
    }

    /// Return whether the entry of `name` has a pending connection.
    pub(super) fn pending(&self, name: &Atom) -> bool {
        self.entries
            .get(&name.index())
            .is_some_and(|entry| entry.state == State::Pending)
    }

    /// Return the flags, the controller, and the connection id of the entry
    /// of `name` when it is connected (`ERTS_DSIG_PREP_CONNECTED`,
    /// `dist.c:3250-3251`).
    pub(in crate::runtime) fn connected(&self, name: &Atom) -> Option<(u64, Controller, u32)> {
        let entry = self.entries.get(&name.index())?;
        if entry.state != State::Connected {
            return None;
        }
        Some((entry.flags, entry.controller?, entry.connection_id))
    }

    /// Queue a send while the entry is pending.
    pub(super) fn enqueue_pending_send(
        &mut self,
        name: &Atom,
        sender: ProcessId,
        to: Term,
        message: Term,
        priority: super::dist_output::SendPriority,
    ) {
        self.entry_mut(name)
            .pending_signals
            .push_back(PendingSignal::Send(sender, to, message, priority));
    }

    /// Queue a monitor on the pending connection.
    pub(super) fn enqueue_pending_monitor(
        &mut self,
        name: &Atom,
        sender: ProcessId,
        control: Term,
    ) {
        self.entry_mut(name)
            .pending_signals
            .push_back(PendingSignal::Monitor(sender, control));
    }

    /// Queue a link on the pending connection.
    pub(super) fn enqueue_pending_link(&mut self, name: &Atom, sender: ProcessId, control: Term) {
        self.entry_mut(name)
            .pending_signals
            .push_back(PendingSignal::Link(sender, control));
    }

    /// Queue a spawn request on the pending connection.
    pub(super) fn enqueue_pending_spawn(
        &mut self,
        name: &Atom,
        sender: ProcessId,
        control: Term,
        arguments: Term,
    ) {
        self.entry_mut(name)
            .pending_signals
            .push_back(PendingSignal::Spawn(sender, control, arguments));
    }

    /// Take pending signals while the entry lock is held.
    pub(super) fn take_pending_signals(&mut self, name: &Atom) -> VecDeque<PendingSignal> {
        std::mem::take(&mut self.entry_mut(name).pending_signals)
    }

    /// Put the packet `elements` at the end of the output queue of the
    /// connected entry of `name`, as `erts_dsig_send` does
    /// (`dist.c:3646-3683`), and return whether the controller gets the
    /// `dist_data` message: once after
    /// `erlang:dist_ctrl_get_data_notification/1` asked for it
    /// (`dist.c:3655-3667`).
    pub(in crate::runtime) fn enqueue(&mut self, name: &Atom, elements: Vec<Binary>) -> bool {
        let entry = self.entry_mut(name);
        entry.queue_size += queue_size(&elements);
        entry.output.push_back(elements);
        std::mem::take(&mut entry.notify)
    }

    /// Take the first packet of the output queue of the entry of `name` and
    /// count it, as `erlang:dist_ctrl_get_data/1` does
    /// (`dist.c:4612-4713`), or return `None` for an empty queue.
    pub(in crate::runtime) fn dequeue(&mut self, name: &Atom) -> Option<Vec<Binary>> {
        let entry = self.entry_mut(name);
        let elements = entry.output.pop_front()?;
        entry.queue_size -= queue_size(&elements);
        entry.out += 1;
        Some(elements)
    }

    /// Ask for the `dist_data` message of the next packet of the entry of
    /// `name`, as `erlang:dist_ctrl_get_data_notification/1` does
    /// (`dist.c:4301-4322`), and return whether the queue has a packet, so
    /// that the controller gets the message now.
    pub(in crate::runtime) fn notify(&mut self, name: &Atom) -> bool {
        // A queue with a packet has no request: `Self::enqueue` takes it.
        let entry = self.entry_mut(name);
        entry.notify = entry.output.is_empty();
        !entry.notify
    }

    /// Return the output count and the queue size of the entry of `name`
    /// (`dep->out` and `dep->total_qsize`, `dist.c:4507-4508`).
    pub(in crate::runtime) fn output(&self, name: &Atom) -> (u64, u64) {
        let entry = &self.entries[&name.index()];
        (entry.out, entry.queue_size)
    }

    /// Return the entry of `name`, which [`Self::insert`] made.
    fn entry_mut(&mut self, name: &Atom) -> &mut Entry {
        self.entries
            .get_mut(&name.index())
            .expect("a dist handle names an entry")
    }

    /// Make the entry that `process` controls idle at the exit of
    /// `process` with the input count 0 and no input handler, as
    /// `erts_do_net_exits` does for a controller (`dist.c:1021-1117`).
    /// Return the links and the monitors of the connection that ended, as
    /// the detached `mld` list of `dist.c:1065-1066`, or `None` with no link
    /// and no monitor.
    pub(in crate::runtime) fn controller_exits(
        &mut self,
        controller: Controller,
    ) -> Option<EndedConnection> {
        let watchers = self.nodes_monitors();
        let entry = self
            .entries
            .values_mut()
            .find(|entry| entry.controller == Some(controller))?;
        let connection = entry.connection_id;
        let links = std::mem::take(&mut entry.links);
        let monitors = std::mem::take(&mut entry.monitors);
        let visible = entry.flags & PUBLISHED != 0;
        let node_monitors = (entry.name.clone(), std::mem::take(&mut entry.node_monitors));
        entry.set_idle(self.next_order);
        entry.input = 0;
        // dist.c:1081.
        entry.out = 0;
        entry.input_handler = None;
        self.idle.insert(self.next_order, entry.name.clone());
        self.next_order += 1;
        Some(EndedConnection {
            connection,
            links,
            monitors,
            node_monitors,
            node_events: (visible, watchers),
        })
    }

    /// Add the link of `process` to `remote` to the connection `connection`
    /// of the entry of its node, at the head of the list, as
    /// `erts_link_dist_insert` does (`erl_monitor_link.h:2523-2548`,
    /// `651-668`, `dist.c:2246-2258`, `3085`), and return whether the
    /// connection has the link. Return `false` when the entry is not
    /// pending or connected to `connection`, so that the caller gives the process
    /// `noconnection` (`dist.c:2250-2258`). A connection that already has
    /// the link keeps it, as a duplicate `LINK` deletes its own dist link
    /// (`erl_proc_sig_queue.c:6635-6663`).
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// assert!(table.insert_link(ProcessId(1), remote, 1));
    /// ```
    pub(in crate::runtime) fn insert_link(
        &mut self,
        process: ProcessId,
        remote: RemotePid,
        connection: u32,
    ) -> bool {
        let entry = self
            .entries
            .get_mut(&remote.node)
            .expect("the entry of the node stays in the table");
        if entry.state == State::Idle || entry.connection_id != connection {
            return false;
        }
        self.insert_link_now(process, remote);
        true
    }

    /// Insert a link while the caller holds the matching connection lock.
    pub(super) fn insert_link_now(&mut self, process: ProcessId, remote: RemotePid) {
        let entry = self
            .entries
            .get_mut(&remote.node)
            .expect("the entry of the node stays in the table");
        if !entry.links.contains(&(process, remote)) {
            entry.links.insert(0, (process, remote));
        }
    }

    /// Remove the link of `process` to `remote` from the connection
    /// `connection` of the entry of its node. The entry holds the links of
    /// its current connection only, because the end of a connection
    /// detaches its list (`dist.c:1065-1066`), so a signal of an older
    /// connection removes nothing here.
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// table.remove_link(ProcessId(1), remote, 1);
    /// ```
    pub(in crate::runtime) fn remove_link(
        &mut self,
        process: ProcessId,
        remote: RemotePid,
        connection: u32,
    ) {
        let Some(entry) = self.entries.get_mut(&remote.node) else {
            return;
        };
        if entry.connection_id != connection {
            return;
        }
        entry.links.retain(|&link| link != (process, remote));
    }

    /// Add the monitor `key` of `process` on the pid `remote` of the node
    /// `node`, or on a name of `node` when `remote` is `None`, to the
    /// connection `connection` of the entry of that node, at the end of the
    /// list, as `erts_monitor_dist_insert` does
    /// (`erl_monitor_link.h:1534-1566`, `651-668`), and return whether the
    /// connection has the monitor.
    /// The end of a connection then reports the oldest monitor first
    /// (`dist.c:1075-1079`).
    /// Return `false` when the entry is not connected to `connection`, so
    /// that the caller gives the watcher of the monitor `noconnection`
    /// (`dist.c:2387-2391`, `erl_monitor_link.h:1533-1566`).
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// assert!(table.insert_monitor(ProcessId(1), &node, Some(remote), key, 1, MonitorSide::Watcher));
    /// ```
    pub(in crate::runtime) fn insert_monitor(
        &mut self,
        process: ProcessId,
        node: &Atom,
        remote: Option<RemotePid>,
        key: MonitorKey,
        connection: u32,
        side: MonitorSide,
    ) -> bool {
        let entry = self
            .entries
            .get_mut(&node.index())
            .expect("the entry of the node stays in the table");
        if entry.state == State::Idle || entry.connection_id != connection {
            return false;
        }
        let monitor = DistMonitor {
            process,
            remote,
            key,
            side,
        };
        if !entry.monitors.contains(&monitor) {
            entry.monitors.push(monitor);
        }
        true
    }

    /// Make the monitor `key` of the process `process` of the entry of `node`
    /// a monitor of the pid `remote`, as the reply of a spawn request turns
    /// the pending monitor into a process monitor (`dist.c:3033-3130`).
    pub(in crate::runtime) fn update_monitor(
        &mut self,
        process: ProcessId,
        node: &Atom,
        key: MonitorKey,
        remote: RemotePid,
    ) {
        let entry = self
            .entries
            .get_mut(&node.index())
            .expect("the entry of the node stays in the table");
        if let Some(monitor) = entry
            .monitors
            .iter_mut()
            .find(|monitor| monitor.process == process && monitor.key == key)
        {
            monitor.remote = Some(remote);
        }
    }

    /// Put `monitor` in the monitor list of the connected entry of `node`,
    /// as the caller of [`Self::insert_monitor`] does after its checks. The
    /// entry is connected: the caller holds the lock of the table and found
    /// the connection.
    pub(in crate::runtime) fn insert_monitor_now(&mut self, node: &Atom, monitor: DistMonitor) {
        let entry = self
            .entries
            .get_mut(&node.index())
            .expect("the entry of the node stays in the table");
        if !entry.monitors.contains(&monitor) {
            entry.monitors.push(monitor);
        }
    }

    /// Return the monitors of the connection of `node`, from the oldest to
    /// the newest, for a test.
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// assert!(table.monitors(&node).is_empty());
    /// ```
    #[cfg(test)]
    pub(in crate::runtime) fn monitors(&self, node: &Atom) -> &[DistMonitor] {
        &self.entries[&node.index()].monitors
    }

    /// Return the local process that the monitor `key` of the pid `remote`
    /// of the node `node` watches, as the monitors of the connection name
    /// each target that a `DEMONITOR_P` packet can ask for by name
    /// (`dist.c:2441-2457`), or `None` without such a monitor.
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// assert_eq!(table.watched_process(remote, key), Some(target));
    /// ```
    pub(in crate::runtime) fn watched_process(
        &self,
        remote: RemotePid,
        key: MonitorKey,
    ) -> Option<ProcessId> {
        self.entries
            .get(&remote.node)?
            .monitors
            .iter()
            .find(|monitor| {
                monitor.side == MonitorSide::Watched
                    && monitor.remote == Some(remote)
                    && monitor.key == key
            })
            .map(|monitor| monitor.process)
    }

    /// Remove the monitor `key` of `process` on the node `node` from the
    /// connection `connection` of the entry of that node. The entry holds
    /// the monitors of its current connection only, because the end of a
    /// connection detaches its list (`dist.c:1065-1066`), so a signal of an
    /// older connection removes nothing here.
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// table.remove_monitor(ProcessId(1), &node, key, 1);
    /// ```
    pub(in crate::runtime) fn remove_monitor(
        &mut self,
        process: ProcessId,
        node: &Atom,
        key: MonitorKey,
        connection: u32,
    ) {
        let Some(entry) = self.entries.get_mut(&node.index()) else {
            return;
        };
        if entry.connection_id != connection {
            return;
        }
        entry
            .monitors
            .retain(|monitor| monitor.process != process || monitor.key != key);
    }

    /// Snapshot the entries at the exit of `net_kernel`. The caller takes
    /// each entry event lock before it cancels or stops that connection.
    /// `erts_do_net_exits` does the controller cleanup at `dist.c:926-1001`.
    pub(in crate::runtime) fn net_kernel_exits(&self) -> Vec<(Atom, Arc<Mutex<()>>)> {
        self.entries
            .values()
            .map(|entry| (entry.name.clone(), Arc::clone(&entry.event_lock)))
            .collect()
    }

    /// Detach one entry during local node shutdown under its event lock.
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// let controller = table.shutdown_entry(&name);
    /// ```
    pub(in crate::runtime) fn shutdown_entry(
        &mut self,
        name: &Atom,
    ) -> (Option<Controller>, Option<EndedConnection>) {
        let entry = self.entry_mut(name);
        match entry.state {
            State::Pending => {
                let connection = entry.connection_id;
                let (_, ended) = self.abort_pending(name, connection);
                (None, ended)
            }
            State::Connected => (entry.controller, None),
            State::Idle => (None, None),
        }
    }

    /// Return the node of each entry in the order of `erlang:nodes/1,2`
    /// (`dist.c:6356-6411`): the pending entries, the idle entries, the
    /// hidden entries, and the visible entries, each from the oldest to the
    /// newest in its list. ERTS builds the result in the reverse order of
    /// its lists, which have the newest entry at the head.
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// table.insert(&a, Reference(7), 0);
    /// table.new_connection(&a);
    /// assert_eq!(table.nodes()[0].kind, "known");
    /// ```
    pub(in crate::runtime) fn nodes(&self) -> Vec<NodeInfo> {
        let mut entries: Vec<&Entry> = self
            .entries
            .values()
            .filter(|entry| entry.state != State::Idle)
            .collect();
        entries.sort_by_key(|entry| {
            let rank = if entry.state == State::Pending {
                0
            } else if entry.flags & PUBLISHED == 0 {
                2
            } else {
                3
            };
            (rank, entry.order)
        });
        let mut result = Vec::new();
        let mut idle_added = false;
        for entry in entries {
            if entry.state != State::Pending && !idle_added {
                result.extend(self.idle.values().map(|name| NodeInfo {
                    name: name.clone(),
                    kind: "known",
                    connection_id: None,
                }));
                idle_added = true;
            }
            result.push(NodeInfo {
                name: entry.name.clone(),
                kind: match (entry.state, entry.flags & PUBLISHED) {
                    (State::Connected, 0) => "hidden",
                    (State::Connected, _) => "visible",
                    _ => "known",
                },
                connection_id: (entry.state == State::Connected).then_some(entry.connection_id),
            });
        }
        if !idle_added {
            result.extend(self.idle.values().map(|name| NodeInfo {
                name: name.clone(),
                kind: "known",
                connection_id: None,
            }));
        }
        result
    }
}

impl Entry {
    /// Make the entry idle at the list position `order`, as
    /// `erts_set_dist_entry_not_connected` does
    /// (`erl_node_tables.c:630-686`): no flags, no options, and no
    /// controller.
    fn set_idle(&mut self, order: u64) {
        self.state = State::Idle;
        self.flags = 0;
        self.atom_cache_out.clear();
        self.atom_cache_in.clear();
        self.fragments.clear();
        self.get_size = false;
        self.output.clear();
        self.pending_signals.clear();
        self.queue_size = 0;
        self.notify = false;
        self.controller = None;
        self.links.clear();
        self.monitors.clear();
        self.order = order;
    }
}

/// Return the queue size of the packet `elements` (`size_obuf`,
/// `dist.c:1255-1270`): its bytes, [`PACKET_OVERHEAD`], and
/// [`ELEMENT_OVERHEAD`] for each element and the empty first element.
fn queue_size(elements: &[Binary]) -> u64 {
    let bytes: usize = elements.iter().map(|element| element.bit_size() / 8).sum();
    let count = elements.len() + 1;
    PACKET_OVERHEAD
        + u64::try_from(bytes).expect("a packet size fits in u64")
        + ELEMENT_OVERHEAD * u64::try_from(count).expect("an element count fits in u64")
}

#[cfg(test)]
#[path = "../../../tests/runtime/scheduler/dist_table.rs"]
mod tests;

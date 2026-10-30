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
//! The entries of other nodes that ERTS makes for a send stay in
//! [`super::known_nodes`]. PBeam keeps each entry of this table until the
//! run ends, where ERTS deletes an entry that no term refers to after
//! `node_tab_delete_delay` (`erl_node_tables.c:429-537`): see row
//! "Distribution" of `docs/erts-differences.md`.

use std::collections::HashMap;

use crate::term::{Atom, PortId, ProcessId, Reference, Term};

/// The mask of a connection id (`ERTS_DIST_CON_ID_MASK`, `external.h:137`).
const CONNECTION_ID_MASK: u32 = 0x00ff_ffff;

/// `DFLAG_PUBLISHED` (`dist.h:30`): the other node is a visible node.
const PUBLISHED: u64 = 0x01;

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
    /// The order of the entry in its list of idle, pending, or connected
    /// entries: ERTS puts an entry that changes its state at the head of
    /// the list of the new state (`erl_node_tables.c:216-227`, `678-684`,
    /// `722-728`, `773-790`).
    order: u64,
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
    /// The order number of the next entry that changes its list. The
    /// entry of `nonode@nohost` has the order 0 (see [`Self::set_local`]).
    next_order: u64,
}

impl Default for DistTable {
    fn default() -> Self {
        DistTable {
            entries: HashMap::new(),
            handles: HashMap::new(),
            next_order: 1,
        }
    }
}

impl DistTable {
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
        let order = self.next_order;
        self.entries.entry(name.index()).or_insert_with(|| {
            self.next_order += 1;
            self.handles.insert(handle, name.index());
            Entry {
                name: name.clone(),
                handle,
                state: State::Idle,
                connection_id: start & CONNECTION_ID_MASK,
                flags: 0,
                controller: None,
                input_handler: None,
                input: 0,
                get_size: false,
                order,
            }
        });
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
            self.entry_mut(nonode).order = 0;
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
            entry.state = State::Pending;
            entry.connection_id = (entry.connection_id + 1) & CONNECTION_ID_MASK;
            entry.order = order;
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
    /// `connection_id`, and return whether the entry was connected, as
    /// `abort_pending_connection` does (`dist.c:5627-5691`). An entry in
    /// another state or of another connection does not change.
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// let (id, _) = table.new_connection(&a, Reference(7), 0);
    /// assert!(!table.abort_pending(&a, id));
    /// assert_eq!(table.new_connection(&a, Reference(7), 0).0, id + 1);
    /// ```
    pub(in crate::runtime) fn abort_pending(&mut self, name: &Atom, connection_id: u32) -> bool {
        let order = self.next_order;
        let entry = self.entry_mut(name);
        if entry.connection_id != connection_id {
            return false;
        }
        match entry.state {
            State::Pending => {
                entry.set_idle(order);
                self.next_order += 1;
                false
            }
            State::Connected => true,
            State::Idle => false,
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
        entry.order = order;
        let connection_id = entry.connection_id;
        self.next_order += 1;
        Some(self.dhandle(name, connection_id))
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

    /// Return the entry of `name`, which [`Self::insert`] made.
    fn entry_mut(&mut self, name: &Atom) -> &mut Entry {
        self.entries
            .get_mut(&name.index())
            .expect("a dist handle names an entry")
    }

    /// Make the entry that `process` controls idle at the exit of
    /// `process` with the input count 0 and no input handler, as
    /// `erts_do_net_exits` does for a controller (`dist.c:1021-1117`).
    pub(in crate::runtime) fn controller_exits(&mut self, controller: Controller) {
        if let Some(entry) = self
            .entries
            .values_mut()
            .find(|entry| entry.controller == Some(controller))
        {
            entry.set_idle(self.next_order);
            entry.input = 0;
            entry.input_handler = None;
            self.next_order += 1;
        }
    }

    /// Make each pending entry idle and return the controller of each
    /// connected entry, as `erts_do_net_exits` does at the exit of
    /// `net_kernel` (`dist.c:926-1001`).
    pub(in crate::runtime) fn net_kernel_exits(&mut self) -> Vec<Controller> {
        let mut controllers = Vec::new();
        for entry in self.entries.values_mut() {
            match entry.state {
                State::Pending => {
                    entry.set_idle(self.next_order);
                    self.next_order += 1;
                }
                State::Connected => controllers.extend(entry.controller),
                State::Idle => {}
            }
        }
        controllers
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
        let mut entries: Vec<&Entry> = self.entries.values().collect();
        entries.sort_by_key(|entry| {
            let rank = match (entry.state, entry.flags & PUBLISHED) {
                (State::Pending, _) => 0,
                (State::Idle, _) => 1,
                (State::Connected, 0) => 2,
                (State::Connected, _) => 3,
            };
            (rank, entry.order)
        });
        entries
            .into_iter()
            .map(|entry| NodeInfo {
                name: entry.name.clone(),
                kind: match (entry.state, entry.flags & PUBLISHED) {
                    (State::Connected, 0) => "hidden",
                    (State::Connected, _) => "visible",
                    _ => "known",
                },
                connection_id: (entry.state == State::Connected).then_some(entry.connection_id),
            })
            .collect()
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
        self.get_size = false;
        self.controller = None;
        self.order = order;
    }
}

#[cfg(test)]
#[path = "../../../tests/runtime/scheduler/dist_table.rs"]
mod tests;

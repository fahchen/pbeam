//! The monitors of local processes to processes of another node, as the
//! `ERTS_MON_TYPE_DIST_PROC` monitors of ERTS: `erlang:monitor(process,
//! Pid)` and `erlang:demonitor/1,2` with a pid of another node
//! (`bif.c:758-981`, `436-541`), the `MONITOR_P`, `DEMONITOR_P`,
//! `MONITOR_P_EXIT`, and `PAYLOAD_MONITOR_P_EXIT` packets of a connection
//! (`dist.c:1496-1570`, `2341-2470`, `2688-2746`), and the `DOWN` messages
//! of a monitor that ends (`erl_process.c:13551-13650`;
//! `erl_proc_sig_queue.c:6360-6510`).
//!
//! The watcher of a monitor makes its reference and keeps the origin part in
//! its monitor tree. The target of another node keeps the target part with the
//! same reference, which is a reference of that node. Thus the monitor tree of
//! one process holds the origins of the monitors that it holds and the targets
//! of the monitors that processes of other nodes hold on it, and one walk of
//! the tree at the exit of the process reports each of them in the ERTS order
//! (`erl_proc_sig_queue.c:6544-6562`; `erl_monitor_link.h`).
//!
//! A packet of a monitor goes on the connection that the monitor uses, as
//! `ErtsMonLnkDist.connection_id` gives it, and the end of that connection
//! removes the monitor from the dist entry and gives the `DOWN` message with
//! the reason `noconnection` to the watcher, from the oldest monitor to the
//! newest (`dist.c:304-321`, `392-397`). The insert of the list keeps its
//! head at the first monitor that entered it, so a walk from the head
//! reports the monitors in the order of their creation
//! (`erts_ml_dl_list_insert__`, `erl_monitor_link.h:651-668`;
//! `ml_dl_list_foreach_delete_yielding`, `erl_monitor_link.c:517-548`). A
//! `MONITOR_P` packet can ask for a registered name too, which this node
//! looks up when it gets the packet (`dist.c:2364-2379`). The local
//! `erlang:monitor/2,3` of `{Name, Node}` stays out of scope: #1494 adds
//! it.

use super::{Scheduler, dist_output::EXIT_PAYLOAD, dist_table::MonitorSide};
use crate::{
    error::{BeamError, error},
    runtime::{
        process::{MonitorKey, RemotePid, RemoteReference, RemoteTarget},
        signal::Signal,
    },
    term::{Atom, Map, ProcessId, Term, external_id},
};

/// `DOP_MONITOR_P` (`dist.h:157`).
const MONITOR_P: i64 = 19;
/// `DOP_DEMONITOR_P` (`dist.h:158`).
const DEMONITOR_P: i64 = 20;
/// `DOP_MONITOR_P_EXIT` (`dist.h:159`).
const MONITOR_P_EXIT: i64 = 21;
/// `DOP_PAYLOAD_MONITOR_P_EXIT` (`dist.h:169`).
const PAYLOAD_MONITOR_P_EXIT: i64 = 28;

/// `DFLAG_DIST_MONITOR` (`dist.h:33`, `105-106`): the node accepts
/// `MONITOR_P`, `DEMONITOR_P`, `MONITOR_P_EXIT`, and
/// `PAYLOAD_MONITOR_P_EXIT` packets.
const DIST_MONITOR: u64 = 0x08;
/// `DFLAG_DIST_MONITOR_NAME` (`dist.h:34`, `105-106`). ERTS sends a monitor
/// packet only when the connection offered this flag and
/// `DFLAG_DIST_MONITOR` (`dist.c:1507`, `1545`, `1558`).
const DIST_MONITOR_NAME: u64 = 0x20;
/// The flags that a connection needs for a monitor packet
/// (`dist.c:1507`, `1545`, `1558`).
const MONITOR_FLAGS: u64 = DIST_MONITOR | DIST_MONITOR_NAME;

impl Scheduler {
    /// Monitor the process `remote` of another node, or a registered name
    /// of that node, for the local process `watcher` with the reference
    /// `key`, as `monitor` does (`bif.c:830-935`): the monitor becomes
    /// active on the connection of the node, and the node gets `MONITOR_P`
    /// (`dist.c:1543-1547`). Without a connection `watcher` gets the
    /// monitor down signal `noconnection` (`bif.c:842-849`), which removes
    /// the monitor.
    ///
    /// A node without the monitor flags gets no packet, so the monitor only
    /// watches the connection (`dist.c:1533-1541`).
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// scheduler.dist_monitor(&target, key, watcher)?;
    /// ```
    ///
    /// # Errors
    ///
    /// The errors of the encode of the control message.
    pub(super) fn dist_monitor(
        &mut self,
        target: &RemoteTarget,
        key: MonitorKey,
        watcher: ProcessId,
    ) -> Result<(), BeamError> {
        let mut used = None;
        let to = target.term();
        let node = target.node();
        let from = Term::Pid(watcher);
        let remote = match target {
            RemoteTarget::Pid(remote) => Some(*remote),
            RemoteTarget::Name { .. } => None,
        };
        let monitor = super::dist_table::DistMonitor {
            process: watcher,
            remote,
            key,
            side: MonitorSide::Watcher,
        };
        let control = monitor_control(MONITOR_P, &from, &to, key, None);
        let already_connected = self.node.dist().connected(&node).is_some();
        if !already_connected
            && let Some(connection) = self.dist_auto_connect(&node, |dist, _| {
                dist.insert_monitor_now(&node, monitor);
                dist.enqueue_pending_monitor(&node, watcher, control);
            })?
        {
            self.monitor_on(watcher, target, key, connection);
            return Ok(());
        }
        self.dist_output(watcher, &node, |flags, connection| {
            used = Some(connection);
            (flags & MONITOR_FLAGS == MONITOR_FLAGS).then(|| {
                let control = monitor_control(MONITOR_P, &from, &to, key, None);
                (control, None)
            })
        })?;
        // No connection gives `noconnection`, as the end of a connection
        // does (`dist.c:1021-1117`, `305-310`).
        let Some(connection) = used else {
            self.monitor_down_noconnection(watcher, key);
            return Ok(());
        };
        self.monitor_on(watcher, target, key, connection);
        Ok(())
    }

    /// Make the monitor `key` that the local process `watcher` holds of
    /// `target`, a pid or a name of another node, active on the connection
    /// `connection`, as `monitor` does (`bif.c:830-863`).
    ///
    /// `dist_output` releases the lock of the dist entry when it writes the
    /// packet, so the connection can end before the insert. The monitor
    /// then ends with `noconnection`, as a monitor of the detached list of
    /// the connection does (`dist.c:1021-1117`, `305-310`).
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// scheduler.monitor_on(watcher, &target, key, connection);
    /// ```
    pub(super) fn monitor_on(
        &mut self,
        watcher: ProcessId,
        target: &RemoteTarget,
        key: MonitorKey,
        connection: u32,
    ) {
        let monitor = self
            .processes
            .get_mut(watcher)
            .and_then(|process| process.monitors.get_mut(&key))
            .and_then(|origin| origin.monitor.as_mut());
        if let Some(monitor) = monitor {
            monitor.connection = connection;
        }
        let remote = match target {
            RemoteTarget::Pid(remote) => Some(*remote),
            RemoteTarget::Name { .. } => None,
        };
        let node = target.node();
        if !self.node.dist().insert_monitor(
            watcher,
            &node,
            remote,
            key,
            connection,
            MonitorSide::Watcher,
        ) {
            self.monitor_down_noconnection(watcher, key);
        }
    }

    /// Give `process` the monitor down signal `noconnection` for its monitor
    /// `key`, as `erts_do_net_exits` does for a monitor whose connection
    /// ended (`dist.c:1021-1117`, `305-310`). The signal removes the
    /// monitor. A watcher that is not alive drops the signal.
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// scheduler.monitor_down_noconnection(watcher, key);
    /// ```
    fn monitor_down_noconnection(&mut self, process: ProcessId, key: MonitorKey) {
        let down = Signal::MonitorDown {
            reference: key,
            reason: Term::static_atom("noconnection"),
        };
        let _ = self.send_signal(None, process, down);
    }

    /// Send node events to the monitor snapshot with the ERTS option forms.
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// scheduler.nodes_event(&name, true, Some(1), None, &watchers);
    /// ```
    #[expect(
        clippy::fn_params_excessive_bools,
        reason = "the flag selects visible or hidden nodes"
    )]
    pub(in crate::runtime) fn nodes_event(
        &mut self,
        name: &Atom,
        visible: bool,
        connection: Option<u32>,
        reason: Option<&Term>,
        watchers: &[(ProcessId, u8, u64)],
    ) {
        let kind = Term::static_atom(if visible { "visible" } else { "hidden" });
        for &(watcher, options, _) in watchers {
            let types = options & 3;
            if (!visible && types & 2 == 0) || (visible && types != 0 && types & 1 == 0) {
                continue;
            }
            let mut fields = Vec::new();
            if options & 16 != 0 {
                fields.push((
                    Term::static_atom("connection_id"),
                    connection.map_or(Term::static_atom("undefined"), |id| {
                        Term::Integer(i64::from(id))
                    }),
                ));
            }
            if options & 4 != 0
                && let Some(reason) = reason
            {
                fields.push((
                    Term::static_atom("nodedown_reason"),
                    if *reason == Term::static_atom("normal") {
                        Term::static_atom("connection_closed")
                    } else {
                        reason.clone()
                    },
                ));
            }
            if types != 0 {
                fields.push((Term::static_atom("node_type"), kind.clone()));
            }
            let mut tuple = vec![
                Term::static_atom(if reason.is_some() {
                    "nodedown"
                } else {
                    "nodeup"
                }),
                Term::Atom(name.clone()),
            ];
            if options != 0 {
                let info = if options & 8 != 0 {
                    Term::Map(
                        Map::from_pairs(fields, &self.atoms).expect("node event keys are atoms"),
                    )
                } else {
                    Term::list_from(
                        fields
                            .into_iter()
                            .map(|(key, value)| Term::tuple(vec![key, value])),
                    )
                };
                tuple.push(info);
            }
            let message = Term::tuple(tuple);
            let _ = self.send_signal(None, watcher, Signal::NodesMonitor { options, message });
        }
    }

    /// Give each local process of `ended`, from the newest monitor to the
    /// oldest, the monitor down signal `noconnection` for a monitor that it
    /// holds, or the removal of a monitor that a process of the other node
    /// holds on it, as `erts_do_net_exits` does for the `mld` list of a
    /// controller (`dist.c:1021-1117`, `304-321`,
    /// `erl_monitor_link.c:523-538`).
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// if let Some(ended) = node.dist().controller_exits(controller) {
    ///     scheduler.dist_monitor_down(&ended, &reason);
    /// }
    /// ```
    pub(in crate::runtime) fn dist_monitor_down(
        &mut self,
        ended: &super::dist_table::EndedConnection,
        reason: &Term,
    ) {
        let (name, watchers) = &ended.node_monitors;
        self.nodes_event(
            name,
            ended.node_events.0,
            Some(ended.connection),
            Some(reason),
            &ended.node_events.1,
        );
        for &watcher in watchers.keys() {
            let _ = self.send_signal(
                None,
                watcher,
                Signal::NodeMonitorDown {
                    node: name.clone(),
                    connection: ended.connection,
                },
            );
        }
        for &monitor in &ended.monitors {
            let signal = if monitor.side == MonitorSide::Watcher {
                // The local process holds the monitor, so the end of the
                // connection ends it.
                Signal::MonitorDown {
                    reference: monitor.key,
                    reason: Term::static_atom("noconnection"),
                }
            } else {
                // A process of the other node holds the monitor, so the end
                // of the connection removes it with no `DOWN` message
                // (`dist.c:315-320`).
                Signal::DistDemonitor {
                    watcher: monitor
                        .remote
                        .expect("a monitor of the other node names its watcher"),
                    reference: monitor.key,
                }
            };
            let _ = self.send_signal(None, monitor.process, signal);
        }
    }

    /// Send `DEMONITOR_P` for the monitor `key` that the local process
    /// `process` holds of `target`, a pid or a name of another node, on the
    /// connection `connection` that the monitor uses, as
    /// `erts_dsig_send_demonitor` does (`dist.c:1554-1570`). A connection
    /// with another id, or no connection, drops the packet.
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// scheduler.dist_demonitor(process, &target, key, connection)?;
    /// ```
    ///
    /// # Errors
    ///
    /// The errors of the encode of the control message.
    pub(super) fn dist_demonitor(
        &mut self,
        process: ProcessId,
        target: &RemoteTarget,
        key: MonitorKey,
        connection: u32,
    ) -> Result<(), BeamError> {
        let node = target.node();
        let to = target.term();
        {
            let node_ref = std::sync::Arc::clone(&self.node);
            let mut dist = node_ref.dist();
            dist.remove_monitor(process, &node, key, connection);
            // A PID identifies its dist entry while it is pending. A name
            // lookup only finds a connected entry (bif.c:533-546).
            if matches!(target, RemoteTarget::Pid(_))
                && dist.pending(&node)
                && dist.connection_id(&node) == connection
            {
                let control = monitor_control(DEMONITOR_P, &Term::Pid(process), &to, key, None);
                dist.enqueue_pending_monitor(&node, process, control);
                return Ok(());
            }
        }
        self.dist_output(process, &node, |flags, current| {
            (flags & MONITOR_FLAGS == MONITOR_FLAGS && current == connection).then(|| {
                let control = monitor_control(DEMONITOR_P, &Term::Pid(process), &to, key, None);
                (control, None)
            })
        })?;
        Ok(())
    }

    /// Send the monitor exit packet of the exit of the watched process
    /// `watched`, which `remote` monitors with the reference `key`, with the
    /// reason `reason` on the connection `connection` that the monitor uses,
    /// as `erts_dsig_send_m_exit` does (`dist.c:1496-1522`):
    /// `PAYLOAD_MONITOR_P_EXIT` with the reason as the message on a
    /// connection with `DFLAG_EXIT_PAYLOAD`, else `MONITOR_P_EXIT`. A
    /// connection with another id, or no connection, drops the packet
    /// (`erl_process.c:14126-14141`).
    ///
    /// The packet names the watched process `watched`, which is the atom of
    /// the name for a monitor that asked for a name (`ERTS_ML_FLG_NAME`,
    /// `dist.c:2368-2372`, `erl_process.c:13844-13847`).
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// scheduler.dist_monitor_exit(process, remote, &watched, key, connection, &reason)?;
    /// ```
    ///
    /// # Errors
    ///
    /// The errors of the encode of the control message and the reason.
    #[expect(clippy::needless_borrow, reason = "the closure passes the term on")]
    pub(super) fn dist_monitor_exit(
        &mut self,
        process: ProcessId,
        remote: RemotePid,
        watched: &Term,
        key: MonitorKey,
        connection: u32,
        reason: &Term,
    ) -> Result<(), BeamError> {
        let to = remote.term();
        self.dist_output(process, &remote.node(), |flags, current| {
            (current == connection).then(|| exit_packet(flags, &watched, &to, key, reason))
        })?;
        Ok(())
    }

    /// Give the `MONITOR_P` with the reference `reference` from the pid
    /// `watcher` of the node `node` on the connection `connection` to the
    /// local target `to`, as `erts_net_message` does (`dist.c:2341-2412`).
    /// The target is the pid of a local process or the atom of a registered
    /// name, which this node looks up now (`dist.c:2364-2379`): a live local
    /// process gets the monitor signal, and the connection that carried the
    /// packet holds the monitor before the process gets the signal. A
    /// connection that ended before the insert gives nothing
    /// (`dist.c:2387-2391`). A name that no process has, a pid of a process
    /// that is not alive, a pid of an old incarnation of the local node, and
    /// each other term that is not a pid answer with `MONITOR_P_EXIT` and the
    /// reason `noproc` from `to` (`dist.c:2398-2410`).
    ///
    /// # Errors
    ///
    /// A `watcher` that is not a pid of `node`, a `reference` that is not a
    /// reference, a `to` that is a pid of a third node (see
    /// [`super::dist_input`]), and the errors of the encode of the answer.
    pub(super) fn net_monitor(
        &mut self,
        node: &Atom,
        connection: u32,
        watcher: &Term,
        to: &Term,
        reference: &Term,
    ) -> Result<(), BeamError> {
        let remote = remote_of(node, watcher)?;
        let Some(key) = monitor_key(reference) else {
            return Err(error(super::dist_input::REJECTED));
        };
        // A local pid names its target, and a name asks for the process
        // that the name has now, as `erts_whereis_name_to_id` gives it
        // (`dist.c:2364-2372`).
        let (name, target) = match to {
            Term::Pid(target) => (None, Some(*target)),
            Term::Atom(name) => (Some(name.index()), self.whereis(self.atoms.name_of(name))),
            _ => (None, None),
        };
        let Some(target) = target else {
            match to {
                // A name that no live process has is not alive, so it
                // answers `noproc` (`dist.c:2368-2372`).
                Term::Atom(_) => {}
                // A pid of an old incarnation of the local node is not alive
                // either (`dist.c:2373-2377`).
                Term::External(id)
                    if external_id::pid_parts(to).is_some()
                        && id.node == self.atoms.this_node().0 => {}
                // A pid of a third node and each other term
                // (`dist.c:2373-2379`).
                _ => return Err(error(super::dist_input::REJECTED)),
            }
            return self.net_monitor_noproc(node, to, watcher, key, connection);
        };
        // The entry of the connection holds the monitor before the target
        // gets the signal, so the end of the connection ends the monitor even
        // when the signal waits in the queue. An insert that fails answers
        // nothing, because the connection of the packet ended
        // (`dist.c:2387-2391`).
        if !self.node.dist().insert_monitor(
            target,
            node,
            Some(remote),
            key,
            connection,
            MonitorSide::Watched,
        ) {
            return Ok(());
        }
        let signal = Signal::DistMonitor {
            watcher: remote,
            reference: key,
            name,
            connection,
        };
        if let Err(signal) = self.send_signal(None, target, signal) {
            // A target that is not alive loses the monitor again and answers
            // `MONITOR_P_EXIT` with `noproc` (`dist.c:2400-2410`).
            self.node
                .dist()
                .remove_monitor(target, node, key, connection);
            self.dead_target_signal(target, &signal);
        }
        Ok(())
    }

    /// Answer the `MONITOR_P` from the pid `watcher` for the target `to`,
    /// which no live process has, with `MONITOR_P_EXIT` and the reason
    /// `noproc` on the connection `connection` that it came on
    /// (`dist.c:2398-2410`). A connection that ended drops the answer.
    ///
    /// # Errors
    ///
    /// The errors of the encode of the control message.
    pub(super) fn net_monitor_noproc(
        &mut self,
        node: &Atom,
        to: &Term,
        watcher: &Term,
        key: MonitorKey,
        connection: u32,
    ) -> Result<(), BeamError> {
        self.dist_output(ProcessId(0), node, |_, current| {
            (current == connection)
                .then(|| exit_packet(0, to, watcher, key, &Term::static_atom("noproc")))
        })?;
        Ok(())
    }

    /// Give the `DEMONITOR_P` with the reference `reference` from the pid
    /// `watcher` of the node `node` to the local target `to`, as
    /// `erts_net_message` does (`dist.c:2414-2470`): a pid of a local
    /// process or the atom of a registered name gives the target the
    /// demonitor signal, which removes its target monitor. A process that is
    /// not alive drops it, and a pid of an old incarnation of the local node
    /// drops it too (`erl_proc_sig_queue.c:6564-6633`).
    ///
    /// # Errors
    ///
    /// A `watcher` that is not a pid of `node`, a `reference` that is not a
    /// reference, and a `to` that is not a local pid, a name of this node, or
    /// a pid of an old incarnation of the local node (see
    /// [`super::dist_input`]).
    pub(super) fn net_demonitor(
        &mut self,
        node: &Atom,
        connection: u32,
        watcher: &Term,
        to: &Term,
        reference: &Term,
    ) -> Result<(), BeamError> {
        let remote = remote_of(node, watcher)?;
        let Some(key) = monitor_key(reference) else {
            return Err(error(super::dist_input::REJECTED));
        };
        // A pid of a local process names its target, and a name names the
        // target that the monitor of this connection holds
        // (`dist.c:2431-2457`).
        let target = match to {
            Term::Pid(target) => Some(*target),
            Term::Atom(_) => self.node.dist().watched_process(remote, key),
            // A pid of an old incarnation of the local node drops the packet
            // (`dist.c:2436-2440`).
            Term::External(id)
                if external_id::pid_parts(to).is_some() && id.node == self.atoms.this_node().0 =>
            {
                return Ok(());
            }
            _ => return Err(error(super::dist_input::REJECTED)),
        };
        // No process has the name now, so no monitor ends
        // (`dist.c:2441-2457`).
        let Some(target) = target else {
            return Ok(());
        };
        self.node
            .dist()
            .remove_monitor(target, node, key, connection);
        let _ = self.send_signal(
            None,
            target,
            Signal::DistDemonitor {
                watcher: remote,
                reference: key,
            },
        );
        Ok(())
    }

    /// Give the `MONITOR_P_EXIT` or `PAYLOAD_MONITOR_P_EXIT` with the reason
    /// that `reason` gives from the pid or the registered name `watched` of
    /// the node `node` to the local pid `watcher`, as `erts_net_message`
    /// does (`dist.c:2688-2746`): a live process gets the monitor down
    /// signal, and a process that is not alive drops it before `reason`
    /// runs. The `DOWN` message names the monitored target from the origin
    /// monitor of `watcher`, so the atom of the packet only has to be a name
    /// (`dist.c:2732-2736`).
    ///
    /// # Errors
    ///
    /// A `watched` that is not a pid of `node` and not an atom, and a
    /// `watcher` that is not a local pid (see [`super::dist_input`]), and
    /// the errors of `reason`.
    /// ERTS drops the packet for a `watcher` of an old incarnation of the
    /// local node (`dist.c:2717-2723`); PBeam rejects it, as
    /// [`super::dist_link::Scheduler::net_link_exit`] rejects such a target
    /// of `EXIT`.
    #[expect(clippy::similar_names, reason = "the names are those of ERTS")]
    pub(super) fn net_monitor_exit(
        &mut self,
        node: &Atom,
        watched: &Term,
        watcher: &Term,
        reference: &Term,
        reason: impl FnOnce(&Self) -> Result<Term, BeamError>,
    ) -> Result<(), BeamError> {
        // The watched term is a pid of the node or the atom of a name
        // (`dist.c:2732-2736`); each other term rejects the packet.
        match watched {
            Term::Atom(_) => {}
            _ if remote_of(node, watched).is_ok() => {}
            _ => return Err(error(super::dist_input::REJECTED)),
        }
        let Some(key) = monitor_key(reference) else {
            return Err(error(super::dist_input::REJECTED));
        };
        let Term::Pid(target) = *watcher else {
            return Err(error(super::dist_input::REJECTED));
        };
        if self.node.live_owner(target).is_some() {
            let down = Signal::MonitorDown {
                reference: key,
                reason: reason(self)?,
            };
            let _ = self.send_signal(None, target, down);
        }
        Ok(())
    }
}

/// Return the term that a monitor exit packet names as the watched process
/// `target`: its pid, or the atom of the name that the `MONITOR_P` packet
/// asked for (`ERTS_ML_FLG_NAME`, `dist.c:2368-2372`,
/// `erl_process.c:13844-13847`).
///
/// # Examples
///
/// ```rust,ignore
/// assert_eq!(monitored(target, None), Term::Pid(target));
/// ```
pub(in crate::runtime) fn monitored(target: ProcessId, name: Option<u32>) -> Term {
    match name {
        Some(name) => Term::Atom(Atom::from_index(name)),
        None => Term::Pid(target),
    }
}

/// Return the pid `from`, which must be a pid of the node `node` of a
/// connection (`dist.c:2346-2354`).
///
/// # Errors
///
/// Another term (see [`super::dist_input`]).
pub(super) fn remote_of(node: &Atom, from: &Term) -> Result<RemotePid, BeamError> {
    match RemotePid::of(from) {
        Some(remote) if remote.node == node.index() => Ok(remote),
        _ => Err(error(super::dist_input::REJECTED)),
    }
}

/// Return the key of the monitor tree of the reference `reference`, or
/// `None` for a term that is not a reference. ERTS asks only for a reference
/// here (`dist.c:2360-2362`, `2702-2704`), and the reference of a monitor
/// that a local process holds comes back from the other node as an internal
/// reference, because it names the local node (`dec_is_this_node`,
/// `external.c:3123-3129`).
pub(super) fn monitor_key(reference: &Term) -> Option<MonitorKey> {
    match reference {
        Term::Reference(reference) => Some(MonitorKey::Local(*reference)),
        external @ Term::External(_) => Some(MonitorKey::Remote(RemoteReference::of(external)?)),
        _ => None,
    }
}

/// Return the control message of a monitor packet:
/// `{Operation, From, To, Ref}` for `MONITOR_P`, `DEMONITOR_P`, and
/// `PAYLOAD_MONITOR_P_EXIT`, and `{Operation, From, To, Ref, Reason}` for
/// `MONITOR_P_EXIT` (`dist.c:1504-1560`).
fn monitor_control(
    operation: i64,
    from: &Term,
    to: &Term,
    key: MonitorKey,
    reason: Option<&Term>,
) -> Term {
    let mut items = vec![
        Term::Integer(operation),
        from.clone(),
        to.clone(),
        key.term(),
    ];
    items.extend(reason.cloned());
    Term::tuple(items)
}

/// Return the packet of a monitor exit signal from `watched` to `to` with the
/// reference `key` and the reason `reason` on a connection with the flags
/// `flags` (`dist.c:1504-1520`): `PAYLOAD_MONITOR_P_EXIT` with the reason as
/// the message on a connection with `DFLAG_EXIT_PAYLOAD`, else
/// `MONITOR_P_EXIT` with the reason in the control message.
fn exit_packet(
    flags: u64,
    watched: &Term,
    to: &Term,
    key: MonitorKey,
    reason: &Term,
) -> (Term, Option<Term>) {
    if flags & EXIT_PAYLOAD != 0 {
        let control = monitor_control(PAYLOAD_MONITOR_P_EXIT, watched, to, key, None);
        (control, Some(reason.clone()))
    } else {
        let control = monitor_control(MONITOR_P_EXIT, watched, to, key, Some(reason));
        (control, None)
    }
}

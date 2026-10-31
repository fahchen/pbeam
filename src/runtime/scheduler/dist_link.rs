//! The links of local processes to processes of another node, as the
//! `ERTS_LNK_TYPE_DIST_PROC` links of ERTS: `erlang:link/1` and
//! `erlang:unlink/1` with a pid of another node (`bif.c:263-394`,
//! `1295-1348`), the `LINK`, `UNLINK_ID`, `UNLINK_ID_ACK`, `EXIT`, and
//! `PAYLOAD_EXIT` packets of a connection (`dist.c:2224-2339`,
//! `2745-2805`), and the link exit signals that the exit of a linked
//! process sends to another node (`erl_process.c:14101-14147`,
//! `13981-14040`). Each link keeps the id of the connection that it uses,
//! as `ErtsELink.dist` does, and a link signal goes only on that
//! connection. The end of a connection gives each local process with a
//! link of that connection the link exit signal `noconnection`
//! (`dist.c:1021-1117`, `erl_proc_sig_queue.c:4693-4706`).

use super::{Scheduler, dist_output::EXIT_PAYLOAD};
use crate::{
    error::{BeamError, error},
    runtime::{
        process::{LinkId, LinkState, RemotePid},
        signal::Signal,
    },
    term::{Atom, ProcessId, Term, external_id},
};
use num_bigint::BigInt;

/// `DOP_LINK` (`dist.h:143`).
const LINK: i64 = 1;
/// `DOP_EXIT` (`dist.h:145`).
const EXIT: i64 = 3;
/// `DOP_PAYLOAD_EXIT` (`dist.h:165`).
const PAYLOAD_EXIT: i64 = 24;
/// `DOP_UNLINK_ID` (`dist.h:179`).
const UNLINK_ID: i64 = 35;
/// `DOP_UNLINK_ID_ACK` (`dist.h:180`).
const UNLINK_ID_ACK: i64 = 36;

impl Scheduler {
    /// Link the current process to the pid `pid` of another node, as
    /// `link_opt` does (`bif.c:263-394`). An active link changes nothing.
    /// Else, with a connection to the node, the link becomes active on the
    /// connection and the node gets `LINK` (`dist.c:1608-1614`). Without a
    /// connection the link stays, and the current process gets the link
    /// exit signal `noconnection` (`bif.c:324-340`), which removes it.
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// scheduler.dist_link(&remote_pid)?;
    /// ```
    ///
    /// # Errors
    ///
    /// The errors of the encode of the control message.
    pub(in crate::runtime) fn dist_link(&mut self, pid: &Term) -> Result<(), BeamError> {
        let remote = RemotePid::of(pid).expect("the caller gives a pid of another node");
        let link = LinkId::Remote(remote);
        let current = self.current_process().id;
        let state = self.current_process_mut().links.get(&link).copied();
        if state.is_some_and(LinkState::is_active) {
            return Ok(());
        }
        let mut used = None;
        self.dist_output(current, &remote.node(), |_, connection| {
            used = Some(connection);
            Some((link_control(LINK, current, pid), None))
        })?;
        let connection = used.unwrap_or_default();
        self.current_process_mut().links.insert(
            link,
            LinkState {
                unlinking: 0,
                connection,
            },
        );
        if let Some(used) = used {
            debug_assert_eq!(used, self.node.dist().connection_id(&remote.node()));
            self.node.dist().insert_link(current, remote);
        }
        if used.is_none() {
            let exit = Signal::Exit {
                from: link,
                reason: Term::static_atom("noconnection"),
                linked: true,
                normal_kills: false,
                alias: None,
            };
            self.send_signal(None, current, exit)
                .expect("the current process gets the exit signal");
        }
        Ok(())
    }

    /// Give each local process of `ended`, from the newest link to the
    /// oldest, the link exit signal `noconnection`, as `erts_do_net_exits`
    /// does for the `mld` list of a controller (`dist.c:1021-1117`,
    /// `380-384`, `erl_monitor_link.c:523-538`,
    /// `erl_proc_sig_queue.c:2552-2576`). A signal of a connection that
    /// ended leaves a link of a newer connection
    /// (`erl_proc_sig_queue.c:4693-4706`).
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// if let Some(ended) = node.dist().controller_exits(controller) {
    ///     scheduler.dist_connection_down(&ended);
    /// }
    /// ```
    pub(in crate::runtime) fn dist_connection_down(
        &mut self,
        ended: &super::dist_table::EndedConnection,
    ) {
        for &(process, remote) in &ended.links {
            let signal = Signal::ConnectionLost {
                from: remote,
                connection: ended.connection,
            };
            let _ = self.send_signal(None, process, signal);
        }
    }

    /// Unlink the current process from the pid `pid` of another node, as
    /// `unlink_1` does (`bif.c:1295-1348`). Only an active link changes: it
    /// becomes unlinking with a new identifier, and the node gets
    /// `UNLINK_ID` when the connection of the link is still up
    /// (`dist.c:1626-1636`). The link goes when `UNLINK_ID_ACK` comes.
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// scheduler.dist_unlink(&remote_pid)?;
    /// ```
    ///
    /// # Errors
    ///
    /// The errors of the encode of the control message.
    pub(in crate::runtime) fn dist_unlink(&mut self, pid: &Term) -> Result<(), BeamError> {
        let remote = RemotePid::of(pid).expect("the caller gives a pid of another node");
        let link = LinkId::Remote(remote);
        let current = self.current_process().id;
        let Some(state) = self.current_process_mut().links.get(&link).copied() else {
            return Ok(());
        };
        if !state.is_active() {
            return Ok(());
        }
        let unlinking = self.node.next_unlink_id();
        self.current_process_mut().links.insert(
            link,
            LinkState {
                unlinking,
                connection: state.connection,
            },
        );
        self.dist_output(current, &remote.node(), |_, connection| {
            (connection == state.connection).then(|| {
                let control = unlink_control(UNLINK_ID, unlinking, current, pid);
                (control, None)
            })
        })?;
        Ok(())
    }

    /// Send the link exit signal with the reason `reason` from the local
    /// pid `from` to the pid `to` of another node on the connection
    /// `connection`, as `erts_dsig_send_exit_tt` does
    /// (`dist.c:1716-1785`): `PAYLOAD_EXIT` with the reason as the message
    /// on a connection with `DFLAG_EXIT_PAYLOAD`, else `EXIT`. A connection
    /// with another id, or no connection, drops it
    /// (`erl_process.c:14126-14141`).
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// scheduler.dist_link_exit(id, remote, connection, &Term::static_atom("normal"))?;
    /// ```
    ///
    /// # Errors
    ///
    /// The errors of the encode of the control message and the reason.
    pub(super) fn dist_link_exit(
        &mut self,
        from: ProcessId,
        to: RemotePid,
        connection: u32,
        reason: &Term,
    ) -> Result<(), BeamError> {
        let pid = to.term();
        self.dist_output(from, &to.node(), |flags, current| {
            (current == connection).then(|| exit_packet(flags, &Term::Pid(from), &pid, reason))
        })?;
        Ok(())
    }

    /// Answer the `UNLINK_ID` with the identifier `id` from the pid `from`
    /// of another node to the local pid `to` with `UNLINK_ID_ACK` on the
    /// connection `connection` that it came on, as `reply_dist_unlink_ack`
    /// does (`erl_proc_sig_queue.c:2752-2790`). A connection with another
    /// id, or no connection, drops the answer.
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// scheduler.dist_unlink_reply(id, remote, 7, connection);
    /// ```
    pub(super) fn dist_unlink_reply(
        &mut self,
        to: ProcessId,
        from: RemotePid,
        id: u64,
        connection: u32,
    ) {
        let pid = from.term();
        self.dist_output(to, &from.node(), |_, current| {
            (current == connection).then(|| (unlink_control(UNLINK_ID_ACK, id, to, &pid), None))
        })
        .expect("a control message of pids and integers encodes");
    }

    /// Answer the `LINK` from the pid `from` of another node to the local
    /// process `to` that is not alive with `EXIT` and the reason `noproc`
    /// on the connection `connection` that it came on
    /// (`erl_proc_sig_queue.c:7363-7368`; `erl_process.c:14115-14141`).
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// scheduler.dist_link_noproc(id, remote, connection);
    /// ```
    pub(super) fn dist_link_noproc(&mut self, to: ProcessId, from: RemotePid, connection: u32) {
        self.dist_link_exit(to, from, connection, &Term::static_atom("noproc"))
            .expect("a control message of pids and atoms encodes");
    }

    /// Give `LINK` from the pid `from` of the node `node` to `to`, as
    /// `erts_net_message` does (`dist.c:2224-2275`): a live local process
    /// gets the link signal. A local pid of a process that is not alive, a
    /// pid of an old incarnation of the local node, and each other term
    /// that is not a pid answer with `EXIT` and the reason `noproc` from
    /// `to`.
    ///
    /// # Errors
    ///
    /// A `from` that is not a pid of `node` and a pid of a third node (see
    /// [`super::dist_input`]), and the errors of the encode of the answer.
    pub(super) fn net_link(
        &mut self,
        node: &Atom,
        from: &Term,
        to: &Term,
    ) -> Result<(), BeamError> {
        let remote = remote_of(node, from)?;
        let connection = self.input_connection(node);
        match to {
            Term::Pid(target) => {
                let signal = Signal::DistLink {
                    from: remote,
                    connection,
                };
                if let Err(signal) = self.send_signal(None, *target, signal) {
                    self.dead_target_signal(*target, &signal);
                }
                return Ok(());
            }
            Term::External(id)
                if external_id::pid_parts(to).is_some() && id.node != self.atoms.this_node().0 =>
            {
                return Err(error(super::dist_input::REJECTED));
            }
            _ => {}
        }
        self.dist_output(ProcessId(0), node, |flags, current| {
            (current == connection)
                .then(|| exit_packet(flags, to, from, &Term::static_atom("noproc")))
        })?;
        Ok(())
    }

    /// Give `UNLINK_ID` with the identifier `id` from the pid `from` of the
    /// node `node` to the local pid `to`, as `erts_net_message` does
    /// (`dist.c:2284-2313`). A process that is not alive answers with
    /// `UNLINK_ID_ACK` (`erl_proc_sig_queue.c:2709-2728`).
    ///
    /// # Errors
    ///
    /// An `id` that is not an integer from 1 to 2^64-1, a `from` that is
    /// not a pid of `node`, and a `to` that is not a local pid (see
    /// [`super::dist_input`]).
    pub(super) fn net_unlink(
        &mut self,
        node: &Atom,
        id: &Term,
        from: &Term,
        to: &Term,
    ) -> Result<(), BeamError> {
        let (id, from, target) = unlink_parts(node, id, from, to)?;
        if id == 0 {
            return Err(error(super::dist_input::REJECTED));
        }
        let connection = self.input_connection(node);
        let signal = Signal::DistUnlink {
            from,
            id,
            connection,
        };
        if let Err(signal) = self.send_signal(None, target, signal) {
            self.dead_target_signal(target, &signal);
        }
        Ok(())
    }

    /// Give `UNLINK_ID_ACK` with the identifier `id` from the pid `from` of
    /// the node `node` to the local pid `to`, as `erts_net_message` does
    /// (`dist.c:2315-2339`). A process that is not alive drops it
    /// (`erl_proc_sig_queue.c:2730-2750`).
    ///
    /// # Errors
    ///
    /// An `id` that is not an integer from 0 to 2^64-1, a `from` that is
    /// not a pid of `node`, and a `to` that is not a local pid (see
    /// [`super::dist_input`]).
    pub(super) fn net_unlink_ack(
        &mut self,
        node: &Atom,
        id: &Term,
        from: &Term,
        to: &Term,
    ) -> Result<(), BeamError> {
        let (id, from, target) = unlink_parts(node, id, from, to)?;
        let _ = self.send_signal(None, target, Signal::DistUnlinkAck { from, id });
        Ok(())
    }

    /// Give `EXIT` or `PAYLOAD_EXIT` from the pid `from` of the node `node`
    /// with the reason that `reason` gives to the local pid `to`, as
    /// `erts_net_message` does (`dist.c:2745-2805`): a live process gets
    /// the link exit signal, and a process that is not alive drops it
    /// before `reason` runs.
    ///
    /// # Errors
    ///
    /// A `from` that is not a pid of `node` and a `to` that is not a local
    /// pid (see [`super::dist_input`]), and the errors of `reason`.
    pub(super) fn net_link_exit(
        &mut self,
        node: &Atom,
        from: &Term,
        to: &Term,
        reason: impl FnOnce(&Self) -> Result<Term, BeamError>,
    ) -> Result<(), BeamError> {
        let remote = remote_of(node, from)?;
        let Term::Pid(target) = *to else {
            return Err(error(super::dist_input::REJECTED));
        };
        if self.node.live_owner(target).is_some() {
            let exit = Signal::Exit {
                from: LinkId::Remote(remote),
                reason: reason(self)?,
                linked: true,
                normal_kills: false,
                alias: None,
            };
            let _ = self.send_signal(None, target, exit);
        }
        Ok(())
    }

    /// Return the id of the connection of `node`, on which a packet came.
    fn input_connection(&self, node: &Atom) -> u32 {
        self.node
            .dist()
            .connected(node)
            .map(|(_, _, connection)| connection)
            .expect("a packet comes on a connection")
    }
}

/// Return the pid `from`, which must be a pid of the node `node` of a
/// connection (`dist.c:2234-2238`).
///
/// # Errors
///
/// Another term (see [`super::dist_input`]).
fn remote_of(node: &Atom, from: &Term) -> Result<RemotePid, BeamError> {
    match RemotePid::of(from) {
        Some(remote) if remote.node == node.index() => Ok(remote),
        _ => Err(error(super::dist_input::REJECTED)),
    }
}

/// Return the identifier, the sender, and the local target of
/// `UNLINK_ID` and `UNLINK_ID_ACK`: `term_to_Uint64` of `id`, a pid of the
/// node `node`, and a local pid (`dist.c:2287-2335`).
///
/// # Errors
///
/// Another term (see [`super::dist_input`]).
fn unlink_parts(
    node: &Atom,
    id: &Term,
    from: &Term,
    to: &Term,
) -> Result<(u64, RemotePid, ProcessId), BeamError> {
    let id = id.bigint().and_then(|id| u64::try_from(id.as_ref()).ok());
    let from = remote_of(node, from)?;
    match (id, to) {
        (Some(id), Term::Pid(target)) => Ok((id, from, *target)),
        _ => Err(error(super::dist_input::REJECTED)),
    }
}

/// Return the control message `{Operation, From, To}` of `LINK`.
fn link_control(operation: i64, from: ProcessId, to: &Term) -> Term {
    Term::tuple(vec![Term::Integer(operation), Term::Pid(from), to.clone()])
}

/// Return the control message `{Operation, Id, From, To}` of
/// `UNLINK_ID` and `UNLINK_ID_ACK` (`dist.c:1626-1660`).
fn unlink_control(operation: i64, id: u64, from: ProcessId, to: &Term) -> Term {
    Term::tuple(vec![
        Term::Integer(operation),
        Term::from_bigint(BigInt::from(id)),
        Term::Pid(from),
        to.clone(),
    ])
}

/// Return the packet of a link exit signal from `from` to `to` on a
/// connection with the flags `flags` (`dist.c:1744-1747`).
fn exit_packet(flags: u64, from: &Term, to: &Term, reason: &Term) -> (Term, Option<Term>) {
    if flags & EXIT_PAYLOAD != 0 {
        let control = Term::tuple(vec![Term::Integer(PAYLOAD_EXIT), from.clone(), to.clone()]);
        (control, Some(reason.clone()))
    } else {
        let control = Term::tuple(vec![
            Term::Integer(EXIT),
            from.clone(),
            to.clone(),
            reason.clone(),
        ]);
        (control, None)
    }
}

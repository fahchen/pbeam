//! The output of a distribution connection: the messages that a local
//! process sends to a pid, a name, or an alias of another node, as
//! `erts_dsig_send_msg` builds them (`dist.c:1592-1712`). A connection can
//! use the atom-cache header and fragments.

use std::sync::Arc;

use super::{Scheduler, dist_table::Controller};
use crate::{
    error::BeamError,
    runtime::interpreter::bif::{encode_dist, encode_dist_cached},
    term::{Atom, Binary, PortId, ProcessId, Term, external_id::ExternalData, pid_word},
};

/// `PASS_THROUGH` (`external.h`): the first byte of a packet without a
/// distribution header.
const PASS_THROUGH: u8 = 112;

/// `DFLAG_FRAGMENTS` and `ERTS_DIST_FRAGMENT_SIZE` (`dist.h:53`, `208`).
const FRAGMENTS: u64 = 0x80_0000;
const FRAGMENT_SIZE: usize = 64 * 1024;

/// `DFLAG_SEND_SENDER` (`dist.h:49`).
const SEND_SENDER: u64 = 0x80000;

/// `DFLAG_ALIAS` (`dist.h:67`).
const ALIAS: u64 = 0x8 << 32;

/// `DFLAG_ALTACT_SIG` (`dist.h:69`).
const ALTACT_SIG: u64 = 0x20 << 32;

/// `ERTS_DOP_ALTACT_SIG_FLG_PRIO` (`dist.h:188`).
const ALTACT_SIG_PRIO: i64 = 1;

/// `ERTS_DOP_ALTACT_SIG_FLG_ALIAS` (`dist.h:190`).
const ALTACT_SIG_ALIAS: i64 = 1 << 2;

/// `ERTS_DOP_ALTACT_SIG_FLG_NAME` (`dist.h:191`).
const ALTACT_SIG_NAME: i64 = 1 << 3;

/// `ERTS_DOP_ALTACT_SIG_FLG_EXIT` (`dist.h:192`).
const ALTACT_SIG_EXIT: i64 = 1 << 4;

/// `DFLAG_EXIT_PAYLOAD` (`dist.h:52`).
pub(super) const EXIT_PAYLOAD: u64 = 0x40_0000;

/// Whether a send has the option `priority` of `erlang:send/3`
/// (`bif.c:2718-2719`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(in crate::runtime) enum SendPriority {
    /// An ordinary send.
    Normal,
    /// A send with the option `priority`.
    Priority,
}

impl Scheduler {
    /// Send `message` from the current process to `to`, a pid, a reference,
    /// or `{Name, Node}` of the node `node`, as `remote_send` does for a
    /// connected node (`bif.c:2405-2441`), and return `true`. Return
    /// `false` when the node has no connection, for the caller to drop the
    /// message.
    ///
    /// The control message is `ALTACT_SIG_SEND` for a reference or for the
    /// send with `SendPriority::Priority` (the option `priority` of `erlang:send/3`,
    /// `bif.c:2718-2719`) on a connection with `DFLAG_ALTACT_SIG`, else
    /// `REG_SEND` for a name, `ALIAS_SEND` for a reference, and
    /// `SEND_SENDER` or `SEND` for a pid, as the flags of the connection
    /// permit (`erts_dsig_send_msg`, `dist.c:1640-1702`). A connection without
    /// `DFLAG_ALIAS` drops a message to a reference (`bif.c:2406-2414`). A
    /// process controller gets the packet in its output queue, and a port
    /// controller writes it.
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// assert!(scheduler.dist_send(&node, &Term::External(pid), &message, SendPriority::Normal)?);
    /// ```
    ///
    /// # Errors
    ///
    /// A term that PBeam does not encode yet, and the errors of the port
    /// controller (see [`Self::send_events`]).
    pub(in crate::runtime) fn dist_send(
        &mut self,
        node: &Atom,
        to: &Term,
        message: &Term,
        priority: SendPriority,
        connect: bool,
    ) -> Result<bool, BeamError> {
        let caller = self.current_process().id;
        let from = Term::Pid(caller);
        // A pending connection accepts an ordinary registered-name send even
        // with noconnect (bif.c:2391-2417). Keep it until the negotiated
        // flags are known, then encode it before publishing the connection.
        if matches!(to, Term::Tuple(_))
            && priority == SendPriority::Normal
            && self.node.net_kernel().is_some()
        {
            let node_ref = Arc::clone(&self.node);
            let mut dist = node_ref.dist();
            if dist.pending(node) {
                dist.enqueue_pending_send(node, caller, to.clone(), message.clone());
                return Ok(true);
            }
            let idle = dist.connected(node).is_none();
            drop(dist);
            if idle && connect && self.dist_auto_connect_send(node, to, message)? {
                return Ok(true);
            }
        }
        self.dist_output(caller, node, |flags, _| {
            if is_remote_reference(to) && flags & ALIAS == 0 {
                return None;
            }
            Some((
                send_control(flags, from, to, priority),
                Some(message.clone()),
            ))
        })
    }

    /// Start an idle connection for a registered-name send (`dist.c:5713-5759`).
    fn dist_auto_connect_send(
        &mut self,
        name: &Atom,
        to: &Term,
        message: &Term,
    ) -> Result<bool, BeamError> {
        let node = Arc::clone(&self.node);
        if node.net_kernel().is_none() {
            return Ok(false);
        }
        let Some(super::node::Named::Process(kernel)) = self.name_owner("net_kernel") else {
            return Ok(false);
        };
        let reference = self.make_magic_ref()?;
        let now = self.timers.now();
        let start = u32::try_from(now & i64::from(u32::MAX)).expect("the low 32 bits fit in u32");
        let event_lock = {
            let mut dist = node.dist();
            dist.expire_known(now);
            dist.insert(name, reference, start);
            dist.event_lock(name)
        };
        let _event = event_lock
            .lock()
            .expect("the node event lock is not poisoned");
        let mut dist = node.dist();
        if node.is_going_down() || node.live_owner(kernel).is_none() {
            return Ok(false);
        }
        if dist.connected(name).is_some() {
            return Ok(false);
        }
        let start = !dist.pending(name);
        if start {
            dist.new_connection(name);
        }
        let handle = dist.current_dhandle(name);
        let caller = self.current_process().id;
        dist.enqueue_pending_send(name, caller, to.clone(), message.clone());
        drop(dist);
        if start {
            self.send_message(
                caller,
                kernel,
                Term::tuple(vec![
                    Term::static_atom("auto_connect"),
                    Term::Atom(name.clone()),
                    handle,
                ]),
            );
        }
        Ok(true)
    }

    /// Send the exit signal of `erlang:exit/2,3` from the current process
    /// to the pid or the alias `to` of the node `node` with the reason
    /// `reason`, as `erts_dsig_send_exit2` does (`dist.c:1788-1828`), and
    /// return `true`; return `false` when the node has no connection. The
    /// control message is `ALTACT_SIG_SEND` with the exit flag for an alias
    /// or with `priority` on a connection with `DFLAG_ALTACT_SIG`, else
    /// `PAYLOAD_EXIT2` with the reason as the message on a connection with
    /// `DFLAG_EXIT_PAYLOAD`, else `EXIT2`. A connection without
    /// `DFLAG_ALTACT_SIG` drops an exit signal to an alias.
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// assert!(scheduler.dist_exit(&node, &Term::External(pid), &reason, SendPriority::Normal)?);
    /// ```
    ///
    /// # Errors
    ///
    /// The errors of [`Self::dist_send`].
    pub(in crate::runtime) fn dist_exit(
        &mut self,
        node: &Atom,
        to: &Term,
        reason: &Term,
        priority: SendPriority,
    ) -> Result<bool, BeamError> {
        let caller = self.current_process().id;
        let from = Term::Pid(caller);
        let alias = is_remote_reference(to);
        let prio = priority == SendPriority::Priority;
        self.dist_output(caller, node, |flags, _| {
            if (prio || alias) && flags & ALTACT_SIG != 0 {
                let mut fs = ALTACT_SIG_EXIT;
                if alias {
                    fs |= ALTACT_SIG_ALIAS;
                }
                if prio {
                    fs |= ALTACT_SIG_PRIO;
                }
                let control =
                    Term::tuple(vec![Term::Integer(37), Term::Integer(fs), from, to.clone()]);
                return Some((control, Some(reason.clone())));
            }
            if alias {
                return None;
            }
            if flags & EXIT_PAYLOAD != 0 {
                let control = Term::tuple(vec![Term::Integer(26), from, to.clone()]);
                return Some((control, Some(reason.clone())));
            }
            let control = Term::tuple(vec![Term::Integer(8), from, to.clone(), reason.clone()]);
            Some((control, None))
        })
    }

    /// Put the packet of the control message and the message that `build`
    /// gives for the flags and the id of the connection of `node` in the
    /// output of the connection, and return `true`; `build` gives `None`
    /// for a signal that the connection drops. Return `false` when the node
    /// has no connection. A process controller gets the packet in its
    /// output queue, with the `dist_data` message from `sender` when it
    /// asked for one, and a port controller writes it.
    ///
    /// # Errors
    ///
    /// The errors of [`Self::dist_send`].
    pub(super) fn dist_output(
        &mut self,
        sender: ProcessId,
        node: &Atom,
        build: impl FnOnce(u64, u32) -> Option<(Term, Option<Term>)>,
    ) -> Result<bool, BeamError> {
        self.dist_output_monitor(sender, node, None, build)
    }

    /// Send the packet that `build` makes as [`Self::dist_output`] does, and
    /// put `monitor` in the monitor list of the connection before the send,
    /// as `erts_internal_dist_spawn_request_4` creates the pending monitor
    /// before `erts_dsig_send_spawn_request` (`dist.c:6111-6120`). Return
    /// whether the node has a connection.
    ///
    /// # Errors
    ///
    /// The errors of [`Self::dist_send`].
    pub(super) fn dist_output_monitor(
        &mut self,
        sender: ProcessId,
        node: &Atom,
        monitor: Option<super::dist_table::DistMonitor>,
        build: impl FnOnce(u64, u32) -> Option<(Term, Option<Term>)>,
    ) -> Result<bool, BeamError> {
        let node_ref = Arc::clone(&self.node);
        let mut dist = node_ref.dist();
        let Some((flags, controller, connection)) = dist.connected(node) else {
            return Ok(false);
        };
        let Some((control, message)) = build(flags, connection) else {
            return Ok(true);
        };
        // The monitor joins the connection before the packet leaves, so the
        // end of the connection sees it (`dist.c:6111-6120`).
        if let Some(monitor) = monitor {
            dist.insert_monitor_now(node, monitor);
        }
        let packets =
            self.dist_encode_packets(&mut dist, sender, node, flags, &control, message)?;
        let mut notify = false;
        for packet in packets {
            notify |= dist.enqueue(node, packet);
        }
        match controller {
            Controller::Process(process) => {
                if notify {
                    drop(dist);
                    self.send_message(sender, process, Term::static_atom("dist_data"));
                }
            }
            Controller::Port(port) => {
                drop(dist);
                self.dist_port_send(port)?;
            }
        }
        Ok(true)
    }

    /// Encode pending sends before releasing the lock that publishes a connection.
    pub(in crate::runtime) fn dist_flush_pending(
        &mut self,
        dist: &mut super::dist_table::DistTable,
        node: &Atom,
        flags: u64,
    ) -> Result<(), BeamError> {
        for (sender, to, message) in dist.take_pending_sends(node) {
            let control = send_control(flags, Term::Pid(sender), &to, SendPriority::Normal);
            for packet in
                self.dist_encode_packets(dist, sender, node, flags, &control, Some(message))?
            {
                dist.enqueue(node, packet);
            }
        }
        Ok(())
    }

    /// Encode one distribution signal with the flags of its connection.
    fn dist_encode_packets(
        &mut self,
        dist: &mut super::dist_table::DistTable,
        sender: ProcessId,
        node: &Atom,
        flags: u64,
        control: &Term,
        message: Option<Term>,
    ) -> Result<Vec<Vec<Binary>>, BeamError> {
        let (elements, header_len, control_len) = if flags & 0x2000 != 0 {
            encode_dist_cached(self, control, message.as_ref(), dist.atom_cache_out(node))?
        } else if flags & FRAGMENTS != 0 {
            let mut packet = vec![131, b'D', 0];
            let mut control_len = 0;
            for term in std::iter::once(control).chain(message.iter()) {
                let bytes: Vec<u8> = encode_dist(self, term)?
                    .iter()
                    .flat_map(Binary::to_bytes)
                    .collect();
                if control_len == 0 {
                    control_len = bytes.len() - 1;
                }
                packet.extend_from_slice(&bytes[1..]);
            }
            (vec![Binary::from(packet)], 3, control_len)
        } else {
            let mut elements = vec![Binary::from(vec![PASS_THROUGH])];
            elements.extend(encode_dist(self, control)?);
            if let Some(message) = message {
                elements.extend(encode_dist(self, &message)?);
            }
            (elements, 0, 0)
        };
        let packets = if flags & FRAGMENTS != 0 {
            fragment_packet(elements, sender, header_len, control_len)
        } else {
            vec![elements]
        };
        Ok(packets)
    }

    /// Drain the connection queue under the port lock. The queue takes each
    /// packet under the same distribution lock as its atom-cache update, so
    /// no later packet can reach the port before an earlier cache update.
    /// The port lock comes before the distribution lock. A port step that
    /// already holds the port lock defers the drain until the step ends.
    ///
    /// # Errors
    ///
    /// The errors of [`Self::send_events`].
    pub(in crate::runtime) fn dist_port_send(&mut self, port: PortId) -> Result<(), BeamError> {
        if self.locked_port == Some(port) {
            self.deferred_dist = true;
            return Ok(());
        }
        let node = Arc::clone(&self.node);
        let exit = node.with_port(port, |item| -> Result<Option<Term>, BeamError> {
            loop {
                let mut dist = node.dist();
                let Some(name) = dist.controlled_by(Controller::Port(port)) else {
                    return Ok(None);
                };
                let Some(elements) = dist.dequeue(&name) else {
                    return Ok(None);
                };
                drop(dist);
                let bytes: Vec<u8> = elements.iter().flat_map(Binary::to_bytes).collect();
                let events = item.inet_mut().map(|tcp| tcp.send_dist(&bytes));
                if let Some(exit) = self.send_events(port, item, events.unwrap_or_default())? {
                    return Ok(Some(exit));
                }
            }
        });
        if let Some(exit) = exit.transpose()? {
            self.end_step(port, exit);
        }
        Ok(())
    }

    /// Drain packets queued during the input step of `port`.
    ///
    /// # Errors
    ///
    /// The errors of [`Self::dist_port_send`].
    pub(in crate::runtime) fn dist_port_flush(&mut self, port: PortId) -> Result<(), BeamError> {
        if std::mem::take(&mut self.deferred_dist) {
            self.dist_port_send(port)?;
        }
        Ok(())
    }
}

/// Return the control message of `erts_dsig_send_msg` (`dist.c:1640-1702`)
/// for a send from `from` to `to` on a connection with the flags `flags`
/// (see [`Scheduler::dist_send`]).
fn send_control(flags: u64, from: Term, to: &Term, priority: SendPriority) -> Term {
    let alias = is_remote_reference(to);
    // `erts_dsig_send_msg` takes the name of `{Name, Node}` (`bif.c:2662`).
    let (name, remote) = match to {
        Term::Tuple(items) => (true, items[0].clone()),
        _ => (false, to.clone()),
    };
    let prio = priority == SendPriority::Priority;
    if (prio || alias) && flags & ALTACT_SIG != 0 {
        let mut fs = if alias {
            ALTACT_SIG_ALIAS
        } else if name {
            ALTACT_SIG_NAME
        } else {
            0
        };
        if prio {
            fs |= ALTACT_SIG_PRIO;
        }
        Term::tuple(vec![Term::Integer(37), Term::Integer(fs), from, remote])
    } else if name {
        Term::tuple(vec![Term::Integer(6), from, Term::static_atom(""), remote])
    } else if alias {
        Term::tuple(vec![Term::Integer(33), from, remote])
    } else if flags & SEND_SENDER != 0 {
        Term::tuple(vec![Term::Integer(22), from, remote])
    } else {
        Term::tuple(vec![Term::Integer(2), Term::static_atom(""), remote])
    }
}

/// Return whether `to` is a reference of another node: an alias.
fn is_remote_reference(to: &Term) -> bool {
    matches!(to, Term::External(id) if matches!(id.data, ExternalData::Reference(_)))
}

#[cfg(test)]
#[path = "../../../tests/runtime/scheduler/dist_output.rs"]
mod tests;

/// Split a distribution packet into fragments with the sender's ERTS pid word.
fn fragment_packet(
    elements: Vec<Binary>,
    sender: ProcessId,
    header_len: usize,
    control_len: usize,
) -> Vec<Vec<Binary>> {
    let bytes: Vec<u8> = elements.iter().flat_map(Binary::to_bytes).collect();
    if bytes.len() - header_len <= FRAGMENT_SIZE {
        return vec![elements];
    }
    let first_end = header_len + control_len.max(FRAGMENT_SIZE).min(bytes.len() - header_len);
    let remaining = &bytes[first_end..];
    if remaining.is_empty() {
        return vec![elements];
    }
    let count = 1 + remaining.len().div_ceil(FRAGMENT_SIZE);
    std::iter::once(&bytes[2..first_end])
        .chain(remaining.chunks(FRAGMENT_SIZE))
        .enumerate()
        .map(|(index, chunk)| {
            let mut packet = vec![131, if index == 0 { b'E' } else { b'F' }];
            packet.extend_from_slice(&pid_word(sender).to_be_bytes());
            packet.extend_from_slice(
                &u64::try_from(count - index)
                    .expect("fragment count fits u64")
                    .to_be_bytes(),
            );
            packet.extend_from_slice(chunk);
            vec![Binary::from(packet)]
        })
        .collect()
}

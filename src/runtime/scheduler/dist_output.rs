//! The output of a distribution connection: the messages that a local
//! process sends to a pid, a name, or an alias of another node, as
//! `erts_dsig_send_msg` builds them (`dist.c:1592-1712`). A connection has
//! no atom cache and no fragments, so each packet is the pass-through byte,
//! the control message, and the message (`external.c:496-512`).

use std::sync::Arc;

use super::{Scheduler, dist_table::Controller};
use crate::{
    error::BeamError,
    runtime::interpreter::bif::encode_dist,
    term::{Atom, Binary, PortId, ProcessId, Term, external_id::ExternalData},
};

/// `PASS_THROUGH` (`external.h`): the first byte of a packet without a
/// distribution header.
const PASS_THROUGH: u8 = 112;

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
    ) -> Result<bool, BeamError> {
        let caller = self.current_process().id;
        let from = Term::Pid(caller);
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
        let node_ref = Arc::clone(&self.node);
        let mut dist = node_ref.dist();
        let Some((flags, controller, connection)) = dist.connected(node) else {
            return Ok(false);
        };
        let Some((control, message)) = build(flags, connection) else {
            return Ok(true);
        };
        let mut elements = vec![Binary::from(vec![PASS_THROUGH])];
        elements.extend(encode_dist(self, &control)?);
        if let Some(message) = message {
            elements.extend(encode_dist(self, &message)?);
        }
        match controller {
            Controller::Process(process) => {
                if dist.enqueue(node, elements) {
                    drop(dist);
                    self.send_message(sender, process, Term::static_atom("dist_data"));
                }
            }
            Controller::Port(port) => {
                drop(dist);
                self.dist_port_send(port, &elements)?;
            }
        }
        Ok(true)
    }

    /// Write the packet `elements` with the port controller `port`, as
    /// `erts_dist_command` and `dist_port_commandv` do
    /// (`dist.c:4051-4060`, `4132-4133`, `3844-3921`), and count it. The port lock
    /// comes before the lock of the dist table. A port that exited after
    /// the lookup of the connection drops the packet, as ERTS drops it for
    /// an exiting connection.
    ///
    /// # Errors
    ///
    /// The errors of [`Self::send_events`].
    fn dist_port_send(&mut self, port: PortId, elements: &[Binary]) -> Result<(), BeamError> {
        let bytes: Vec<u8> = elements.iter().flat_map(Binary::to_bytes).collect();
        let node = Arc::clone(&self.node);
        let exit = node.with_port(port, |item| {
            let mut dist = node.dist();
            if let Some(name) = dist.controlled_by(Controller::Port(port)) {
                dist.count_output(&name);
            }
            drop(dist);
            let events = item.inet_mut().map(|tcp| tcp.send_dist(&bytes));
            self.send_events(port, item, events.unwrap_or_default())
        });
        if let Some(exit) = exit.transpose()? {
            self.end_step(port, exit);
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

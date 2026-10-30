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
    term::{Atom, Binary, PortId, Term, external_id::ExternalData},
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

/// `ERTS_DOP_ALTACT_SIG_FLG_ALIAS` (`dist.h:190`).
const ALTACT_SIG_ALIAS: i64 = 1 << 2;

impl Scheduler {
    /// Send `message` from the current process to `to`, a pid, a reference,
    /// or `{Name, Node}` of the node `node`, as `remote_send` does for a
    /// connected node (`bif.c:2405-2441`), and return `true`. Return
    /// `false` when the node has no connection, for the caller to drop the
    /// message.
    ///
    /// The control message is `REG_SEND` for a name, `ALIAS_SEND` or
    /// `ALTACT_SIG_SEND` for a reference, and `SEND_SENDER` or `SEND` for a
    /// pid, as the flags of the connection permit
    /// (`erts_dsig_send_msg`, `dist.c:1640-1702`). A connection without
    /// `DFLAG_ALIAS` drops a message to a reference (`bif.c:2406-2414`). A
    /// process controller gets the packet in its output queue, and a port
    /// controller writes it.
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// assert!(scheduler.dist_send(&node, &Term::External(pid), &message)?);
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
    ) -> Result<bool, BeamError> {
        let node_ref = Arc::clone(&self.node);
        let mut dist = node_ref.dist();
        let Some((flags, controller)) = dist.connected(node) else {
            return Ok(false);
        };
        let caller = self.current_process().id;
        let from = Term::Pid(caller);
        let control = match to {
            Term::Tuple(items) => Term::tuple(vec![
                Term::Integer(6),
                from,
                Term::static_atom(""),
                items[0].clone(),
            ]),
            Term::External(id) if matches!(id.data, ExternalData::Reference(_)) => {
                if flags & ALIAS == 0 {
                    return Ok(true);
                }
                if flags & ALTACT_SIG == 0 {
                    Term::tuple(vec![Term::Integer(33), from, to.clone()])
                } else {
                    Term::tuple(vec![
                        Term::Integer(37),
                        Term::Integer(ALTACT_SIG_ALIAS),
                        from,
                        to.clone(),
                    ])
                }
            }
            _ if flags & SEND_SENDER != 0 => Term::tuple(vec![Term::Integer(22), from, to.clone()]),
            _ => Term::tuple(vec![Term::Integer(2), Term::static_atom(""), to.clone()]),
        };
        let mut elements = vec![Binary::from(vec![PASS_THROUGH])];
        elements.extend(encode_dist(self, &control)?);
        elements.extend(encode_dist(self, message)?);
        match controller {
            Controller::Process(process) => {
                if dist.enqueue(node, elements) {
                    drop(dist);
                    self.send_message(caller, process, Term::static_atom("dist_data"));
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

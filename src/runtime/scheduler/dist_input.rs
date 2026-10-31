//! The input of a distribution connection: the messages that another node
//! sends to a local process or to a registered name, as
//! `erts_net_message` gives them (`dist.c:2017-2600`). A connection has no
//! atom cache and no fragments, so each packet is the pass-through byte,
//! the control message, and the message (`external.c:496-512`, `902-961`).

use super::{Scheduler, node::Named};
use crate::{
    error::{BeamError, error},
    runtime::{interpreter::bif::decode_dist, signal::Signal},
    term::{
        Atom, Term,
        external_id::{self, ExternalData},
    },
};

/// `VERSION_MAGIC` (`external.h`).
const VERSION_MAGIC: u8 = 131;

/// The second byte of a packet with a distribution header or a fragment
/// (`DIST_HEADER`, `DIST_FRAG_HEADER`, and `DIST_FRAG_CONT`,
/// `external.h`), which a connection without these flags rejects
/// (`external.c:956-977`).
const DIST_HEADERS: [u8; 3] = *b"DEF";

/// `DFLAG_DIST_HDR_ATOM_CACHE` and `DFLAG_FRAGMENTS` (`dist.h:43`, `53`):
/// with either flag a packet has a distribution header
/// (`external.c:922-926`, `953-977`).
const HEADER_FLAGS: u64 = 0x2000 | 0x0080_0000;

/// The runtime error of a packet that ERTS rejects: ERTS logs it and ends
/// the connection (`dist.c:3144-3165`).
pub(super) const REJECTED: &str = "unsupported distribution message that ERTS rejects: ERTS ends the connection: see \"Distribution\" in docs/erts-differences.md";

impl Scheduler {
    /// Give the packet `packet` with data of a connection with the flags
    /// `flags` to its target, as
    /// `erts_net_message` does for `SEND` (2), `SEND_SENDER` (22),
    /// `REG_SEND` (6), and `ALIAS_SEND` (33) (`dist.c:2463-2578`). The first
    /// byte is skipped (`external.c:922-926`). A message to a pid that is
    /// not a live local process or to a name that no process has is dropped
    /// without a decode (`dist.c:2489-2507`, `2544-2551`). A message to an
    /// alias goes to the owner of a local pid reference as a signal, and
    /// the owner checks the alias when it handles the signal; a reference
    /// without a local owner drops it (`erl_proc_sig_queue.c:2314-2331`).
    /// `EXIT2` (8) and `PAYLOAD_EXIT2` (26), whose reason is the message,
    /// give the exit signal of a pid of the node `node` of the connection
    /// to a live local process; a process that is not alive and a pid of an
    /// old incarnation of the local node drop it without a decode
    /// (`dist.c:2806-2870`). `LINK` (1), `UNLINK_ID` (35),
    /// `UNLINK_ID_ACK` (36), `EXIT` (3), and `PAYLOAD_EXIT` (24) go to the
    /// links of the node (see [`super::dist_link`]).
    ///
    /// The id `connection` is the id of the connection that carried the
    /// packet, which its controller took with the packet
    /// (`dist.c:4672-4680`). A `LINK` goes to that connection, not to the
    /// current connection of the entry (`dist.c:2246-2258`).
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// scheduler.net_message(&packet)?;
    /// ```
    ///
    /// # Errors
    ///
    /// A connection with the atom cache or fragments (see [`HEADER_FLAGS`]),
    /// a packet that ERTS rejects (see [`REJECTED`]), a control message
    /// with a trace token, another operation, and a term that PBeam cannot
    /// hold.
    pub(in crate::runtime) fn net_message(
        &mut self,
        node: &Atom,
        connection: u32,
        flags: u64,
        packet: &[u8],
    ) -> Result<(), BeamError> {
        if flags & HEADER_FLAGS != 0 {
            return Err(error(
                "unsupported distribution message on a connection with the atom cache or fragments: see \"Distribution\" in docs/erts-differences.md",
            ));
        }
        let ext = packet.get(1..).unwrap_or_default();
        if ext.len() < 2 || ext[0] != VERSION_MAGIC || DIST_HEADERS.contains(&ext[1]) {
            return Err(error(REJECTED));
        }
        let Some((control, used)) = decode_dist(self, ext)? else {
            return Err(error(REJECTED));
        };
        let message = &ext[used..];
        let Term::Tuple(items) = &control else {
            return Err(error(REJECTED));
        };
        let operation = items.first().and_then(Term::small_integer);
        match (operation, items.as_ref()) {
            (Some(2 | 22), [_, _, to]) => match to {
                Term::Pid(target) => {
                    if self.node.live_owner(*target).is_some() {
                        let message = self.dist_message(message)?;
                        self.port_send(*target, message);
                    }
                    Ok(())
                }
                _ if external_id::pid_parts(to).is_some() => Ok(()),
                _ => Err(error(REJECTED)),
            },
            (Some(6), [_, from, _, Term::Atom(name)]) if external_id::pid_parts(from).is_some() => {
                if let Some(Named::Process(target)) = self.name_owner(self.atoms.name_of(name)) {
                    let message = self.dist_message(message)?;
                    self.port_send(target, message);
                }
                Ok(())
            }
            (Some(33), [_, _, to]) => match to {
                Term::Reference(alias) => {
                    if let Some(owner) = self.node.alias_owner(*alias)
                        && self.node.live_owner(owner).is_some()
                    {
                        let message = self.dist_message(message)?;
                        let signal = Signal::AliasMessage {
                            alias: *alias,
                            message,
                        };
                        let _ = self.send_signal(None, owner, signal);
                    }
                    Ok(())
                }
                _ if external_id::is_reference(to) => Ok(()),
                _ => Err(error(REJECTED)),
            },
            (Some(8), [_, from, to, reason]) => {
                self.dist_exit_signal(node, from, to, |_| Ok(reason.clone()))
            }
            (Some(26), [_, from, to]) => {
                self.dist_exit_signal(node, from, to, |scheduler| scheduler.dist_message(message))
            }
            (Some(1), [_, from, to]) => self.net_link(node, connection, from, to),
            (Some(35), [_, id, from, to]) => self.net_unlink(node, connection, id, from, to),
            (Some(36), [_, id, from, to]) => self.net_unlink_ack(node, id, from, to),
            (Some(3), [_, from, to, reason]) => {
                self.net_link_exit(node, from, to, |_| Ok(reason.clone()))
            }
            (Some(24), [_, from, to]) => {
                self.net_link_exit(node, from, to, |scheduler| scheduler.dist_message(message))
            }
            (Some(12 | 13 | 16 | 18 | 23 | 25 | 27 | 34), _) => Err(error(
                "unsupported distribution message with a trace token: see \"Distribution\" in docs/erts-differences.md",
            )),
            // ERTS reads the operation as unsigned (`unsigned_val`,
            // dist.c:2223), so a negative one is not valid (dist.c:3131-3132).
            // `UNLINK` (4) is not valid since OTP 26 (dist.c:2277-2282).
            (Some(1..=4 | 6 | 8 | 22 | 24 | 26 | 33 | 35 | 36) | None, _) => Err(error(REJECTED)),
            (Some(operation), _) if operation < 0 => Err(error(REJECTED)),
            (Some(operation), _) => Err(error(format!(
                "unsupported distribution operation {operation}: see \"Distribution\" in docs/erts-differences.md"
            ))),
        }
    }

    /// Give the exit signal from `from` with the reason that `reason` gives
    /// to the process `to`, as the `EXIT2` case of `erts_net_message` does
    /// (`dist.c:2842-2869`): `from` must be a pid of the node `node` of the
    /// connection, and `to` a local pid or a pid of an old incarnation of
    /// the local node, which drops the signal. A process that is not alive
    /// drops it before `reason` runs.
    ///
    /// # Errors
    ///
    /// Another `from` or `to` (see [`REJECTED`]), and the errors of
    /// `reason`.
    fn dist_exit_signal(
        &mut self,
        node: &Atom,
        from: &Term,
        to: &Term,
        reason: impl FnOnce(&Self) -> Result<Term, BeamError>,
    ) -> Result<(), BeamError> {
        let remote_pid = |term: &Term| matches!(term, Term::External(id) if matches!(id.data, ExternalData::Pid { .. }));
        if !(remote_pid(from) && matches!(from, Term::External(id) if id.node == *node)) {
            return Err(error(REJECTED));
        }
        match to {
            Term::Pid(target) => {
                if self.node.live_owner(*target).is_some() {
                    let reason = reason(self)?;
                    let signal = Signal::DistExit {
                        from: from.clone(),
                        reason,
                    };
                    let _ = self.send_signal(None, *target, signal);
                }
                Ok(())
            }
            Term::External(id) if remote_pid(to) && id.node == self.atoms.this_node().0 => Ok(()),
            _ => Err(error(REJECTED)),
        }
    }

    /// Decode the message `bytes` after the control message.
    ///
    /// # Errors
    ///
    /// Bytes that ERTS does not decode (see [`REJECTED`]) and a term that
    /// PBeam cannot hold.
    pub(super) fn dist_message(&self, bytes: &[u8]) -> Result<Term, BeamError> {
        decode_dist(self, bytes)?
            .map(|(message, _)| message)
            .ok_or_else(|| error(REJECTED))
    }
}

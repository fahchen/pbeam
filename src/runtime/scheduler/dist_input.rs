//! The input of a distribution connection: the messages that another node
//! sends to a local process or to a registered name, as
//! `erts_net_message` gives them (`dist.c:2017-2600`). A connection has no
//! atom cache and no fragments, so each packet is the pass-through byte,
//! the control message, and the message (`external.c:496-512`, `902-961`).

use super::{Scheduler, node::Named};
use crate::{
    error::{BeamError, error},
    runtime::interpreter::bif::decode_dist,
    term::{Term, external_id},
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
const REJECTED: &str = "unsupported distribution message that ERTS rejects: ERTS ends the connection: see \"Distribution\" in docs/erts-differences.md";

impl Scheduler {
    /// Give the packet `packet` with data of a connection with the flags
    /// `flags` to its target, as
    /// `erts_net_message` does for `SEND` (2), `SEND_SENDER` (22), and
    /// `REG_SEND` (6) (`dist.c:2463-2553`). The first byte is skipped
    /// (`external.c:922-926`). A message to a pid that is not a live local
    /// process or to a name that no process has is dropped without a decode
    /// (`dist.c:2489-2507`, `2544-2551`). PBeam does not offer
    /// `DFLAG_ALIAS`, so another node sends no `ALIAS_SEND` (33), and it is
    /// another operation here (#1455).
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
            (Some(12 | 16 | 23 | 34), _) => Err(error(
                "unsupported distribution message with a trace token: see \"Distribution\" in docs/erts-differences.md",
            )),
            // ERTS reads the operation as unsigned (`unsigned_val`,
            // dist.c:2223), so a negative one is not valid (dist.c:3131-3132).
            (Some(2 | 6 | 22) | None, _) => Err(error(REJECTED)),
            (Some(operation), _) if operation < 0 => Err(error(REJECTED)),
            (Some(operation), _) => Err(error(format!(
                "unsupported distribution operation {operation}: see \"Distribution\" in docs/erts-differences.md"
            ))),
        }
    }

    /// Decode the message `bytes` after the control message.
    ///
    /// # Errors
    ///
    /// Bytes that ERTS does not decode (see [`REJECTED`]) and a term that
    /// PBeam cannot hold.
    fn dist_message(&self, bytes: &[u8]) -> Result<Term, BeamError> {
        decode_dist(self, bytes)?
            .map(|(message, _)| message)
            .ok_or_else(|| error(REJECTED))
    }
}

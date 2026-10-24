//! The socket resources of `prim_socket` on the node, as the
//! `ESockDescriptor` objects and the global counters of
//! `prim_socket_nif.c` are.
//!
//! ERTS frees a socket resource when no term refers to it and the socket
//! is closed. PBeam has no garbage collection of resources, so it keeps
//! each socket until the end of the run, as it keeps a buffer of
//! `prim_buffer`.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use socket2::Socket;

use crate::term::{ProcessId, Reference, Term};

/// The state `ESOCK_STATE_BOUND` of a read or write state (`prim_socket_nif.c:4829-4890`).
pub(super) const BOUND: u32 = 1 << 0;
/// The state `ESOCK_STATE_CLOSING` of a read or write state (`prim_socket_nif.c:4829-4890`).
pub(super) const CLOSING: u32 = 1 << 1;
/// The state `ESOCK_STATE_CLOSED` of a read or write state (`prim_socket_nif.c:4829-4890`).
pub(super) const CLOSED: u32 = 1 << 2;

/// The sockets of the node and the global data of `prim_socket_nif.c`
/// that PBeam reads.
#[derive(Debug, Default)]
pub(super) struct Sockets {
    /// The socket registry process of the load info, as `data.regPid`
    /// (`prim_socket_nif.c:18862-18867`).
    pub(super) registry: Option<ProcessId>,
    /// The global counters of `esock_inc_socket` and `esock_dec_socket`
    /// (`prim_socket_nif.c:17127-17218`).
    pub(super) counters: Counters,
    /// Each socket by the reference of its resource.
    by_reference: HashMap<Reference, Arc<Mutex<SocketState>>>,
}

impl Sockets {
    /// Keep a new socket under the reference `reference`.
    pub(super) fn insert(&mut self, reference: Reference, socket: SocketState) {
        self.by_reference
            .insert(reference, Arc::new(Mutex::new(socket)));
    }

    /// Return the socket of the resource `reference`, as
    /// `ESOCK_GET_RESOURCE` does: `None` for each other reference.
    pub(super) fn get(&self, reference: Reference) -> Option<Arc<Mutex<SocketState>>> {
        self.by_reference.get(&reference).cloned()
    }

    /// Run the monitor of the socket `reference` on the process `pid`
    /// after it exits, as `essio_down` and `esock_close_socket` do
    /// (`unix_socket_syncio.c:9640-9750`, `10532-10570`,
    /// `prim_socket_nif.c:7648-7705`). For the closer, clear the closer and
    /// close the socket (`unix_socket_syncio.c:9696-9723`). For the owner,
    /// clear the owner, and close the socket when it is still open. A close
    /// closes the host socket, adds the states `closing` and `closed`, and
    /// counts the socket out. Return the registry to send
    /// `{'$socket', del, Socket}` to, if the registry knows the socket and
    /// the socket closed.
    pub(super) fn monitor_down(
        &mut self,
        reference: Reference,
        pid: ProcessId,
    ) -> Option<ProcessId> {
        let socket = self.by_reference.get(&reference)?;
        let mut socket = socket.lock().unwrap_or_else(|poison| poison.into_inner());
        if socket.closer == Some(pid) {
            socket.closer = None;
        } else if socket.owner == Some(pid) {
            socket.owner = None;
            if socket.read & (CLOSING | CLOSED) != 0 {
                return None;
            }
        } else {
            return None;
        }
        socket.socket = None;
        socket.read |= CLOSING | CLOSED;
        socket.write |= CLOSING | CLOSED;
        let (domain, kind, protocol) = (socket.domain, socket.kind, socket.protocol);
        let registered = socket.registered;
        drop(socket);
        self.counters.count(domain, kind, protocol, -1);
        registered.then_some(self.registry).flatten()
    }
}

/// The global socket counters of `esock_global_info`
/// (`prim_socket_nif.c:4592-4672`). ERTS has no counter of the local
/// domain: `esock_inc_socket` counts a local socket in `num_dinet6`
/// (`prim_socket_nif.c:17189-17191`), and so does PBeam.
#[derive(Debug, Default, Clone, Copy)]
pub(super) struct Counters {
    /// `num_sockets`.
    pub(super) sockets: u64,
    /// `num_tdgrams`.
    pub(super) dgrams: u64,
    /// `num_tstreams`.
    pub(super) streams: u64,
    /// `num_tseqpkgs`.
    pub(super) seqpackets: u64,
    /// `num_dinet`.
    pub(super) inet: u64,
    /// `num_dinet6`.
    pub(super) inet6: u64,
    /// `num_pip`.
    pub(super) ip: u64,
    /// `num_ptcp`.
    pub(super) tcp: u64,
    /// `num_pudp`.
    pub(super) udp: u64,
    /// `num_psctp`. The pinned build has no `HAVE_SCTP`, so no socket
    /// counts in it (`prim_socket_nif.c:17165-17168`).
    pub(super) sctp: u64,
}

impl Counters {
    /// Add `step` (1 or -1) to the counters of a socket with `domain`,
    /// `kind`, and `protocol`, as `esock_inc_socket` and
    /// `esock_dec_socket` do (`prim_socket_nif.c:17127-17218`).
    pub(super) fn count(&mut self, domain: i32, kind: i32, protocol: i32, step: i64) {
        let add = |counter: &mut u64| *counter = counter.wrapping_add_signed(step);
        add(&mut self.sockets);
        match domain {
            libc::AF_INET => add(&mut self.inet),
            libc::AF_INET6 | libc::AF_UNIX => add(&mut self.inet6),
            _ => {}
        }
        match kind {
            libc::SOCK_STREAM => add(&mut self.streams),
            libc::SOCK_DGRAM => add(&mut self.dgrams),
            libc::SOCK_SEQPACKET => add(&mut self.seqpackets),
            _ => {}
        }
        match protocol {
            libc::IPPROTO_IP => add(&mut self.ip),
            libc::IPPROTO_TCP => add(&mut self.tcp),
            libc::IPPROTO_UDP => add(&mut self.udp),
            #[cfg(target_os = "linux")]
            libc::IPPROTO_MPTCP => add(&mut self.tcp),
            _ => {}
        }
    }
}

/// One socket of `prim_socket:nif_open/4`, as the fields of
/// `ESockDescriptor` that PBeam reads.
#[derive(Debug)]
pub(super) struct SocketState {
    /// The host socket, `None` after `nif_finalize_close/1`.
    pub(super) socket: Option<Socket>,
    /// The domain number.
    pub(super) domain: i32,
    /// The type number.
    pub(super) kind: i32,
    /// The protocol number.
    pub(super) protocol: i32,
    /// The controlling process, `None` after `nif_finalize_close/1`.
    pub(super) owner: Option<ProcessId>,
    /// The process that called `nif_close/1`.
    pub(super) closer: Option<ProcessId>,
    /// The read state bits.
    pub(super) read: u32,
    /// The write state bits.
    pub(super) write: u32,
    /// Whether the socket registry knows the socket.
    pub(super) registered: bool,
    /// The value of the option `{otp, meta}`, as `meta.ref`.
    pub(super) meta: Term,
}

#[cfg(test)]
#[path = "../../tests/runtime/sockets.rs"]
mod tests;

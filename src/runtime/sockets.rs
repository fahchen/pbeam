//! The socket resources of `prim_socket` on the node, as the
//! `ESockDescriptor` objects and the global counters of
//! `prim_socket_nif.c` are.
//!
//! ERTS frees a socket resource when no term refers to it and the socket
//! is closed. PBeam has no garbage collection of resources, so it keeps
//! each socket until the end of the run, as it keeps a buffer of
//! `prim_buffer`.

use std::collections::HashMap;
use std::fs::File;
use std::sync::{Arc, Mutex};

use socket2::Socket;

use crate::runtime::scheduler::poll::{Direction, NodePoll};
use crate::term::{ProcessId, Reference, Term};

/// The state `ESOCK_STATE_BOUND` of a read or write state (`prim_socket_nif.c:4829-4890`).
pub(super) const BOUND: u32 = 1 << 0;
/// The state `ESOCK_STATE_CLOSING` of a read or write state (`prim_socket_nif.c:4829-4890`).
pub(super) const CLOSING: u32 = 1 << 1;
/// The state `ESOCK_STATE_CLOSED` of a read or write state (`prim_socket_nif.c:4829-4890`).
pub(super) const CLOSED: u32 = 1 << 2;
/// The read state `ESOCK_STATE_LISTENING` (`prim_socket_int.h:44`).
pub(super) const LISTENING: u32 = 1 << 3;
/// The read state `ESOCK_STATE_ACCEPTING` (`prim_socket_int.h:45`).
pub(super) const ACCEPTING: u32 = 1 << 4;
/// The write state `ESOCK_STATE_CONNECTING` (`prim_socket_int.h:46`).
pub(super) const CONNECTING: u32 = 1 << 5;
/// The write state `ESOCK_STATE_CONNECTED` (`prim_socket_int.h:47`).
pub(super) const CONNECTED: u32 = 1 << 6;
/// The state `ESOCK_STATE_SELECTED` of a read or write state
/// (`prim_socket_int.h:53`). ERTS does not clear it after a select.
pub(super) const SELECTED: u32 = 1 << 7;

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
    /// counts the socket out. A close also stops the selects (see
    /// [`SocketState::stop`]). The exit of the connector or the current
    /// acceptor clears it. Return the requestors to abort, and the registry
    /// to send `{'$socket', del, Socket}` to, if the registry knows the
    /// socket and the socket closed.
    pub(super) fn monitor_down(
        &mut self,
        reference: Reference,
        pid: ProcessId,
        poll: &NodePoll,
    ) -> (Vec<(ProcessId, Term)>, Option<ProcessId>) {
        // PBeam keeps each socket until the end of the run.
        let socket = self
            .by_reference
            .get(&reference)
            .expect("a socket monitor refers to a kept socket");
        let mut socket = socket.lock().unwrap_or_else(|poison| poison.into_inner());
        // The connector and the current acceptor: clear the state and keep
        // the select, as `essio_down` and `essio_down_acceptor` do
        // (`unix_socket_syncio.c:9762-9780`, `10583-10608`).
        if socket
            .connector
            .as_ref()
            .is_some_and(|(owner, _)| *owner == pid)
        {
            socket.connector = None;
            socket.write &= !CONNECTING;
        }
        if socket
            .acceptor
            .as_ref()
            .is_some_and(|(owner, _)| *owner == pid)
        {
            socket.acceptor = None;
            socket.read &= !ACCEPTING;
        }
        // The current reader and writer: clear them and keep the select, as
        // `essio_down_reader` and `essio_down_writer` do
        // (`unix_socket_syncio.c:10632-10716`).
        if socket
            .reader
            .as_ref()
            .is_some_and(|(owner, _)| *owner == pid)
        {
            socket.reader = None;
        }
        if socket
            .writer
            .as_ref()
            .is_some_and(|(owner, _)| *owner == pid)
        {
            socket.writer = None;
        }
        if socket.closer == Some(pid) {
            socket.closer = None;
        } else if socket.owner == Some(pid) {
            socket.owner = None;
            if socket.read & (CLOSING | CLOSED) != 0 {
                return (Vec::new(), None);
            }
        } else {
            return (Vec::new(), None);
        }
        // A close cancels each select before the fd closes.
        let aborted = socket.stop(poll);
        socket.socket = None;
        socket.sendfile = None;
        socket.read |= CLOSING | CLOSED;
        socket.write |= CLOSING | CLOSED;
        let (domain, kind, protocol) = (socket.domain, socket.kind, socket.protocol);
        let registered = socket.registered;
        drop(socket);
        self.counters.count(domain, kind, protocol, -1);
        (aborted, registered.then_some(self.registry).flatten())
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

/// The read and write counters of a socket of `esock_socket_info_counters`
/// (`prim_socket_nif.c:4920-4990`). The I/O backend never adds to
/// `read_waits`, so PBeam has no field for it.
#[derive(Debug, Default, Clone, Copy)]
pub(super) struct IoCounters {
    /// `readByteCnt`.
    pub(super) read_byte: u64,
    /// `readFails`.
    pub(super) read_fails: u64,
    /// `readPkgCnt`.
    pub(super) read_pkg: u64,
    /// `readPkgMax`.
    pub(super) read_pkg_max: u64,
    /// `readPkgMaxCnt`, the bytes of the read in progress.
    pub(super) read_pkg_max_count: u64,
    /// `readTries`.
    pub(super) read_tries: u64,
    /// `writeByteCnt`.
    pub(super) write_byte: u64,
    /// `writeFails`.
    pub(super) write_fails: u64,
    /// `writePkgCnt`.
    pub(super) write_pkg: u64,
    /// `writePkgMax`.
    pub(super) write_pkg_max: u64,
    /// `writePkgMaxCnt`, the bytes of the write in progress.
    pub(super) write_pkg_max_count: u64,
    /// `writeTries`.
    pub(super) write_tries: u64,
    /// `writeWaits`.
    pub(super) write_waits: u64,
}

/// The sendfile counters of `ESockSendfileCounters`, which
/// `esock_socket_info_counters` adds to the counters after the first
/// sendfile (`prim_socket_nif.c:4958-4989`).
#[derive(Debug, Default, Clone, Copy)]
pub(super) struct SendfileCounters {
    /// `cnt`, the `sendfile(2)` calls.
    pub(super) calls: u64,
    /// `byteCnt`.
    pub(super) byte: u64,
    /// `fails`.
    pub(super) fails: u64,
    /// `max`.
    pub(super) max: u64,
    /// `maxCnt`, the bytes of the sendfile in progress.
    pub(super) max_count: u64,
    /// `pkg`, the calls that sent bytes.
    pub(super) pkg: u64,
    /// `pkgMax`.
    pub(super) pkg_max: u64,
    /// `tries`.
    pub(super) tries: u64,
    /// `waits`.
    pub(super) waits: u64,
}

/// The `otp` level options that a socket keeps, as `alloc_descriptor` sets
/// them (`prim_socket_nif.c:17061-17068`). An accepted socket inherits them
/// (`unix_socket_syncio.c:2843-2849`).
#[derive(Debug, Clone, Copy)]
pub(super) struct OtpOptions {
    /// `iow`, the value of `{otp, iow}`.
    pub(super) iow: bool,
    /// `rBufSz`, the buffer size of a receive with length 0.
    pub(super) recv_buffer: usize,
    /// `rNum`, the count `N` of `{otp, rcvbuf}` `{N, Size}`: a receive with
    /// length 0 that fills the buffer gives `{more, Bin}` until `N` buffers
    /// came (`unix_socket_syncio.c:10050-10090`), or 0.
    pub(super) recv_count: u32,
    /// `rCtrlSz`, the value of `{otp, rcvctrlbuf}`.
    pub(super) recv_ctrl: usize,
    /// `wCtrlSz`, the value of `{otp, sndctrlbuf}`.
    pub(super) send_ctrl: usize,
}

impl Default for OtpOptions {
    /// The defaults `ESOCK_RECV_BUFFER_SIZE_DEFAULT` (8 KiB on Unix),
    /// `ESOCK_RECV_CTRL_BUFFER_SIZE_DEFAULT`, and
    /// `ESOCK_SEND_CTRL_BUFFER_SIZE_DEFAULT` (`prim_socket_nif.c:505-513`).
    fn default() -> Self {
        Self {
            iow: false,
            recv_buffer: 8 * 1024,
            recv_count: 0,
            recv_ctrl: 1024,
            send_ctrl: 1024,
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
    /// The process of the connect in progress and its reference, as
    /// `descP->connectorP`.
    pub(super) connector: Option<(ProcessId, Term)>,
    /// The current acceptor and its reference, as
    /// `descP->currentAcceptorP`.
    pub(super) acceptor: Option<(ProcessId, Term)>,
    /// The current reader and its reference, as `descP->currentReaderP`.
    pub(super) reader: Option<(ProcessId, Term)>,
    /// The current writer and its reference, as `descP->currentWriterP`.
    pub(super) writer: Option<(ProcessId, Term)>,
    /// The read and write counters.
    pub(super) counters: IoCounters,
    /// The `otp` level options.
    pub(super) options: OtpOptions,
    /// `selectRead`, the value of `{otp, select_read}`, which an accepted
    /// socket does not inherit (`unix_socket_syncio.c:2843-2850`).
    pub(super) select_read: bool,
    /// `rNumCnt`, the buffers of [`OtpOptions::recv_count`] that came.
    pub(super) recv_count_done: u32,
    /// `sendfileHandle`, the copy of the file of the sendfile in progress.
    pub(super) sendfile: Option<File>,
    /// `sendfileCountersP`, from the first sendfile of the socket on.
    pub(super) sendfile_counters: Option<SendfileCounters>,
}

impl SocketState {
    /// Cancel both selects of the socket on `poll` and clear the current
    /// writer, the connect in progress, the current reader, and the current
    /// acceptor, as `do_stop` does at a close
    /// (`unix_socket_syncio.c:4600-4730`). Return the requestors whose
    /// select was still active, in this order: each gets `{'$socket',
    /// Socket, abort, {Ref, closed}}` (`esock_stop_handle_current`).
    pub(super) fn stop(&mut self, poll: &NodePoll) -> Vec<(ProcessId, Term)> {
        let fd = std::os::fd::AsRawFd::as_raw_fd(
            self.socket
                .as_ref()
                .expect("a socket that closes has a host socket"),
        );
        let write = poll.cancel_select(fd, Direction::Write);
        let read = poll.cancel_select(fd, Direction::Read);
        [
            (self.writer.take(), write),
            (self.connector.take(), write),
            (self.reader.take(), read),
            (self.acceptor.take(), read),
        ]
        .into_iter()
        .filter_map(|(requestor, cancelled)| requestor.filter(|_| cancelled))
        .collect()
    }
}

#[cfg(test)]
#[path = "../../tests/runtime/sockets.rs"]
mod tests;

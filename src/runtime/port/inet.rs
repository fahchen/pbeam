//! The `tcp_inet` and `udp_inet` port drivers of `inet_drv.c`, which
//! `prim_inet` opens with `open_port({spawn_driver, "tcp_inet"}, [binary])`
//! and `open_port({spawn_driver, "udp_inet"}, [binary])`
//! (prim_inet.erl:99-112) and drives with `erlang:port_control/3`.
//!
//! Each request is `port_control(Port, Request + 16#03f1a300, Bytes)`
//! (prim_inet.erl:2969-2975, `ERTS_INET_DRV_CONTROL_MAGIC_NUMBER`). The
//! reply is `[1 | Data]` for OK and `[0 | Name]` for an error, where `Name`
//! is the bytes of the error name, for example `[0 | "eaddrinuse"]`
//! (`ctl_reply` and `ctl_error`, inet_drv.c:2240-2283). PBeam runs these
//! requests of `INET_REQ_*` (inet_int.hrl:76-105) for an IPv4 or IPv6 TCP socket
//! of a `tcp_inet` port, and the requests but `ACCEPT` for an
//! IPv4 UDP socket of a `udp_inet` port:
//!
//! | Request | Code | Bytes | OK reply |
//! | --- | --- | --- | --- |
//! | `OPEN` | 1 | `[Family, Type, Protocol]`, `[1, 1, 1]` or `[1, 2, 2]` | `[1]` |
//! | `PEER` | 4 | `[]` | `[1, 1, P1, P0, A, B, C, D]` |
//! | `NAME` | 5 | `[]` | `[1, 1, P1, P0, A, B, C, D]` |
//! | `BIND` | 6 | `[1, P1, P0, A, B, C, D]` | `[1, P1, P0]` |
//! | `SETOPTS` | 7 | `[Option, V3, V2, V1, V0, ...]` | `[1]` |
//! | `GETHOSTNAME` | 12 | `[]` | `[1 \| Name]` |
//! | `GETOPTS` | 8 | `[Option, ...]` | `[1, Option, V3, V2, V1, V0, ...]` |
//! | `SUBSCRIBE` | 24 | `[1, ...]` | `[1, 1, Q3, Q2, Q1, Q0, ...]` |
//! | `ACCEPT` | 26 | `[T3, T2, T1, T0]` | `[1, R1, R0]` |
//! | `LISTEN` | 27 | `[B1, B0]` | `[1]` |
//! | `SHUTDOWN` | 44 | `[How]`, 0 read, 1 write, 2 both | `[1]` |
//!
//! `P1, P0` is the port number, `B1, B0` the backlog, `T3..T0` the timeout
//! in milliseconds (`16#ffffffff` for `infinity`), `Q3..Q0` the size of
//! the output queue, and `R1, R0` the async reference of the accept,
//! big-endian; each option value is a big-endian
//! 32-bit integer. `ACCEPT` replies at once and sends `{inet_async, L, R,
//! {ok, S}}` or `{inet_async, L, R, {error, Reason}}` later (see
//! `scheduler/tcp_inet.rs`). The options are those of the listen socket of
//! Thousand Island: `reuseaddr` (0), `nodelay` (10), `active` (22), `mode`
//! (24), `send_timeout` (30), and `send_timeout_close` (35), and those that
//! `prim_inet:accept_opts/3` copies to an accepted socket: `keepalive` (1),
//! `linger` (3), `priority` (8), `tos` (9), `delay_send` (31), `recvtos`
//! (43), `ttl` (46), and `recvttl` (47) (prim_inet.erl:502-518;
//! `inet_tcp.erl:249`). Each other request, option, family, and protocol
//! returns a runtime error (row "`tcp_inet` ports" of
//! `docs/erts-differences.md`).
//!
//! A `udp_inet` port runs the requests of `packet_inet_ctl`, which runs
//! `OPEN`, replies `exbadport` to `LISTEN`, and gives each other request
//! to `inet_ctl` (inet_drv.c:14564-14892). The port sends datagrams,
//! and receives them for a receive in passive mode or for the owner in
//! active mode (see `inet/datagram.rs`).

mod connect;
mod datagram;
mod stream;

use std::{
    collections::VecDeque,
    ffi::c_int,
    io,
    net::{Ipv4Addr, Ipv6Addr},
    os::fd::AsRawFd,
    time::Duration,
};

use nix::sys::socket::{getsockopt, setsockopt, sockopt};
use socket2::{Domain, Protocol, SockAddr, Socket, Type};

use crate::{
    error::{BeamError, error},
    runtime::{
        host::{errno::errno_id, host_name},
        timer::PortTimer,
    },
    term::{ProcessId, Reference},
};
use stream::Stream;
pub(in crate::runtime) use stream::{Activation, Event, Output, Receiver};

/// The number that `prim_inet` adds to each request code
/// (`ERTS_INET_DRV_CONTROL_MAGIC_NUMBER`, `prim_inet.erl:2969`).
const MAGIC: u32 = 0x03f1_a300;

/// A byte count or position as `u64`, which holds every `usize`.
fn to_u64(count: usize) -> u64 {
    u64::try_from(count).expect("a usize fits in u64")
}

/// The low 32 bits of `value`, as the C conversion of a 64-bit counter to
/// `Uint32` keeps them.
fn low_u32(value: u64) -> u32 {
    let [b0, b1, b2, b3, ..] = value.to_le_bytes();
    u32::from_le_bytes([b0, b1, b2, b3])
}

/// The `size_t` of a C `int`, which keeps the sign bits as the C conversion
/// does.
fn sign_extended(value: i32) -> usize {
    isize::try_from(value)
        .expect("an isize holds an i32 on every supported target")
        .cast_unsigned()
}

/// The largest reply of `GETOPTS` and `SUBSCRIBE` (`INET_MAX_OPT_BUFFER`,
/// `inet_drv.c:1046`).
const MAX_OPT_BUFFER: usize = 64 * 1024;

/// The `how` of `TCP_REQ_SHUTDOWN` for the read side (`TCP_SHUT_RD`,
/// `inet_drv.c:684`).
const SHUT_READ: u8 = 0;

/// The `how` of `TCP_REQ_SHUTDOWN` for the write side (`TCP_SHUT_WR`,
/// `inet_drv.c:683`).
const SHUT_WRITE: u8 = 1;

/// The `how` of `TCP_REQ_SHUTDOWN` for both sides (`TCP_SHUT_RDWR`,
/// `inet_drv.c:685`).
const SHUT_READ_WRITE: u8 = 2;

/// The reply code of a request that succeeded (`INET_REP_OK`).
pub(in crate::runtime) const OK: u8 = 1;

/// The reply code of a request that failed (`INET_REP_ERROR`).
const ERROR: u8 = 0;

/// The family code of IPv4 (`INET_AF_INET`, `inet_int.hrl:31`).
const AF_INET: u8 = 1;

/// The family code of IPv6 (`INET_AF_INET6`, `inet_int.hrl:32`).
const AF_INET6: u8 = 2;

/// The family code of the `local` family (`INET_AF_LOCAL`, `inet_int.hrl:35`).
const AF_LOCAL: u8 = 5;

/// The size of `sun_path` of `struct sockaddr_un` on the host.
#[cfg(target_os = "linux")]
const SUN_PATH: usize = 108;
#[cfg(not(target_os = "linux"))]
const SUN_PATH: usize = 104;

/// The state of the socket of a port (`INET_STATE_*`, `inet_drv.c`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum State {
    /// No socket: the port opened, and `OPEN` did not run.
    Closed,
    /// `OPEN` made the socket.
    Open,
    /// `LISTEN` ran on the socket. The accept queue tells
    /// `INET_STATE_LISTENING` (empty) from `INET_STATE_ACCEPTING` and
    /// `INET_STATE_MULTI_ACCEPTING`.
    Listening,
    /// `CONNECT` waits for the write select of the socket
    /// (`INET_STATE_CONNECTING`).
    Connecting,
    /// The socket of an accepted or a connected connection
    /// (`INET_STATE_CONNECTED`).
    Connected,
}

/// A caller that waits in the accept queue of a listen port: an
/// `inet_async_op` of the queue of `enq_async_w_tmo` or `enq_multi_op`
/// (inet_drv.c:2320-2450).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(in crate::runtime) struct Acceptor {
    /// The async reference of the accept, which the reply and the
    /// `inet_async` message give.
    pub(in crate::runtime) id: u16,
    /// The process that called `ACCEPT` and gets the `inet_async` message.
    pub(in crate::runtime) caller: ProcessId,
    /// The reference of the monitor of the caller that the port holds
    /// (`driver_monitor_process`). The caller keeps the target part.
    pub(in crate::runtime) monitor: Reference,
    /// The timer of the accept, or `None` for `infinity`. The timer service
    /// owns it; the end of the accept cancels it.
    pub(in crate::runtime) timer: Option<PortTimer>,
}

/// The socket type of a port: the driver that `open_port/2` names.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(in crate::runtime) enum SocketType {
    /// A `tcp_inet` port, with a `SOCK_STREAM` socket (`tcp_inet_start`,
    /// inet_drv.c:10891-10935).
    Stream,
    /// A `udp_inet` port, with a `SOCK_DGRAM` socket (`udp_inet_start`,
    /// inet_drv.c:14515-14520).
    Datagram,
}

impl SocketType {
    /// Return the driver name of the socket type, which the runtime errors
    /// name.
    fn driver(self) -> &'static str {
        match self {
            SocketType::Stream => "tcp_inet",
            SocketType::Datagram => "udp_inet",
        }
    }
}

/// The descriptor of one `tcp_inet` port (`tcp_descriptor`) or `udp_inet`
/// port (`udp_descriptor`), as `prep_tcp_inet_start`, `packet_inet_start`,
/// and `inet_start` make it (inet_drv.c:10891-10987, 11726-11784,
/// 14496-14520).
#[derive(Debug)]
pub(in crate::runtime) struct Inet {
    /// The socket type of the driver.
    socket_type: SocketType,
    /// The family code of the socket that `OPEN` makes, [`AF_INET`] or
    /// [`AF_INET6`] (`sfamily`), which the address tags `any` and
    /// `loopback` read.
    family: u8,
    /// The non-blocking host socket that `OPEN` makes, or `None` before it.
    /// The port owns it; dropping the port closes it.
    socket: Option<Socket>,
    /// The state of the socket.
    state: State,
    /// The option `mode`: 0 for `list` (the start value), 1 for `binary`.
    mode: i32,
    /// The option `active`: 0 for `false` (the start value), 1 for `true`,
    /// 2 for `once`, and 3 for `{active, N}` (`INET_MULTI`, only on a
    /// `udp_inet` port). A delivery in `once` mode sets it to 0, and the
    /// last delivery of `{active, N}` sets it to 0
    /// (`INET_CHECK_ACTIVE_TO_PASSIVE`, inet_drv.c:1551-1560).
    active: i32,
    /// The count of `{active, N}` (`active_count`, inet_drv.c:6975-7000).
    active_count: i16,
    /// The option `read_packets` of a `udp_inet` port: the most datagrams
    /// that one poll event reads (`INET_PACKET_POLL`, 5, at the start,
    /// `inet_drv.c:1072`, 14508).
    read_packets: i32,
    /// A `udp_inet` socket whose read select is off: it reads no datagram,
    /// also in active mode, until a receive request or a change of the
    /// option `active` selects it again. The timeout of a receive
    /// (inet_drv.c:15021-15024), the last datagram of an active socket that
    /// becomes passive (inet_drv.c:15570-15573), and a change to passive
    /// (inet_drv.c:7960-7967) turn the select off.
    read_off: bool,
    /// The option `send_timeout` in milliseconds, -1 for `infinity` (the
    /// start value, `INET_INFINITY`).
    send_timeout: i32,
    /// The option `send_timeout_close`: 0 (the start value) or not 0.
    send_timeout_close: i32,
    /// The option `delay_send` (`TCP_ADDF_DELAY_SEND`): false at the start.
    delay_send: bool,
    /// Whether a `SETOPTS` set the option `buffer` (`INET_FLG_BUFFER_SET`),
    /// after which `recbuf` does not change `buffer`. An accepted socket
    /// starts without it (`inet_drv.c:14457`).
    buffer_set: bool,
    /// The next async reference (`op_ref`, `inet_drv.c:1549`, 10937).
    op_ref: u16,
    /// The callers that wait for a connection, in arrival order.
    pub(in crate::runtime) acceptors: VecDeque<Acceptor>,
    /// The receive and send state of a connected socket.
    pub(in crate::runtime) stream: Box<Stream>,
}

/// The result of a request.
#[derive(Debug, PartialEq, Eq)]
pub(in crate::runtime) enum Control {
    /// The reply bytes.
    Reply(Vec<u8>),
    /// The driver returned -1, so `port_control/3` gives `badarg`
    /// (io.c:4065-4066).
    Badarg,
    /// A valid `ACCEPT` on a listen socket with this timeout in
    /// milliseconds, `u32::MAX` for `infinity`. The scheduler runs it, as
    /// it makes a port, a monitor, and a timer.
    Accept(u32),
    /// A `CONNECT` that waits, with this timeout in milliseconds,
    /// `u32::MAX` for `infinity`. The scheduler starts the timer, and the
    /// reply is `[1, R1, R0]` of the async reference of the connect.
    Connect(u32),
    /// A valid `SETOPTS` that made a connected socket active, as
    /// `inet_set_opts` returns 1 or 2 (inet_drv.c:7949-7981): the reply is
    /// `[1]` after the scheduler delivers the data. With
    /// [`Activation::Read`], the change to `once` reads the socket (`tcp_recv`), else the change to `true`
    /// delivers the bytes of the input buffer (`tcp_deliver`)
    /// (inet_drv.c:11131-11148).
    Active {
        /// What the port does first.
        activation: Activation,
    },
    /// A `SETOPTS` that ended `{active, N}` this many times
    /// (inet_drv.c:6993-6998): the port sends `{udp_passive, S}` to the
    /// owner for each, and the reply follows.
    Passive {
        /// The reply bytes.
        reply: Vec<u8>,
        /// The number of `{udp_passive, S}` messages.
        count: usize,
    },
    /// A valid `TCP_REQ_RECV` with this timeout in milliseconds,
    /// `u32::MAX` for `infinity`, and this length. The scheduler runs it,
    /// as it makes an async reference, a timer, and messages.
    Receive {
        /// The timeout.
        timeout: u32,
        /// The length, 0 for the bytes of one read.
        length: usize,
    },
}

/// Whether a failed set of a socket option fails the request.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum SetErrors {
    /// A failed set does not fail the request.
    Ignore,
    /// A failed set fails the request (`propagate` of `inet_set_opts`).
    Propagate,
}

/// Whether a membership option joins or leaves a multicast group.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum GroupAction {
    /// `ip_add_membership` and `ipv6_add_membership`.
    Join,
    /// `ip_drop_membership` and `ipv6_drop_membership`.
    Leave,
}

impl Inet {
    /// Return the option `send_timeout` in milliseconds, -1 for `infinity`
    /// (the start value, `INET_INFINITY`).
    pub(in crate::runtime) fn send_timeout(&self) -> i32 {
        self.send_timeout
    }

    /// Return the descriptor of a new port of `socket_type`: no socket,
    /// `list` mode, passive, and an infinite send timeout.
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// let mut port = Inet::new(SocketType::Stream);
    /// assert_eq!(port.control(0x03f1_a301, &[1, 1, 1])?, Control::Reply(vec![1]));
    /// let mut port = Inet::new(SocketType::Datagram);
    /// assert_eq!(port.control(0x03f1_a301, &[1, 2, 2])?, Control::Reply(vec![1]));
    /// ```
    pub(in crate::runtime) fn new(socket_type: SocketType) -> Inet {
        Inet {
            socket_type,
            family: AF_INET,
            socket: None,
            state: State::Closed,
            mode: 0,
            active: 0,
            active_count: 0,
            read_packets: 5,
            read_off: false,
            send_timeout: -1,
            send_timeout_close: 0,
            delay_send: false,
            buffer_set: false,
            op_ref: 0,
            acceptors: VecDeque::new(),
            stream: Box::new(Stream::new(stream::BUFFER)),
        }
    }

    /// Return the descriptor of the port of a connection that the listen
    /// socket of `self` accepted, as `tcp_inet_copy` makes it
    /// (inet_drv.c:11795-11858): the socket, non-blocking, connected, with
    /// the `mode`, `send_timeout`, and `send_timeout_close` of the listen
    /// port. `SET_NONBLOCKING` ignores its result.
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// let port = listen.copy(listen.accept_socket()?);
    /// ```
    pub(in crate::runtime) fn copy(&self, socket: Socket) -> Inet {
        let _ = socket.set_nonblocking(true);
        Inet {
            socket: Some(socket),
            family: self.family,
            state: State::Connected,
            mode: self.mode,
            send_timeout: self.send_timeout,
            send_timeout_close: self.send_timeout_close,
            stream: Box::new(Stream::new(self.stream.buffer)),
            ..Inet::new(SocketType::Stream)
        }
    }

    /// Return the next async reference, as `NEW_ASYNC_ID` does
    /// (`inet_drv.c:1549)`: 0 first, then 1 more each time, modulo 2^16.
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// assert_eq!(port.next_async_id(), 0);
    /// assert_eq!(port.next_async_id(), 1);
    /// ```
    pub(in crate::runtime) fn next_async_id(&mut self) -> u16 {
        let id = self.op_ref;
        self.op_ref = id.wrapping_add(1);
        id
    }

    /// Accept one connection of the listen socket without a wait
    /// (`sock_accept`), and return `None` when no connection is pending
    /// (`ERRNO_BLOCK`).
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// assert!(listen.accept_now()?.is_none());
    /// ```
    ///
    /// # Errors
    ///
    /// Each other error of `accept`.
    pub(in crate::runtime) fn accept_now(&self) -> io::Result<Option<Socket>> {
        let socket = self.socket.as_ref().expect("a listen port has a socket");
        let blocked = |failure: io::Error| {
            (failure.kind() == io::ErrorKind::WouldBlock)
                .then_some(None)
                .ok_or(failure)
        };
        socket
            .accept()
            .map(|(socket, _)| Some(socket))
            .or_else(blocked)
    }

    /// Return the fd of the listen socket when a caller waits for a
    /// connection (`sock_select(desc, FD_ACCEPT, 1)`), else `None`.
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// assert_eq!(port.accept_fd(), None);
    /// ```
    pub(in crate::runtime) fn accept_fd(&self) -> Option<c_int> {
        let socket = self.socket.as_ref()?;
        (!self.acceptors.is_empty()).then(|| socket.as_raw_fd())
    }

    /// Return the fd of an active datagram socket whose read select is on,
    /// which the port reads for datagrams (`sock_select(desc, FD_READ, 1)`,
    /// inet_drv.c:7960-7967, 14619-14621), else `None`.
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// assert_eq!(port.receive_fd(), None);
    /// ```
    pub(in crate::runtime) fn receive_fd(&self) -> Option<c_int> {
        let socket = self.socket.as_ref()?;
        (self.socket_type == SocketType::Datagram && self.active != 0 && !self.read_off)
            .then(|| socket.as_raw_fd())
    }

    /// Run the request `command` of the process `caller` with the bytes
    /// `request`, as `tcp_inet_ctl`, `packet_inet_ctl`, and `inet_ctl` do
    /// (inet_drv.c:10988-11409, 11952-12062, 14564-14892).
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// let reply = port.control(0x03f1_a305, &[], caller)?;
    /// assert_eq!(reply, Control::Reply(b"\0ebadf".to_vec()));
    /// ```
    ///
    /// # Errors
    ///
    /// A request, option, family, or protocol that PBeam does not run.
    pub(in crate::runtime) fn control(
        &mut self,
        command: u32,
        request: &[u8],
        caller: ProcessId,
    ) -> Result<Control, BeamError> {
        let reply = match command.wrapping_sub(MAGIC) {
            1 => self.open(request)?,
            4 => self.peer(),
            5 => self.name(),
            6 => self.bind(request),
            7 => {
                let old = self.active;
                let mut count = 0;
                let reply = self.set_options(request, &mut count)?;
                if reply == [OK] {
                    self.select_datagram_read(old);
                }
                if count > 0 {
                    return Ok(Control::Passive { reply, count });
                }
                return Ok(self.set_active_result(old, reply));
            }
            8 => self.get_options(request)?,
            11 => return Ok(self.statistics(request)),
            3 if self.socket_type == SocketType::Stream => return Ok(self.connect(request, caller)),
            3 => self.datagram_connect(request)?,
            12 => get_host_name(request),
            24 => return Ok(self.subscribe(request, caller)),
            26 if self.socket_type == SocketType::Stream => return Ok(self.accept(request)),
            27 if self.socket_type == SocketType::Stream => self.listen(request),
            // `LISTEN` is only for an SCTP socket (inet_drv.c:14846-14858).
            27 => error_reply("exbadport"),
            // `prim_inet:sendfile/4` asks for the sendfile of a file handle.
            // PBeam runs no host sendfile call, so the request gives the
            // error of a platform without `HAVE_SENDFILE`
            // (inet_drv.c:12440-12444), and `file:sendfile/8` reads the
            // file instead (`file.erl:2788-2797`).
            45 if self.socket_type == SocketType::Stream => error_reply("enotsup"),
            44 if self.socket_type == SocketType::Stream => self.shutdown_request(request),
            42 if self.socket_type == SocketType::Stream => return self.receive_request(request),
            60 if self.socket_type == SocketType::Datagram => {
                return Ok(self.datagram_receive_request(request));
            }
            other => return Err(self.unsupported("request", other)),
        };
        Ok(Control::Reply(reply))
    }

    /// Run `TCP_REQ_SHUTDOWN` with the one byte of `shutdown/2`: 0 for the
    /// read side, 1 for the write side, and 2 for both
    /// (inet_drv.c:12339-12378). A port without a connection gives
    /// `enotconn`, and a request that is not one byte gives `einval`. A
    /// shutdown other than of the read side waits while the output queue
    /// holds bytes (`TCP_ADDF_PENDING_SHUT_WR` and
    /// `TCP_ADDF_PENDING_SHUT_RDWR`); the queue empties in
    /// [`Self::send_ready`], which runs the shutdown.
    fn shutdown_request(&mut self, request: &[u8]) -> Vec<u8> {
        if self.state != State::Connected {
            return error_reply("enotconn");
        }
        let &[how] = request else {
            return error_reply("einval");
        };
        if how != SHUT_READ && self.stream.output_size() > 0 {
            // ERTS adds the flag of each request, and the queue end runs
            // the write shutdown when its flag is set (inet_drv.c:12362-12367,
            // 13968-13969), so a write request stays over a later both. Another
            // `how` gives `ok` and adds no flag.
            if how == SHUT_WRITE
                || (how == SHUT_READ_WRITE && self.stream.pending_shutdown.is_none())
            {
                self.stream.pending_shutdown = Some(how);
            }
            return vec![OK];
        }
        self.shutdown(how)
    }

    /// Shut down the socket with the `shutdown/2` how `how`, and return the
    /// reply, as `sock_shutdown` gives it (inet_drv.c:12368-12377). A how
    /// that is not 0, 1, or 2 gives `einval`.
    fn shutdown(&mut self, how: u8) -> Vec<u8> {
        let socket = self.socket.as_ref().expect("a connected port has a socket");
        let how = match how {
            SHUT_READ => std::net::Shutdown::Read,
            SHUT_WRITE => std::net::Shutdown::Write,
            SHUT_READ_WRITE => std::net::Shutdown::Both,
            _ => return error_reply("einval"),
        };
        reply(socket.shutdown(how), |()| vec![OK])
    }

    /// Return the runtime error of a `what` code that PBeam does not run.
    fn unsupported(&self, what: &str, code: impl std::fmt::Display) -> BeamError {
        let driver = self.socket_type.driver();
        error(format!(
            "unsupported {driver} {what} {code}: see \"{driver} ports\" in docs/erts-differences.md"
        ))
    }

    /// Run `INET_REQ_OPEN` with `[Family, Type, Protocol]`, as
    /// `tcp_inet_ctl` (inet_drv.c:11961-12000) and `packet_inet_ctl`
    /// (inet_drv.c:14577-14636) do, and make the non-blocking socket as
    /// `inet_ctl_open` does (inet_drv.c:5119-5221). A `tcp_inet` port takes
    /// only the stream type. A `udp_inet` port checks the type (stream,
    /// datagram, or seqpacket) before the protocol (default, UDP, or SCTP).
    /// The host makes the socket, so a stream socket with the UDP protocol
    /// gives the errno of the host, for example `eprototype`. The stream
    /// type with the default protocol, the seqpacket type, and the SCTP
    /// protocol return a runtime error.
    fn open(&mut self, request: &[u8]) -> Result<Vec<u8>, BeamError> {
        let &[family, kind, protocol] = request else {
            return Ok(error_reply("einval"));
        };
        // `tcp_inet` and `udp_inet` take IPv6 and `local`
        // (inet_drv.c:11969-11984, 14585-14595).
        let domain = match family {
            AF_INET => Domain::IPV4,
            AF_INET6 => Domain::IPV6,
            AF_LOCAL => Domain::UNIX,
            _ => return Ok(error_reply("eafnosupport")),
        };
        let (socket_type, protocol) = match self.socket_type {
            SocketType::Stream => {
                if kind != 1 {
                    return Ok(error_reply("einval"));
                }
                match protocol {
                    0 => (Type::STREAM, None),
                    1 => (Type::STREAM, Some(Protocol::TCP)),
                    4 => return Err(self.unsupported("protocol", protocol)),
                    _ => return Ok(error_reply("eprotonosupport")),
                }
            }
            SocketType::Datagram => {
                let socket_type = match kind {
                    1 => Type::STREAM,
                    2 => Type::DGRAM,
                    3 => return Err(self.unsupported("type", kind)),
                    _ => return Ok(error_reply("einval")),
                };
                let protocol = match protocol {
                    0 if kind == 1 => return Err(self.unsupported("type", kind)),
                    0 => None,
                    2 => Some(Protocol::UDP),
                    3 => return Err(self.unsupported("protocol", protocol)),
                    _ => return Ok(error_reply("eprotonosupport")),
                };
                (socket_type, protocol)
            }
        };
        if self.state != State::Closed {
            return Ok(error_reply("exbadseq"));
        }
        Ok(reply(
            self.open_socket(domain, socket_type, protocol),
            |()| vec![OK],
        ))
    }

    /// Make the non-blocking socket of `domain`, `socket_type`, and
    /// `protocol`. `SET_NONBLOCKING` ignores its result (`inet_drv.c:5212`).
    fn open_socket(
        &mut self,
        domain: Domain,
        socket_type: Type,
        protocol: Option<Protocol>,
    ) -> io::Result<()> {
        // A `local` socket takes the protocol 0 (inet_drv.c:5167).
        let protocol = protocol.filter(|_| domain != Domain::UNIX);
        let socket = Socket::new(domain, socket_type, protocol)?;
        let _ = socket.set_nonblocking(true);
        self.family = match domain {
            Domain::IPV6 => AF_INET6,
            Domain::UNIX => AF_LOCAL,
            _ => AF_INET,
        };
        self.socket = Some(socket);
        self.state = State::Open;
        self.stream.ended = false;
        Ok(())
    }

    /// Run `INET_REQ_NAME` (inet_drv.c:11360-11380): the address of the
    /// socket. Without a socket, `getsockname` of `INVALID_SOCKET` fails
    /// with `EBADF`.
    fn name(&self) -> Vec<u8> {
        let Some(socket) = &self.socket else {
            return error_reply("ebadf");
        };
        if self.family == AF_LOCAL {
            return reply(local_name(socket, nix::sys::socket::getsockname), |name| {
                [&[OK][..], &name].concat()
            });
        }
        reply(socket.local_addr(), |address| {
            [&[OK][..], &address_bytes(&address)].concat()
        })
    }

    /// Run `INET_REQ_PEER` (inet_drv.c:11272-11300): the address of the
    /// peer of a connected socket, also of a `udp_inet` socket after
    /// `CONNECT`. Each other state gives `enotconn`.
    fn peer(&self) -> Vec<u8> {
        let (State::Connected, Some(socket)) = (self.state, &self.socket) else {
            return error_reply("enotconn");
        };
        if self.family == AF_LOCAL {
            return reply(local_name(socket, nix::sys::socket::getpeername), |name| {
                [&[OK][..], &name].concat()
            });
        }
        reply(socket.peer_addr(), |address| {
            [&[OK][..], &address_bytes(&address)].concat()
        })
    }

    /// Check `INET_REQ_ACCEPT` with the timeout `[T3, T2, T1, T0]`
    /// (inet_drv.c:12144-12162): a socket that does not listen and a
    /// request that is not 4 bytes give `einval`.
    fn accept(&self, request: &[u8]) -> Control {
        match (self.state, request) {
            (State::Listening, &[t3, t2, t1, t0]) => {
                Control::Accept(u32::from_be_bytes([t3, t2, t1, t0]))
            }
            _ => Control::Reply(error_reply("einval")),
        }
    }

    /// Run `INET_REQ_BIND` with `[1, P1, P0, A, B, C, D]`
    /// (inet_drv.c:11409-11437; `inet_set_faddress`, inet_drv.c:4795-4880):
    /// bind the socket, and reply with the port number, which the host
    /// selects for port 0.
    fn bind(&mut self, request: &[u8]) -> Vec<u8> {
        if request.len() < 2 {
            return error_reply("einval");
        }
        if self.state != State::Open {
            return error_reply("exbadport");
        }
        let local = match self.address(request) {
            Ok(local) => local,
            Err(reply) => return reply,
        };
        let port = local.as_socket().map_or(0, |address| address.port());
        let socket = self.socket.as_ref().expect("an open port has a socket");
        reply(socket.bind(&local), |()| {
            let port = match port {
                0 => socket
                    .local_addr()
                    .ok()
                    .and_then(|address| address.as_socket())
                    .map_or(0, |address| address.port()),
                port => port,
            };
            [&[OK][..], &port.to_be_bytes()].concat()
        })
    }

    /// Read the address of a request of at least 2 bytes, as
    /// `inet_set_faddress` and `inet_set_address` do
    /// (inet_drv.c:4706-4880), for the request `what`: `[1, P1, P0, A, B,
    /// C, D]` for IPv4, `[2, P1, P0, A1, ..., A16, F3, ..., F0, S3, ...,
    /// S0]` for IPv6 with the flow info and the scope id, and `[3, P1, P0]`
    /// (`any`) and `[4, P1, P0]` (`loopback`) for the address of the
    /// family of the socket, and `[5, N, B1, ..., BN]` for the `local`
    /// file name of N bytes. An unknown code gives `eafnosupport`, and a
    /// short address `einval`.
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// let address = port.address(&[1, 0, 80, 127, 0, 0, 1]);
    /// assert_eq!(address.map(|a| a.as_socket()), Ok(Some("127.0.0.1:80".parse().unwrap())));
    /// assert_eq!(port.address(&[9, 0]), Err(b"\0eafnosupport".to_vec()));
    /// ```
    fn address(&self, request: &[u8]) -> Result<SockAddr, Vec<u8>> {
        use std::net::{SocketAddrV4, SocketAddrV6};
        let (&tag, address) = request.split_first().expect("a request of 2 bytes");
        let address: SockAddr = match (tag, address) {
            (AF_INET, &[p1, p0, a, b, c, d, ..]) => {
                SocketAddrV4::new([a, b, c, d].into(), u16::from_be_bytes([p1, p0])).into()
            }
            // ERTS checks 18 bytes and reads 26 (inet_drv.c:4728-4742);
            // `prim_inet` always gives the 26.
            (AF_INET6, &[p1, p0, ref rest @ ..]) if rest.len() >= 16 + 8 => {
                let word =
                    |at: usize| u32::from_be_bytes(rest[at..at + 4].try_into().expect("4 bytes"));
                let ip = <[u8; 16]>::try_from(&rest[..16]).expect("16 bytes");
                let port = u16::from_be_bytes([p1, p0]);
                SocketAddrV6::new(ip.into(), port, word(16), word(20)).into()
            }
            (3 | 4, &[p1, p0, ..]) => {
                let port = u16::from_be_bytes([p1, p0]);
                match (self.family, tag) {
                    // `inet_set_faddress` has no `any` or `loopback` of
                    // `AF_UNIX` (inet_drv.c:4822-4872).
                    (AF_LOCAL, _) => return Err(error_reply("einval")),
                    (AF_INET6, 3) => SocketAddrV6::new(Ipv6Addr::UNSPECIFIED, port, 0, 0).into(),
                    (AF_INET6, _) => SocketAddrV6::new(Ipv6Addr::LOCALHOST, port, 0, 0).into(),
                    (_, 3) => SocketAddrV4::new(Ipv4Addr::UNSPECIFIED, port).into(),
                    _ => SocketAddrV4::new(Ipv4Addr::LOCALHOST, port).into(),
                }
            }
            (AF_LOCAL, &[n, ref name @ ..]) => local_address(n, name)?,
            (AF_INET | AF_INET6 | 3 | 4 | AF_LOCAL, _) => return Err(error_reply("einval")),
            _ => return Err(error_reply("eafnosupport")),
        };
        Ok(address)
    }

    /// Run `INET_REQ_SETOPTS` (`inet_set_opts`, inet_drv.c:6871-7530): set
    /// each option of 5 bytes in order, and ignore a rest of fewer bytes.
    /// `linger` takes 4 more bytes, the time in seconds, and a missing time
    /// gives `einval`. A failed `setsockopt` of `reuseaddr`, `keepalive`,
    /// `linger`, and `nodelay` does not fail the request (`propagate` is
    /// 0), also without a socket. A failed `setsockopt` of `priority`,
    /// `tos`, `ttl`, `recvtos`, and `recvttl` gives `einval` (`propagate`
    /// is 1, inet_drv.c:7519-7530), and the options before it stay set.
    /// `send_timeout`, `send_timeout_close`, and `delay_send` change only a
    /// stream socket, so they change nothing before `OPEN` and nothing on a
    /// `udp_inet` port (inet_drv.c:7099-7131). Without `SO_PRIORITY`, as on macOS,
    /// `priority` changes nothing (inet_drv.c:7389-7398).
    ///
    /// `{active, N}` on a `udp_inet` port takes 2 more bytes, the signed
    /// count, and adds it to the count: a sum outside `i16` gives `einval`,
    /// a sum below 0 becomes 0, and a count of 0 makes the socket passive
    /// and adds 1 to `passive`, the number of `{udp_passive, S}` messages
    /// (inet_drv.c:6974-7000). `read_packets` below 1 gives `einval`
    /// (inet_drv.c:7138-7148).
    ///
    /// `buffer` below 1 becomes 1 and stops the change of `buffer` by
    /// `recbuf` (inet_drv.c:6958-6966). Before that, `recbuf` above
    /// `buffer` sets `buffer` to it, also when the `setsockopt` fails, and a
    /// `recbuf` above 65536 on a `udp_inet` port sets `buffer` to 65536
    /// (inet_drv.c:7321-7360). A failed `setsockopt` of `dontroute`,
    /// `broadcast`, `sndbuf`, `recbuf`, `multicast_ttl`, and
    /// `multicast_loop` does not fail the request (inet_drv.c:7293-7360,
    /// 7571-7587, 7923-7942).
    ///
    /// `multicast_if`, `add_membership`, and `drop_membership` on a `local`
    /// socket, and a membership whose family is not the family of the
    /// socket, give `einval`. A membership takes 12 more bytes on IPv4 and
    /// 20 more bytes on IPv6, and fewer bytes give a runtime error. A failed
    /// `setsockopt` of these options does not fail the request
    /// (inet_drv.c:7589-7802).
    #[expect(
        clippy::too_many_lines,
        reason = "option table: one arm for each `INET_REQ_SETOPTS` option code, as `inet_set_opts` in `inet_drv.c` (7018-7802); each arm is short"
    )]
    fn set_options(
        &mut self,
        mut request: &[u8],
        passive: &mut usize,
    ) -> Result<Vec<u8>, BeamError> {
        while let [option, v3, v2, v1, v0, rest @ ..] = request {
            let value = i32::from_be_bytes([*v3, *v2, *v1, *v0]);
            request = rest;
            let set = match option {
                // PBeam reads a `tcp_inet` socket with `{packet, raw}` only,
                // so a `SETOPTS` that keeps `raw` changes nothing. The port
                // stores the packet type for each other value
                // (inet_drv.c:7018-7024), and PBeam cannot frame that type.
                23 if value == 0 => true,
                23 => return Err(self.unsupported("option packet", value)),
                24 => {
                    self.mode = value;
                    true
                }
                22 if value == 3 && self.socket_type == SocketType::Stream => {
                    return Err(error(ACTIVE_N));
                }
                22 if value == 3 => {
                    let [n1, n0, rest @ ..] = request else {
                        return Err(
                            self.unsupported("option active with a count of", "fewer than 2 bytes")
                        );
                    };
                    request = rest;
                    self.active = value;
                    let Ok(count) = i16::try_from(
                        i32::from(self.active_count) + i32::from(i16::from_be_bytes([*n1, *n0])),
                    ) else {
                        return Ok(error_reply("einval"));
                    };
                    self.active_count = count.max(0);
                    if self.active_count == 0 {
                        self.active = 0;
                        *passive += 1;
                    }
                    true
                }
                33 if self.socket_type == SocketType::Datagram => {
                    if value > 0 {
                        self.read_packets = value;
                    }
                    value > 0
                }
                22 => {
                    self.active = value;
                    self.active_count = 0;
                    // The end of the port skips the next options
                    // (inet_drv.c:7001-7011).
                    if value != 0 && self.stream.ended {
                        break;
                    }
                    true
                }
                20 => {
                    self.stream.buffer =
                        usize::try_from(value.max(1)).expect("the buffer is at least 1");
                    self.buffer_set = true;
                    true
                }
                2 => self.set_socket(libc::SO_DONTROUTE, SetErrors::Ignore, |socket| {
                    setsockopt(socket, sockopt::DontRoute, &(value != 0)).map_err(io::Error::from)
                }),
                4 => self.set_socket(libc::SO_BROADCAST, SetErrors::Ignore, |socket| {
                    socket.set_broadcast(value != 0)
                }),
                6 => self.set_socket(libc::SO_SNDBUF, SetErrors::Ignore, |socket| {
                    socket.set_send_buffer_size(sign_extended(value))
                }),
                7 => {
                    if !self.buffer_set {
                        if value > MAX_UDP_BUFFER && self.socket_type == SocketType::Datagram {
                            self.stream.buffer = usize::try_from(MAX_UDP_BUFFER)
                                .expect("the UDP buffer limit is positive");
                        } else if i64::from(value)
                            > i64::try_from(self.stream.buffer).expect("a buffer size fits in i64")
                        {
                            self.stream.buffer = usize::try_from(value)
                                .expect("a value above the buffer is positive");
                        }
                    }
                    self.set_socket(libc::SO_RCVBUF, SetErrors::Ignore, |socket| {
                        socket.set_recv_buffer_size(sign_extended(value))
                    })
                }
                12 => self.set_socket(libc::IP_MULTICAST_TTL, SetErrors::Ignore, |socket| {
                    socket.set_multicast_ttl_v4(value.cast_unsigned())
                }),
                13 => self.set_socket(libc::IP_MULTICAST_LOOP, SetErrors::Ignore, |socket| {
                    socket.set_multicast_loop_v4(value != 0)
                }),
                11 => match self.family {
                    AF_INET => {
                        self.set_socket(libc::IP_MULTICAST_IF, SetErrors::Ignore, |socket| {
                            socket.set_multicast_if_v4(&Ipv4Addr::from(value.cast_unsigned()))
                        })
                    }
                    // ERTS gives the index in network byte order
                    // (`sock_htonl`, inet_drv.c:7608).
                    AF_INET6 => {
                        self.set_socket(libc::IPV6_MULTICAST_IF, SetErrors::Ignore, |socket| {
                            socket.set_multicast_if_v6(value.cast_unsigned().to_be())
                        })
                    }
                    _ => false,
                },
                14 | 15 => {
                    let action = if *option == 14 {
                        GroupAction::Join
                    } else {
                        GroupAction::Leave
                    };
                    match (value, self.family) {
                        (1, AF_INET) => {
                            let Some((head, rest)) = request.split_first_chunk::<12>() else {
                                return Err(self
                                    .unsupported("option membership with", "fewer than 12 bytes"));
                            };
                            request = rest;
                            let [i3, i2, i1, i0, g3, g2, g1, g0, a3, a2, a1, a0] = *head;
                            let index = u32::from_be_bytes([i3, i2, i1, i0]);
                            let group = Ipv4Addr::new(g3, g2, g1, g0);
                            let interface = Ipv4Addr::new(a3, a2, a1, a0);
                            let kind = if action == GroupAction::Join {
                                libc::IP_ADD_MEMBERSHIP
                            } else {
                                libc::IP_DROP_MEMBERSHIP
                            };
                            self.set_socket(kind, SetErrors::Ignore, |socket| {
                                membership_v4(socket, action, group, index, interface)
                            })
                        }
                        (2, AF_INET6) => {
                            let Some((head, _)) = request.split_first_chunk::<20>() else {
                                return Err(self
                                    .unsupported("option membership with", "fewer than 20 bytes"));
                            };
                            // The driver skips the options after it
                            // (`len -= len`, inet_drv.c:7780).
                            request = &[];
                            let [i3, i2, i1, i0, ref group @ ..] = *head;
                            let index = u32::from_be_bytes([i3, i2, i1, i0]);
                            let group = Ipv6Addr::from(*group);
                            self.set_socket(
                                IPV6_MEMBERSHIP[usize::from(action == GroupAction::Join)],
                                SetErrors::Ignore,
                                |socket| {
                                    if action == GroupAction::Join {
                                        socket.join_multicast_v6(&group, index)
                                    } else {
                                        socket.leave_multicast_v6(&group, index)
                                    }
                                },
                            )
                        }
                        _ => false,
                    }
                }
                0 => self.set_socket(libc::SO_REUSEADDR, SetErrors::Ignore, |socket| {
                    socket.set_reuse_address(value != 0)
                }),
                1 => self.set_socket(libc::SO_KEEPALIVE, SetErrors::Ignore, |socket| {
                    socket.set_keepalive(value != 0)
                }),
                10 => self.set_socket(libc::TCP_NODELAY, SetErrors::Ignore, |socket| {
                    socket.set_tcp_nodelay(value != 0)
                }),
                3 => {
                    let [s3, s2, s1, s0, rest @ ..] = request else {
                        return Ok(error_reply("einval"));
                    };
                    request = rest;
                    let linger = linger_time(value, u32::from_be_bytes([*s3, *s2, *s1, *s0]))?;
                    self.set_socket(libc::SO_LINGER, SetErrors::Ignore, |socket| {
                        socket.set_linger(linger)
                    })
                }
                8 => self.set_priority(value),
                9 => self.set_socket(libc::IP_TOS, SetErrors::Propagate, |socket| {
                    socket.set_tos_v4(value.cast_unsigned())
                }),
                46 => self.set_socket(libc::IP_TTL, SetErrors::Propagate, |socket| {
                    socket.set_ttl_v4(value.cast_unsigned())
                }),
                // A datagram receive reads no ancillary data, so it cannot
                // give `{tos, TOS}` (inet_drv.c:15452-15466).
                43 if value != 0 && self.socket_type == SocketType::Datagram => {
                    return Err(error(UDP_RECVTOS_ON));
                }
                43 => self.set_socket(libc::IP_RECVTOS, SetErrors::Propagate, |socket| {
                    socket.set_recv_tos_v4(value != 0)
                }),
                16 => self.set_socket(libc::IPV6_V6ONLY, SetErrors::Propagate, |socket| {
                    socket.set_only_v6(value != 0)
                }),
                41 => self.set_socket(libc::IPV6_TCLASS, SetErrors::Propagate, |socket| {
                    socket.set_tclass_v6(value.cast_unsigned())
                }),
                44 => self.set_socket(libc::IPV6_RECVTCLASS, SetErrors::Propagate, |socket| {
                    socket.set_recv_tclass_v6(value != 0)
                }),
                47 if value != 0 => return Err(error(RECVTTL_ON)),
                // PBeam never sets `IP_RECVTTL`, so a socket has it off.
                47 => self.socket.is_some(),
                // ERTS sets `TCP_CORK` where the host has it, and skips the
                // option on macOS, which has no `TCP_NOPUSH` for ERTS
                // (inet_drv.c:1101-1105, 7506-7521).
                #[cfg(target_os = "linux")]
                48 => self.set_socket(libc::TCP_CORK, SetErrors::Ignore, |socket| {
                    socket.set_tcp_cork(value != 0)
                }),
                #[cfg(not(target_os = "linux"))]
                48 => true,
                30 | 35 | 31 if !self.has_stream_socket() => true,
                30 => {
                    self.send_timeout = value;
                    true
                }
                35 => {
                    self.send_timeout_close = value;
                    true
                }
                // A send that waits for a busy port runs at the unbusy, which
                // cannot return the error of `delay_send` (see `Inet::send`).
                31 if value != 0 && !self.stream.waiters.is_empty() => {
                    return Err(self.unsupported("option", "delay_send while a send waits"));
                }
                31 => {
                    self.delay_send = value != 0;
                    true
                }
                &other => return Err(self.unsupported("option", other)),
            };
            if !set {
                return Ok(error_reply("einval"));
            }
        }
        Ok(vec![OK])
    }

    /// Set the read select of an open `udp_inet` socket after a valid
    /// `SETOPTS` that changed the option `active` from `old`, as
    /// `inet_set_opts` does (inet_drv.c:7945-7967): an active socket reads,
    /// and a passive socket does not. A change to passive also stops the
    /// timer of the receive that waits, which then waits until the close of
    /// the port (inet_drv.c:7961-7964). The timer service drops the stopped
    /// timer when it comes, as no receive holds it.
    fn select_datagram_read(&mut self, old: i32) {
        if self.socket_type != SocketType::Datagram || self.socket.is_none() || self.active == old {
            return;
        }
        self.read_off = self.active == 0;
        if self.read_off
            && let Some(receiver) = &mut self.stream.receiver
        {
            receiver.timer = None;
        }
    }

    /// Return the result of a `SETOPTS` with the reply `reply` that changed
    /// the option `active` from `old`: [`Control::Active`] when a connected
    /// socket became active or a socket whose connection ended is active,
    /// else the reply (inet_drv.c:7001-7011, 7949-7981). A connected
    /// `udp_inet` socket gives the reply, so a receive that waits does not
    /// read for the owner (inet_drv.c:4335-4357). PBeam runs no
    /// `{active, N}`, so only `once` reads.
    fn set_active_result(&self, old: i32, reply: Vec<u8>) -> Control {
        let ended = self.stream.ended && self.active != 0;
        if reply != [OK]
            || !ended
                && (old != 0
                    || self.active == 0
                    || self.state != State::Connected
                    || self.socket_type != SocketType::Stream)
        {
            return Control::Reply(reply);
        }
        Control::Active {
            activation: if self.active == 2 {
                Activation::Read
            } else {
                Activation::Deliver
            },
        }
    }

    /// Return whether the port has a socket of the stream type, the only type
    /// that has `send_timeout`, `send_timeout_close`, and `delay_send`
    /// (inet_drv.c:7099-7131, 9236-9265).
    fn has_stream_socket(&self) -> bool {
        self.socket.is_some() && self.socket_type == SocketType::Stream
    }

    /// Run `set` on the socket of the option with the socket constant
    /// `kind`, such as `IP_TOS` (see [`set_option`]). Return `false` when `errors` is [`SetErrors::Propagate`] and the
    /// socket is missing or `set` fails, else `true`.
    fn set_socket(
        &self,
        kind: c_int,
        errors: SetErrors,
        set: impl FnOnce(&Socket) -> io::Result<()>,
    ) -> bool {
        let result = self
            .socket
            .as_ref()
            .map(|socket| set_option(socket, kind, errors, set));
        errors == SetErrors::Ignore || matches!(result, Some(Ok(())))
    }

    /// Run `INET_REQ_GETOPTS` (`inet_fill_opts`, inet_drv.c:9071-9754): give
    /// each option code with its value in order. A socket option whose
    /// `getsockopt` fails, also without a socket, is not in the reply, and
    /// `send_timeout`, `send_timeout_close`, and `delay_send` are not in it
    /// before `OPEN`. Without `SO_PRIORITY`, as on macOS, `priority` is 0
    /// (inet_drv.c:9316-9324). A reply of more than 64 KiB gives `einval`,
    /// also when only the last 4 bytes of `linger` exceed it.
    fn get_options(&self, request: &[u8]) -> Result<Vec<u8>, BeamError> {
        let mut reply = vec![OK];
        for &option in request {
            if reply.len() + 5 > MAX_OPT_BUFFER {
                return Ok(error_reply("einval"));
            }
            let socket = self.socket.as_ref();
            let flag = |get: fn(&Socket) -> io::Result<bool>, bit: i32| {
                socket
                    .and_then(|socket| get(socket).ok())
                    .map(|on| i32::from(on) * getsockopt_scale(bit))
            };
            let integer = |get: fn(&Socket) -> io::Result<u32>| {
                socket
                    .and_then(|socket| get(socket).ok())
                    .map(u32::cast_signed)
            };
            let size = |get: fn(&Socket) -> io::Result<usize>| {
                socket
                    .and_then(|socket| get(socket).ok())
                    .map(|value| low_u32(to_u64(value)).cast_signed())
            };
            let value = match option {
                24 => Some(self.mode),
                22 if self.active == 3 => {
                    // The count needs 2 more bytes (inet_drv.c:9179-9183).
                    if reply.len() + 7 > MAX_OPT_BUFFER {
                        return Ok(error_reply("einval"));
                    }
                    reply.push(option);
                    reply.extend(self.active.to_be_bytes());
                    reply.extend(self.active_count.to_be_bytes());
                    continue;
                }
                22 => Some(self.active),
                33 if self.socket_type == SocketType::Datagram => Some(self.read_packets),
                20 => Some(low_u32(to_u64(self.stream.buffer)).cast_signed()),
                2 => socket
                    .and_then(|socket| getsockopt(socket, sockopt::DontRoute).ok())
                    .map(|on| i32::from(on) * getsockopt_scale(libc::SO_DONTROUTE)),
                4 => flag(Socket::broadcast, libc::SO_BROADCAST),
                6 => size(Socket::send_buffer_size),
                7 => size(Socket::recv_buffer_size),
                12 => integer(Socket::multicast_ttl_v4),
                13 => flag(Socket::multicast_loop_v4, 1),
                // The driver writes the family after 4 bytes that it does
                // not write, and the interface after the end of the reply
                // (inet_drv.c:9497-9528). PBeam gives 0 for the 4 bytes.
                11 => {
                    if !matches!(self.family, AF_INET | AF_INET6) {
                        return Ok(error_reply("einval"));
                    }
                    if reply.len() + 9 > MAX_OPT_BUFFER {
                        return Ok(error_reply("einval"));
                    }
                    reply.push(option);
                    reply.extend([0; 4]);
                    reply.extend(i32::from(self.family).to_be_bytes());
                    continue;
                }
                // ERTS reads `TCP_CORK` where the host has it, and gives 0 on
                // macOS (inet_drv.c:9451-9460).
                #[cfg(target_os = "linux")]
                48 => flag(Socket::tcp_cork, 1),
                #[cfg(not(target_os = "linux"))]
                48 => Some(0),
                30 => self.has_stream_socket().then_some(self.send_timeout),
                35 => self.has_stream_socket().then_some(self.send_timeout_close),
                31 => self
                    .has_stream_socket()
                    .then_some(i32::from(self.delay_send)),
                0 => flag(Socket::reuse_address, APPLE_SO_REUSEADDR),
                1 => flag(Socket::keepalive, APPLE_SO_KEEPALIVE),
                10 => flag(Socket::tcp_nodelay, APPLE_TF_NODELAY),
                43 => flag(Socket::recv_tos_v4, 1),
                16 => flag(Socket::only_v6, 1),
                44 => flag(Socket::recv_tclass_v6, 1),
                41 => integer(Socket::tclass_v6),
                8 => priority(socket),
                9 => integer(Socket::tos_v4),
                46 => integer(Socket::ttl_v4),
                47 => socket.map(|_| 0),
                3 => {
                    if let Some(linger) = socket.and_then(|socket| socket.linger().ok()) {
                        // `linger` needs 4 more bytes (inet_drv.c:9749).
                        if reply.len() + 9 > MAX_OPT_BUFFER {
                            return Ok(error_reply("einval"));
                        }
                        let (on, seconds) = linger.map_or((0, 0), |time| (1, time.as_secs()));
                        reply.push(option);
                        reply.extend(i32::to_be_bytes(on));
                        reply.extend(low_u32(seconds).to_be_bytes());
                    }
                    continue;
                }
                other => return Err(self.unsupported("option", other)),
            };
            if let Some(value) = value {
                reply.push(option);
                reply.extend(value.to_be_bytes());
            }
        }
        Ok(reply)
    }

    /// Run `INET_REQ_LISTEN` with the backlog `[B1, B0]`
    /// (inet_drv.c:12042-12062).
    fn listen(&mut self, request: &[u8]) -> Vec<u8> {
        let Some(socket) = &self.socket else {
            return error_reply("exbadport");
        };
        let &[b1, b0] = request else {
            return error_reply("einval");
        };
        let result = socket.listen(u16::from_be_bytes([b1, b0]).into());
        reply(result, |()| {
            self.state = State::Listening;
            vec![OK]
        })
    }
}

/// The runtime error of `{active, N}`.
/// The `buffer` that `recbuf` sets on a `udp_inet` port at most
/// (`INET_UDP_THEORETICAL_MAX_MTU`, `inet_drv.c:1059`).
const MAX_UDP_BUFFER: i32 = 1 << 16;

const ACTIVE_N: &str = "unsupported tcp_inet option active with a count: see \"tcp_inet ports\" in docs/erts-differences.md";

/// The runtime error of `{recvttl, true}`: `socket2` has no `IP_RECVTTL`
/// call.
const RECVTTL_ON: &str =
    "unsupported tcp_inet option recvttl true: see \"tcp_inet ports\" in docs/erts-differences.md";

/// The runtime error of a `linger` time that `socket2` cannot set as ERTS
/// does: a time with `linger` off, which `SO_LINGER` keeps, and on macOS
/// each time, where `socket2` sets `SO_LINGER_SEC` and ERTS sets
/// `SO_LINGER`, which xnu reads in clock ticks.
/// The runtime error of `{recvtos, true}` on a `udp_inet` port.
const UDP_RECVTOS_ON: &str = "unsupported udp_inet option recvtos with true: see \"udp_inet ports\" in docs/erts-differences.md";

const LINGER_TIME: &str = "unsupported tcp_inet option linger with a time: see \"tcp_inet ports\" in docs/erts-differences.md";

/// The flag bits that xnu gives for an option that is on: `SO_REUSEADDR`,
/// `SO_KEEPALIVE`, and `TF_NODELAY` (see [`getsockopt_value`]).
const APPLE_SO_REUSEADDR: i32 = 4;
const APPLE_SO_KEEPALIVE: i32 = 8;
const APPLE_TF_NODELAY: i32 = 4;

/// Return the time of `linger` for `SETOPTS`: `None` when `on` is 0 and
/// `seconds` is 0.
///
/// # Examples
///
/// ```rust,ignore
/// assert_eq!(linger_time(0, 0)?, None);
/// ```
///
/// # Errors
///
/// Each other time (see [`LINGER_TIME`]).
#[cfg(target_vendor = "apple")]
fn linger_time(on: i32, seconds: u32) -> Result<Option<Duration>, BeamError> {
    (on == 0 && seconds == 0)
        .then_some(None)
        .ok_or_else(|| error(LINGER_TIME))
}

/// Return the time of `linger` for `SETOPTS`: `None` when `on` is 0, else
/// `seconds`.
///
/// # Examples
///
/// ```rust,ignore
/// assert_eq!(linger_time(1, 5)?, Some(Duration::from_secs(5)));
/// ```
///
/// # Errors
///
/// A time that is not 0 when `on` is 0 (see [`LINGER_TIME`]).
#[cfg(not(target_vendor = "apple"))]
fn linger_time(on: i32, seconds: u32) -> Result<Option<Duration>, BeamError> {
    let time = (on != 0).then(|| Duration::from_secs(seconds.into()));
    (on != 0 || seconds == 0)
        .then_some(time)
        .ok_or_else(|| error(LINGER_TIME))
}

/// The socket constants of `add_membership` and `drop_membership` on an
/// IPv6 socket, by `add` (`INET_ADD_MEMBERSHIP` and `INET_DROP_MEMBERSHIP`,
/// inet_drv.c:1126-1137).
#[cfg(target_os = "linux")]
const IPV6_MEMBERSHIP: [c_int; 2] = [libc::IPV6_DROP_MEMBERSHIP, libc::IPV6_ADD_MEMBERSHIP];
#[cfg(not(target_os = "linux"))]
const IPV6_MEMBERSHIP: [c_int; 2] = [libc::IPV6_LEAVE_GROUP, libc::IPV6_JOIN_GROUP];

/// Join or leave the IPv4 group `group` with `ip_mreqn`, as
/// `inet_set_opts` does on Linux (inet_drv.c:7696-7711). The kernel finds
/// the device by the index `index` when it is not 0, else by the address
/// `interface` (`ip_mc_find_dev`), so the call gives only one of them.
#[cfg(target_os = "linux")]
fn membership_v4(
    socket: &Socket,
    action: GroupAction,
    group: Ipv4Addr,
    index: u32,
    interface: Ipv4Addr,
) -> io::Result<()> {
    use socket2::InterfaceIndexOrAddress;
    let interface = if index == 0 {
        InterfaceIndexOrAddress::Address(interface)
    } else {
        InterfaceIndexOrAddress::Index(index)
    };
    if action == GroupAction::Join {
        socket.join_multicast_v4_n(&group, &interface)
    } else {
        socket.leave_multicast_v4_n(&group, &interface)
    }
}

/// Join or leave the IPv4 group `group` with `ip_mreq`, which has
/// no index, as `inet_set_opts` does without `ip_mreqn`
/// (inet_drv.c:7728-7742).
#[cfg(not(target_os = "linux"))]
fn membership_v4(
    socket: &Socket,
    action: GroupAction,
    group: Ipv4Addr,
    _index: u32,
    interface: Ipv4Addr,
) -> io::Result<()> {
    if action == GroupAction::Join {
        socket.join_multicast_v4(&group, &interface)
    } else {
        socket.leave_multicast_v4(&group, &interface)
    }
}

/// Set an option with `set`, as `inet_setopt` does without `SO_PRIORITY`
/// (inet_drv.c:6833-6860).
///
/// # Examples
///
/// ```rust,ignore
/// set_option(&socket, libc::SO_KEEPALIVE, SetErrors::Ignore, |socket| socket.set_keepalive(true))?;
/// ```
///
/// # Errors
///
/// The error of `set`.
#[cfg(not(target_os = "linux"))]
fn set_option(
    socket: &Socket,
    _kind: c_int,
    _errors: SetErrors,
    set: impl FnOnce(&Socket) -> io::Result<()>,
) -> io::Result<()> {
    set(socket)
}

/// Set an option with `set`, as `setopt_prio_tos_trick` does on the
/// socket (see [`prio_tos_trick`]).
///
/// # Examples
///
/// ```rust,ignore
/// set_option(&socket, libc::IP_TOS, SetErrors::Propagate, |socket| socket.set_tos_v4(16))?;
/// ```
///
/// # Errors
///
/// The error of `set`, and with [`SetErrors::Propagate`] the error of a set
/// again.
#[cfg(target_os = "linux")]
fn set_option(
    socket: &Socket,
    kind: c_int,
    errors: SetErrors,
    set: impl FnOnce(&Socket) -> io::Result<()>,
) -> io::Result<()> {
    prio_tos_trick(
        kind,
        errors,
        (socket.priority(), socket.tos_v4()),
        || set(socket),
        |tos| socket.set_tos_v4(tos),
        |priority| socket.set_priority(priority),
    )
}

/// Run `setopt_prio_tos_trick` (inet_drv.c:6773-6827) with `saved`, the
/// `getsockopt` results of `SO_PRIORITY` and `IP_TOS` before `set`: run
/// `set`, and then set `tos` with `set_tos` and `priority` with
/// `set_priority` again, because Linux changes `priority` when `tos`
/// changes. ERTS compares only the socket constant `kind`, not the level
/// (inet_drv.c:6798-6799): an option with the constant of `IP_TOS`, such
/// as `TCP_NODELAY`, does not set `tos` again, and an option with the
/// constant of `SO_PRIORITY` sets nothing again. A saved value whose
/// `getsockopt` failed is not set again. With [`SetErrors::Propagate`], a failed set of
/// `tos` again is the result and stops the set of `priority`, and a failed
/// set of `priority` again is the result unless it fails with `EPERM`.
///
/// # Examples
///
/// ```rust,ignore
/// prio_tos_trick(libc::IP_TTL, SetErrors::Propagate, (Ok(0), Ok(16)), set, set_tos, set_priority)?;
/// ```
///
/// # Errors
///
/// The error of `set`, and with [`SetErrors::Propagate`] the error of a set
/// again.
#[cfg(target_os = "linux")]
fn prio_tos_trick(
    kind: c_int,
    errors: SetErrors,
    saved: (io::Result<u32>, io::Result<u32>),
    set: impl FnOnce() -> io::Result<()>,
    set_tos: impl FnOnce(u32) -> io::Result<()>,
    set_priority: impl FnOnce(u32) -> io::Result<()>,
) -> io::Result<()> {
    let (priority, tos) = saved;
    set()?;
    if kind == libc::SO_PRIORITY {
        return Ok(());
    }
    if let (true, Ok(tos)) = (kind != libc::IP_TOS, tos) {
        let restored = set_tos(tos);
        if errors == SetErrors::Propagate {
            restored?;
        }
    }
    let Ok(priority) = priority else {
        return Ok(());
    };
    match set_priority(priority) {
        Err(failure)
            if errors == SetErrors::Propagate && failure.raw_os_error() != Some(libc::EPERM) =>
        {
            Err(failure)
        }
        _ => Ok(()),
    }
}

/// Return the `GETOPTS` value of `priority`. Without `SO_PRIORITY`, as on
/// macOS, it is 0 (inet_drv.c:9316-9324).
///
/// # Examples
///
/// ```rust,ignore
/// assert_eq!(priority(None), Some(0));
/// ```
#[cfg(not(target_os = "linux"))]
#[expect(
    clippy::unnecessary_wraps,
    reason = "same signature as the Linux variant, which reads the socket option and can fail"
)]
fn priority(_socket: Option<&Socket>) -> Option<i32> {
    Some(0)
}

/// Return the `GETOPTS` value of `priority`: the value of `getsockopt`, or
/// `None` when it fails, also without a socket.
///
/// # Examples
///
/// ```rust,ignore
/// assert_eq!(priority(None), None);
/// ```
#[cfg(target_os = "linux")]
fn priority(socket: Option<&Socket>) -> Option<i32> {
    socket
        .and_then(|socket| socket.priority().ok())
        .map(u32::cast_signed)
}

impl Inet {
    /// Run `INET_REQ_SUBSCRIBE` of `caller` (`inet_subscribe`,
    /// inet_drv.c:10819-10848; the size check, inet_drv.c:11025-11044):
    /// give each `subs_empty_out_q` (1) with the size of the queue, 32
    /// bits. With a size above 0, each code adds `caller` to the
    /// subscribers, which get `{empty_out_q, S}` when the queue becomes
    /// empty. A code that is not 1 gives `badarg` and keeps the
    /// subscriptions of the codes before it, and a reply of more than 64
    /// KiB is empty.
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// assert_eq!(port.subscribe(&[1], caller), Control::Reply(vec![1, 1, 0, 0, 0, 0]));
    /// ```
    fn subscribe(&mut self, request: &[u8], caller: ProcessId) -> Control {
        if 1 + request.len() * 5 > MAX_OPT_BUFFER {
            return Control::Reply(Vec::new());
        }
        let size = self.stream.output_size();
        let mut reply = vec![OK];
        for &code in request {
            if code != 1 {
                return Control::Badarg;
            }
            if size > 0 {
                self.stream.subscribers.push(caller);
            }
            reply.push(code);
            reply.extend(low_u32(to_u64(size)).to_be_bytes());
        }
        Control::Reply(reply)
    }

    /// Run `SETOPTS` of `priority` with the value `value`: without
    /// `SO_PRIORITY`, as on macOS, it changes nothing
    /// (inet_drv.c:7389-7398).
    #[cfg(not(target_os = "linux"))]
    #[expect(
        clippy::unused_self,
        reason = "same signature as the Linux variant, which sets `SO_PRIORITY` on the socket"
    )]
    fn set_priority(&self, _value: i32) -> bool {
        true
    }

    /// Run `SETOPTS` of `priority` with the value `value` (`propagate` is
    /// 1, inet_drv.c:7389-7398).
    #[cfg(target_os = "linux")]
    fn set_priority(&self, value: i32) -> bool {
        self.set_socket(libc::SO_PRIORITY, SetErrors::Propagate, |socket| {
            socket.set_priority(value.cast_unsigned())
        })
    }
}

/// Read the `local` address of the file name of `n` bytes at the start of
/// `name`, as `inet_set_address` does (inet_drv.c:4748-4778): fewer than
/// `n` bytes, or a name with its terminating zero byte longer than
/// `sun_path`, give `einval`. On Linux a name that starts with a zero
/// byte is an abstract address, which has no terminating zero byte.
fn local_address(n: u8, name: &[u8]) -> Result<SockAddr, Vec<u8>> {
    use std::os::unix::ffi::OsStrExt;
    let name = name.get(..n.into()).ok_or_else(|| error_reply("einval"))?;
    let terminator = usize::from(!(cfg!(target_os = "linux") && name.first() == Some(&0)));
    if name.len() + terminator > SUN_PATH {
        return Err(error_reply("einval"));
    }
    Ok(SockAddr::unix(std::ffi::OsStr::from_bytes(name)).expect("a name that fits sun_path"))
}

/// Return `[5, N, B1, ..., BN]` of the `local` address that `get` reads
/// from `socket`, as `inet_get_address` gives it (inet_drv.c:4915-4936):
/// the name ends at its first zero byte, except a Linux abstract name,
/// which starts with a zero byte, and an unnamed address gives the empty
/// name. `nix` keeps each byte of the length that the host gives, which
/// on macOS has no terminating zero byte for a name that ERTS binds
/// (`inet_drv.c:4772`).
fn local_name(
    socket: &Socket,
    get: fn(std::os::fd::RawFd) -> nix::Result<nix::sys::socket::UnixAddr>,
) -> io::Result<Vec<u8>> {
    use std::os::fd::AsRawFd;
    Ok(local_name_bytes(&get(socket.as_raw_fd())?))
}

/// Return `[5, N, B1, ..., BN]` of the `local` address `address`, as
/// `inet_get_address` gives it (inet_drv.c:4915-4936).
fn local_name_bytes(address: &nix::sys::socket::UnixAddr) -> Vec<u8> {
    use std::os::unix::ffi::OsStrExt;
    #[cfg(target_os = "linux")]
    let abstract_name = address.as_abstract();
    #[cfg(not(target_os = "linux"))]
    let abstract_name: Option<&[u8]> = None;
    let name: Vec<u8> = match (address.path(), abstract_name) {
        (Some(path), _) => path
            .as_os_str()
            .as_bytes()
            .iter()
            .copied()
            .take_while(|&byte| byte != 0)
            .collect(),
        (None, Some(name)) => [&[0][..], name].concat(),
        (None, None) => Vec::new(),
    };
    let length = u8::try_from(name.len()).expect("a name that fits sun_path");
    [&[AF_LOCAL, length][..], &name].concat()
}

/// Return `[1, P1, P0, A, B, C, D]` of an IPv4 address and `[2, P1, P0,
/// A1, ..., A16]` of an IPv6 address, as `inet_get_address` gives them
/// (inet_drv.c:4890-4913).
fn address_bytes(address: &SockAddr) -> Vec<u8> {
    let address = address.as_socket().expect("an IP socket");
    let (family, ip) = match address {
        std::net::SocketAddr::V4(address) => (AF_INET, address.ip().octets().to_vec()),
        std::net::SocketAddr::V6(address) => (AF_INET6, address.ip().octets().to_vec()),
    };
    [&[family][..], &address.port().to_be_bytes(), &ip].concat()
}

/// Return `ok` of the value of `result`, or the error reply of its errno,
/// when `ok` can fail.
///
/// # Examples
///
/// ```rust,ignore
/// let reply = try_reply(socket.local_addr(), |_| Ok::<_, BeamError>(vec![OK]))?;
/// ```
///
/// # Errors
///
/// The error of `ok`.
pub(in crate::runtime) fn try_reply<T, E>(
    result: io::Result<T>,
    ok: impl FnOnce(T) -> Result<Vec<u8>, E>,
) -> Result<Vec<u8>, E> {
    result.map_or_else(|failure| Ok(errno_reply(&failure)), ok)
}

/// Return the error name of the errno of `failure`, as `erl_errno_id`
/// gives it.
///
/// # Examples
///
/// ```rust,ignore
/// assert_eq!(errno_name(&io::Error::from_raw_os_error(libc::EMFILE)), "emfile");
/// ```
pub(in crate::runtime) fn errno_name(failure: &io::Error) -> String {
    let errno = failure
        .raw_os_error()
        .expect("a socket call fails with an errno");
    errno_id(errno)
}

/// Return `ok` of the value of `result`, or the error reply of its errno.
fn reply<T>(result: io::Result<T>, ok: impl FnOnce(T) -> Vec<u8>) -> Vec<u8> {
    result.map_or_else(|failure| errno_reply(&failure), ok)
}

/// Return the error reply `[0 | Name]`.
fn error_reply(name: &str) -> Vec<u8> {
    [&[ERROR][..], name.as_bytes()].concat()
}

/// Return the error reply of the errno of `failure`, as `ctl_error` gives
/// it with `erl_errno_id` (inet_drv.c:2240-2250).
fn errno_reply(failure: &io::Error) -> Vec<u8> {
    error_reply(&errno_name(failure))
}

/// Run `INET_REQ_GETHOSTNAME` (inet_drv.c:11219-11233): the name of the
/// host, as `gethostname` gives it ([`host_name`]). A request with bytes
/// gives `einval`.
fn get_host_name(request: &[u8]) -> Vec<u8> {
    if !request.is_empty() {
        return error_reply("einval");
    }
    [&[OK][..], &host_name()].concat()
}

/// Give the value that `getsockopt` returns for a flag option that is on,
/// which `GETOPTS` puts in its reply as is (inet_drv.c:9727-9750). macOS gives
/// the flag bit `bit` of an option that is on: `so_options & SO_REUSEADDR`
/// (4), `so_options & SO_KEEPALIVE` (8), and `t_flags & TF_NODELAY` (4) in
/// xnu `sogetoptlock` and `tcp_ctloutput`, and 1 for `IP_RECVTOS`
/// (`OPTBIT`). Linux gives 1 (`sk_reuse`, `sock_flag(sk, SOCK_KEEPOPEN)`,
/// and `!!(nonagle & TCP_NAGLE_OFF)`). `socket2` gives only a boolean, so
/// the caller multiplies this value by it.
///
/// # Examples
///
/// ```rust,ignore
/// assert_eq!(getsockopt_scale(8), if cfg!(target_vendor = "apple") { 8 } else { 1 });
/// ```
fn getsockopt_scale(bit: i32) -> i32 {
    if APPLE { bit } else { 1 }
}

/// True on macOS and the other Apple hosts.
const APPLE: bool = cfg!(target_vendor = "apple");

#[cfg(all(test, target_os = "linux"))]
#[path = "../../../tests/runtime/port/inet.rs"]
mod tests;

//! The `tcp_inet` port driver of `inet_drv.c`, which `prim_inet` opens
//! with `open_port({spawn_driver, "tcp_inet"}, [binary])`
//! (prim_inet.erl:99-112) and drives with `erlang:port_control/3`.
//!
//! Each request is `port_control(Port, Request + 16#03f1a300, Bytes)`
//! (prim_inet.erl:2969-2975, `ERTS_INET_DRV_CONTROL_MAGIC_NUMBER`). The
//! reply is `[1 | Data]` for OK and `[0 | Name]` for an error, where `Name`
//! is the bytes of the error name, for example `[0 | "eaddrinuse"]`
//! (`ctl_reply` and `ctl_error`, inet_drv.c:2240-2283). PBeam runs these
//! requests of `INET_REQ_*` (inet_int.hrl:76-105) for an IPv4 TCP socket:
//!
//! | Request | Code | Bytes | OK reply |
//! | --- | --- | --- | --- |
//! | `OPEN` | 1 | `[Family, Type, Protocol]`, `[1, 1, 1]` | `[1]` |
//! | `PEER` | 4 | `[]` | `[1, 1, P1, P0, A, B, C, D]` |
//! | `NAME` | 5 | `[]` | `[1, 1, P1, P0, A, B, C, D]` |
//! | `BIND` | 6 | `[1, P1, P0, A, B, C, D]` | `[1, P1, P0]` |
//! | `SETOPTS` | 7 | `[Option, V3, V2, V1, V0, ...]` | `[1]` |
//! | `GETOPTS` | 8 | `[Option, ...]` | `[1, Option, V3, V2, V1, V0, ...]` |
//! | `SUBSCRIBE` | 24 | `[1, ...]` | `[1, 1, 0, 0, 0, 0, ...]` |
//! | `ACCEPT` | 26 | `[T3, T2, T1, T0]` | `[1, R1, R0]` |
//! | `LISTEN` | 27 | `[B1, B0]` | `[1]` |
//!
//! `P1, P0` is the port number, `B1, B0` the backlog, `T3..T0` the timeout
//! in milliseconds (`16#ffffffff` for `infinity`), and `R1, R0` the async
//! reference of the accept, big-endian; each option value is a big-endian
//! 32-bit integer. `ACCEPT` replies at once and sends `{inet_async, L, R,
//! {ok, S}}` or `{inet_async, L, R, {error, Reason}}` later (see
//! `scheduler/tcp_inet.rs`). The options are those of the listen socket of
//! Thousand Island: `reuseaddr` (0), `nodelay` (10), `active` (22), `mode`
//! (24), `send_timeout` (30), and `send_timeout_close` (35), and those that
//! `prim_inet:accept_opts/3` copies to an accepted socket: `keepalive` (1),
//! `linger` (3), `priority` (8), `tos` (9), `delay_send` (31), `recvtos`
//! (43), `ttl` (46), and `recvttl` (47) (prim_inet.erl:502-518;
//! inet_tcp.erl:249). Each other request, option, family, and protocol
//! returns a runtime error (row "tcp_inet ports" of
//! `docs/erts-differences.md`).

use std::{collections::VecDeque, ffi::c_int, io, os::fd::AsRawFd, time::Duration};

use socket2::{Domain, Protocol, SockAddr, Socket, Type};

use crate::{
    error::{BeamError, error},
    runtime::{host::errno::errno_id, timer::TimerKey},
    term::{ProcessId, Reference},
};

/// The number that `prim_inet` adds to each request code
/// (`ERTS_INET_DRV_CONTROL_MAGIC_NUMBER`, prim_inet.erl:2969).
const MAGIC: u32 = 0x03f1_a300;

/// The largest reply of `GETOPTS` and `SUBSCRIBE` (`INET_MAX_OPT_BUFFER`,
/// inet_drv.c:1046).
const MAX_OPT_BUFFER: usize = 64 * 1024;

/// The reply code of a request that succeeded (`INET_REP_OK`).
pub(in crate::runtime) const OK: u8 = 1;

/// The reply code of a request that failed (`INET_REP_ERROR`).
const ERROR: u8 = 0;

/// The family code of IPv4 (`INET_AF_INET`, inet_int.hrl:31).
const AF_INET: u8 = 1;

/// The state of the socket of a port (`INET_STATE_*`, inet_drv.c).
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
    /// The socket of an accepted connection (`INET_STATE_CONNECTED`).
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
    pub(in crate::runtime) timer: Option<TimerKey>,
}

/// The descriptor of one `tcp_inet` port (`tcp_descriptor`), as
/// `prep_tcp_inet_start` and `inet_start` make it
/// (inet_drv.c:10891-10987, 11726-11784).
#[derive(Debug)]
pub(in crate::runtime) struct TcpInet {
    /// The non-blocking host socket that `OPEN` makes, or `None` before it.
    /// The port owns it; dropping the port closes it.
    socket: Option<Socket>,
    /// The state of the socket.
    state: State,
    /// The option `mode`: 0 for `list` (the start value), 1 for `binary`.
    mode: i32,
    /// The option `active`: 0 for `false` (the start value), 1 for `true`,
    /// and 2 for `once`.
    active: i32,
    /// The option `send_timeout` in milliseconds, -1 for `infinity` (the
    /// start value, `INET_INFINITY`).
    send_timeout: i32,
    /// The option `send_timeout_close`: 0 (the start value) or not 0.
    send_timeout_close: i32,
    /// The option `delay_send` (`TCP_ADDF_DELAY_SEND`): false at the start.
    delay_send: bool,
    /// The next async reference (`op_ref`, inet_drv.c:1549, 10937).
    op_ref: u16,
    /// The callers that wait for a connection, in arrival order.
    pub(in crate::runtime) acceptors: VecDeque<Acceptor>,
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
}

impl TcpInet {
    /// Return the descriptor of a new port: no socket, `list` mode,
    /// passive, and an infinite send timeout.
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// let mut port = TcpInet::new();
    /// assert_eq!(port.control(0x03f1_a301, &[1, 1, 1])?, Control::Reply(vec![1]));
    /// ```
    pub(in crate::runtime) fn new() -> TcpInet {
        TcpInet {
            socket: None,
            state: State::Closed,
            mode: 0,
            active: 0,
            send_timeout: -1,
            send_timeout_close: 0,
            delay_send: false,
            op_ref: 0,
            acceptors: VecDeque::new(),
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
    pub(in crate::runtime) fn copy(&self, socket: Socket) -> TcpInet {
        let _ = socket.set_nonblocking(true);
        TcpInet {
            socket: Some(socket),
            state: State::Connected,
            mode: self.mode,
            send_timeout: self.send_timeout,
            send_timeout_close: self.send_timeout_close,
            ..TcpInet::new()
        }
    }

    /// Return the next async reference, as `NEW_ASYNC_ID` does
    /// (inet_drv.c:1549): 0 first, then 1 more each time, modulo 2^16.
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

    /// Run the request `command` with the bytes `request`, as
    /// `tcp_inet_ctl` and `inet_ctl` do (inet_drv.c:10988-11409,
    /// 11952-12062).
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// assert_eq!(port.control(0x03f1_a305, &[])?, Control::Reply(b"\0ebadf".to_vec()));
    /// ```
    ///
    /// # Errors
    ///
    /// A request, option, family, or protocol that PBeam does not run.
    pub(in crate::runtime) fn control(
        &mut self,
        command: u32,
        request: &[u8],
    ) -> Result<Control, BeamError> {
        let reply = match command.wrapping_sub(MAGIC) {
            1 => self.open(request)?,
            4 => self.peer(),
            5 => self.name(),
            6 => self.bind(request)?,
            7 => self.set_options(request)?,
            8 => self.get_options(request)?,
            24 => return Ok(subscribe(request)),
            26 => return Ok(self.accept(request)),
            27 => self.listen(request),
            other => {
                return Err(error(format!(
                    "unsupported tcp_inet request {other}: see \"tcp_inet ports\" in docs/erts-differences.md"
                )));
            }
        };
        Ok(Control::Reply(reply))
    }

    /// Run `INET_REQ_OPEN` with `[Family, Type, Protocol]`
    /// (inet_drv.c:11961-12000), and make the non-blocking socket as
    /// `inet_ctl_open` does (inet_drv.c:5119-5221).
    fn open(&mut self, request: &[u8]) -> Result<Vec<u8>, BeamError> {
        let &[family, kind, protocol] = request else {
            return Ok(error_reply("einval"));
        };
        match family {
            AF_INET => {}
            2 | 5 => return Err(unsupported("family", family)),
            _ => return Ok(error_reply("eafnosupport")),
        }
        if kind != 1 {
            return Ok(error_reply("einval"));
        }
        let protocol = match protocol {
            0 => None,
            1 => Some(Protocol::TCP),
            4 => return Err(unsupported("protocol", protocol)),
            _ => return Ok(error_reply("eprotonosupport")),
        };
        if self.state != State::Closed {
            return Ok(error_reply("exbadseq"));
        }
        Ok(reply(self.open_socket(protocol), |()| vec![OK]))
    }

    /// Make the non-blocking IPv4 stream socket of `protocol`.
    /// `SET_NONBLOCKING` ignores its result (inet_drv.c:5212).
    fn open_socket(&mut self, protocol: Option<Protocol>) -> io::Result<()> {
        let socket = Socket::new(Domain::IPV4, Type::STREAM, protocol)?;
        let _ = socket.set_nonblocking(true);
        self.socket = Some(socket);
        self.state = State::Open;
        Ok(())
    }

    /// Run `INET_REQ_NAME` (inet_drv.c:11360-11380): the address of the
    /// socket. Without a socket, `getsockname` of `INVALID_SOCKET` fails
    /// with `EBADF`.
    fn name(&self) -> Vec<u8> {
        let Some(socket) = &self.socket else {
            return error_reply("ebadf");
        };
        reply(socket.local_addr(), |address| {
            [&[OK][..], &address_bytes(&address)].concat()
        })
    }

    /// Run `INET_REQ_PEER` (inet_drv.c:11272-11300): the address of the
    /// peer of a connected socket. Each other state gives `enotconn`.
    fn peer(&self) -> Vec<u8> {
        let (State::Connected, Some(socket)) = (self.state, &self.socket) else {
            return error_reply("enotconn");
        };
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
    fn bind(&mut self, request: &[u8]) -> Result<Vec<u8>, BeamError> {
        if request.len() < 2 {
            return Ok(error_reply("einval"));
        }
        if self.state != State::Open {
            return Ok(error_reply("exbadport"));
        }
        let socket = self.socket.as_ref().expect("an open port has a socket");
        let (&tag, address) = request.split_first().expect("a request of 2 bytes");
        match tag {
            AF_INET => {}
            2..=5 => return Err(unsupported("bind address family", tag)),
            _ => return Ok(error_reply("eafnosupport")),
        }
        let &[p1, p0, a, b, c, d, ..] = address else {
            return Ok(error_reply("einval"));
        };
        let port = u16::from_be_bytes([p1, p0]);
        let local = std::net::SocketAddrV4::new([a, b, c, d].into(), port);
        Ok(reply(socket.bind(&local.into()), |()| {
            let port = match port {
                0 => socket
                    .local_addr()
                    .ok()
                    .and_then(|address| address.as_socket_ipv4())
                    .map_or(0, |address| address.port()),
                port => port,
            };
            [&[OK][..], &port.to_be_bytes()].concat()
        }))
    }

    /// Run `INET_REQ_SETOPTS` (inet_set_opts, inet_drv.c:6871-7530): set
    /// each option of 5 bytes in order, and ignore a rest of fewer bytes.
    /// `linger` takes 4 more bytes, the time in seconds, and a missing time
    /// gives `einval`. A failed `setsockopt` of `reuseaddr`, `keepalive`,
    /// `linger`, and `nodelay` does not fail the request (`propagate` is
    /// 0), also without a socket. A failed `setsockopt` of `priority`,
    /// `tos`, `ttl`, `recvtos`, and `recvttl` gives `einval` (`propagate`
    /// is 1, inet_drv.c:7519-7530), and the options before it stay set.
    /// `send_timeout`, `send_timeout_close`, and `delay_send` change only a
    /// stream socket, so they change nothing before `OPEN`
    /// (inet_drv.c:7099-7131). Without `SO_PRIORITY`, as on macOS,
    /// `priority` changes nothing (inet_drv.c:7389-7398).
    fn set_options(&mut self, mut request: &[u8]) -> Result<Vec<u8>, BeamError> {
        while let [option, v3, v2, v1, v0, rest @ ..] = request {
            let value = i32::from_be_bytes([*v3, *v2, *v1, *v0]);
            request = rest;
            let set = match option {
                24 => {
                    self.mode = value;
                    true
                }
                22 if value == 3 => return Err(error(ACTIVE_N)),
                22 if value != 0 && self.state == State::Connected => {
                    return Err(error(ACTIVE_CONNECTED));
                }
                22 => {
                    self.active = value;
                    true
                }
                0 => self.set_socket(0, false, |socket| socket.set_reuse_address(value != 0)),
                1 => self.set_socket(1, false, |socket| socket.set_keepalive(value != 0)),
                10 => self.set_socket(10, false, |socket| socket.set_tcp_nodelay(value != 0)),
                3 => {
                    let [s3, s2, s1, s0, rest @ ..] = request else {
                        return Ok(error_reply("einval"));
                    };
                    request = rest;
                    let linger = linger_time(value, u32::from_be_bytes([*s3, *s2, *s1, *s0]))?;
                    self.set_socket(3, false, |socket| socket.set_linger(linger))
                }
                8 => self.set_priority(value),
                9 => self.set_socket(9, true, |socket| socket.set_tos_v4(value as u32)),
                46 => self.set_socket(46, true, |socket| socket.set_ttl_v4(value as u32)),
                43 => self.set_socket(43, true, |socket| socket.set_recv_tos_v4(value != 0)),
                47 if value != 0 => return Err(error(RECVTTL_ON)),
                // PBeam never sets `IP_RECVTTL`, so a socket has it off.
                47 => self.socket.is_some(),
                30 | 35 | 31 if self.socket.is_none() => true,
                30 => {
                    self.send_timeout = value;
                    true
                }
                35 => {
                    self.send_timeout_close = value;
                    true
                }
                31 => {
                    self.delay_send = value != 0;
                    true
                }
                &other => return Err(unsupported("option", other)),
            };
            if !set {
                return Ok(error_reply("einval"));
            }
        }
        Ok(vec![OK])
    }

    /// Run `set` on the socket of the option `option` (see
    /// [`set_option`]). Return `false` when `propagate` is true and the
    /// socket is missing or `set` fails, else `true`.
    fn set_socket(
        &self,
        option: u8,
        propagate: bool,
        set: impl FnOnce(&Socket) -> io::Result<()>,
    ) -> bool {
        let result = self
            .socket
            .as_ref()
            .map(|socket| set_option(socket, option, propagate, set));
        !propagate || matches!(result, Some(Ok(())))
    }

    /// Run `INET_REQ_GETOPTS` (inet_fill_opts, inet_drv.c:9071-9754): give
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
                    .map(|on| getsockopt_value(on, bit))
            };
            let integer = |get: fn(&Socket) -> io::Result<u32>| {
                socket
                    .and_then(|socket| get(socket).ok())
                    .map(|value| value as i32)
            };
            let value = match option {
                24 => Some(self.mode),
                22 => Some(self.active),
                30 => socket.map(|_| self.send_timeout),
                35 => socket.map(|_| self.send_timeout_close),
                31 => socket.map(|_| i32::from(self.delay_send)),
                0 => flag(Socket::reuse_address, APPLE_SO_REUSEADDR),
                1 => flag(Socket::keepalive, APPLE_SO_KEEPALIVE),
                10 => flag(Socket::tcp_nodelay, APPLE_TF_NODELAY),
                43 => flag(Socket::recv_tos_v4, 1),
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
                        reply.extend((seconds as u32).to_be_bytes());
                    }
                    continue;
                }
                other => return Err(unsupported("option", other)),
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
const ACTIVE_N: &str = "unsupported tcp_inet option active with a count: see \"tcp_inet ports\" in docs/erts-differences.md";

/// The runtime error of an active mode on a connected socket, whose data
/// delivery PBeam does not run.
const ACTIVE_CONNECTED: &str = "unsupported tcp_inet option active on a connected socket: see \"tcp_inet ports\" in docs/erts-differences.md";

/// The runtime error of `{recvttl, true}`: `socket2` has no `IP_RECVTTL`
/// call.
const RECVTTL_ON: &str =
    "unsupported tcp_inet option recvttl true: see \"tcp_inet ports\" in docs/erts-differences.md";

/// The runtime error of a `linger` time that `socket2` cannot set as ERTS
/// does: a time with `linger` off, which `SO_LINGER` keeps, and on macOS
/// each time, where `socket2` sets `SO_LINGER_SEC` and ERTS sets
/// `SO_LINGER`, which xnu reads in clock ticks.
const LINGER_TIME: &str = "unsupported tcp_inet option linger with a time: see \"tcp_inet ports\" in docs/erts-differences.md";

/// The option code of `priority` (`INET_OPT_PRIORITY`).
#[cfg(target_os = "linux")]
const PRIORITY: u8 = 8;

/// The option code of `tos` (`INET_OPT_TOS`).
#[cfg(target_os = "linux")]
const TOS: u8 = 9;

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

/// Set an option with `set`, as `inet_setopt` does without `SO_PRIORITY`
/// (inet_drv.c:6833-6860).
///
/// # Examples
///
/// ```rust,ignore
/// set_option(&socket, 1, false, |socket| socket.set_keepalive(true))?;
/// ```
///
/// # Errors
///
/// The error of `set`.
#[cfg(not(target_os = "linux"))]
fn set_option(
    socket: &Socket,
    _option: u8,
    _propagate: bool,
    set: impl FnOnce(&Socket) -> io::Result<()>,
) -> io::Result<()> {
    set(socket)
}

/// Set an option with `set`, as `setopt_prio_tos_trick` does
/// (inet_drv.c:6769-6830): read `priority` and `tos` first, and set them
/// again after the option, because Linux changes `priority` when `tos`
/// changes. A `tos` option keeps its new value, and a `priority` option
/// sets nothing again. With `propagate`, a failed set of `tos` again is
/// the result, and a failed set of `priority` again is the result unless
/// it fails with `EPERM`.
///
/// # Examples
///
/// ```rust,ignore
/// set_option(&socket, 9, true, |socket| socket.set_tos_v4(16))?;
/// ```
///
/// # Errors
///
/// The error of `set`, and with `propagate` the error of a set again.
#[cfg(target_os = "linux")]
fn set_option(
    socket: &Socket,
    option: u8,
    propagate: bool,
    set: impl FnOnce(&Socket) -> io::Result<()>,
) -> io::Result<()> {
    let priority = socket.priority();
    let tos = socket.tos_v4();
    set(socket)?;
    if option == PRIORITY {
        return Ok(());
    }
    let tos = tos.ok().filter(|_| option != TOS);
    let restored = tos.map_or(Ok(()), |tos| socket.set_tos_v4(tos));
    if propagate {
        restored?;
    }
    let restored = priority.map_or(Ok(()), |priority| socket.set_priority(priority));
    let ignored = |failure: &io::Error| !propagate || failure.raw_os_error() == Some(libc::EPERM);
    restored.or_else(|failure| {
        if ignored(&failure) {
            Ok(())
        } else {
            Err(failure)
        }
    })
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
        .map(|value| value as i32)
}

impl TcpInet {
    /// Run `SETOPTS` of `priority` with the value `value`: without
    /// `SO_PRIORITY`, as on macOS, it changes nothing
    /// (inet_drv.c:7389-7398).
    #[cfg(not(target_os = "linux"))]
    fn set_priority(&self, _value: i32) -> bool {
        true
    }

    /// Run `SETOPTS` of `priority` with the value `value` (`propagate` is
    /// 1, inet_drv.c:7389-7398).
    #[cfg(target_os = "linux")]
    fn set_priority(&self, value: i32) -> bool {
        self.set_socket(PRIORITY, true, |socket| socket.set_priority(value as u32))
    }
}

/// Run `INET_REQ_SUBSCRIBE` (inet_subscribe, inet_drv.c:10819-10848; the
/// size check, inet_drv.c:11025-11044): give each `subs_empty_out_q` (1)
/// with the size of the output queue, which is 0 for a port without
/// output. A code that is not 1 gives `badarg`, and a reply of more than
/// 64 KiB is empty.
fn subscribe(request: &[u8]) -> Control {
    if 1 + request.len() * 5 > MAX_OPT_BUFFER {
        return Control::Reply(Vec::new());
    }
    let mut reply = vec![OK];
    for &code in request {
        if code != 1 {
            return Control::Badarg;
        }
        reply.extend([code, 0, 0, 0, 0]);
    }
    Control::Reply(reply)
}

/// Return `[Family, P1, P0, A, B, C, D]` of an IPv4 address, as
/// `inet_get_address` gives it (inet_drv.c).
fn address_bytes(address: &SockAddr) -> Vec<u8> {
    let address = address.as_socket_ipv4().expect("an IPv4 socket");
    [
        &[AF_INET][..],
        &address.port().to_be_bytes(),
        &address.ip().octets(),
    ]
    .concat()
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

/// Return the runtime error of a `what` code that PBeam does not run.
fn unsupported(what: &str, code: u8) -> BeamError {
    error(format!(
        "unsupported tcp_inet {what} {code}: see \"tcp_inet ports\" in docs/erts-differences.md"
    ))
}

/// Give the value that `getsockopt` returns for a flag option, which
/// `GETOPTS` puts in its reply as is (inet_drv.c:9727-9750). macOS gives
/// the flag bit `bit` of an option that is on: `so_options & SO_REUSEADDR`
/// (4), `so_options & SO_KEEPALIVE` (8), and `t_flags & TF_NODELAY` (4) in
/// xnu `sogetoptlock` and `tcp_ctloutput`, and 1 for `IP_RECVTOS`
/// (`OPTBIT`). Linux gives 1 (`sk_reuse`, `sock_flag(sk, SOCK_KEEPOPEN)`,
/// and `!!(nonagle & TCP_NAGLE_OFF)`). `socket2` gives only a boolean, so
/// this function maps it back.
///
/// # Examples
///
/// ```rust,ignore
/// assert_eq!(getsockopt_value(false, 8), 0);
/// assert_eq!(getsockopt_value(true, 8), if cfg!(target_vendor = "apple") { 8 } else { 1 });
/// ```
fn getsockopt_value(on: bool, bit: i32) -> i32 {
    i32::from(on) * if APPLE { bit } else { 1 }
}

/// True on macOS and the other Apple hosts.
const APPLE: bool = cfg!(target_vendor = "apple");

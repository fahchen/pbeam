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
//! requests of `INET_REQ_*` (inet_int.hrl:76-105) for an IPv4 TCP socket
//! of a `tcp_inet` port and an IPv4 UDP socket of a `udp_inet` port:
//!
//! | Request | Code | Bytes | OK reply |
//! | --- | --- | --- | --- |
//! | `OPEN` | 1 | `[Family, Type, Protocol]`, `[1, 1, 1]` or `[1, 2, 2]` | `[1]` |
//! | `NAME` | 5 | `[]` | `[1, 1, P1, P0, A, B, C, D]` |
//! | `BIND` | 6 | `[1, P1, P0, A, B, C, D]` | `[1, P1, P0]` |
//! | `SETOPTS` | 7 | `[Option, V3, V2, V1, V0, ...]` | `[1]` |
//! | `GETOPTS` | 8 | `[Option, ...]` | `[1, Option, V3, V2, V1, V0, ...]` |
//! | `GETHOSTNAME` | 12 | `[]` | `[1 \| Name]` |
//! | `SUBSCRIBE` | 24 | `[1, ...]` | `[1, 1, 0, 0, 0, 0, ...]` |
//! | `LISTEN` | 27 | `[B1, B0]` | `[1]` |
//!
//! A `udp_inet` port runs the requests of `packet_inet_ctl`, which runs
//! `OPEN`, replies `exbadport` to `LISTEN`, and gives each other request
//! to `inet_ctl` (inet_drv.c:14564-14892). It reads no datagram: the
//! socket is not in the idle wait (row "udp_inet ports" of
//! `docs/erts-differences.md`).
//!
//! `P1, P0` is the port number and `B1, B0` the backlog, big-endian; each
//! option value is a big-endian 32-bit integer. The options are those of
//! the listen socket of Thousand Island: `reuseaddr` (0), `nodelay` (10),
//! `active` (22), `mode` (24), `send_timeout` (30), and
//! `send_timeout_close` (35), and `linger` (3) for `GETOPTS`, which
//! `prim_inet:close/1` reads (prim_inet.erl:175-215). Each other request,
//! option, family, and protocol returns a runtime error (row "tcp_inet
//! ports" of `docs/erts-differences.md`).

use std::io;

use socket2::{Domain, Protocol, SockAddr, Socket, Type};

use crate::{
    error::{BeamError, error},
    runtime::host::{errno::errno_id, host_name},
};

/// The number that `prim_inet` adds to each request code
/// (`ERTS_INET_DRV_CONTROL_MAGIC_NUMBER`, prim_inet.erl:2969).
const MAGIC: u32 = 0x03f1_a300;

/// The largest reply of `GETOPTS` and `SUBSCRIBE` (`INET_MAX_OPT_BUFFER`,
/// inet_drv.c:1046).
const MAX_OPT_BUFFER: usize = 64 * 1024;

/// The reply code of a request that succeeded (`INET_REP_OK`).
const OK: u8 = 1;

/// The reply code of a request that failed (`INET_REP_ERROR`).
const ERROR: u8 = 0;

/// The family code of IPv4 (`INET_AF_INET`, inet_int.hrl:31).
const AF_INET: u8 = 1;

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

/// The state of the socket of a port (`INET_STATE_*`, inet_drv.c).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum State {
    /// No socket: the port opened, and `OPEN` did not run.
    Closed,
    /// `OPEN` made the socket.
    Open,
    /// `LISTEN` ran on the socket.
    Listening,
}

/// The descriptor of one `tcp_inet` port (`tcp_descriptor`) or `udp_inet`
/// port (`udp_descriptor`), as `prep_tcp_inet_start`, `packet_inet_start`,
/// and `inet_start` make it (inet_drv.c:10891-10987, 11726-11784,
/// 14496-14520).
#[derive(Debug)]
pub(in crate::runtime) struct Inet {
    /// The socket type of the driver.
    socket_type: SocketType,
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
}

/// The result of a request.
#[derive(Debug, PartialEq, Eq)]
pub(in crate::runtime) enum Control {
    /// The reply bytes.
    Reply(Vec<u8>),
    /// The driver returned -1, so `port_control/3` gives `badarg`
    /// (io.c:4065-4066).
    Badarg,
}

impl Inet {
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
            socket: None,
            state: State::Closed,
            mode: 0,
            active: 0,
            send_timeout: -1,
            send_timeout_close: 0,
        }
    }

    /// Run the request `command` with the bytes `request`, as
    /// `tcp_inet_ctl`, `packet_inet_ctl`, and `inet_ctl` do
    /// (inet_drv.c:10988-11409, 11952-12062, 14564-14892).
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
            5 => self.name(),
            6 => self.bind(request)?,
            7 => self.set_options(request)?,
            8 => self.get_options(request)?,
            12 => get_host_name(request),
            24 => return Ok(subscribe(request)),
            27 if self.socket_type == SocketType::Stream => self.listen(request),
            // `LISTEN` is only for an SCTP socket (inet_drv.c:14846-14858).
            27 => error_reply("exbadport"),
            other => return Err(self.unsupported("request", other)),
        };
        Ok(Control::Reply(reply))
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
        match family {
            AF_INET => {}
            2 | 5 => return Err(self.unsupported("family", family)),
            _ => return Ok(error_reply("eafnosupport")),
        }
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
        Ok(reply(self.open_socket(socket_type, protocol), |()| {
            vec![OK]
        }))
    }

    /// Make the non-blocking IPv4 socket of `socket_type` and `protocol`.
    /// `SET_NONBLOCKING` ignores its result (inet_drv.c:5212).
    fn open_socket(&mut self, socket_type: Type, protocol: Option<Protocol>) -> io::Result<()> {
        let socket = Socket::new(Domain::IPV4, socket_type, protocol)?;
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
            2..=5 => return Err(self.unsupported("bind address family", tag)),
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

    /// Run `INET_REQ_SETOPTS` (inet_set_opts, inet_drv.c:6871-7060): set
    /// each option of 5 bytes in order, and ignore a rest of fewer bytes. A
    /// failed `setsockopt` of `reuseaddr` and `nodelay` does not fail the
    /// request (`propagate` is 0), also without a socket. `send_timeout`
    /// and `send_timeout_close` change only a stream socket, so they change
    /// nothing before `OPEN` (inet_drv.c:7099-7120).
    fn set_options(&mut self, mut request: &[u8]) -> Result<Vec<u8>, BeamError> {
        while let [option, v3, v2, v1, v0, rest @ ..] = request {
            let value = i32::from_be_bytes([*v3, *v2, *v1, *v0]);
            request = rest;
            match option {
                24 => self.mode = value,
                22 if value == 3 => return Err(error(ACTIVE_N)),
                22 => self.active = value,
                0 => self.with_socket(|socket| socket.set_reuse_address(value != 0)),
                10 => self.with_socket(|socket| socket.set_tcp_nodelay(value != 0)),
                30 if self.socket.is_some() => self.send_timeout = value,
                35 if self.socket.is_some() => self.send_timeout_close = value,
                30 | 35 => {}
                &other => return Err(self.unsupported("option", other)),
            }
        }
        Ok(vec![OK])
    }

    /// Run `set` on the socket, and ignore its result and a missing socket.
    fn with_socket(&self, set: impl FnOnce(&Socket) -> io::Result<()>) {
        if let Some(socket) = &self.socket {
            let _ = set(socket);
        }
    }

    /// Run `INET_REQ_GETOPTS` (inet_fill_opts, inet_drv.c:9071-9754): give
    /// each option code with its value in order. A socket option whose
    /// `getsockopt` fails, also without a socket, is not in the reply, and
    /// `send_timeout` and `send_timeout_close` are not in it before `OPEN`.
    /// A reply of more than 64 KiB gives `einval`, also when only the last
    /// 4 bytes of `linger` exceed it.
    fn get_options(&self, request: &[u8]) -> Result<Vec<u8>, BeamError> {
        let mut reply = vec![OK];
        for &option in request {
            if reply.len() + 5 > MAX_OPT_BUFFER {
                return Ok(error_reply("einval"));
            }
            let socket = self.socket.as_ref();
            let value = match option {
                24 => Some(self.mode),
                22 => Some(self.active),
                30 => socket.map(|_| self.send_timeout),
                35 => socket.map(|_| self.send_timeout_close),
                0 => socket
                    .and_then(|socket| socket.reuse_address().ok())
                    .map(getsockopt_value),
                10 => socket
                    .and_then(|socket| socket.tcp_nodelay().ok())
                    .map(getsockopt_value),
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

/// Run `INET_REQ_GETHOSTNAME` (inet_drv.c:11219-11233): the name of the
/// host, as `gethostname` gives it ([`host_name`]). A request with bytes
/// gives `einval`.
fn get_host_name(request: &[u8]) -> Vec<u8> {
    if !request.is_empty() {
        return error_reply("einval");
    }
    [&[OK][..], &host_name()].concat()
}

/// The runtime error of `{active, N}`.
const ACTIVE_N: &str = "unsupported tcp_inet option active with a count: see \"tcp_inet ports\" in docs/erts-differences.md";

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
    let errno = failure
        .raw_os_error()
        .expect("a socket call fails with an errno");
    error_reply(&errno_id(errno))
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

/// Give the value that `getsockopt` returns for `reuseaddr` or `nodelay`,
/// which `GETOPTS` puts in its reply as is (inet_drv.c:9727-9750). macOS
/// gives the flag bit of the option for an option that is on: `so_options &
/// SO_REUSEADDR` and `t_flags & TF_NODELAY` in xnu `sogetoptlock` and
/// `tcp_ctloutput`, both 4. Linux gives 1 (`sk_reuse` and `!!(nonagle &
/// TCP_NAGLE_OFF)`). `socket2` gives only a boolean, so this function maps
/// it back.
///
/// # Examples
///
/// ```rust,ignore
/// assert_eq!(getsockopt_value(false), 0);
/// assert_eq!(getsockopt_value(true), if cfg!(target_vendor = "apple") { 4 } else { 1 });
/// ```
fn getsockopt_value(on: bool) -> i32 {
    const ON: i32 = if cfg!(target_vendor = "apple") { 4 } else { 1 };
    i32::from(on) * ON
}

//! The send and the receive of a `udp_inet` port: the output of
//! `prim_inet:sendto/4` (prim_inet.erl:652-699) and `PACKET_REQ_RECV` of
//! `prim_inet:recvfrom/3` (prim_inet.erl:853-889), as `packet_inet_command`,
//! `packet_inet_ctl`, and `packet_inet_input` run them
//! (inet_drv.c:14968-14998, 15037-15316, 15361-15576).
//!
//! The output is `[Family, P1, P0, A, B, C, D, L3, L2, L1, L0 | Data]`
//! after the header of the caller tag, where `L3..L0` is the size of the
//! ancillary data. A receive reads one datagram into a buffer of `buffer`
//! bytes, so the host drops the bytes of a longer datagram, and gives
//! `{inet_async, S, Ref, {ok, [Family, P1, P0, A, B, C, D | Data]}}`.

use std::{io, net::UdpSocket};

use socket2::SockAddr;

use super::{
    AF_INET, Control, Event, Inet, OK, State, errno_name, errno_reply, error_reply, stream::Sender,
};
use crate::error::{BeamError, error};

/// The address tag `INET_AF_ANY` (inet_drv.c:759).
const ANY: u8 = 3;
/// The address tag `INET_AF_LOOPBACK` (inet_drv.c:760).
const LOOPBACK: u8 = 4;

/// The bytes of a datagram and `[Family, P1, P0, A, B, C, D]` of its
/// sender.
type Datagram = (Vec<u8>, Vec<u8>);

impl Inet {
    /// Run `INET_REQ_CONNECT` of a `udp_inet` socket with `[T3, T2, T1, T0,
    /// 1, P1, P0, A, B, C, D]` (inet_drv.c:14710-14842): a port without a
    /// socket gives `exbadport`, a request of 1 to 5 bytes gives `einval`,
    /// and the address gives the errors of `inet_set_faddress`. The connect
    /// sets the default peer at once, so `PEER` gives the peer and a send
    /// ignores its address. A failed connect gives the errno, and the socket
    /// is not connected.
    ///
    /// # Errors
    ///
    /// The request without an address, which dissolves the association,
    /// and a failed connect of a connected socket: both need a connect to
    /// an `AF_UNSPEC` address (inet_drv.c:14803-14806, 14832-14836), which
    /// safe Rust cannot make. The `any`, `loopback`, `inet6`, and `local`
    /// address tags.
    pub(super) fn datagram_connect(&mut self, request: &[u8]) -> Result<Vec<u8>, BeamError> {
        if self.state == State::Closed {
            return Ok(error_reply("exbadport"));
        }
        let address = match request {
            [] => return Err(self.unsupported("request", "CONNECT without an address")),
            [_, _, _, _, address @ ..] if address.len() >= 2 => address,
            _ => return Ok(error_reply("einval")),
        };
        let remote = match self.address(address, "connect")? {
            Ok(remote) => remote,
            Err(reply) => return Ok(reply),
        };
        let socket = self.socket.as_ref().expect("an open port has a socket");
        match socket.connect(&remote.into()) {
            Ok(()) => {
                self.state = State::Connected;
                Ok(vec![OK])
            }
            Err(_) if self.state == State::Connected => Err(error(
                "unsupported failed udp_inet connect of a connected socket: see \"udp_inet ports\" in docs/erts-differences.md",
            )),
            Err(failure) => Ok(errno_reply(&failure)),
        }
    }

    /// Check `PACKET_REQ_RECV` with `[T3, T2, T1, T0, N3, N2, N1, N0]`
    /// (inet_drv.c:14981-14991): a port without a socket gives
    /// `exbadport`, an active socket and a request that is not 8 bytes give
    /// `einval`, and a receive that waits gives `ealready`. The length is
    /// not used, as a datagram is one message (inet_drv.c:14987-14988).
    pub(super) fn datagram_receive_request(&self, request: &[u8]) -> Control {
        let reply = match (self.state, self.active, request) {
            (State::Closed, ..) => error_reply("exbadport"),
            (_, 0, &[t3, t2, t1, t0, _, _, _, _]) if self.stream.receiver.is_none() => {
                return Control::Receive {
                    timeout: u32::from_be_bytes([t3, t2, t1, t0]),
                    length: 0,
                };
            }
            (_, 0, &[_, _, _, _, _, _, _, _]) => error_reply("ealready"),
            _ => error_reply("einval"),
        };
        Control::Reply(reply)
    }

    /// Read one datagram for the receive that waits, as `packet_inet_input`
    /// does in passive mode (inet_drv.c:15445-15575): the datagram, with
    /// the address of the sender and at most `buffer` bytes, or the error
    /// of the read. No message means that the receive waits. Without a
    /// receive, as after a timeout that came before the poll event, the
    /// port reads nothing.
    pub(in crate::runtime) fn receive_datagram(&mut self) -> Vec<Event> {
        if self.stream.receiver.is_none() {
            return Vec::new();
        }
        let (data, address) = match self.read_datagram() {
            Ok(Some(read)) => read,
            Ok(None) => return Vec::new(),
            Err(name) => return self.end_receive(Err(name)),
        };
        let receiver = self.stream.receiver.take().expect("the receive waits");
        vec![Event::Datagram {
            receiver,
            address,
            data,
        }]
    }

    /// Read at most `read_packets` datagrams of an active socket for the
    /// owner, as `packet_inet_input` does (inet_drv.c:15361-15588): each
    /// datagram gives `{udp, S, Address, Port, Data}`, and then `once`
    /// becomes passive, and the last datagram of `{active, N}` makes the
    /// socket passive and gives `{udp_passive, S}`
    /// (`packet_reply_binary_data`, inet_drv.c:4335-4355). A passive
    /// socket reads no more. A read error gives `{udp_error, S, Reason}`
    /// and ends the step; no datagram ends it too.
    pub(in crate::runtime) fn receive_active_datagrams(&mut self) -> Vec<Event> {
        let mut events = Vec::new();
        for _ in 0..self.read_packets {
            match self.read_datagram() {
                Ok(Some((data, address))) => events.push(Event::ActiveDatagram { address, data }),
                Ok(None) => break,
                Err(name) => {
                    events.push(Event::DatagramError(name));
                    break;
                }
            }
            match self.active {
                2 => self.active = 0,
                3 => {
                    self.active_count -= 1;
                    if self.active_count == 0 {
                        self.active = 0;
                        events.push(Event::Passive);
                    }
                }
                _ => {}
            }
            if self.active == 0 {
                break;
            }
        }
        events
    }

    /// Read one datagram of at most `buffer` bytes, and count it: the bytes
    /// and the address of the sender, `None` for `ERRNO_BLOCK`, or the
    /// error name of the read (inet_drv.c:15469-15528).
    fn read_datagram(&mut self) -> Result<Option<Datagram>, String> {
        // The host socket moves into a `UdpSocket` for the read into an
        // initialized buffer, and back; the fd does not change.
        let socket = UdpSocket::from(self.socket.take().expect("an open port has a socket"));
        let mut data = vec![0; self.stream.buffer];
        let read = socket.recv_from(&mut data);
        self.socket = Some(socket.into());
        let (count, from) = match read {
            Ok(read) => read,
            Err(failure) if failure.kind() == io::ErrorKind::WouldBlock => return Ok(None),
            Err(failure) => return Err(errno_name(&failure)),
        };
        self.stream.statistics.input(count as u64);
        data.truncate(count);
        Ok(Some((data, super::address_bytes(&SockAddr::from(from)))))
    }

    /// Send the datagram of the output `bytes` of `sender`, after the tag,
    /// as `packet_inet_command` does for UDP (inet_drv.c:15068-15183,
    /// 15240-15316): a port without a socket, a short address or size, and
    /// a size larger than the rest give `einval`, and a family code that is
    /// not 1 to 5 gives `eafnosupport` (`inet_set_faddress`,
    /// inet_drv.c:4795-4880). The tags `any` and `loopback` take only the
    /// port, for the address 0.0.0.0 or 127.0.0.1 (inet_drv.c:4817-4844).
    /// The send counts the data before `sendto`, and a failed `sendto`
    /// gives its errno. A connected socket ignores the address and sends
    /// to its peer (inet_drv.c:15142-15182).
    ///
    /// # Errors
    ///
    /// A family other than IPv4, the codes 2 and 5, and ancillary data.
    pub(super) fn send_datagram(
        &mut self,
        bytes: &[u8],
        sender: Sender,
    ) -> Result<Vec<Event>, BeamError> {
        let reply = |result: Result<(), &str>| {
            let result = result.map_err(str::to_owned);
            Ok(vec![Event::Replied { sender, result }])
        };
        let Some(socket) = self.socket.as_ref() else {
            return reply(Err("einval"));
        };
        let (ip, p1, p0, rest) = match bytes {
            [] => return reply(Err("einval")),
            [AF_INET, p1, p0, a, b, c, d, rest @ ..] => ([*a, *b, *c, *d], p1, p0, rest),
            [ANY, p1, p0, rest @ ..] => ([0, 0, 0, 0], p1, p0, rest),
            [LOOPBACK, p1, p0, rest @ ..] => ([127, 0, 0, 1], p1, p0, rest),
            [AF_INET | ANY | LOOPBACK, ..] => return reply(Err("einval")),
            [family @ (2 | 5), ..] => return Err(self.unsupported("output address family", family)),
            _ => return reply(Err("eafnosupport")),
        };
        let [l3, l2, l1, l0, data @ ..] = rest else {
            return reply(Err("einval"));
        };
        match u32::from_be_bytes([*l3, *l2, *l1, *l0]) {
            0 => {}
            size if size as usize > data.len() => return reply(Err("einval")),
            _ => return Err(self.unsupported("output", "with ancillary data")),
        }
        self.stream.statistics.output(data.len() as u64);
        let to = std::net::SocketAddrV4::new(ip.into(), u16::from_be_bytes([*p1, *p0]));
        let sent = if self.state == State::Connected {
            socket.send(data)
        } else {
            socket.send_to(data, &to.into())
        };
        match sent {
            Ok(_) => reply(Ok(())),
            Err(failure) => reply(Err(&errno_name(&failure))),
        }
    }
}

#[cfg(test)]
#[path = "../../../../tests/runtime/port/inet/datagram.rs"]
mod tests;

//! The send and the receive of a passive `udp_inet` port: the output of
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

use super::{AF_INET, Control, Event, Inet, State, errno_name, error_reply, stream::Sender};
use crate::error::BeamError;

impl Inet {
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
        // The host socket moves into a `UdpSocket` for the read into an
        // initialized buffer, and back; the fd does not change.
        let socket = UdpSocket::from(self.socket.take().expect("an open port has a socket"));
        let mut data = vec![0; self.stream.buffer];
        let read = socket.recv_from(&mut data);
        self.socket = Some(socket.into());
        let (count, from) = match read {
            Ok(read) => read,
            Err(failure) if failure.kind() == io::ErrorKind::WouldBlock => return Vec::new(),
            Err(failure) => return self.end_receive(Err(errno_name(&failure))),
        };
        self.stream.statistics.input(count as u64);
        data.truncate(count);
        let receiver = self.stream.receiver.take().expect("the receive waits");
        vec![Event::Datagram {
            receiver,
            address: super::address_bytes(&SockAddr::from(from)),
            data,
        }]
    }

    /// Send the datagram of the output `bytes` of `sender`, after the tag,
    /// as `packet_inet_command` does for UDP (inet_drv.c:15068-15183,
    /// 15240-15316): a port without a socket, a short address or size, and
    /// a size larger than the rest give `einval`, and a family code that is
    /// not 1 to 5 gives `eafnosupport` (`inet_set_faddress`,
    /// inet_drv.c:4795-4880). The send counts the data before `sendto`,
    /// and a failed `sendto` gives its errno.
    ///
    /// # Errors
    ///
    /// A family other than IPv4, the codes 2 to 5, and ancillary data.
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
        match bytes.first() {
            None => return reply(Err("einval")),
            Some(&AF_INET) => {}
            Some(&family @ 2..=5) => return Err(self.unsupported("output address family", family)),
            Some(_) => return reply(Err("eafnosupport")),
        }
        let [_, p1, p0, a, b, c, d, rest @ ..] = bytes else {
            return reply(Err("einval"));
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
        let to =
            std::net::SocketAddrV4::new([*a, *b, *c, *d].into(), u16::from_be_bytes([*p1, *p0]));
        match socket.send_to(data, &to.into()) {
            Ok(_) => reply(Ok(())),
            Err(failure) => reply(Err(&errno_name(&failure))),
        }
    }
}

#[cfg(test)]
#[path = "../../../../tests/runtime/port/inet/datagram.rs"]
mod tests;

//! The receive and send state of a connected socket of a `tcp_inet` port
//! in passive and active mode with `{packet, raw}`, as `tcp_descriptor` keeps it
//! (inet_drv.c:1308, 1050-1064, 12268-12325, 12657-12731, 12783-12890,
//! 12967-13240, 13613-13852, 14234-14370).
//!
//! `TCP_REQ_RECV` (42) takes `[T3, T2, T1, T0, N3, N2, N1, N0]`: the
//! timeout in milliseconds (`16#ffffffff` for `infinity`) and the length,
//! big-endian. It replies `[1, R1, R0]` with an async reference, and the
//! caller later gets `{inet_async, S, Ref, {ok, Data}}` or `{inet_async, S,
//! Ref, {error, Reason}}` (prim_inet.erl:808-824). A length of 0 gives the
//! bytes of one read, at most the buffer size (9216 bytes,
//! `INET_BUFFER_TCP_DEFAULT`); a length N gives N bytes.
//!
//! The output of the port is `<<Size:16, Tag:Size/binary, Data/binary>>`,
//! where `Tag` is `term_to_binary(Mref, [local])` (prim_inet.erl:575-585;
//! `init_caller_iov`, inet_drv.c:1827-1850). The port writes the data to
//! the socket without a wait and queues the bytes that the socket does not
//! take. The caller gets `{inet_reply, S, ok, Mref}` or `{inet_reply, S,
//! {error, Reason}, Mref}`. The queue is written when the socket is ready
//! for write. A send while the queue holds 8192 bytes or more
//! (`INET_HIGH_WATERMARK`) waits for its reply until the queue holds 4096
//! bytes or fewer (`INET_LOW_WATERMARK`).
//!
//! In active mode (`true` or `once`), each read of at most the buffer
//! size gives `{tcp, S, Data}` to the owner, and `once` becomes `false`
//! after one message. The end of the connection gives `{tcp_error, S,
//! Reason}` for a failed read, then `{tcp_closed, S}`, and the port ends
//! (inet_drv.c:3987-4095, 12783-12890, 13042-13128, 13613-13662).

use std::{
    collections::VecDeque,
    ffi::c_int,
    io::{self, Read},
    os::fd::AsRawFd,
};

use super::{Control, Inet, MAX_OPT_BUFFER, OK, SocketType, State, errno_name, error_reply};
use crate::{
    error::BeamError,
    runtime::timer::PortTimer,
    term::{ProcessId, Reference, Term, external},
};

/// The buffer size of a receive of length 0 (`INET_BUFFER_TCP_DEFAULT`,
/// inet_drv.c:1050).
pub(super) const BUFFER: usize = 9216;

/// The largest receive length (`TCP_MAX_PACKET_SIZE`, inet_drv.c:1308).
const MAX_PACKET: i32 = 0x400_0000;

/// The queue size that makes a send wait (`INET_HIGH_WATERMARK`,
/// inet_drv.c:1062).
const HIGH: usize = 8 * 1024;

/// The queue size that ends the wait of a send (`INET_LOW_WATERMARK`,
/// inet_drv.c:1063).
const LOW: usize = 4 * 1024;

/// The caller of the receive that waits (`enq_async`, one at a time,
/// `INET_MAX_ASYNC`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(in crate::runtime) struct Receiver {
    /// The async reference that the reply and the message give.
    pub(in crate::runtime) id: u16,
    /// The process that gets the `inet_async` message.
    pub(in crate::runtime) caller: ProcessId,
    /// The timer of the receive, or `None` without one. The timer service
    /// owns it; the end of the receive cancels it.
    pub(in crate::runtime) timer: Option<PortTimer>,
}

/// The caller of a send and the monitor reference of its header, which
/// the `inet_reply` message gives (`caller` and `caller_ref`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(in crate::runtime) struct Sender {
    /// The process that gets the `inet_reply` message.
    pub(in crate::runtime) caller: ProcessId,
    /// The reference of the header.
    pub(in crate::runtime) tag: Reference,
    /// The send timer of the busy port, or `None` without one. The timer
    /// service owns it; the end of the send cancels it.
    pub(in crate::runtime) timer: Option<PortTimer>,
}

/// A send that arrived while the port was busy: its caller and the bytes
/// of its command, which runs when the port unbusies.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(in crate::runtime) struct Waiter {
    /// The caller of the send.
    pub(in crate::runtime) sender: Sender,
    /// The bytes of the send, without the header.
    pub(in crate::runtime) data: Vec<u8>,
}

/// A message that a step of the driver sends, or the end of the port.
#[derive(Debug, PartialEq, Eq)]
pub(in crate::runtime) enum Event {
    /// `{tcp, S, Data}` to the owner in active mode (`tcp_message`,
    /// inet_drv.c:3987-4015).
    Data(Vec<u8>),
    /// `{tcp_closed, S}` to the owner in active mode, once for the port
    /// (`tcp_closed_message`, inet_drv.c:4058-4074).
    Closed,
    /// `{tcp_error, S, Reason}` to the owner in active mode
    /// (`tcp_error_message`, inet_drv.c:4079-4095).
    Error(String),
    /// `{empty_out_q, S}` to each of these processes, the subscribers of
    /// `SUBSCRIBE`, when the queue becomes empty (`send_empty_out_q_msgs`,
    /// inet_drv.c:10795-10813).
    EmptyQueue(Vec<ProcessId>),
    /// The port ends with this reason, `normal` or an error name, as
    /// `driver_exit` does (io.c:7228-7249). It is the last event.
    Exit(String),
    /// `{inet_async, S, Ref, {ok, Data}}` or `{inet_async, S, Ref, {error,
    /// Reason}}` of the receive of `receiver`.
    Received {
        /// The receive that ends.
        receiver: Receiver,
        /// The bytes, or the error name.
        result: Result<Vec<u8>, String>,
    },
    /// `{inet_async, S, Ref, {ok, [Family, P1, P0, A, B, C, D | Data]}}`
    /// of the receive of `receiver` on a `udp_inet` port
    /// (`packet_binary_message`, inet_drv.c:4106-4160).
    Datagram {
        /// The receive that ends.
        receiver: Receiver,
        /// `[Family, P1, P0, A, B, C, D]` of the sender.
        address: Vec<u8>,
        /// The bytes of the datagram.
        data: Vec<u8>,
    },
    /// `{udp, S, Address, Port, Data}` to the owner of an active
    /// `udp_inet` port (`packet_binary_message`, inet_drv.c:4106-4160).
    ActiveDatagram {
        /// `[Family, P1, P0, A, B, C, D]` of the sender.
        address: Vec<u8>,
        /// The bytes of the datagram.
        data: Vec<u8>,
    },
    /// `{udp_passive, S}` to the owner at the end of `{active, N}`
    /// (`packet_passive_message`, inet_drv.c:4215-4239).
    Passive,
    /// `{udp_error, S, Reason}` to the owner of an active `udp_inet` port
    /// (`packet_error_message`, inet_drv.c:4245-4267).
    DatagramError(String),
    /// `{inet_async, S, Ref, ok}` or `{inet_async, S, Ref, {error,
    /// Reason}}` of the connect of `connector` (`async_ok` and
    /// `async_error`, inet_drv.c:2563-2572).
    Connected {
        /// The connect that ends.
        connector: Receiver,
        /// `Ok` or the error name.
        result: Result<(), String>,
    },
    /// `{inet_reply, S, ok, Tag}` or `{inet_reply, S, {error, Reason},
    /// Tag}` of the send of `sender`.
    Replied {
        /// The send that ends.
        sender: Sender,
        /// `Ok` or the error name.
        result: Result<(), String>,
    },
}

/// The result of the output of a port.
#[derive(Debug, PartialEq, Eq)]
pub(in crate::runtime) enum Output {
    /// The messages of the send.
    Sent(Vec<Event>),
    /// The header is too short, so the port fails with `einval`
    /// (`driver_failure_posix`, inet_drv.c:12665-12669).
    Failed,
}

/// The counts of `INET_REQ_GETSTAT` (`inet_input_count` and
/// `inet_output_count`, inet_drv.c:11560-11610).
#[derive(Debug, Default)]
pub(super) struct Statistics {
    /// The delivered packets (`recv_cnt`).
    receive_count: u64,
    /// The largest delivered packet (`recv_max`).
    receive_max: u64,
    /// The average packet length (`recv_avg`).
    receive_average: f64,
    /// The average deviation from the average length (`recv_dvi`).
    receive_deviation: f64,
    /// The delivered bytes (`recv_oct`).
    receive_bytes: u64,
    /// The sends (`send_cnt`).
    send_count: u64,
    /// The largest send (`send_max`).
    send_max: u64,
    /// The average send length (`send_avg`).
    send_average: f64,
    /// The sent bytes (`send_oct`).
    send_bytes: u64,
}

impl Statistics {
    /// Count a delivered packet of `length` bytes (`inet_input_count`).
    pub(super) fn input(&mut self, length: u64) {
        let count = self.receive_count.wrapping_add(1).max(1);
        self.receive_bytes = self.receive_bytes.wrapping_add(length);
        let average = self.receive_average + (length as f64 - self.receive_average) / count as f64;
        self.receive_average = average;
        self.receive_max = self.receive_max.max(length);
        let deviation = self.receive_deviation;
        self.receive_deviation = deviation + ((length as f64 - average) - deviation) / count as f64;
        self.receive_count = count;
    }

    /// Count a send of `length` bytes (`inet_output_count`).
    pub(super) fn output(&mut self, length: u64) {
        let count = self.send_count.wrapping_add(1).max(1);
        self.send_bytes = self.send_bytes.wrapping_add(length);
        self.send_average += (length as f64 - self.send_average) / count as f64;
        self.send_max = self.send_max.max(length);
        self.send_count = count;
    }
}

/// The receive and send state of a connected socket.
#[derive(Debug)]
pub(in crate::runtime) struct Stream {
    /// The size of the buffer of a receive of length 0 (`bufsz`).
    pub(super) buffer: usize,
    /// The input buffer (`i_buf`), zeroed once when the receive allocates
    /// it, empty without a buffer. The reads write into it after `filled`.
    input: Vec<u8>,
    /// The bytes at the start of `input` that the socket gave and no
    /// receive took yet (`i_ptr_start` to `i_ptr`).
    filled: usize,
    /// The size of the input buffer, 0 without a buffer (`i_bufsz`).
    input_size: usize,
    /// The bytes that the receive of a length still needs (`i_remain`).
    remain: usize,
    /// The receive that waits for data.
    pub(in crate::runtime) receiver: Option<Receiver>,
    /// The connect that waits for the write select of the socket.
    pub(in crate::runtime) connector: Option<Receiver>,
    /// The bytes that the socket did not take yet (the queue of the port).
    output: VecDeque<u8>,
    /// The send that waits until the queue is small (`INET_F_BUSY`).
    pub(in crate::runtime) busy: Option<Sender>,
    /// The sends that arrived while the port was busy, in arrival order.
    /// ERTS suspends their callers and runs their commands only when the
    /// port unbusies (`set_busy_port`, inet_drv.c:13783-13784), so their
    /// bytes stay out of the queue until then.
    pub(in crate::runtime) waiters: VecDeque<Waiter>,
    /// The callers of `SUBSCRIBE` while the queue held bytes, in call
    /// order, which get `{empty_out_q, S}` once when the queue becomes
    /// empty (`empty_out_q_subs`, inet_drv.c:10819-10848).
    pub(super) subscribers: Vec<ProcessId>,
    /// `shutdown/2` how of the `TCP_REQ_SHUTDOWN` that waits for this queue
    /// to empty (`TCP_ADDF_PENDING_SHUT_WR` and
    /// `TCP_ADDF_PENDING_SHUT_RDWR`).
    pub(super) pending_shutdown: Option<u8>,
    /// The next send gives `{error, closed}` (`TCP_ADDF_DELAYED_CLOSE_SEND`).
    closed_send: bool,
    /// The next receive gives `{error, closed}`
    /// (`TCP_ADDF_DELAYED_CLOSE_RECV`).
    closed_receive: bool,
    /// The port sent `{tcp_closed, S}` (`TCP_ADDF_CLOSE_SENT`).
    closed_sent: bool,
    /// The connection ended and the socket closed: `INET_STATE_CLOSED`
    /// with the `stype` `SOCK_STREAM` of `OPEN`, which a port without
    /// `OPEN` does not have (inet_drv.c:5215, 10925).
    pub(super) ended: bool,
    /// The counts of `GETSTAT`.
    pub(super) statistics: Statistics,
}

impl Stream {
    /// Return the state of a new socket with the buffer size `buffer`.
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// let stream = Stream::new(BUFFER);
    /// ```
    pub(super) fn new(buffer: usize) -> Self {
        Stream {
            buffer,
            input: Vec::new(),
            filled: 0,
            input_size: 0,
            remain: 0,
            receiver: None,
            connector: None,
            output: VecDeque::new(),
            busy: None,
            waiters: VecDeque::new(),
            subscribers: Vec::new(),
            pending_shutdown: None,
            closed_send: false,
            closed_receive: false,
            closed_sent: false,
            ended: false,
            statistics: Statistics::default(),
        }
    }

    /// Return the size of the queue in bytes (`driver_sizeq`).
    pub(super) fn output_size(&self) -> usize {
        self.output.len()
    }

    /// Make the input buffer hold at least `length` bytes, as
    /// `tcp_expand_buffer` does (inet_drv.c:11631-11662). The bytes start at
    /// the start of the buffer, so the used space before them is 0. A
    /// larger allocation keeps the filled bytes and zeroes only the new
    /// bytes.
    fn expand(&mut self, length: usize) {
        if self.input.len() < length {
            self.input.reserve_exact(length - self.input.len());
            self.input.resize(length, 0);
        }
        self.input_size = self.input_size.max(length);
    }
}

impl Inet {
    /// Check `TCP_REQ_RECV` with `[T3, T2, T1, T0, N3, N2, N1, N0]`
    /// (inet_drv.c:12268-12307): a socket that is not connected gives
    /// `closed` once after a failed send, else `enotconn`; an active socket
    /// and a request that is not 8 bytes give `einval`; a length above 64
    /// MiB gives `enomem`; and a receive that waits gives `ealready`.
    ///
    /// # Errors
    ///
    /// A negative length, which `prim_inet` does not send.
    pub(super) fn receive_request(&mut self, request: &[u8]) -> Result<Control, BeamError> {
        if self.state != State::Connected {
            if std::mem::take(&mut self.stream.closed_receive) {
                self.stream.closed_send = false;
                return Ok(Control::Reply(error_reply("closed")));
            }
            return Ok(Control::Reply(error_reply("enotconn")));
        }
        let (0, &[t3, t2, t1, t0, n3, n2, n1, n0]) = (self.active, request) else {
            return Ok(Control::Reply(error_reply("einval")));
        };
        let length = i32::from_be_bytes([n3, n2, n1, n0]);
        let reply = match length {
            ..0 => return Err(self.unsupported("receive length", length)),
            _ if length > MAX_PACKET => error_reply("enomem"),
            _ if self.stream.receiver.is_some() => error_reply("ealready"),
            _ => {
                return Ok(Control::Receive {
                    timeout: u32::from_be_bytes([t3, t2, t1, t0]),
                    length: length as usize,
                });
            }
        };
        Ok(Control::Reply(reply))
    }

    /// Read for the receive of `length` bytes, 0 for the bytes of one read,
    /// as `tcp_recv` does (inet_drv.c:13130-13240), and return the messages
    /// of the step: the data when the receive has its bytes, or the end of
    /// the receive when the peer closed or the read failed. No message
    /// means that the receive waits.
    pub(in crate::runtime) fn receive(&mut self, length: usize) -> Vec<Event> {
        if self.socket_type == SocketType::Datagram {
            return self.receive_datagram();
        }
        let stream = &mut self.stream;
        let read = if stream.input_size == 0 {
            let size = if length > 0 { length } else { stream.buffer };
            stream.input = vec![0; size];
            stream.input_size = size;
            stream.remain = length;
            size
        } else if length > 0 {
            if stream.filled >= length {
                return self.deliver(length);
            }
            stream.expand(length);
            stream.remain = length - stream.filled;
            stream.remain
        } else if stream.remain == 0 {
            if stream.filled > 0 {
                let length = stream.filled;
                return self.deliver(length);
            }
            stream.input_size - stream.filled
        } else {
            stream.remain
        };
        let mut socket = self.socket.as_ref().expect("a connected port has a socket");
        let start = stream.filled;
        let count = match socket.read(&mut stream.input[start..start + read]) {
            Ok(count) if count > 0 => count,
            Err(failure) if failure.kind() == io::ErrorKind::WouldBlock => return Vec::new(),
            result => return self.receive_ended(result.err()),
        };
        let stream = &mut self.stream;
        stream.filled += count;
        if stream.remain > 0 {
            stream.remain -= count;
            if stream.remain > 0 {
                return Vec::new();
            }
        }
        self.deliver(self.stream.filled)
    }

    /// End the receive that waits with `timeout`, as
    /// `tcp_inet_recv_timeout` does (inet_drv.c:11941-11949): the bytes
    /// that it read stay in the buffer for the next receive. A `udp_inet`
    /// socket turns its read select off, also in active mode
    /// (`packet_inet_timeout`, inet_drv.c:15021-15024).
    pub(in crate::runtime) fn receive_timeout(&mut self) -> Vec<Event> {
        self.read_off = self.socket_type == SocketType::Datagram;
        self.stream.remain = 0;
        self.end_receive(Err("timeout".to_owned()))
    }

    /// End the receive that waits with `timeout` at once, for a timeout of
    /// 0 (`async_error_am`, inet_drv.c:12310-12311).
    pub(in crate::runtime) fn receive_now_timeout(&mut self) -> Vec<Event> {
        self.end_receive(Err("timeout".to_owned()))
    }

    /// Deliver the bytes of the input buffer to the owner of a socket that
    /// became active, as `tcp_recv` (`read`, for `once`) and `tcp_deliver`
    /// do after `SETOPTS` (inet_drv.c:11131-11148, 13042-13060). Without
    /// `read`, an empty buffer and a buffer that waits for more bytes give
    /// no message. A socket whose connection ended gives `{tcp_closed, S}`
    /// once, and the port ends with the reason `normal`
    /// (inet_drv.c:7001-7011).
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// assert_eq!(port.activate(false), vec![Event::Data(vec![1, 2])]);
    /// ```
    pub(in crate::runtime) fn activate(&mut self, read: bool) -> Vec<Event> {
        let stream = &self.stream;
        if stream.ended {
            return self.active_end(Vec::new(), None, "normal".to_owned());
        }
        if read {
            self.receive(0)
        } else if stream.input_size == 0 || stream.remain > 0 || stream.filled == 0 {
            Vec::new()
        } else {
            self.deliver(stream.filled)
        }
    }

    /// Give the first `length` bytes of the buffer to the receive, as
    /// `tcp_deliver` does (inet_drv.c:13042-13128), or to the owner as
    /// `{tcp, S, Data}` in active mode, which then becomes passive for
    /// `once` (`tcp_reply_data`, inet_drv.c:4278-4303). When
    /// the receive takes all the filled bytes, the buffer itself becomes
    /// the data, shrunk to `length`, and the port has no buffer
    /// (`tcp_clear_input`). Else the receive gets a copy of the `length`
    /// bytes, and the bytes after them move to the start of the buffer.
    fn deliver(&mut self, length: usize) -> Vec<Event> {
        let stream = &mut self.stream;
        stream.statistics.input(length as u64);
        let data = if length == stream.filled {
            let mut data = std::mem::take(&mut stream.input);
            data.truncate(length);
            data.shrink_to_fit();
            stream.filled = 0;
            stream.input_size = 0;
            data
        } else {
            let data = stream.input[..length].to_vec();
            stream.input.copy_within(length..stream.filled, 0);
            stream.filled -= length;
            data
        };
        stream.remain = 0;
        match self.active {
            0 => self.end_receive(Ok(data)),
            active => {
                if active == 2 {
                    self.active = 0;
                }
                vec![Event::Data(data)]
            }
        }
    }

    /// Return the message of the end of the receive that waits, or no
    /// message without one.
    pub(super) fn end_receive(&mut self, result: Result<Vec<u8>, String>) -> Vec<Event> {
        self.stream
            .receiver
            .take()
            .map(|receiver| Event::Received { receiver, result })
            .into_iter()
            .collect()
    }

    /// Handle the end of file of the socket in passive mode, without
    /// `failure` or with `ECONNRESET`, as `tcp_recv_closed` does
    /// (inet_drv.c:12783-12847), and a failed read, as `tcp_recv_error`
    /// does (inet_drv.c:12849-12890): a send that waits gets `{error,
    /// closed}` and its queue goes, the socket closes, and the receive gets
    /// `{error, closed}` or the error name. After the end of file without a
    /// send that waits, the next send gives `{error, closed}`. In active
    /// mode, the owner gets `{tcp_error, S, Reason}` for a failed read and
    /// then `{tcp_closed, S}`, and the port ends with the reason `normal` or
    /// the error name (inet_drv.c:12836-12844, 12870-12877).
    fn receive_ended(&mut self, failure: Option<io::Error>) -> Vec<Event> {
        let failure = failure.filter(|failure| failure.kind() != io::ErrorKind::ConnectionReset);
        let name = failure.as_ref().map(errno_name);
        let mut events = self.end_busy();
        self.stream.closed_send |= events.is_empty() && failure.is_none();
        if self.active != 0 {
            events.extend(name.clone().map(Event::Error));
            return self.active_end(events, None, name.unwrap_or("normal".to_owned()));
        }
        let reason = name.unwrap_or("closed".to_owned());
        events.extend(self.close_socket());
        events.extend(self.end_receive(Err(reason)));
        // The sends that waited run on the closed socket and give
        // `{error, enotconn}` (inet_drv.c:12681-12693).
        events.extend(self.unbusy());
        events
    }

    /// Add `{tcp_closed, S}` once for the port, then `reply`, the end of
    /// each send that waits behind the busy port (see
    /// [`Self::end_waiters`]), and then the end of the port with `reason`
    /// to `events`, as an active socket does
    /// at the end of its connection (`tcp_closed_message` and
    /// `driver_exit`), and drop the input buffer (`tcp_clear_input`).
    fn active_end(
        &mut self,
        mut events: Vec<Event>,
        reply: Option<Event>,
        reason: String,
    ) -> Vec<Event> {
        let stream = &mut self.stream;
        stream.input = Vec::new();
        stream.filled = 0;
        stream.input_size = 0;
        stream.remain = 0;
        if !std::mem::replace(&mut stream.closed_sent, true) {
            events.push(Event::Closed);
        }
        events.extend(reply);
        events.extend(self.end_waiters());
        events.push(Event::Exit(reason));
        events
    }

    /// End the send that waits for the queue with `{error, timeout}` when
    /// its `send_timeout` ends, as `tcp_inet_send_timeout` does
    /// (inet_drv.c:12458-12534): the port unbusies and keeps the queue, so
    /// a later send makes it busy again with a new timer (12529-12533).
    /// With `send_timeout_close`, the socket closes: in active mode after
    /// `{tcp_closed, S}` and the end of the port (12484-12505), and in
    /// passive mode without a message, so the next send gives
    /// `{error, enotconn}` (12506-12521). The sends that arrived while the
    /// port was busy then run (see [`Self::unbusy`]).
    pub(in crate::runtime) fn send_timeout_expired(&mut self) -> Vec<Event> {
        let Some(sender) = self.stream.busy.take() else {
            return Vec::new();
        };
        let mut events = vec![Event::Replied {
            sender,
            result: Err("timeout".to_owned()),
        }];
        if self.send_timeout_close != 0 {
            if self.active != 0 {
                return self.active_end(events, None, "normal".to_owned());
            }
            events.extend(self.close_socket());
        }
        events.extend(self.unbusy());
        events
    }

    /// Run the sends that arrived while the port was busy, in order, as
    /// ERTS runs their suspended commands when the port unbusies
    /// (`set_busy_port`, inet_drv.c:13783-13784): each runs the send path
    /// of `tcp_inet_commandv` (inet_drv.c:12681-12698), so its bytes join
    /// the queue only now, and a queue whose size and the new bytes reach
    /// the high watermark makes it the send that waits, with a new
    /// `send_timeout` timer (`tcp_sendv`, inet_drv.c:13771-13792,
    /// `Scheduler::tcp_send_timer`). The sends after it wait again.
    fn unbusy(&mut self) -> Vec<Event> {
        let mut events = Vec::new();
        while self.stream.busy.is_none()
            && let Some(Waiter { sender, data }) = self.stream.waiters.pop_front()
        {
            // `Inet::send` keeps no waiter with `delay_send`, and
            // `setopts` sets no `delay_send` while one waits.
            let sent = self.send_data(&data, sender);
            events.extend(sent.expect("a send that waits runs without delay_send"));
        }
        events
    }

    /// End each send that arrived while the port was busy with
    /// `{error, einval}`, in order, as the end of the port ends them: ERTS
    /// aborts their suspended `erlang:port_command/3`, which raises
    /// `badarg`, so `prim_inet:send/2` gives `{error, einval}`
    /// (prim_inet.erl:587-588, 625-628). Their bytes never join the queue.
    pub(in crate::runtime) fn end_waiters(&mut self) -> Vec<Event> {
        self.stream
            .waiters
            .drain(..)
            .map(|Waiter { sender, .. }| Event::Replied {
                sender,
                result: Err("einval".to_owned()),
            })
            .collect()
    }

    /// End the send that waits for the queue with `{error, closed}` after
    /// the drop of the queue (inet_drv.c:12791-12801).
    fn end_busy(&mut self) -> Vec<Event> {
        let Some(sender) = self.stream.busy.take() else {
            return Vec::new();
        };
        let mut events = self.clear_output();
        events.push(Event::Replied {
            sender,
            result: Err("closed".to_owned()),
        });
        events
    }

    /// Drop the queue and return `{empty_out_q, S}` for the subscribers, as
    /// `tcp_clear_output` does (inet_drv.c:11718-11725).
    fn clear_output(&mut self) -> Vec<Event> {
        self.stream.output.clear();
        self.empty_queue()
    }

    /// Return `{empty_out_q, S}` for the subscribers, which then have no
    /// subscription, or no message without one (`send_empty_out_q_msgs`,
    /// inet_drv.c:10795-10813).
    fn empty_queue(&mut self) -> Vec<Event> {
        let subscribers = std::mem::take(&mut self.stream.subscribers);
        (!subscribers.is_empty())
            .then_some(Event::EmptyQueue(subscribers))
            .into_iter()
            .collect()
    }

    /// Close the socket and drop the buffers, as `tcp_desc_close` does
    /// (inet_drv.c:11932-11937, 5053-5080), and return `{empty_out_q, S}`
    /// for the subscribers. The port stays open.
    pub(super) fn close_socket(&mut self) -> Vec<Event> {
        self.socket = None;
        self.state = State::Closed;
        self.stream.ended = true;
        self.stream.pending_shutdown = None;
        self.stream.input = Vec::new();
        self.stream.filled = 0;
        self.stream.input_size = 0;
        self.stream.remain = 0;
        self.clear_output()
    }

    /// Run the output `bytes` of the port from `caller`, as
    /// `tcp_inet_commandv` and `tcp_sendv` do (inet_drv.c:12657-12731,
    /// 13736-13850). `seed` is the hash seed of the node, which the tag
    /// of the header needs.
    ///
    /// # Errors
    ///
    /// The errors of [`Inet::send_datagram`], a tag that is not the
    /// reference bytes of `term_to_binary/2` with `[local]`, and a send
    /// with `delay_send`.
    pub(in crate::runtime) fn send(
        &mut self,
        bytes: &[u8],
        caller: ProcessId,
        seed: u32,
    ) -> Result<Output, BeamError> {
        let header = bytes
            .split_first_chunk::<2>()
            .and_then(|(&size, rest)| rest.split_at_checked(usize::from(u16::from_be_bytes(size))));
        let Some((tag, data)) = header else {
            return Ok(Output::Failed);
        };
        let tag = external::decode_local_reference(tag, seed).ok_or_else(|| {
            self.unsupported("output header", "other than term_to_binary(Ref, [local])")
        })?;
        let sender = Sender {
            caller,
            tag,
            timer: None,
        };
        if self.stream.busy.is_some() {
            // ERTS suspends a command to a busy port and runs it when the
            // port unbusies (`set_busy_port`, inet_drv.c:13783-13784), so
            // the send keeps its bytes and its caller waits for its reply
            // (see `Self::unbusy`). The run at the unbusy cannot return the
            // error of `delay_send`, so the send gives it now.
            if self.delay_send {
                return Err(self.unsupported("output", "with delay_send to a busy port"));
            }
            let data = data.to_vec();
            self.stream.waiters.push_back(Waiter { sender, data });
            return Ok(Output::Sent(Vec::new()));
        }
        self.send_data(data, sender).map(Output::Sent)
    }

    /// Run the send of `data` from `sender` on a port that is not busy, as
    /// `tcp_inet_commandv` does (inet_drv.c:12681-12698), and return its
    /// messages.
    ///
    /// # Errors
    ///
    /// The errors of [`Inet::send_datagram`] and a send with `delay_send`.
    fn send_data(&mut self, data: &[u8], sender: Sender) -> Result<Vec<Event>, BeamError> {
        if self.socket_type == SocketType::Datagram {
            return self.send_datagram(data, sender);
        }
        if self.stream.pending_shutdown.is_some() {
            // `tcp_inet_commandv` gives the error of `tcp_shutdown_error` to
            // a send after a shutdown that waits for the queue
            // (inet_drv.c:12695-12696). A send that waited reaches this
            // branch when the shutdown arrived while the port was busy.
            return Ok(self.send_failed(Some(sender)));
        }
        let reply = |result| Ok(vec![Event::Replied { sender, result }]);
        if self.state != State::Connected {
            if std::mem::take(&mut self.stream.closed_send) {
                return reply(Err("closed".to_owned()));
            }
            return reply(Err("enotconn".to_owned()));
        }
        self.stream.statistics.output(data.len() as u64);
        if !self.stream.output.is_empty() {
            self.stream.output.extend(data);
            match self.send_timeout {
                _ if self.stream.output.len() < HIGH => {}
                // The bytes are in the queue before the check; a queue that
                // reaches the high watermark makes the port busy, and the
                // caller waits with no reply until the queue drains
                // (inet_drv.c:13771-13792). A `send_timeout` of 0 answers
                // `{error, timeout}` at once (13776-13780); each other
                // value only marks the port busy, and a value other than
                // `infinity` also starts its send timer, which
                // `Scheduler::tcp_send_timer` starts (13783-13792).
                0 => return reply(Err("timeout".to_owned())),
                _ => {
                    self.stream.busy = Some(sender);
                    return Ok(Vec::new());
                }
            }
        } else if !data.is_empty() {
            if self.delay_send {
                return Err(self.unsupported("output", "with delay_send"));
            }
            let socket = self.socket.as_ref().expect("a connected port has a socket");
            let written = match socket.send(data) {
                Err(failure) if !blocked(&failure) => {
                    return Ok(self.send_failed(Some(sender)));
                }
                result => result.unwrap_or(0),
            };
            self.stream.output.extend(&data[written..]);
        }
        reply(Ok(()))
    }

    /// Write the queue while the socket takes bytes, as `tcp_inet_output`
    /// does (inet_drv.c:14234-14370), and return the messages: the reply of
    /// the send that waits when the queue holds 4096 bytes or fewer, then
    /// `{empty_out_q, S}` when the queue is empty (inet_drv.c:14301-14306),
    /// or the end of the socket after a failed write. A socket that
    /// connects ends its connect instead (see [`Self::connect_ready`]).
    pub(in crate::runtime) fn send_ready(&mut self) -> Vec<Event> {
        if self.state == State::Connecting {
            return self.connect_ready();
        }
        let mut events = Vec::new();
        while !self.stream.output.is_empty() {
            let socket = self
                .socket
                .as_ref()
                .expect("a port with output has a socket");
            let written = match socket.send(self.stream.output.as_slices().0) {
                Ok(written) => written,
                Err(failure) if blocked(&failure) => break,
                Err(_) => {
                    // A reply of this step goes before the end of the socket.
                    let sender = self.stream.busy.take();
                    events.extend(self.send_failed(sender));
                    return events;
                }
            };
            self.stream.output.drain(..written);
            if self.stream.output.len() <= LOW
                && let Some(sender) = self.stream.busy.take()
            {
                // The queue reached the low watermark, so the port unbusies,
                // cancels the send timer, and answers the caller with `ok`
                // (inet_drv.c:14350-14359).
                events.push(Event::Replied {
                    sender,
                    result: Ok(()),
                });
            }
        }
        if self.stream.output.is_empty() {
            events.extend(self.empty_queue());
            if let Some(how) = self.stream.pending_shutdown.take() {
                // `tcp_shutdown_async`: a failed shutdown ends the socket as
                // `tcp_shutdown_error` does.
                if self.shutdown(how).first() != Some(&OK) {
                    events.extend(self.send_failed(None));
                }
            }
        }
        // The sends that arrived while the port was busy run after this
        // write, as ERTS runs their suspended commands after
        // `tcp_inet_output` (`set_busy_port`, inet_drv.c:13783-13784).
        events.extend(self.unbusy());
        events
    }

    /// Handle a failed write in passive mode, as `tcp_send_error` and
    /// `tcp_send_or_shutdown_error` do (inet_drv.c:13613-13734): the
    /// receive that waits and the send of `sender` get `{error, closed}`,
    /// or the next send gives `{error, closed}` without a send; the socket
    /// closes, and the next receive gives `{error, closed}`. In active mode,
    /// the owner gets `{tcp_closed, S}`, then the send of `sender` gets
    /// `{error, closed}`, and the port ends with the reason `normal`
    /// (inet_drv.c:13648-13662).
    fn send_failed(&mut self, sender: Option<Sender>) -> Vec<Event> {
        if self.active != 0 {
            let reply = sender.map(|sender| Event::Replied {
                sender,
                result: Err("closed".to_owned()),
            });
            return self.active_end(Vec::new(), reply, "normal".to_owned());
        }
        let mut events = self.end_receive(Err("closed".to_owned()));
        match sender {
            Some(sender) => events.push(Event::Replied {
                sender,
                result: Err("closed".to_owned()),
            }),
            None => self.stream.closed_send = true,
        }
        events.extend(self.close_socket());
        self.stream.closed_receive = true;
        // The sends that waited run on the closed socket: the first gets
        // `{error, closed}` after a failure without a send, as
        // `TCP_ADDF_DELAYED_CLOSE_SEND` gives, and each other
        // `{error, enotconn}` (inet_drv.c:13657-13670, 12681-12693).
        events.extend(self.unbusy());
        events
    }

    /// Return the fd of a socket with a receive that waits and a read
    /// select that is on (`sock_select(desc, FD_READ, 1)`,
    /// inet_drv.c:12317, 15496; see `Inet::read_off`) or of a
    /// connected stream socket in active mode (inet_drv.c:7966), else
    /// `None`.
    pub(in crate::runtime) fn stream_receive_fd(&self) -> Option<c_int> {
        let socket = self.socket.as_ref()?;
        let active = self.active != 0
            && self.state == State::Connected
            && self.socket_type == SocketType::Stream;
        (self.stream.receiver.is_some() && !self.read_off || active).then(|| socket.as_raw_fd())
    }

    /// Return the fd of a socket with queued output (`sock_select(desc,
    /// FD_WRITE, 1)`, inet_drv.c:13846) or that connects (`sock_select(desc,
    /// FD_CONNECT, 1)`, inet_drv.c:12109), else `None`.
    pub(in crate::runtime) fn send_fd(&self) -> Option<c_int> {
        let socket = self.socket.as_ref()?;
        (!self.stream.output.is_empty() || self.state == State::Connecting)
            .then(|| socket.as_raw_fd())
    }

    /// Return the term of received `bytes` in the `mode` of the port: a
    /// binary, or a list of bytes in `list` mode (`inet_async_data`,
    /// inet_drv.c:3162-3200).
    pub(in crate::runtime) fn data_term(&self, bytes: Vec<u8>) -> Term {
        if self.mode == 0 {
            Term::list_from(bytes.into_iter().map(|byte| Term::Integer(byte.into())))
        } else {
            Term::binary(bytes)
        }
    }

    /// Return true when the queue of the port holds bytes.
    pub(in crate::runtime) fn has_output(&self) -> bool {
        !self.stream.output.is_empty()
    }

    /// Return true when the close of the port with a reason other than
    /// `kill` keeps the port in the `CLOSING` state to write its queue
    /// (io.c:3939-3948): the queue holds bytes and `tcp_inet_flush` keeps
    /// them, as `linger` is not `{true, 0}` (`TCP_ADDF_LINGER_ZERO`,
    /// inet_drv.c:12708-12730). With `{true, 0}`, the port terminates at
    /// once in the `CLOSING` state, and the close of the socket drops the
    /// queue.
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// assert!(!port.flushes());
    /// ```
    pub(in crate::runtime) fn flushes(&self) -> bool {
        let linger = self.socket.as_ref().and_then(|socket| socket.linger().ok());
        self.has_output() && linger != Some(Some(std::time::Duration::ZERO))
    }

    /// Run `INET_REQ_GETSTAT` with the codes `request` (inet_drv.c:10732-10800,
    /// 10995-11021): for each code, the code and its value, 32 bits, or 64
    /// bits for `recv_oct` (9) and `send_oct` (10). A code that is not 1 to
    /// 10 gives `badarg`, and a reply of more than 64 KiB is empty.
    pub(super) fn statistics(&self, request: &[u8]) -> Control {
        let size: usize = 1 + request
            .iter()
            .map(|&code| if matches!(code, 9 | 10) { 9 } else { 5 })
            .sum::<usize>();
        if size > MAX_OPT_BUFFER {
            return Control::Reply(Vec::new());
        }
        let counts = &self.stream.statistics;
        let mut reply = vec![OK];
        for &code in request {
            reply.push(code);
            let value = match code {
                1 => counts.receive_count,
                2 => counts.receive_max,
                3 => counts.receive_average as u64,
                4 => counts.receive_deviation.abs() as u64,
                5 => counts.send_count,
                6 => counts.send_max,
                7 => counts.send_average as u64,
                8 => self.stream.output.len() as u64,
                9 | 10 => {
                    let bytes = if code == 9 {
                        counts.receive_bytes
                    } else {
                        counts.send_bytes
                    };
                    reply.extend(bytes.to_be_bytes());
                    continue;
                }
                _ => return Control::Badarg,
            };
            reply.extend((value as u32).to_be_bytes());
        }
        Control::Reply(reply)
    }
}

/// True for an error of a write that would block (`ERRNO_BLOCK`) or that a
/// signal stopped (`EINTR`).
fn blocked(failure: &io::Error) -> bool {
    matches!(
        failure.kind(),
        io::ErrorKind::WouldBlock | io::ErrorKind::Interrupted
    )
}

#[cfg(test)]
#[path = "../../../../tests/runtime/port/inet/stream.rs"]
mod tests;

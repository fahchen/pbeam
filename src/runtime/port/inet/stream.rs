//! The receive and send state of a connected socket of a `tcp_inet` port
//! in passive mode with `{packet, raw}`, as `tcp_descriptor` keeps it
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

use std::{
    collections::VecDeque,
    ffi::c_int,
    io::{self, Read},
    os::fd::AsRawFd,
};

use super::{Control, Inet, MAX_OPT_BUFFER, OK, SocketType, State, errno_name, error_reply};
use crate::{
    error::BeamError,
    runtime::timer::TimerKey,
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

/// The timeout of `infinity` (`INET_INFINITY`).
const INFINITY: i32 = -1;

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
    pub(in crate::runtime) timer: Option<TimerKey>,
}

/// The caller of a send and the monitor reference of its header, which
/// the `inet_reply` message gives (`caller` and `caller_ref`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(in crate::runtime) struct Sender {
    /// The process that gets the `inet_reply` message.
    pub(in crate::runtime) caller: ProcessId,
    /// The reference of the header.
    pub(in crate::runtime) tag: Reference,
}

/// A message that a step of the driver sends.
#[derive(Debug, PartialEq, Eq)]
pub(in crate::runtime) enum Event {
    /// `{inet_async, S, Ref, {ok, Data}}` or `{inet_async, S, Ref, {error,
    /// Reason}}` of the receive of `receiver`.
    Received {
        /// The receive that ends.
        receiver: Receiver,
        /// The bytes, or the error name.
        result: Result<Vec<u8>, String>,
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
struct Statistics {
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
    fn input(&mut self, length: u64) {
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
    fn output(&mut self, length: u64) {
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
    /// The bytes that the socket gave and no receive took yet (`i_buf`
    /// from `i_ptr_start` to `i_ptr`).
    input: Vec<u8>,
    /// The size of the input buffer, 0 without a buffer (`i_bufsz`).
    input_size: usize,
    /// The bytes that the receive of a length still needs (`i_remain`).
    remain: usize,
    /// The receive that waits for data.
    pub(in crate::runtime) receiver: Option<Receiver>,
    /// The bytes that the socket did not take yet (the queue of the port).
    output: VecDeque<u8>,
    /// The send that waits until the queue is small (`INET_F_BUSY`).
    busy: Option<Sender>,
    /// The next send gives `{error, closed}` (`TCP_ADDF_DELAYED_CLOSE_SEND`).
    closed_send: bool,
    /// The next receive gives `{error, closed}`
    /// (`TCP_ADDF_DELAYED_CLOSE_RECV`).
    closed_receive: bool,
    /// The counts of `GETSTAT`.
    statistics: Statistics,
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
            input_size: 0,
            remain: 0,
            receiver: None,
            output: VecDeque::new(),
            busy: None,
            closed_send: false,
            closed_receive: false,
            statistics: Statistics::default(),
        }
    }
}

impl Inet {
    /// Check `TCP_REQ_RECV` with `[T3, T2, T1, T0, N3, N2, N1, N0]`
    /// (inet_drv.c:12268-12307): a socket that is not connected gives
    /// `closed` once after a failed send, else `enotconn`; an active socket
    /// and a request that is not 8 bytes give `einval` (PBeam has no
    /// connected active socket yet); a length above 64
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
        let stream = &mut self.stream;
        let read = if stream.input_size == 0 {
            stream.input_size = if length > 0 { length } else { stream.buffer };
            stream.remain = length;
            stream.input_size
        } else if length > 0 {
            if stream.input.len() >= length {
                return self.deliver(length);
            }
            stream.input_size = length;
            stream.remain = length - stream.input.len();
            stream.remain
        } else if stream.remain == 0 {
            if !stream.input.is_empty() {
                let length = stream.input.len();
                return self.deliver(length);
            }
            stream.input_size - stream.input.len()
        } else {
            stream.remain
        };
        let mut socket = self.socket.as_ref().expect("a connected port has a socket");
        let mut bytes = vec![0; read];
        let count = match socket.read(&mut bytes) {
            Ok(count) if count > 0 => count,
            Err(failure) if failure.kind() == io::ErrorKind::WouldBlock => return Vec::new(),
            result => return self.receive_ended(result.err()),
        };
        let stream = &mut self.stream;
        stream.input.extend_from_slice(&bytes[..count]);
        if stream.remain > 0 {
            stream.remain -= count;
            if stream.remain > 0 {
                return Vec::new();
            }
        }
        self.deliver(self.stream.input.len())
    }

    /// End the receive that waits with `timeout`, as
    /// `tcp_inet_recv_timeout` does (inet_drv.c:11941-11949): the bytes
    /// that it read stay in the buffer for the next receive.
    pub(in crate::runtime) fn receive_timeout(&mut self) -> Vec<Event> {
        self.stream.remain = 0;
        self.end_receive(Err("timeout".to_owned()))
    }

    /// End the receive that waits with `timeout` at once, for a timeout of
    /// 0 (`async_error_am`, inet_drv.c:12310-12311).
    pub(in crate::runtime) fn receive_now_timeout(&mut self) -> Vec<Event> {
        self.end_receive(Err("timeout".to_owned()))
    }

    /// Give the first `length` bytes of the buffer to the receive, as
    /// `tcp_deliver` does in passive mode (inet_drv.c:13042-13128).
    fn deliver(&mut self, length: usize) -> Vec<Event> {
        let stream = &mut self.stream;
        stream.statistics.input(length as u64);
        let data: Vec<u8> = stream.input.drain(..length).collect();
        stream.remain = 0;
        if stream.input.is_empty() {
            stream.input_size = 0;
        }
        self.end_receive(Ok(data))
    }

    /// Return the message of the end of the receive that waits, or no
    /// message without one.
    fn end_receive(&mut self, result: Result<Vec<u8>, String>) -> Vec<Event> {
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
    /// send that waits, the next send gives `{error, closed}`.
    fn receive_ended(&mut self, failure: Option<io::Error>) -> Vec<Event> {
        let failure = failure.filter(|failure| failure.kind() != io::ErrorKind::ConnectionReset);
        let reason = failure.as_ref().map_or("closed".to_owned(), errno_name);
        let mut events = self.end_busy();
        self.stream.closed_send |= events.is_empty() && failure.is_none();
        self.close_socket();
        events.extend(self.end_receive(Err(reason)));
        events
    }

    /// End the send that waits for the queue with `{error, closed}` and
    /// drop the queue (inet_drv.c:12796-12806).
    fn end_busy(&mut self) -> Vec<Event> {
        let Some(sender) = self.stream.busy.take() else {
            return Vec::new();
        };
        self.stream.output.clear();
        vec![Event::Replied {
            sender,
            result: Err("closed".to_owned()),
        }]
    }

    /// Close the socket and drop the buffers, as `tcp_desc_close` does
    /// (inet_drv.c:11932-11937, 5053-5080). The port stays open.
    fn close_socket(&mut self) {
        self.socket = None;
        self.state = State::Closed;
        self.stream.input.clear();
        self.stream.input_size = 0;
        self.stream.remain = 0;
        self.stream.output.clear();
    }

    /// Run the output `bytes` of the port from `caller`, as
    /// `tcp_inet_commandv` and `tcp_sendv` do (inet_drv.c:12657-12731,
    /// 13736-13850). `seed` is the hash seed of the node, which the tag
    /// of the header needs.
    ///
    /// # Errors
    ///
    /// A tag that is not the reference bytes of `term_to_binary/2` with
    /// `[local]`, a send while a send waits for the queue (ERTS suspends the
    /// caller of a busy port), a send that would wait with a `send_timeout`
    /// other than `infinity` and 0, and a send with `delay_send`.
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
        if self.stream.busy.is_some() {
            return Err(self.unsupported("output", "to a busy port"));
        }
        let sender = Sender { caller, tag };
        let reply = |result| Ok(Output::Sent(vec![Event::Replied { sender, result }]));
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
                0 => return reply(Err("timeout".to_owned())),
                INFINITY => {
                    self.stream.busy = Some(sender);
                    return Ok(Output::Sent(Vec::new()));
                }
                _ => return Err(self.unsupported("output", "to a busy port with a send_timeout")),
            }
        } else if !data.is_empty() {
            if self.delay_send {
                return Err(self.unsupported("output", "with delay_send"));
            }
            let socket = self.socket.as_ref().expect("a connected port has a socket");
            let written = match socket.send(data) {
                Err(failure) if !blocked(&failure) => {
                    return Ok(Output::Sent(self.send_failed(Some(sender))));
                }
                result => result.unwrap_or(0),
            };
            self.stream.output.extend(&data[written..]);
        }
        reply(Ok(()))
    }

    /// Write the queue while the socket takes bytes, as `tcp_inet_output`
    /// does (inet_drv.c:14234-14370), and return the messages: the reply of
    /// the send that waits when the queue holds 4096 bytes or fewer, or the
    /// end of the socket after a failed write.
    pub(in crate::runtime) fn send_ready(&mut self) -> Vec<Event> {
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
                    let sender = self.stream.busy.take();
                    return self.send_failed(sender);
                }
            };
            self.stream.output.drain(..written);
            if self.stream.output.len() <= LOW
                && let Some(sender) = self.stream.busy.take()
            {
                events.push(Event::Replied {
                    sender,
                    result: Ok(()),
                });
            }
        }
        events
    }

    /// Handle a failed write in passive mode, as `tcp_send_error` and
    /// `tcp_send_or_shutdown_error` do (inet_drv.c:13613-13734): the
    /// receive that waits and the send of `sender` get `{error, closed}`,
    /// or the next send gives `{error, closed}` without a send; the socket
    /// closes, and the next receive gives `{error, closed}`.
    fn send_failed(&mut self, sender: Option<Sender>) -> Vec<Event> {
        let mut events = self.end_receive(Err("closed".to_owned()));
        match sender {
            Some(sender) => events.push(Event::Replied {
                sender,
                result: Err("closed".to_owned()),
            }),
            None => self.stream.closed_send = true,
        }
        self.close_socket();
        self.stream.closed_receive = true;
        events
    }

    /// Return the fd of a connected stream socket with a receive that
    /// waits (`sock_select(desc, FD_READ, 1)`, inet_drv.c:12317), else
    /// `None`.
    pub(in crate::runtime) fn stream_receive_fd(&self) -> Option<c_int> {
        let socket = self.socket.as_ref()?;
        (self.socket_type == SocketType::Stream && self.stream.receiver.is_some())
            .then(|| socket.as_raw_fd())
    }

    /// Return the fd of a socket with queued output (`sock_select(desc,
    /// FD_WRITE, 1)`, inet_drv.c:13846), else `None`.
    pub(in crate::runtime) fn send_fd(&self) -> Option<c_int> {
        let socket = self.socket.as_ref()?;
        (!self.stream.output.is_empty()).then(|| socket.as_raw_fd())
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
    pub(super) fn has_output(&self) -> bool {
        !self.stream.output.is_empty()
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

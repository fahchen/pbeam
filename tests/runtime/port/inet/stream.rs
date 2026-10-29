use std::io::{Read, Write};

use socket2::{Domain, Socket, Type};

use super::{
    super::{Control, MAGIC},
    Event, HIGH, Output, Receiver, Sender,
};
use crate::{
    runtime::{
        port::inet::{Inet, SocketType},
        timer::PortTimer,
    },
    term::{ProcessId, Reference, external::local_reference},
};

/// The hash seed of the node that the sends of the tests use.
const SEED: u32 = 9;

/// Return the port output of `data` with the header of `tag`, as
/// `prim_inet:send/2` sends it (prim_inet.erl:580-585).
fn output_bytes(tag: Reference, data: &[u8]) -> Vec<u8> {
    let header = local_reference(tag, SEED);
    let mut bytes = u16::try_from(header.len()).unwrap().to_be_bytes().to_vec();
    bytes.extend(header);
    bytes.extend(data);
    bytes
}

/// A send that waits for the queue of the port, with the timer `timer`.
fn sender(caller: u64, tag: u64, timer: Option<PortTimer>) -> Sender {
    Sender {
        caller: ProcessId(caller),
        tag: Reference(tag),
        timer,
    }
}

/// A receive of 64 bytes that arrive in 64 reads of 1 byte reads into the
/// one buffer that the receive allocates, and gives that buffer as the data.
#[test]
fn fills_one_buffer_for_a_receive_in_many_reads() {
    let (local, mut peer) = Socket::pair(Domain::UNIX, Type::STREAM, None).unwrap();
    let mut port = Inet::new(SocketType::Stream).copy(local);
    let receiver = Receiver {
        id: 0,
        caller: ProcessId(1),
        timer: None,
    };
    port.stream.receiver = Some(receiver);
    assert_eq!(port.receive(64), Vec::new());
    assert_eq!(port.stream.input.capacity(), 64);
    for byte in 0..63 {
        peer.write_all(&[byte]).unwrap();
        assert_eq!(port.receive(0), Vec::new());
        assert_eq!(port.stream.filled, usize::from(byte) + 1);
        assert_eq!(port.stream.input.len(), 64);
        assert_eq!(port.stream.input.capacity(), 64);
    }
    peer.write_all(&[63]).unwrap();
    let mut events = port.receive(0);
    let Some(Event::Received {
        receiver: received,
        result: Ok(data),
    }) = events.pop()
    else {
        panic!("the receive gives its data");
    };
    assert_eq!((received, events), (receiver, Vec::new()));
    assert_eq!(data, (0..64).collect::<Vec<u8>>());
    assert_eq!(data.capacity(), 64);
    assert_eq!(port.stream.input.capacity(), 0);
    assert_eq!(port.stream.input_size, 0);
}

/// A receive of 8 bytes after a receive of 4 bytes that timed out with 2
/// bytes makes the buffer larger, keeps the 2 bytes, and reads the 6 bytes
/// after them (`tcp_expand_buffer`).
#[test]
fn makes_the_buffer_larger_for_a_longer_receive() {
    let (local, mut peer) = Socket::pair(Domain::UNIX, Type::STREAM, None).unwrap();
    let mut port = Inet::new(SocketType::Stream).copy(local);
    peer.write_all(&[1, 2]).unwrap();
    assert_eq!(port.receive(4), Vec::new());
    assert_eq!(port.receive_timeout(), Vec::new());
    peer.write_all(&[3, 4, 5, 6, 7, 8]).unwrap();
    let receiver = Receiver {
        id: 1,
        caller: ProcessId(1),
        timer: None,
    };
    port.stream.receiver = Some(receiver);
    assert_eq!(
        port.receive(8),
        vec![Event::Received {
            receiver,
            result: Ok(vec![1, 2, 3, 4, 5, 6, 7, 8]),
        }]
    );
    assert_eq!(port.stream.input_size, 0);
}

/// A failed read in active mode gives `{tcp_error, S, Reason}`, then
/// `{tcp_closed, S}`, and ends the port with the error name; a later end
/// sends no second `{tcp_closed, S}` (inet_drv.c:4058-4074, 12870-12877).
#[test]
fn ends_an_active_socket_at_a_failed_read() {
    let (local, _peer) = Socket::pair(Domain::UNIX, Type::STREAM, None).unwrap();
    let mut port = Inet::new(SocketType::Stream).copy(local);
    port.active = 1;
    let failure = std::io::Error::from_raw_os_error(libc::ETIMEDOUT);
    assert_eq!(
        port.receive_ended(Some(failure)),
        [
            Event::Error("etimedout".to_owned()),
            Event::Closed,
            Event::Exit("etimedout".to_owned())
        ]
    );
    assert_eq!(port.receive_ended(None), [Event::Exit("normal".to_owned())]);
}

/// A failed write in active mode gives `{tcp_closed, S}`, then the reply
/// `{error, closed}` of the send, and ends the port with `normal`
/// (inet_drv.c:13648-13662).
#[test]
fn ends_an_active_socket_at_a_failed_write() {
    let (local, _peer) = Socket::pair(Domain::UNIX, Type::STREAM, None).unwrap();
    let mut port = Inet::new(SocketType::Stream).copy(local);
    port.active = 2;
    let sender = Sender {
        caller: ProcessId(1),
        tag: Reference(2),
        timer: None,
    };
    assert_eq!(
        port.send_failed(Some(sender)),
        [
            Event::Closed,
            Event::Replied {
                sender,
                result: Err("closed".to_owned())
            },
            Event::Exit("normal".to_owned())
        ]
    );
}

/// A change to `{active, true}` delivers the bytes that a receive left in
/// the buffer, and gives no message without them (inet_drv.c:13042-13060).
#[test]
fn delivers_the_buffer_when_a_socket_becomes_active() {
    let (local, mut peer) = Socket::pair(Domain::UNIX, Type::STREAM, None).unwrap();
    let mut port = Inet::new(SocketType::Stream).copy(local);
    port.active = 1;
    assert_eq!(port.activate(false), Vec::new());
    port.active = 0;
    peer.write_all(&[1, 2]).unwrap();
    assert_eq!(port.receive(4), Vec::new());
    assert_eq!(port.receive_timeout(), Vec::new());
    port.active = 1;
    assert_eq!(port.activate(false), [Event::Data(vec![1, 2])]);
    assert_eq!(port.active, 1);
}

/// `TCP_REQ_SHUTDOWN` of the write side of a connected socket gives `ok`,
/// and the peer reads the end of the file (inet_drv.c:12339-12378).
#[test]
fn shuts_down_the_write_side_of_a_connected_socket() {
    let (local, mut peer) = Socket::pair(Domain::UNIX, Type::STREAM, None).unwrap();
    let mut port = Inet::new(SocketType::Stream).copy(local);
    assert_eq!(
        port.control(MAGIC + 44, &[1], ProcessId(1)).unwrap(),
        Control::Reply(vec![1])
    );
    let mut byte = [0; 1];
    assert_eq!(peer.read(&mut byte).unwrap(), 0);
}

/// `TCP_REQ_SHUTDOWN` gives `enotconn` without a connection, and `einval`
/// to a request that is not one byte (inet_drv.c:12339-12352).
#[test]
fn gives_the_error_of_a_shutdown_without_a_connection_or_with_a_wrong_request() {
    let (local, _peer) = Socket::pair(Domain::UNIX, Type::STREAM, None).unwrap();
    let mut port = Inet::new(SocketType::Stream).copy(local);
    assert_eq!(
        port.control(MAGIC + 44, &[1, 1], ProcessId(1)).unwrap(),
        Control::Reply(b"\0einval".to_vec())
    );
    let mut closed = Inet::new(SocketType::Stream);
    assert_eq!(
        closed.control(MAGIC + 44, &[1], ProcessId(1)).unwrap(),
        Control::Reply(b"\0enotconn".to_vec())
    );
}

/// A `TCP_REQ_SHUTDOWN` of the write side while the queue holds bytes
/// waits for the queue, which `send_ready` writes before the shutdown
/// (`TCP_ADDF_PENDING_SHUT_WR`, inet_drv.c:12362-12376, 14298-14306).
#[test]
fn shuts_down_the_write_side_after_the_queue_empties() {
    let (local, mut peer) = Socket::pair(Domain::UNIX, Type::STREAM, None).unwrap();
    let mut port = Inet::new(SocketType::Stream).copy(local);
    port.stream.output.extend([1, 2, 3]);
    assert_eq!(
        port.control(MAGIC + 44, &[1], ProcessId(1)).unwrap(),
        Control::Reply(vec![1])
    );
    assert_eq!(port.stream.pending_shutdown, Some(1));
    assert_eq!(port.send_ready(), Vec::new());
    assert_eq!(port.stream.pending_shutdown, None);
    let mut bytes = Vec::new();
    peer.read_to_end(&mut bytes).unwrap();
    assert_eq!(bytes, [1, 2, 3]);
}

/// A send that arrives while the port is busy joins the queue and waits
/// for its reply behind the send that the port is busy for, with no
/// message (`set_busy_port`, inet_drv.c:13774-13784).
#[test]
fn suspends_a_send_that_arrives_while_the_port_is_busy() {
    let (local, _peer) = Socket::pair(Domain::UNIX, Type::STREAM, None).unwrap();
    let mut port = Inet::new(SocketType::Stream).copy(local);
    port.stream.busy = Some(sender(1, 2, Some((0, (1, 1)))));
    port.stream.output.extend([0; HIGH]);
    let output = port
        .send(&output_bytes(Reference(4), b"wait"), ProcessId(3), SEED)
        .unwrap();
    assert_eq!(output, Output::Sent(Vec::new()));
    assert_eq!(port.stream.waiters.len(), 1);
    assert_eq!(port.stream.output.len(), HIGH + 4);
}

/// A send whose queue reaches the high watermark with a finite
/// `send_timeout` makes the port busy with no reply, and the caller waits
/// (`tcp_sendv`, inet_drv.c:13771-13792).
#[test]
fn makes_a_finite_send_timeout_busy_without_a_reply() {
    let (local, _peer) = Socket::pair(Domain::UNIX, Type::STREAM, None).unwrap();
    let mut port = Inet::new(SocketType::Stream).copy(local);
    port.stream.output.extend([0; HIGH]);
    let output = port
        .send(&output_bytes(Reference(2), b"fill"), ProcessId(1), SEED)
        .unwrap();
    assert_eq!(output, Output::Sent(Vec::new()));
    assert_eq!(
        port.stream.busy,
        Some(sender(1, 2, None)),
        "the timer starts after the step"
    );
}

/// The send timer ends the busy send with `{error, timeout}` and runs the
/// sends that arrived while the port was busy: a queue at the low
/// watermark or below gives each caller `ok`, and a queue above it makes
/// the first caller the send that waits (`tcp_inet_send_timeout`,
/// inet_drv.c:12458-12480, 12529-12533; `tcp_sendv`, 13771-13792).
#[test]
fn runs_the_waiters_at_the_expiry_of_the_send_timer() {
    let (local, _peer) = Socket::pair(Domain::UNIX, Type::STREAM, None).unwrap();
    let mut port = Inet::new(SocketType::Stream).copy(local);
    // A queue above the low watermark keeps the first waiter busy.
    port.stream.busy = Some(sender(1, 2, Some((0, (1, 1)))));
    port.stream.waiters.push_back(sender(3, 4, None));
    port.stream.waiters.push_back(sender(5, 6, None));
    port.stream.output.extend([0; HIGH]);
    assert_eq!(
        port.send_timeout_expired(),
        [Event::Replied {
            sender: sender(1, 2, Some((0, (1, 1)))),
            result: Err("timeout".to_owned())
        }]
    );
    assert_eq!(port.stream.busy, Some(sender(3, 4, None)));
    assert_eq!(port.stream.waiters.make_contiguous(), &[sender(5, 6, None)]);
    // A queue at the low watermark or below answers `ok` in order.
    port.stream.output.clear();
    port.stream.busy = Some(sender(3, 4, Some((0, (2, 2)))));
    assert_eq!(
        port.send_timeout_expired(),
        [
            Event::Replied {
                sender: sender(3, 4, Some((0, (2, 2)))),
                result: Err("timeout".to_owned())
            },
            Event::Replied {
                sender: sender(5, 6, None),
                result: Ok(())
            },
        ]
    );
    assert_eq!(port.stream.busy, None);
}

/// The expiry of the send timer of a passive socket with
/// `send_timeout_close` closes the socket without a message, so the next
/// send gives `{error, enotconn}`, and a send that arrived while the port
/// was busy gives the same (`tcp_inet_send_timeout`, inet_drv.c:12506-12521,
/// 12681-12693).
#[test]
fn closes_a_passive_socket_at_the_expiry_with_send_timeout_close() {
    let (local, _peer) = Socket::pair(Domain::UNIX, Type::STREAM, None).unwrap();
    let mut port = Inet::new(SocketType::Stream).copy(local);
    port.send_timeout_close = 1;
    port.stream.busy = Some(sender(1, 2, Some((0, (1, 1)))));
    port.stream.waiters.push_back(sender(3, 4, None));
    assert_eq!(
        port.send_timeout_expired(),
        [
            Event::Replied {
                sender: sender(1, 2, Some((0, (1, 1)))),
                result: Err("timeout".to_owned())
            },
            Event::Replied {
                sender: sender(3, 4, None),
                result: Err("enotconn".to_owned())
            },
        ]
    );
    assert_eq!(port.stream.busy, None);
    assert!(port.socket.is_none());
    assert!(!port.stream.closed_send);
    let output = port
        .send(&output_bytes(Reference(6), b"next"), ProcessId(5), SEED)
        .unwrap();
    assert_eq!(
        output,
        Output::Sent(vec![Event::Replied {
            sender: sender(5, 6, None),
            result: Err("enotconn".to_owned())
        }])
    );
}

/// The expiry of the send timer of an active socket with
/// `send_timeout_close` sends `{tcp_closed, S}`, ends each send that
/// arrived while the port was busy with `{error, closed}`, and ends the
/// port with the reason `normal` (inet_drv.c:12484-12505).
#[test]
fn closes_an_active_socket_at_the_expiry_with_send_timeout_close() {
    let (local, _peer) = Socket::pair(Domain::UNIX, Type::STREAM, None).unwrap();
    let mut port = Inet::new(SocketType::Stream).copy(local);
    port.active = 1;
    port.send_timeout_close = 1;
    port.stream.busy = Some(sender(1, 2, Some((0, (1, 1)))));
    port.stream.waiters.push_back(sender(3, 4, None));
    assert_eq!(
        port.send_timeout_expired(),
        [
            Event::Replied {
                sender: sender(1, 2, Some((0, (1, 1)))),
                result: Err("timeout".to_owned())
            },
            Event::Replied {
                sender: sender(3, 4, None),
                result: Err("closed".to_owned())
            },
            Event::Closed,
            Event::Exit("normal".to_owned())
        ]
    );
}

/// The drain of the queue to the low watermark answers the send that the
/// port is busy for with `ok`, cancels its timer, and runs the sends that
/// arrived while the port was busy in order
/// (inet_drv.c:14350-14359, 13783-13784).
#[test]
fn answers_the_busy_send_and_its_waiters_when_the_queue_drains() {
    let (local, mut peer) = Socket::pair(Domain::UNIX, Type::STREAM, None).unwrap();
    let mut port = Inet::new(SocketType::Stream).copy(local);
    port.stream.busy = Some(sender(1, 2, Some((0, (1, 1)))));
    port.stream.waiters.push_back(sender(3, 4, None));
    port.stream.waiters.push_back(sender(5, 6, None));
    port.stream.output.extend([1, 2, 3, 4]);
    assert_eq!(
        port.send_ready(),
        [
            Event::Replied {
                sender: sender(1, 2, Some((0, (1, 1)))),
                result: Ok(())
            },
            Event::Replied {
                sender: sender(3, 4, None),
                result: Ok(())
            },
            Event::Replied {
                sender: sender(5, 6, None),
                result: Ok(())
            },
        ]
    );
    // The peer reads to the end of the queue, so the port must close its
    // socket first, as `gen_tcp:close/1` does for such a peer.
    drop(port);
    let mut bytes = Vec::new();
    peer.read_to_end(&mut bytes).unwrap();
    assert_eq!(bytes, [1, 2, 3, 4]);
}

/// A queue that stays above the low watermark makes the first send that
/// arrived while the port was busy the send that waits, with a new send
/// timer (`tcp_sendv`, inet_drv.c:13771-13792).
#[test]
fn makes_the_first_waiter_the_busy_send_when_the_socket_blocks() {
    let (local, _peer) = Socket::pair(Domain::UNIX, Type::STREAM, None).unwrap();
    let mut port = Inet::new(SocketType::Stream).copy(local);
    // The socket takes no bytes, as a peer that reads nothing does.
    let block = vec![0; HIGH * 4];
    while port.socket.as_ref().unwrap().send(&block).is_ok() {}
    port.stream.waiters.push_back(sender(3, 4, None));
    port.stream.output.extend([0; HIGH]);
    assert_eq!(port.send_ready(), Vec::new());
    assert_eq!(port.stream.busy, Some(sender(3, 4, None)));
}

/// The end of the connection ends the send that the port is busy for and
/// the sends that arrived while it was busy with `{error, closed}` after
/// the drop of the queue (inet_drv.c:12791-12801).
#[test]
fn ends_the_busy_send_and_its_waiters_at_the_end_of_the_connection() {
    let (local, _peer) = Socket::pair(Domain::UNIX, Type::STREAM, None).unwrap();
    let mut port = Inet::new(SocketType::Stream).copy(local);
    port.stream.busy = Some(sender(1, 2, Some((0, (1, 1)))));
    port.stream.waiters.push_back(sender(3, 4, None));
    port.stream.output.extend([1, 2]);
    assert_eq!(
        port.receive_ended(None),
        [
            Event::Replied {
                sender: sender(1, 2, Some((0, (1, 1)))),
                result: Err("closed".to_owned())
            },
            Event::Replied {
                sender: sender(3, 4, None),
                result: Err("closed".to_owned())
            },
        ]
    );
    assert_eq!(port.stream.waiters.len(), 0);
}

/// A failed write ends each send that arrived while the port was busy
/// with `{error, closed}` (`tcp_send_or_shutdown_error`,
/// inet_drv.c:13663-13682), and cancels its send timer with the reply.
#[test]
fn ends_the_waiters_at_a_failed_write() {
    let (local, _peer) = Socket::pair(Domain::UNIX, Type::STREAM, None).unwrap();
    let mut port = Inet::new(SocketType::Stream).copy(local);
    port.stream
        .waiters
        .push_back(sender(3, 4, Some((0, (1, 1)))));
    assert_eq!(
        port.send_failed(None),
        [Event::Replied {
            sender: sender(3, 4, Some((0, (1, 1)))),
            result: Err("closed".to_owned())
        }]
    );
    assert!(port.stream.closed_send);
    assert_eq!(
        port.send(&output_bytes(Reference(6), b"next"), ProcessId(5), SEED),
        Ok(Output::Sent(vec![Event::Replied {
            sender: sender(5, 6, None),
            result: Err("closed".to_owned())
        }]))
    );
}

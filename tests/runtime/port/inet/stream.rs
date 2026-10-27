use std::io::{Read, Write};

use socket2::{Domain, Socket, Type};

use super::{
    super::{Control, MAGIC},
    Event, Receiver, Sender,
};
use crate::{
    runtime::port::inet::{Inet, SocketType},
    term::{ProcessId, Reference},
};

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

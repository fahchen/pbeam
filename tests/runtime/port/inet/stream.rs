use std::io::Write;

use socket2::{Domain, Socket, Type};

use super::{Event, Receiver};
use crate::{
    runtime::port::inet::{Inet, SocketType},
    term::ProcessId,
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

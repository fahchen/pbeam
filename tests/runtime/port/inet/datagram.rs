use super::*;
use crate::runtime::port::inet::{Receiver, SocketType};

#[test]
fn reads_no_datagram_without_a_receive() {
    // A poll event that comes after the end of the receive, as after a
    // timeout on another scheduler, leaves the datagram in the socket.
    let mut port = Inet::new(SocketType::Datagram);
    let caller = crate::term::ProcessId(1);
    assert_eq!(
        port.control(0x03f1_a301, &[1, 2, 2], caller).unwrap(),
        Control::Reply(vec![1])
    );
    assert!(port.receive_datagram().is_empty());
}

#[test]
fn ends_the_receive_at_a_failed_read() {
    // A read error other than `ERRNO_BLOCK` ends the receive with its name
    // (`async_error`, inet_drv.c:15473-15489): a connected socket gets the
    // ICMP error of a send to a closed port as `econnrefused`.
    let closed = std::net::UdpSocket::bind("127.0.0.1:0").unwrap();
    let to = closed.local_addr().unwrap();
    drop(closed);
    let caller = crate::term::ProcessId(1);
    let mut port = Inet::new(SocketType::Datagram);
    port.control(0x03f1_a301, &[1, 2, 2], caller).unwrap();
    let socket = port.socket.as_ref().unwrap();
    socket.connect(&to.into()).unwrap();
    socket.send(b"x").unwrap();
    let receiver = Receiver {
        id: 0,
        caller,
        timer: None,
    };
    let mut events = Vec::new();
    for _ in 0..100 {
        port.stream.receiver = Some(receiver);
        events = port.receive_datagram();
        if !events.is_empty() {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
    assert_eq!(
        events,
        vec![Event::Received {
            receiver,
            result: Err("econnrefused".to_owned()),
        }]
    );
}

#[test]
fn gives_the_error_of_a_failed_read_to_the_owner_of_an_active_socket() {
    // A read error other than `ERRNO_BLOCK` on an active socket gives
    // `{udp_error, S, Reason}` (`packet_error_message`,
    // inet_drv.c:4245-4267, 15484-15488). `udp_inet` has no connect, so
    // the test connects the host socket to a closed port.
    let closed = std::net::UdpSocket::bind("127.0.0.1:0").unwrap();
    let to = closed.local_addr().unwrap();
    drop(closed);
    let caller = crate::term::ProcessId(1);
    let mut port = Inet::new(SocketType::Datagram);
    port.control(0x03f1_a301, &[1, 2, 2], caller).unwrap();
    port.control(0x03f1_a307, &[22, 0, 0, 0, 1], caller)
        .unwrap();
    let socket = port.socket.as_ref().unwrap();
    socket.connect(&to.into()).unwrap();
    socket.send(b"x").unwrap();
    let mut events = Vec::new();
    for _ in 0..100 {
        events = port.receive_active_datagrams();
        if !events.is_empty() {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
    assert_eq!(
        events,
        vec![Event::DatagramError("econnrefused".to_owned())]
    );
}

#[test]
fn reads_nothing_at_once_when_a_connected_socket_becomes_active() {
    // `{active, once}` of a connected `udp_inet` socket only replies, so a
    // datagram in the socket is not read for a receive that waits; the
    // poll gives it to the owner (inet_drv.c:4335-4357).
    let caller = crate::term::ProcessId(1);
    let mut port = Inet::new(SocketType::Datagram);
    port.control(0x03f1_a301, &[1, 2, 2], caller).unwrap();
    let request = [0, 0, 0, 0, 1, 0, 9, 127, 0, 0, 1];
    assert_eq!(
        port.control(0x03f1_a303, &request, caller).unwrap(),
        Control::Reply(vec![1])
    );
    assert_eq!(
        port.control(0x03f1_a307, &[22, 0, 0, 0, 2], caller)
            .unwrap(),
        Control::Reply(vec![1])
    );
}

#[test]
fn ends_the_receive_of_a_local_socket_at_a_failed_read() {
    // A read error other than `ERRNO_BLOCK` ends the receive
    // (inet_drv.c:15473-15489). An unconnected `local` stream socket in
    // place of the datagram socket makes `recvfrom` fail at once: the host
    // gives `enotconn` (macOS) or `einval` (Linux).
    let caller = crate::term::ProcessId(1);
    let mut port = Inet::new(SocketType::Datagram);
    port.control(0x03f1_a301, &[5, 2, 0], caller).unwrap();
    port.socket =
        Some(socket2::Socket::new(socket2::Domain::UNIX, socket2::Type::STREAM, None).unwrap());
    let receiver = Receiver {
        id: 0,
        caller,
        timer: None,
    };
    port.stream.receiver = Some(receiver);
    let events = port.receive_datagram();
    let name = if cfg!(target_os = "linux") {
        "einval"
    } else {
        "enotconn"
    };
    assert_eq!(
        events,
        vec![Event::Received {
            receiver,
            result: Err(name.to_owned()),
        }]
    );
}

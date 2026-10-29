use super::*;
use crate::runtime::capability::{AllowAll, Policy};
use crate::runtime::process::tests::frame;
use crate::runtime::sockets::{IoCounters, OtpOptions, Sockets};
use crate::term::AtomTable;
use socket2::{Domain, Socket, Type};

/// Return a scheduler whose current process is a new root process.
fn scheduler() -> Scheduler {
    let mut scheduler = Scheduler::new(AtomTable::new());
    scheduler
        .start(frame(), vec![], AllowAll.initial())
        .unwrap();
    scheduler.schedule_next().unwrap();
    scheduler
}

/// Keep a new IPv4 stream socket that `change` sets up as a socket
/// resource of the current process, and return its key and term.
fn resource(scheduler: &mut Scheduler, change: impl FnOnce(&mut SocketState)) -> (Reference, Term) {
    let socket = Socket::new(Domain::IPV4, Type::STREAM, None).unwrap();
    socket.set_nonblocking(true).unwrap();
    let mut state = SocketState {
        socket: Some(socket),
        domain: libc::AF_INET,
        kind: libc::SOCK_STREAM,
        protocol: libc::IPPROTO_TCP,
        owner: Some(scheduler.current_process().id),
        closer: None,
        read: 0,
        write: 0,
        registered: false,
        meta: Term::static_atom("undefined"),
        connector: None,
        acceptor: None,
        reader: None,
        writer: None,
        counters: IoCounters::default(),
        options: OtpOptions::default(),
        select_read: false,
        recv_count_done: 0,
    };
    change(&mut state);
    let reference = scheduler.make_magic_ref().unwrap();
    lock(&scheduler.node().resources.kind::<Sockets>()).insert(reference, state);
    (reference, Term::Reference(reference))
}

/// Keep a new IPv4 datagram socket that `change` sets up as a socket
/// resource of the current process, and return its key and term.
fn datagram(scheduler: &mut Scheduler) -> (Reference, Term) {
    let socket = Socket::new(Domain::IPV4, Type::DGRAM, None).unwrap();
    socket.set_nonblocking(true).unwrap();
    socket
        .bind(&std::net::SocketAddr::from(([127, 0, 0, 1], 0)).into())
        .unwrap();
    let state = SocketState {
        socket: Some(socket),
        domain: libc::AF_INET,
        kind: libc::SOCK_DGRAM,
        protocol: libc::IPPROTO_UDP,
        owner: Some(scheduler.current_process().id),
        closer: None,
        read: 0,
        write: 0,
        registered: false,
        meta: Term::static_atom("undefined"),
        connector: None,
        acceptor: None,
        reader: None,
        writer: None,
        counters: IoCounters::default(),
        options: OtpOptions::default(),
        select_read: false,
        recv_count_done: 0,
    };
    let reference = scheduler.make_magic_ref().unwrap();
    lock(&scheduler.node().resources.kind::<Sockets>()).insert(reference, state);
    (reference, Term::Reference(reference))
}

/// Return the text of the runtime error of `result`.
fn error_text(result: Result<BifResult, BeamError>) -> String {
    match result {
        Err(error) => error.to_string(),
        Ok(other) => panic!("a runtime error, not {other:?}"),
    }
}

/// Return the printed result of `result`.
fn printed(scheduler: &Scheduler, result: Result<BifResult, BeamError>) -> String {
    match result.unwrap() {
        BifResult::Value(value) => {
            String::from_utf8(crate::term::print_term(&value, &scheduler.atoms)).unwrap()
        }
        other => panic!("a value, not {other:?}"),
    }
}

#[test]
fn stops_the_run_at_a_second_writer_or_reader() {
    // ERTS queues a writer or reader of another process
    // (unix_socket_syncio.c:6809-6850, 9894-9927). PBeam has no queue. The
    // waiting process of a socket of a fixture cannot keep waiting while a
    // test process calls, so a unit test builds the state.
    let mut scheduler = scheduler();
    let other = (ProcessId(u64::MAX), Term::Reference(Reference(0)));
    let (_, socket) = resource(&mut scheduler, |state| {
        state.writer = Some(other.clone());
        state.reader = Some(other.clone());
    });
    let reference = Term::Reference(scheduler.make_ref().unwrap());
    let data = Term::binary(vec![1]);
    let zero = Term::Integer(0);
    assert!(
        error_text(send(&mut scheduler, &socket, &data, &zero, &reference))
            .contains("a second writer process is not supported")
    );
    assert!(
        error_text(recv(&mut scheduler, &socket, &zero, &zero, &reference))
            .contains("a second reader process is not supported")
    );
}

#[test]
fn checks_the_recv_flags_and_stops_the_run_at_a_length_above_the_ssize_t_range() {
    // `prim_socket:recv/4` encodes the flags of a list
    // (prim_socket.erl:824-830), so only `nif_recv/4` gets a flag integer
    // outside the `int` range (prim_socket_nif.c:7081-7091). A length above
    // the `ssize_t` range makes ERTS give a term of the integer itself
    // (prim_socket_nif.c:7092-7094).
    let mut scheduler = scheduler();
    let (_, socket) = resource(&mut scheduler, |_| {});
    let reference = Term::Reference(scheduler.make_ref().unwrap());
    let zero = Term::Integer(0);
    let huge = Term::from_bigint(num_bigint::BigInt::from(1_u8) << 40_u32 << 30_u32);
    let result = recv(&mut scheduler, &socket, &zero, &huge, &reference);
    assert_eq!(
        printed(&scheduler, result),
        "{invalid,{integer_range,1180591620717411303424}}"
    );
    let above = Term::from_bigint(num_bigint::BigInt::from(1_u64 << 63));
    assert!(
        error_text(recv(&mut scheduler, &socket, &above, &zero, &reference))
            .contains("the length 9223372036854775808 above the ssize_t range")
    );
}

#[test]
fn gives_badarg_for_send_flags_that_are_not_an_integer() {
    // `prim_socket:send/4` encodes a flag list before `nif_send/4`
    // (prim_socket.erl:569-576), so only a direct call gives the NIF flags
    // that are not an integer; `enif_get_int` fails and `nif_send` gives
    // `badarg` (prim_socket_nif.c:6440-6509).
    let mut scheduler = scheduler();
    let (_, socket) = resource(&mut scheduler, |_| {});
    let reference = Term::Reference(scheduler.make_ref().unwrap());
    let data = Term::binary(Vec::new());
    let flags = Term::static_atom("ok");
    let result = send(&mut scheduler, &socket, &data, &flags, &reference);
    assert_eq!(
        format!("{:?}", result.unwrap()),
        format!("{:?}", BifResult::badarg())
    );
}

#[test]
fn gives_invalid_state_for_a_send_during_a_connect() {
    // `essio_send` refuses a send while a connect waits
    // (unix_socket_syncio.c:3012-3013). A loopback connect of a fixture can
    // end before the send, on Linux at once, so a unit test builds the
    // state.
    let mut scheduler = scheduler();
    let owner = scheduler.current_process().id;
    let (_, socket) = resource(&mut scheduler, |state| {
        state.connector = Some((owner, Term::Reference(Reference(0))));
    });
    let reference = Term::Reference(scheduler.make_ref().unwrap());
    let result = send(
        &mut scheduler,
        &socket,
        &Term::binary(vec![]),
        &Term::Integer(0),
        &reference,
    );
    assert_eq!(printed(&scheduler, result), "{error,{invalid,state}}");
}

#[test]
fn counts_a_failed_send_and_releases_the_current_writer() {
    // `send_check_fail` counts `write_fails` and releases the current
    // writer for each errno but `EINVAL` (unix_socket_syncio.c:6998-7033).
    // A send on a socket that does not connect gives `EPIPE` on Linux and
    // `ENOTCONN` on macOS.
    let mut scheduler = scheduler();
    let owner = scheduler.current_process().id;
    let (key, socket) = resource(&mut scheduler, |state| {
        state.writer = Some((owner, Term::Reference(Reference(0))));
    });
    let reference = Term::Reference(scheduler.make_ref().unwrap());
    let result = send(
        &mut scheduler,
        &socket,
        &Term::binary(vec![1]),
        &Term::Integer(0),
        &reference,
    );
    let expected = if cfg!(target_os = "linux") {
        "{error,epipe}"
    } else {
        "{error,enotconn}"
    };
    assert_eq!(printed(&scheduler, result), expected);
    let sockets = scheduler.node().resources.kind::<Sockets>();
    let state = lock(&sockets).get(key).unwrap();
    let state = lock(&state);
    assert_eq!(
        (state.counters.write_tries, state.counters.write_fails),
        (1, 1)
    );
    assert!(state.writer.is_none());
}

#[test]
fn clears_the_current_reader_and_writer_at_their_exit() {
    // `essio_down_reader` and `essio_down_writer` clear the current reader
    // and writer and keep the socket open (unix_socket_syncio.c:10632-10716).
    let mut scheduler = scheduler();
    let other = ProcessId(u64::MAX);
    let pending = (other, Term::Reference(Reference(0)));
    let (key, _) = resource(&mut scheduler, |state| {
        state.reader = Some(pending.clone());
        state.writer = Some(pending.clone());
    });
    let poll = scheduler.node().poll.clone();
    let sockets = scheduler.node().resources.kind::<Sockets>();
    let (aborted, registry) = lock(&sockets).monitor_down(key, other, &poll);
    assert!(aborted.is_empty() && registry.is_none());
    let state = lock(&sockets).get(key).unwrap();
    let state = lock(&state);
    assert!(state.reader.is_none() && state.writer.is_none() && state.socket.is_some());
}

#[test]
fn counts_each_read_attempt_of_a_timed_read() {
    // `essio_recvfrom` increments `read_tries` before each `recvfrom(2)`
    // (`unix_socket_syncio.c:3919-3920`). A read of the `zero` reference that
    // finds no data counts one attempt and returns `{error, timeout}`, and a
    // read with a reference that finds no data counts one attempt and returns
    // `select`; its later attempt counts another.
    let mut scheduler = scheduler();
    let (key, socket) = datagram(&mut scheduler);
    let counters = |scheduler: &mut Scheduler| {
        let sockets = scheduler.node().resources.kind::<Sockets>();
        let state = lock(&sockets).get(key).unwrap();
        lock(&state).counters.read_tries
    };
    assert_eq!(counters(&mut scheduler), 0);
    let zero = Term::Atom(scheduler.atoms.put("zero").unwrap());
    let result = super::recvfrom(
        &mut scheduler,
        &socket,
        &Term::Integer(0),
        &Term::Integer(0),
        &zero,
    );
    assert_eq!(printed(&scheduler, result), "timeout");
    assert_eq!(counters(&mut scheduler), 1);
    let result = super::recvfrom(
        &mut scheduler,
        &socket,
        &Term::Integer(0),
        &Term::Integer(0),
        &Term::Reference(Reference(0)),
    );
    assert_eq!(printed(&scheduler, result), "select");
    assert_eq!(counters(&mut scheduler), 2);
}

#[test]
fn checks_the_arguments_of_sendmsg_and_recvmsg_in_the_nif_order() {
    // `prim_socket:sendmsg/4` and `recvmsg/5` encode their arguments
    // (prim_socket.erl:645-726, 840-851), so only a direct NIF call gives
    // these terms: `nif_sendmsg` checks the socket, `Ref` and `Msg`, then
    // `Flags` (prim_socket_nif.c:6636-6653), and `nif_recvmsg` checks the
    // socket, `Ref`, `BufSz`, `CtrlSz`, and `Flags`
    // (prim_socket_nif.c:7287-7389).
    let mut scheduler = scheduler();
    let (_, socket) = resource(&mut scheduler, |_| {});
    let reference = Term::Reference(scheduler.make_ref().unwrap());
    let message = Term::Map(crate::term::Map::new());
    let zero = Term::Integer(0);
    let atom = Term::static_atom("ok");
    let big = Term::Integer(1 << 40);
    let negative = Term::Integer(-1);
    let above = Term::from_bigint(num_bigint::BigInt::from(1_u64 << 63));
    let iov = Term::Nil;
    let badarg = format!("{:?}", BifResult::badarg());
    let send = |scheduler: &mut Scheduler,
                socket: &Term,
                message: &Term,
                flags: &Term,
                reference: &Term| {
        format!(
            "{:?}",
            sendmsg(scheduler, socket, message, flags, reference, &iov).unwrap()
        )
    };
    assert_eq!(
        send(&mut scheduler, &zero, &message, &zero, &reference),
        badarg
    );
    assert_eq!(
        send(&mut scheduler, &socket, &message, &zero, &zero),
        badarg
    );
    assert_eq!(
        send(&mut scheduler, &socket, &zero, &zero, &reference),
        badarg
    );
    assert_eq!(
        send(&mut scheduler, &socket, &message, &atom, &reference),
        badarg
    );
    let result = sendmsg(&mut scheduler, &socket, &message, &big, &reference, &iov);
    assert_eq!(
        printed(&scheduler, result),
        "{invalid,{integer_range,1099511627776}}"
    );
    let receive = |scheduler: &mut Scheduler, arguments: [&Term; 5]| {
        recvmsg(
            scheduler,
            arguments[0],
            arguments[1],
            arguments[2],
            arguments[3],
            arguments[4],
        )
    };
    for arguments in [
        [&zero, &zero, &zero, &zero, &reference],
        [&socket, &zero, &zero, &zero, &atom],
        [&socket, &atom, &zero, &zero, &reference],
        [&socket, &zero, &atom, &zero, &reference],
        [&socket, &zero, &zero, &atom, &reference],
    ] {
        let result = receive(&mut scheduler, arguments);
        assert_eq!(format!("{:?}", result.unwrap()), badarg);
    }
    let result = receive(
        &mut scheduler,
        [&socket, &negative, &zero, &zero, &reference],
    );
    assert_eq!(printed(&scheduler, result), "{invalid,{integer_range,-1}}");
    let result = receive(
        &mut scheduler,
        [&socket, &zero, &negative, &zero, &reference],
    );
    assert_eq!(printed(&scheduler, result), "{invalid,{integer_range,-1}}");
    let result = receive(&mut scheduler, [&socket, &zero, &zero, &big, &reference]);
    assert_eq!(
        printed(&scheduler, result),
        "{invalid,{integer_range,1099511627776}}"
    );
    let result = receive(&mut scheduler, [&socket, &above, &zero, &zero, &reference]);
    assert!(error_text(result).contains("the length 9223372036854775808 above the ssize_t range"));
}

#[test]
fn checks_the_state_address_and_iov_of_sendmsg_and_the_state_of_recvmsg() {
    // `essio_sendmsg` checks the state, then `addr`, then the I/O vector
    // (unix_socket_syncio.c:3127-3260). `prim_socket:sendmsg/4` encodes a
    // valid address and checks the I/O vector first (prim_socket.erl:645-726),
    // so a fixture gives none of these results.
    let mut scheduler = scheduler();
    let reference = Term::Reference(scheduler.make_ref().unwrap());
    let zero = Term::Integer(0);
    let atoms = &scheduler.atoms;
    let entry = |name: &str, value: Term| (Term::Atom(atoms.put(name).unwrap()), value);
    let family = entry("family", Term::Atom(atoms.put("inet").unwrap()));
    let address = Term::Map(crate::term::Map::from_pairs(vec![family], atoms).unwrap());
    let message =
        Term::Map(crate::term::Map::from_pairs(vec![entry("addr", address)], atoms).unwrap());
    let empty = Term::Map(crate::term::Map::new());
    let (_, closed) = resource(&mut scheduler, |state| {
        state.write = CLOSED;
        state.read = CLOSED;
    });
    let result = sendmsg(
        &mut scheduler,
        &closed,
        &empty,
        &zero,
        &reference,
        &Term::Nil,
    );
    assert_eq!(printed(&scheduler, result), "{error,closed}");
    let result = recvmsg(&mut scheduler, &closed, &zero, &zero, &zero, &reference);
    let (_, socket) = datagram(&mut scheduler);
    assert_eq!(printed(&scheduler, result), "{error,closed}");
    let result = sendmsg(
        &mut scheduler,
        &socket,
        &message,
        &zero,
        &reference,
        &Term::Nil,
    );
    assert_eq!(printed(&scheduler, result), "{invalid,addr}");
    let result = sendmsg(&mut scheduler, &socket, &empty, &zero, &reference, &zero);
    assert_eq!(printed(&scheduler, result), "{invalid,iov}");
    let long: Vec<Term> = (0..=IOV_MAX).map(|_| Term::binary(vec![1])).collect();
    let result = sendmsg(
        &mut scheduler,
        &socket,
        &empty,
        &zero,
        &reference,
        &Term::list_from(long),
    );
    assert_eq!(printed(&scheduler, result), "{invalid,iov}");
}

#[test]
fn stops_the_run_for_recvmsg_on_a_socket_whose_source_address_is_not_known() {
    // The safe `recvmsg` of nix reads an address that it does not
    // initialize when the kernel gives one of length 0, as on a local socket
    // of an unbound peer (row "NIF stubs").
    let mut scheduler = scheduler();
    let (_, socket) = resource(&mut scheduler, |state| state.domain = libc::AF_UNIX);
    let reference = Term::Reference(scheduler.make_ref().unwrap());
    let zero = Term::Integer(0);
    let result = recvmsg(&mut scheduler, &socket, &zero, &zero, &zero, &reference);
    assert!(
        error_text(result)
            .contains("prim_socket:nif_recvmsg/5: the flags 0 on a socket of the domain 1")
    );
}

#[test]
fn encodes_each_message_flag_of_the_host_and_the_other_bits() {
    // `esock_encode_msg_flags` names each flag of `esock_msg_flags` and
    // gives the other bits as one integer (prim_socket_nif.c:14235-14251).
    let mut scheduler = scheduler();
    let trunc = MSG_FLAGS
        .iter()
        .find(|(name, _)| *name == "trunc")
        .unwrap()
        .1[HOST];
    let ctrunc = MSG_FLAGS
        .iter()
        .find(|(name, _)| *name == "ctrunc")
        .unwrap()
        .1[HOST];
    let term = message_flags_term(&mut scheduler, trunc | ctrunc | 1 << 29).unwrap();
    let text = String::from_utf8(crate::term::print_term(&term, &scheduler.atoms)).unwrap();
    assert_eq!(text, "[ctrunc,trunc,536870912]");
    let term = message_flags_term(&mut scheduler, 0).unwrap();
    assert_eq!(term, Term::Nil);
}

#[test]
fn checks_the_arguments_of_sendmmsg_and_recvmmsg_in_the_nif_order() {
    // `prim_socket:sendmmsg/4` encodes `Flags` and each message
    // (prim_socket.erl:812-820), so only a direct NIF call gives these
    // terms: `nif_sendmmsg` checks the socket, `Ref` and `Msgs`, `Flags`,
    // and then the length of `Msgs` (prim_socket_nif.c:6707-6730).
    // `nif_recvmmsg` checks the socket, `Ref`, the lengths, and `Flags`
    // (prim_socket_nif.c:7467-7508).
    let mut scheduler = scheduler();
    let (_, socket) = resource(&mut scheduler, |_| {});
    let reference = Term::Reference(scheduler.make_ref().unwrap());
    let zero = Term::Integer(0);
    let one = Term::Integer(1);
    let atom = Term::static_atom("ok");
    let big = Term::Integer(1 << 40);
    let above = Term::from_bigint(num_bigint::BigInt::from(1_u64 << 63));
    let improper = Term::list_with_tail(vec![Term::Map(crate::term::Map::new())], atom.clone());
    let badarg = format!("{:?}", BifResult::badarg());
    for [socket, messages, flags, reference] in [
        [&zero, &Term::Nil, &zero, &reference],
        [&socket, &Term::Nil, &zero, &zero],
        [&socket, &zero, &zero, &reference],
        [&socket, &Term::Nil, &atom, &reference],
        [&socket, &improper, &zero, &reference],
    ] {
        let result = sendmmsg(&mut scheduler, socket, messages, flags, reference);
        assert_eq!(format!("{:?}", result.unwrap()), badarg);
    }
    let result = sendmmsg(&mut scheduler, &socket, &improper, &big, &reference);
    assert_eq!(
        printed(&scheduler, result),
        "{invalid,{integer_range,1099511627776}}"
    );
    let result = recvmmsg(
        &mut scheduler,
        &zero,
        [&one, &zero, &zero, &zero, &reference],
    );
    assert_eq!(format!("{:?}", result.unwrap()), badarg);
    let result = recvmmsg(
        &mut scheduler,
        &socket,
        [&one, &zero, &zero, &big, &reference],
    );
    assert_eq!(
        printed(&scheduler, result),
        "{invalid,{integer_range,1099511627776}}"
    );
    let result = recvmmsg(
        &mut scheduler,
        &socket,
        [&one, &zero, &above, &zero, &reference],
    );
    assert!(error_text(result).contains(
        "prim_socket:nif_recvmmsg/6: the length 9223372036854775808 above the ssize_t range"
    ));
}

#[cfg(target_os = "linux")]
#[test]
fn checks_the_state_and_the_messages_of_sendmmsg_and_the_state_of_recvmmsg() {
    // `essio_sendmmsg` checks the state, then each element, then the `addr`
    // of each message (unix_socket_syncio.c:4342-4431), and `essio_recvmmsg`
    // checks the state (unix_socket_syncio.c:4148-4153).
    // `prim_socket:sendmmsg/4` encodes each message and a valid address
    // (prim_socket.erl:812-820, 1194-1208), so a fixture gives none of these
    // results. A local socket of an unbound peer gives a source address of
    // length 0, which the safe `recvmsg` of nix does not initialize (row
    // "NIF stubs").
    let mut scheduler = scheduler();
    let reference = Term::Reference(scheduler.make_ref().unwrap());
    let zero = Term::Integer(0);
    let one = Term::Integer(1);
    let atoms = &scheduler.atoms;
    let entry = |name: &str, value: Term| (Term::Atom(atoms.put(name).unwrap()), value);
    let family = entry("family", Term::Atom(atoms.put("inet").unwrap()));
    let address = Term::Map(crate::term::Map::from_pairs(vec![family], atoms).unwrap());
    let message =
        Term::Map(crate::term::Map::from_pairs(vec![entry("addr", address)], atoms).unwrap());
    let (_, closed) = resource(&mut scheduler, |state| {
        state.write = CLOSED;
        state.read = CLOSED;
    });
    let result = sendmmsg(&mut scheduler, &closed, &Term::Nil, &zero, &reference);
    assert_eq!(printed(&scheduler, result), "{error,closed}");
    let result = recvmmsg(
        &mut scheduler,
        &closed,
        [&one, &zero, &zero, &zero, &reference],
    );
    assert_eq!(printed(&scheduler, result), "{error,closed}");
    let (_, socket) = datagram(&mut scheduler);
    let messages = Term::list_from([zero.clone()]);
    let result = sendmmsg(&mut scheduler, &socket, &messages, &zero, &reference);
    assert_eq!(
        format!("{:?}", result.unwrap()),
        format!("{:?}", BifResult::badarg())
    );
    let messages = Term::list_from([message]);
    let result = sendmmsg(&mut scheduler, &socket, &messages, &zero, &reference);
    assert_eq!(printed(&scheduler, result), "{invalid,addr}");
    let (_, local) = resource(&mut scheduler, |state| state.domain = libc::AF_UNIX);
    let result = recvmmsg(
        &mut scheduler,
        &local,
        [&one, &zero, &zero, &zero, &reference],
    );
    assert!(
        error_text(result)
            .contains("prim_socket:nif_recvmmsg/6: the flags 0 on a socket of the domain 1")
    );
}

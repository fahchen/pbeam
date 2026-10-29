use super::*;
use crate::runtime::capability::{AllowAll, Policy};
use crate::runtime::process::tests::frame;
use crate::runtime::sockets::{IoCounters, OtpOptions, Sockets};
use crate::term::AtomTable;
use socket2::{Domain, Socket, Type};

/// Return a scheduler whose current process is a new root process.
fn scheduler() -> Scheduler {
    let mut scheduler = Scheduler::new(AtomTable::new());
    scheduler.start(frame(), vec![], AllowAll.initial());
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
    let poll = Arc::clone(&scheduler.node().poll);
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

use super::*;
use crate::runtime::capability::{AllowAll, Policy};
use crate::runtime::process::tests::frame;
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

/// Keep `socket` as a socket resource of the current process with the
/// read state `read` and the current acceptor `acceptor`, and return its
/// term.
fn resource(scheduler: &mut Scheduler, socket: Socket, read: u32, acceptor: Option<Term>) -> Term {
    socket.set_nonblocking(true).unwrap();
    let owner = scheduler.current_process().id;
    let reference = scheduler.make_magic_ref().unwrap();
    lock(&scheduler.node().resources.kind::<Sockets>()).insert(
        reference,
        SocketState {
            socket: Some(socket),
            domain: libc::AF_INET,
            kind: libc::SOCK_STREAM,
            protocol: libc::IPPROTO_TCP,
            owner: Some(owner),
            closer: None,
            read,
            write: 0,
            registered: false,
            meta: Term::static_atom("undefined"),
            connector: None,
            acceptor: acceptor.map(|reference| (owner, reference)),
            reader: None,
            writer: None,
            counters: IoCounters::default(),
            options: OtpOptions::default(),
            select_read: false,
            recv_count_done: 0,
        },
    );
    Term::Reference(reference)
}

/// Return the name of the atom of the result `result`.
fn atom_result(scheduler: &Scheduler, result: BifResult) -> String {
    match result {
        BifResult::Value(Term::Atom(ref atom)) => scheduler.atoms.name_of(atom).to_owned(),
        other => panic!("an atom, not {other:?}"),
    }
}

#[test]
fn gives_select_sent_when_the_select_message_of_the_current_acceptor_went_out() {
    // An accept of the current acceptor that blocks again cancels its
    // select; when the message went out, the cancel gives no
    // `ERL_NIF_SELECT_READ_CANCELLED` and the accept gives `select_sent`
    // (unix_socket_syncio.c:2673-2701). Only a race reaches it: the poll
    // thread sends the message and another accept takes the connection.
    let mut scheduler = scheduler();
    let listener = Socket::new(Domain::IPV4, Type::STREAM, None).unwrap();
    listener
        .bind(&std::net::SocketAddr::from(([127, 0, 0, 1], 0)).into())
        .unwrap();
    listener.listen(1).unwrap();
    let fd = listener.as_raw_fd();
    let socket = resource(&mut scheduler, listener, LISTENING, None);
    let first = Term::Reference(scheduler.make_ref().unwrap());
    let result = accept(&mut scheduler, &socket, &first).unwrap();
    assert_eq!(atom_result(&scheduler, result), "select");
    // The poll thread removes a select when it sends its message.
    assert!(scheduler.node().poll.cancel_select(fd, Direction::Read));
    let second = Term::Reference(scheduler.make_ref().unwrap());
    let result = accept(&mut scheduler, &socket, &second).unwrap();
    assert_eq!(atom_result(&scheduler, result), "select_sent");
}

#[test]
fn clears_the_current_acceptor_and_keeps_accepting_after_an_accept_error() {
    // An accept of the current acceptor that fails with an errno other
    // than `EAGAIN` releases the acceptor and keeps the state `accepting`
    // (unix_socket_syncio.c:2703-2733). A socket that does not listen
    // gives `EINVAL`; no fixture makes a waiting acceptor fail so.
    let mut scheduler = scheduler();
    let plain = Socket::new(Domain::IPV4, Type::STREAM, None).unwrap();
    let old = Term::Reference(scheduler.make_ref().unwrap());
    let socket = resource(&mut scheduler, plain, ACCEPTING, Some(old));
    let reference = Term::Reference(scheduler.make_ref().unwrap());
    let result = accept(&mut scheduler, &socket, &reference).unwrap();
    let BifResult::Value(Term::Tuple(ref tuple)) = result else {
        panic!("an error tuple, not {result:?}");
    };
    let Term::Atom(reason) = &tuple[1] else {
        panic!("an errno atom");
    };
    assert_eq!(scheduler.atoms.name_of(reason), "einval");
    let (_, state) = socket_argument(&scheduler, &socket).unwrap();
    let state = lock(&state);
    assert!(state.acceptor.is_none());
    assert_eq!(state.read & ACCEPTING, ACCEPTING);
}

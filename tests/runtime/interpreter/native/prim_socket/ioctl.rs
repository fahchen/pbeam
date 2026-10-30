use super::*;
use crate::runtime::capability::{AllowAll, Policy};
use crate::runtime::process::tests::frame;
use crate::runtime::sockets::{IoCounters, OtpOptions, SocketState, Sockets};
use crate::term::AtomTable;
use socket2::{Domain, Socket, Type};

#[test]
fn gives_badarg_for_a_request_that_is_not_an_integer() {
    // `prim_socket:ioctl/2,3,4` pass only integers to `nif_ioctl`
    // (prim_socket.erl:1013-1065), so no Erlang call gives the NIF another
    // request; `GET_ULONG` rejects it with `badarg`
    // (prim_socket_nif.c:13811-13814).
    let mut scheduler = Scheduler::new(AtomTable::new());
    scheduler.start(frame(), vec![], AllowAll.initial());
    scheduler.schedule_next().unwrap();
    let socket = Socket::new(Domain::IPV4, Type::DGRAM, None).unwrap();
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
        acceptors: std::collections::VecDeque::new(),
        reader: None,
        writer: None,
        counters: IoCounters::default(),
        options: OtpOptions::default(),
        select_read: false,
        recv_count_done: 0,
        sendfile: None,
        sendfile_counters: None,
    };
    let reference = scheduler.make_magic_ref().unwrap();
    lock(&scheduler.node().resources.kind::<Sockets>()).insert(reference, state);
    let result = ioctl(
        &mut scheduler,
        &Term::Reference(reference),
        &[Term::static_atom("ok")],
    )
    .unwrap();
    assert_eq!(format!("{result:?}"), format!("{:?}", BifResult::badarg()));
}

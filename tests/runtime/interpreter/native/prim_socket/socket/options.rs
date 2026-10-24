use super::*;
use crate::term::AtomTable;

#[test]
fn gives_badarg_for_a_level_or_option_that_is_not_an_integer() {
    // `nif_setopt` and `nif_getopt` raise `error:badarg` for a level or an
    // option that is neither an integer nor a known atom
    // (prim_socket_nif.c:7835-7842, 7862-7865). `prim_socket` gives the NIFs
    // only integers and the atoms `socket` and `otp`
    // (prim_socket.erl:1237-1283), so no fixture reaches this case.
    let scheduler = &mut Scheduler::new(AtomTable::new());
    let Ok(BifResult::Raise { reason, .. }) = range_or_badarg(scheduler, &Term::Nil) else {
        panic!("no raise");
    };
    assert_eq!(reason, Term::static_atom("badarg"));
}

#[cfg(target_os = "linux")]
#[test]
fn allows_only_eperm_from_the_reset_of_so_priority() {
    // `socket_setopt` ignores a failed reset of `SO_PRIORITY` only for
    // `EPERM` (prim_socket_nif.c:11460-11469). An unprivileged process
    // can always set back the priority that it read, so no fixture
    // reaches the error.
    use nix::errno::Errno;
    assert_eq!(allow_eperm(Err(Errno::EPERM)), Ok(()));
    assert_eq!(allow_eperm(Err(Errno::EINVAL)), Err(Errno::EINVAL));
    assert_eq!(allow_eperm(Ok(())), Ok(()));
}

#[test]
fn raises_invalid_for_an_otp_option_number_without_a_getter() {
    // `esock_getopt_otp` raises `error:{invalid, {otp_socket_option, Opt}}`
    // for a number without a handler (prim_socket_nif.c:11692-11706).
    // `prim_socket` gives the NIF only the numbers of its `otp` options, and
    // each has a getter (prim_socket.erl:1237-1252), so no fixture reaches
    // this case.
    let scheduler = &mut Scheduler::new(AtomTable::new());
    let state = Arc::new(Mutex::new(SocketState {
        socket: None,
        domain: libc::AF_INET,
        kind: libc::SOCK_STREAM,
        protocol: 0,
        owner: None,
        closer: None,
        read: 0,
        write: 0,
        registered: false,
        meta: Term::Nil,
        connector: None,
        acceptor: None,
        reader: None,
        writer: None,
        counters: IoCounters::default(),
        options: OtpOptions::default(),
    }));
    let Ok(BifResult::Raise { reason, .. }) = getopt_otp(scheduler, &state, 1005) else {
        panic!("no raise");
    };
    let what = Term::tuple(vec![
        atom(scheduler, "otp_socket_option").unwrap(),
        Term::Integer(1005),
    ]);
    assert_eq!(
        reason,
        Term::tuple(vec![Term::static_atom("invalid"), what])
    );
}

#[test]
fn gives_invalid_data_size_for_a_binary_value_spec_of_4_gib() {
    // `esock_getopt_bin_opt` gives `{error, {invalid, data_size}}` for a
    // binary value spec that a `socklen_t` cannot hold
    // (prim_socket_nif.c:13187-13189). A fixture cannot make the binary
    // without writing 4 GiB; the zero pages of this one stay unused.
    let scheduler = &mut Scheduler::new(AtomTable::new());
    let host = Socket::new(socket2::Domain::IPV4, socket2::Type::DGRAM, None).unwrap();
    let spec = Term::binary(vec![0; 1 << 32]);
    let Ok(BifResult::Value(value)) =
        getopt_native(scheduler, &host, libc::SOL_SOCKET, libc::SO_TYPE, &spec)
    else {
        panic!("no value");
    };
    let reason = Term::tuple(vec![
        Term::static_atom("invalid"),
        atom(scheduler, "data_size").unwrap(),
    ]);
    assert_eq!(value, Term::tuple(vec![Term::static_atom("error"), reason]));
}

#[cfg(target_os = "linux")]
#[test]
fn stops_at_a_control_message_that_ip_pktoptions_does_not_give() {
    // The Linux kernel gives only `IP_PKTINFO`, `IP_TTL`, and `IP_TOS` for
    // `IP_PKTOPTIONS` (net/ipv4/ip_sockglue.c), so no fixture reaches
    // another control message.
    let scheduler = &mut Scheduler::new(AtomTable::new());
    let mut buffer = 20_usize.to_ne_bytes().to_vec();
    buffer.extend(libc::IPPROTO_IP.to_ne_bytes());
    buffer.extend(libc::IP_RECVTTL.to_ne_bytes());
    buffer.extend([0; 8]);
    assert!(pktoptions(scheduler, &buffer).is_err());
}

use super::*;
use crate::runtime::sockets::IoCounters;
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

#[test]
fn has_a_set_and_a_get_handler_for_each_host_option() {
    // `setopt` gives `{invalid, socket_option}` for an option without a set
    // form, and `getopt` expects a get handler for each option with a get
    // handler in ERTS. The ERTS tables have no set handler only for these
    // options (prim_socket_nif.c:3272-3274, 3377-3379, 3481-3488,
    // 3580-3586, 3654-3660), so each other option of the host table must
    // have a set form.
    let get_only: &[(&str, &str)] = &[
        ("socket", "type"),
        ("socket", "domain"),
        ("socket", "protocol"),
        ("ip", "mtu"),
        ("ip", "pktoptions"),
    ];
    let host = Socket::new(socket2::Domain::IPV4, socket2::Type::STREAM, None).expect("socket");
    let mut count = 0;
    for level in [
        libc::SOL_SOCKET,
        libc::IPPROTO_IP,
        libc::IPPROTO_IPV6,
        libc::IPPROTO_UDP,
        libc::IPPROTO_TCP,
    ] {
        for opt in 0..5000 {
            let Some(name) = option_name(level, opt) else {
                continue;
            };
            count += 1;
            assert!(
                set_form(name).is_some() || get_only.contains(&name),
                "{name:?}"
            );
            let get = set_form_only(name)
                || name == ("ip", "pktoptions")
                || get_host(&host, name, (level, opt)).is_some();
            assert!(get, "{name:?}");
        }
    }
    assert!(count > 40, "{count}");
}

#[cfg(target_os = "linux")]
#[test]
fn ends_ip_pktoptions_at_a_header_with_a_zero_length() {
    // A `cmsg_len` of 0 wraps the data size, so `esock_getopt_pktoptions`
    // ends the list (prim_socket_nif.c:13336-13339). `raw_get` cuts the
    // buffer to the size that the host gives, so no fixture reaches a
    // header after the last message.
    let scheduler = &mut Scheduler::new(AtomTable::new());
    let buffer = [0; std::mem::size_of::<libc::cmsghdr>()];
    assert_eq!(pktoptions(scheduler, &buffer).ok(), Some(Term::Nil));
}

#[cfg(target_os = "linux")]
#[test]
fn gives_the_value_of_an_ip_tos_control_message_of_ip_pktoptions() {
    // `esock_cmsg_encode_ip_tos` encodes the first data byte with
    // `encode_ip_tos` (prim_socket_nif.c:14335-14349, 16637-16816). ERTS
    // gives `#{native => 16, tos => #{tos => lowdelay, precedence =>
    // routine}, dscp => 4}` for the value 16. The CI kernel gives no
    // `IP_TOS` data in the default buffer, so no fixture reaches it.
    let scheduler = &mut Scheduler::new(AtomTable::new());
    let header = std::mem::size_of::<libc::cmsghdr>();
    let mut buffer = (header + 4).to_ne_bytes().to_vec();
    buffer.extend(libc::IPPROTO_IP.to_ne_bytes());
    buffer.extend(libc::IP_TOS.to_ne_bytes());
    buffer.extend([16, 0, 0, 0]);
    let [routine, lowdelay, kind] =
        ["routine", "lowdelay", "tos"].map(|name| atom(scheduler, name).expect("atom"));
    let tos = pairs(scheduler, &[("precedence", routine), ("tos", lowdelay)]).expect("atoms");
    let tos = map(scheduler, tos);
    let value = pairs(
        scheduler,
        &[
            ("native", Term::Integer(16)),
            ("tos", tos),
            ("dscp", Term::Integer(4)),
        ],
    )
    .expect("atoms");
    let value = map(scheduler, value);
    let message = pairs(
        scheduler,
        &[
            ("level", Term::Integer(0)),
            ("type", kind),
            ("data", Term::binary(vec![16, 0, 0, 0])),
            ("value", value),
        ],
    )
    .expect("atoms");
    let message = map(scheduler, message);
    assert_eq!(
        pktoptions(scheduler, &buffer).ok(),
        Some(Term::list_from([message]))
    );
}

use super::*;

#[test]
fn checks_the_command_map_as_nif_command() {
    // `prim_net:debug/1` always gives a map with `command => debug`, and
    // `nif_command/1` is not exported, so only a direct call reaches these
    // checks (`prim_net_nif.c:897-954`).
    let atoms = AtomTable::new();
    let atom = |name| Term::Atom(atoms.put(name).unwrap());
    let map = |pairs: Vec<(Term, Term)>| Term::Map(Map::from_pairs(pairs, &atoms).unwrap());
    let badarg = format!("{:?}", BifResult::badarg());
    let run = |argument: &Term| format!("{:?}", command(&atoms, argument).unwrap());
    assert_eq!(run(&atom("x")), badarg);
    assert_eq!(run(&map(vec![(atom("data"), atom("false"))])), badarg);
    assert_eq!(run(&map(vec![(atom("command"), atom("debug"))])), badarg);
    let other = map(vec![
        (atom("command"), atom("x")),
        (atom("data"), atom("false")),
    ]);
    let BifResult::Raise { reason, .. } = command(&atoms, &other).unwrap() else {
        panic!("the command raises");
    };
    let detail = Term::tuple(vec![atom("command"), atom("x")]);
    assert_eq!(reason, Term::tuple(vec![atom("invalid"), detail]));
}

#[test]
fn gives_minus_one_for_a_failed_gethostname() {
    assert_eq!(
        gethostname(Err(nix::Error::EINVAL)),
        Term::tuple(vec![Term::static_atom("error"), Term::Integer(-1)])
    );
}

#[test]
fn gives_the_reason_of_each_lookup_error_as_prim_net_nif() {
    // The host files of a test host give none of these errors, so the
    // probe of `tests/integration/net.rs` does not reach them
    // (`prim_net_nif.c:1108-1174`, `1280-1359`).
    let codes = [
        (eai::ADDRFAMILY, "eaddrfamily", "einval"),
        (eai::AGAIN, "eagain", "eagain"),
        (eai::BADFLAGS, "ebadflags", "ebadflags"),
        (eai::FAIL, "efail", "efail"),
        (eai::FAMILY, "efamily", "efamily"),
        (eai::MEMORY, "emem", "emem"),
        (eai::NODATA, "enodata", "einval"),
        (eai::NONAME, "enoname", "enoname"),
        (eai::SERVICE, "eservice", "einval"),
        (eai::SOCKTYPE, "esocktype", "einval"),
        (eai::SYSTEM, "esystem", "eperm"),
        (eai::OVERFLOW, "einval", "eoverflow"),
        (1000, "einval", "einval"),
    ];
    for (code, addrinfo, nameinfo) in codes {
        assert_eq!(addrinfo_error(code), addrinfo, "{code}");
        assert_eq!(nameinfo_error(code, libc::EPERM), nameinfo, "{code}");
    }
}

#[test]
fn encodes_each_family_and_socket_type_as_socket_util() {
    // `getaddrinfo` with NULL hints gives only `inet` and `inet6` entries
    // (`socket_util.c:2229-2259`, `2420-2452`).
    let atoms = AtomTable::new();
    let atom = |name| Term::Atom(atoms.put(name).unwrap());
    let families = [
        (libc::AF_INET, atom("inet")),
        (libc::AF_INET6, atom("inet6")),
        (libc::AF_LOCAL, atom("local")),
        (libc::AF_UNSPEC, atom("unspec")),
        (200, Term::Integer(200)),
    ];
    for (family, term) in families {
        assert_eq!(domain(&atoms, family).unwrap(), term);
    }
    let types = [
        (0, atom("any")),
        (libc::SOCK_STREAM, atom("stream")),
        (libc::SOCK_DGRAM, atom("dgram")),
        (libc::SOCK_RAW, atom("raw")),
        (libc::SOCK_SEQPACKET, atom("seqpacket")),
        (libc::SOCK_RDM, atom("rdm")),
        (200, Term::Integer(200)),
    ];
    for (value, term) in types {
        assert_eq!(socktype(&atoms, value).unwrap(), term);
    }
}

#[test]
fn stops_the_run_at_a_socket_address_of_another_family() {
    let atoms = AtomTable::new();
    let mut local = header(4, libc::AF_LOCAL);
    local.extend_from_slice(b"/x");
    for bytes in [vec![0], local, header(8, libc::AF_INET6)] {
        assert_eq!(
            encode_sockaddr(&atoms, &bytes).unwrap_err().to_string(),
            "prim_net:nif_getaddrinfo/3 socket address of this family is not supported"
        );
    }
}

#[test]
fn checks_the_names_of_nif_getaddrinfo() {
    // `prim_net:getaddrinfo/2` passes only lists and `undefined`, and not
    // two `undefined` names (`prim_net.erl:448-452`), so only a direct call
    // reaches these checks (`prim_net_nif.c:1221-1233`).
    let atoms = AtomTable::new();
    let atom = |name| Term::Atom(atoms.put(name).unwrap());
    let badarg = format!("{:?}", BifResult::badarg());
    let run =
        |host: &Term, service: &Term| format!("{:?}", getaddrinfo(&atoms, host, service).unwrap());
    let host = Term::list_from(b"127.0.0.1".map(|byte| Term::Integer(byte.into())));
    assert_eq!(run(&atom("x"), &host), badarg);
    assert_eq!(run(&host, &atom("x")), badarg);
    assert_eq!(run(&atom("undefined"), &atom("undefined")), badarg);
}

#[test]
fn decodes_no_socket_address_without_its_fields() {
    // `prim_socket:enc_sockaddr/1` adds each missing field of an `inet` or
    // an `inet6` address and checks the path of a `local` address, so only
    // a direct call reaches these checks (`socket_util.c:495-499`,
    // `1139-1152`, `1259-1310`, `1404-1409`).
    let atoms = AtomTable::new();
    let atom = |name| Term::Atom(atoms.put(name).unwrap());
    let map = |pairs: Vec<(Term, Term)>| Term::Map(Map::from_pairs(pairs, &atoms).unwrap());
    let family = |name| (atom("family"), atom(name));
    let port = (atom("port"), Term::Integer(80));
    let cases = [
        Term::Integer(1),
        map(vec![]),
        map(vec![family("inet")]),
        map(vec![family("inet"), port.clone()]),
        map(vec![family("inet6"), port]),
        map(vec![family("local"), (atom("path"), Term::Integer(1))]),
    ];
    for case in cases {
        assert_eq!(sockaddr(&atoms, &case).unwrap(), None, "{case:?}");
    }
}

#[test]
fn runs_the_service_lookups_on_a_dirty_io_thread() {
    // `nif_getservbyname/2` and `nif_getservbyport/2` have the flag
    // `ERL_NIF_DIRTY_JOB_IO_BOUND` (`prim_net_nif.c:5014-5015`).
    for name in ["nif_getservbyname", "nif_getservbyport"] {
        let binding = BINDINGS
            .iter()
            .find(|binding| binding.name == name)
            .unwrap();
        assert!(matches!(binding.job, Job::IoBound), "{name}");
    }
    let protocol = Term::list_from(b"tcp".map(|byte| Term::Integer(byte.into())));
    let result = getservbyport(&Term::Integer(80), &protocol);
    assert!(matches!(result, Ok(BifResult::Dirty(_))));
}

#[test]
fn raises_badarg_for_extra_that_is_not_a_map() {
    // `prim_net:getifaddrs/1` checks the map, so only a direct call
    // reaches this check (`prim_net_nif.c:1395-1398`).
    let atoms = AtomTable::new();
    let result = getifaddrs(&atoms, &Term::Integer(1)).unwrap();
    assert_eq!(format!("{result:?}"), format!("{:?}", BifResult::badarg()));
}

#[test]
fn gives_the_errno_of_a_failed_if_nameindex() {
    let atoms = AtomTable::new();
    let BifResult::Value(value) = if_names(&atoms, Err(nix::Error::ENOBUFS)).unwrap() else {
        panic!("if_names gives a value");
    };
    let reason = Term::Atom(atoms.put("enobufs").unwrap());
    assert_eq!(value, Term::tuple(vec![Term::static_atom("error"), reason]));
}

#[test]
fn encodes_each_interface_address_form_as_prim_net_nif() {
    // A test host has no point-to-point interface and no unknown packet
    // type, so the probe does not reach these forms
    // (`prim_net_nif.c:1630-1664`, `socket_util.c:2478-2722`).
    use nix::{ifaddrs::InterfaceAddress, net::if_::InterfaceFlags, sys::socket::SockaddrStorage};
    let atoms = AtomTable::new();
    let inet = |text: &str| {
        Some(SockaddrStorage::from(
            text.parse::<std::net::SocketAddr>().unwrap(),
        ))
    };
    let interface = |destination, broadcast| InterfaceAddress {
        interface_name: "x0".into(),
        flags: InterfaceFlags::IFF_UP | InterfaceFlags::IFF_POINTOPOINT,
        address: inet("[::1]:0"),
        netmask: None,
        broadcast,
        destination,
    };
    let entries = vec![
        ifaddrs::entry(interface(inet("10.0.0.1:0"), None))
            .ok()
            .unwrap(),
        ifaddrs::entry(interface(None, inet("10.0.0.255:0")))
            .ok()
            .unwrap(),
        ifaddrs::entry(interface(None, None)).ok().unwrap(),
        ifaddrs::Entry {
            name: b"p0".to_vec(),
            flags: 0,
            addr: ifaddrs::packet(3, 2, 1, 0, &[9; 8], 6).ok(),
            netmask: ifaddrs::packet(3, 2, 999, 200, &[9; 8], 8).ok(),
            extra: None,
        },
    ];
    let BifResult::Value(value) = ifaddrs::result(&atoms, Ok(entries)).unwrap() else {
        panic!("the result is a value");
    };
    let text = format!("{value:?}");
    let atom = |name: &str| format!("{:?}", Term::Atom(atoms.put(name).unwrap()));
    for name in [
        "dstaddr",
        "broadaddr",
        "pointopoint",
        "packet",
        "ether",
        "host",
    ] {
        assert!(text.contains(&atom(name)), "{name}");
    }
    assert!(text.contains("Integer(999)") && text.contains("Integer(200)"));
    assert!(ifaddrs::packet(3, 2, 1, 0, &[9; 8], 9).is_err());
    let errno = ifaddrs::result(&atoms, Err(ifaddrs::Failure::Errno(libc::EPERM))).unwrap();
    assert!(format!("{errno:?}").contains(&atom("eperm")));
    let replaced = InterfaceAddress {
        interface_name: "x\u{FFFD}".into(),
        ..interface(None, None)
    };
    assert!(ifaddrs::entry(replaced).is_err());
    let failure = ifaddrs::Failure::Unsupported("message");
    assert_eq!(
        ifaddrs::result(&atoms, Err(failure))
            .unwrap_err()
            .to_string(),
        "message"
    );
}

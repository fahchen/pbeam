use super::*;
use crate::runtime::capability::{AllowAll, Policy};
use crate::runtime::process::tests::frame;
use crate::term::AtomTable;

/// Return a scheduler whose current process is a new root process.
fn scheduler() -> Scheduler {
    let mut scheduler = Scheduler::new(AtomTable::new());
    scheduler
        .start(frame(), vec![], AllowAll.initial())
        .unwrap();
    scheduler.schedule_next().unwrap();
    scheduler
}

/// Return the bytes of one control message of `level` and `kind` with
/// `data`, with its padding.
fn message(level: i32, kind: i32, data: &[u8]) -> Vec<u8> {
    let mut bytes = [
        &length_bytes(HEADER + data.len())[..],
        &level.to_ne_bytes(),
        &kind.to_ne_bytes(),
    ]
    .concat();
    bytes.extend_from_slice(data);
    bytes.resize(HEADER + align(data.len()), 0);
    bytes
}

/// Return the printed list of the control messages of `bytes`.
fn encoded(scheduler: &mut Scheduler, bytes: &[u8]) -> String {
    let term = encode_cmsgs(scheduler, bytes).unwrap();
    String::from_utf8(crate::term::print_term(&term, &scheduler.atoms)).unwrap()
}

#[test]
fn encodes_each_row_of_the_host_with_and_without_enough_data() {
    // `esock_encode_cmsg` gives the name of a row, and `value` only when
    // the row has an encode function and the data is long enough
    // (prim_socket_nif.c:14187-14214, 14266-14490, 15605-15626). The host
    // gives only some rows, so a unit test builds each one.
    let mut scheduler = scheduler();
    for &(level, types, name, encode, _) in &SPECS {
        let Some(kind) = types[HOST] else { continue };
        let full = message(level, kind, &[1; 24]);
        let short = message(level, kind, &[]);
        if matches!(encode, Encode::Unsupported) {
            let failure = encode_cmsgs(&mut scheduler, &full).unwrap_err().to_string();
            assert!(
                failure.contains(&format!("the control message {name} of the level {level}")),
                "{failure}"
            );
            continue;
        }
        let text = encoded(&mut scheduler, &full);
        assert!(text.contains(&format!("type=>{name}")), "{text}");
        assert_eq!(
            text.contains("value=>"),
            !matches!(encode, Encode::None),
            "{text}"
        );
        let text = encoded(&mut scheduler, &short);
        assert!(!text.contains("value=>"), "{text}");
    }
}

#[test]
fn encodes_an_unknown_level_and_type_and_a_message_after_the_buffer() {
    // An unknown level or type gives the type number and no `value`
    // (prim_socket_nif.c:14208-14214), and a `cmsg_len` past the buffer
    // gives `bad_data` and ends the list (unix_socket_syncio.c:9061-9081).
    let mut scheduler = scheduler();
    let mut bytes = message(99, 5, &[7]);
    bytes.extend(message(libc::IPPROTO_IP, 77, &[]));
    let mut last = message(libc::IPPROTO_IP, 1, &[1, 2]);
    last.truncate(HEADER);
    bytes.extend(last);
    let text = encoded(&mut scheduler, &bytes);
    assert!(text.starts_with("[#{"), "{text}");
    assert!(
        text.contains("type=>5") && text.contains("level=>99"),
        "{text}"
    );
    assert!(
        text.contains("type=>77") && text.ends_with(",bad_data]"),
        "{text}"
    );
    assert_eq!(encoded(&mut scheduler, &[0; 32]), "[]");
}

#[test]
fn decodes_the_ctrl_list_of_a_message_to_send() {
    // `decode_cmsghdrs` takes a proper list of maps with `level` and
    // `type`, and `data` or `value`, that fits the buffer
    // (unix_socket_syncio.c:7214-7542, prim_socket_nif.c:16056-16074).
    let scheduler = scheduler();
    let atoms = &scheduler.atoms;
    let atom = |name: &str| Term::Atom(atoms.put(name).unwrap());
    let cmsg = |entries: &[(&str, Term)]| {
        let pairs = entries
            .iter()
            .map(|(key, value)| (atom(key), value.clone()))
            .collect();
        Term::Map(crate::term::Map::from_pairs(pairs, atoms).unwrap())
    };
    let ip = Term::Integer(libc::IPPROTO_IP.into());
    let decode = |ctrl: Term, capacity: usize| decode_cmsgs(&scheduler, &ctrl, capacity).unwrap();
    let tos = cmsg(&[
        ("level", ip.clone()),
        ("type", atom("tos")),
        ("value", atom("lowdelay")),
    ]);
    let bytes = decode(Term::list_from([tos.clone()]), 1024).unwrap();
    assert_eq!(
        bytes,
        message(
            libc::IPPROTO_IP,
            SPECS[4].1[HOST].unwrap(),
            &16i32.to_ne_bytes()
        )
    );
    assert_eq!(
        decode(Term::list_from([tos.clone()]), bytes.len() - 1),
        None
    );
    assert_eq!(
        decode(Term::cons(tos.clone(), Term::Integer(1)), 1024),
        None
    );
    assert_eq!(decode(Term::Integer(1), 1024), None);
    let socket = atom("socket");
    for data in [atom("true"), atom("false"), Term::Integer(3)] {
        let rights = cmsg(&[
            ("level", socket.clone()),
            ("type", atom("rights")),
            ("data", data),
        ]);
        assert_eq!(
            decode(Term::list_from([rights]), 1024).unwrap().len(),
            HEADER + align(4)
        );
    }
    let failures = [
        cmsg(&[("level", ip.clone())]),
        cmsg(&[
            ("level", atom("ip")),
            ("type", Term::Integer(1)),
            ("data", Term::Integer(1)),
        ]),
        cmsg(&[
            ("level", ip.clone()),
            ("type", Term::Integer(1)),
            ("data", atom("other")),
        ]),
        cmsg(&[
            ("level", ip.clone()),
            ("type", Term::Integer(1)),
            ("value", Term::Integer(1)),
        ]),
        cmsg(&[
            ("level", ip.clone()),
            ("type", atom("nosuch")),
            ("value", Term::Integer(1)),
        ]),
        cmsg(&[
            ("level", ip.clone()),
            ("type", atom("recvttl")),
            ("value", Term::Integer(1)),
        ]),
        cmsg(&[
            ("level", ip.clone()),
            ("type", Term::Nil),
            ("data", Term::Integer(1)),
        ]),
    ];
    for failure in failures {
        assert_eq!(
            decode(Term::list_from([failure.clone()]), 1024),
            None,
            "{failure:?}"
        );
    }
    let timestamp = cmsg(&[
        ("level", socket),
        ("type", atom("timestamp")),
        ("value", Term::Integer(1)),
    ]);
    let failure = decode_cmsgs(&scheduler, &Term::list_from([timestamp]), 1024).unwrap_err();
    assert!(
        failure
            .to_string()
            .contains("the control message timestamp of the level"),
        "{failure}"
    );
}

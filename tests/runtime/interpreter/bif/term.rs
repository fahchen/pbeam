//! Unit tests of `src/runtime/interpreter/bif/term.rs` for values that a
//! compiled fixture cannot make on PBeam: the public API has no
//! `erlang:list_to_pid/1`, `erlang:list_to_ref/1`, or
//! `erlang:list_to_port/1`, and `Term::Reference` has no constructor outside
//! the runtime. The expected bytes are measured on the pinned ERTS.

use super::*;
use crate::term::{Binary, MatchState, PortId};

/// Return the local node atom bytes of the pinned ERTS: `SMALL_ATOM_UTF8_EXT`
/// and `nonode@nohost`.
const NODE: [u8; 15] = [
    119, 13, 110, 111, 110, 111, 100, 101, 64, 110, 111, 104, 111, 115, 116,
];

#[test]
fn encodes_a_local_pid_port_and_reference_as_pinned_erts() {
    // `term_to_binary(erlang:list_to_pid("<0.5.0>"))`,
    // `term_to_binary(erlang:list_to_port("#Port<0.5>"))`, and
    // `term_to_binary(erlang:list_to_ref("#Ref<0.0.1.5>"))` on the pinned
    // ERTS.
    let atoms = AtomTable::new();
    let options = Options::default();
    let pid = encode(&Term::Pid(ProcessId(5)), &atoms, &options).unwrap();
    let port = encode(&Term::Port(PortId(5)), &atoms, &options).unwrap();
    let reference = encode(
        &Term::Reference(Reference::ordinary(5).unwrap()),
        &atoms,
        &options,
    )
    .unwrap();
    assert_eq!(
        pid,
        [&[88][..], &NODE, &[0, 0, 0, 5, 0, 0, 0, 0, 0, 0, 0, 0],].concat()
    );
    assert_eq!(
        port,
        [&[120][..], &NODE, &[0, 0, 0, 0, 0, 0, 0, 5, 0, 0, 0, 0]].concat()
    );
    assert_eq!(
        reference,
        [
            &[90, 0, 3][..],
            &NODE,
            &[0, 0, 0, 0, 0, 0, 0, 5, 0, 0, 0, 1, 0, 0, 0, 0],
        ]
        .concat()
    );
}

#[test]
fn encodes_local_pid_port_and_reference_with_a_nil_sysname() {
    // `term_to_binary(T, [local])` writes `NIL_EXT` for the node and
    // creation 0 (`external.c:3651-3654`, `3686-3689`). The pinned ERTS
    // gives `<<131,88,106,0,0,0,5,0,0,0,0,0,0,0,0>>` for the local pid
    // `<0.5.0>`.
    let atoms = AtomTable::new();
    let options = Options {
        local: true,
        ..Options::default()
    };
    assert_eq!(
        encode(&Term::Pid(ProcessId(5)), &atoms, &options).unwrap(),
        [88, 106, 0, 0, 0, 5, 0, 0, 0, 0, 0, 0, 0, 0]
    );
}

#[test]
fn parses_options_as_pinned_erts() {
    let list = |terms: Vec<Term>| Term::list_from(terms);
    let pair =
        |name: &str, value: i64| Term::tuple(vec![Term::static_atom(name), Term::Integer(value)]);
    assert!(parse_options(&Term::nil()).is_some());
    assert_eq!(parse_options(&Term::nil()).unwrap().minor, 2);
    assert_eq!(
        parse_options(&list(vec![Term::static_atom("compressed")]))
            .unwrap()
            .level,
        6
    );
    assert_eq!(
        parse_options(&list(vec![pair("compressed", 0)]))
            .unwrap()
            .level,
        0
    );
    assert_eq!(
        parse_options(&list(vec![pair("compressed", 9)]))
            .unwrap()
            .level,
        9
    );
    assert_eq!(
        parse_options(&list(vec![pair("minor_version", 1)]))
            .unwrap()
            .minor,
        1
    );
    assert!(parse_options(&list(vec![pair("compressed", 10)])).is_none());
    assert!(parse_options(&list(vec![pair("compressed", -1)])).is_none());
    assert!(parse_options(&list(vec![pair("minor_version", 3)])).is_none());
    assert!(parse_options(&list(vec![pair("info", 1)])).is_none());
    assert!(
        parse_options(&list(vec![Term::tuple(vec![
            Term::static_atom("minor_version"),
            Term::Nil,
        ])]))
        .is_none()
    );
    assert!(binary_options(&list(vec![Term::Integer(1)])).is_none());
    assert!(
        parse_options(&list(vec![
            Term::static_atom("local"),
            Term::static_atom("deterministic"),
        ]))
        .is_none()
    );
    assert!(parse_options(&Term::static_atom("compressed")).is_none());
}

/// The local node atom bytes: `SMALL_ATOM_UTF8_EXT` and `nonode@nohost`.
fn node() -> Vec<u8> {
    NODE.to_vec()
}

/// The seed of the local block hash that the tests use.
const SEED: u32 = 7;

/// Decode one term body with the atom table `atoms`: the input is
/// `VERSION_MAGIC` and the bytes.
fn decode_with(atoms: &AtomTable, bytes: &[u8], safe: bool) -> Result<Term, DecodeError> {
    let input = [&[VERSION_MAGIC][..], bytes].concat();
    decode_full(atoms, &input, safe)
}

/// Decode one external term with the atom table `atoms`, including
/// `VERSION_MAGIC`.
fn decode_full(atoms: &AtomTable, bytes: &[u8], safe: bool) -> Result<Term, DecodeError> {
    Decoder {
        input: bytes,
        pos: 0,
        safe,
        local: false,
        atoms,
        seed: SEED,
        code: &CodeTable::default(),
        ets: &crate::runtime::ets::NodeTables::default(),
    }
    .decode_input(SEED)
    .map(|(term, _)| term)
}

/// Return the bytes of the tags `tags` and a big-endian `u32` value.
fn with_words(tags: &[u8], values: &[u32]) -> Vec<u8> {
    let mut bytes = tags.to_vec();
    for value in values {
        bytes.extend_from_slice(&value.to_be_bytes());
    }
    bytes
}

#[test]
fn decodes_the_extended_integer_and_float_forms() {
    let atoms = AtomTable::new();
    // A zero-width bignum is 0 (`external.c:4478-4485`).
    assert_eq!(
        decode_with(&atoms, &[110, 1, 0, 0], false).unwrap(),
        Term::Integer(0)
    );
    // `LARGE_BIG_EXT` of the two-byte magnitude 1.
    assert_eq!(
        decode_with(&atoms, &[111, 0, 0, 0, 2, 0, 1, 0], false).unwrap(),
        Term::Integer(1)
    );
    // `FLOAT_EXT` of 31 bytes with the text `1.5` and NUL padding.
    let mut float = vec![99];
    float.extend_from_slice(b"1.5");
    float.resize(1 + 31, 0);
    assert_eq!(
        decode_with(&atoms, &float, false).unwrap(),
        Term::Float(1.5)
    );
    // A comma is the other radix character that `sys_chars_to_double`
    // accepts (`sys/unix/sys_float.c:74-135`).
    let mut comma = vec![99];
    comma.extend_from_slice(b"1,5");
    comma.resize(1 + 31, 0);
    assert_eq!(
        decode_with(&atoms, &comma, false).unwrap(),
        Term::Float(1.5)
    );
    // An exponent with a sign is part of the text that ERTS writes.
    let mut exponent = vec![99];
    exponent.extend_from_slice(b"1.5e+00");
    exponent.resize(1 + 31, 0);
    assert_eq!(
        decode_with(&atoms, &exponent, false).unwrap(),
        Term::Float(1.5)
    );
    let mut negative_exponent = vec![99];
    negative_exponent.extend_from_slice(b"1.5E-2");
    negative_exponent.resize(1 + 31, 0);
    assert_eq!(
        decode_with(&atoms, &negative_exponent, false).unwrap(),
        Term::Float(0.015)
    );
    // A text without a decimal point, a text without digits after it, an
    // exponent without digits, and a byte after the text raise badarg.
    for text in [&b"1"[..], b"1e2", b"1.", b"1.5e", b"1.5x"] {
        let mut invalid = vec![99];
        invalid.extend_from_slice(text);
        invalid.resize(1 + 31, 0);
        assert!(matches!(
            decode_with(&atoms, &invalid, false),
            Err(DecodeError::Badarg)
        ));
    }
    // A text that is not UTF-8, a text that parses as an infinity, and a
    // `NEW_FLOAT_EXT` of an infinity raise badarg (`external.c:4636-4665`).
    let mut invalid = vec![99];
    invalid.resize(1 + 31, 0xff);
    assert!(matches!(
        decode_with(&atoms, &invalid, false),
        Err(DecodeError::Badarg)
    ));
    let mut infinite = vec![99];
    infinite.extend_from_slice(b"inf");
    infinite.resize(1 + 31, 0);
    assert!(matches!(
        decode_with(&atoms, &infinite, false),
        Err(DecodeError::Badarg)
    ));
}

#[test]
fn decodes_each_atom_tag_and_checks_the_limits() {
    let atoms = AtomTable::new();
    // `ATOM_EXT` holds Latin-1 bytes, and the other tags hold UTF-8 names.
    let latin1 = decode_with(&atoms, &[100, 0, 1, 233], false).unwrap();
    assert_eq!(latin1, Term::Atom(atoms.get("é").unwrap()));
    let small_latin1 = decode_with(&atoms, &[115, 1, 97], false).unwrap();
    assert_eq!(small_latin1, Term::Atom(atoms.get("a").unwrap()));
    let utf8 = decode_with(&atoms, &[118, 0, 1, 97], false).unwrap();
    assert_eq!(utf8, Term::Atom(atoms.get("a").unwrap()));
    // An invalid UTF-8 name and a name of more than 255 characters raise
    // badarg (`external.c:4535-4545`).
    assert!(matches!(
        decode_with(&atoms, &[119, 1, 0xff], false),
        Err(DecodeError::Badarg)
    ));
    let mut long = vec![118, 1, 0];
    long.resize(3 + 256, b'a');
    assert!(matches!(
        decode_with(&atoms, &long, false),
        Err(DecodeError::Badarg)
    ));
    // The `safe` option takes only an atom that the run has.
    assert!(matches!(
        decode_with(&AtomTable::new(), &[119, 3, 122, 122, 122], true),
        Err(DecodeError::Badarg)
    ));
    assert!(matches!(
        decode_with(&atoms, &[119, 3, 97, 98, 115], true),
        Ok(Term::Atom(_))
    ));
}

#[test]
fn decodes_the_large_tuple_bitstring_and_guards() {
    let atoms = AtomTable::new();
    assert_eq!(
        decode_with(&atoms, &[105, 0, 0, 0, 1, 97, 1], false).unwrap(),
        Term::tuple(vec![Term::Integer(1)])
    );
    // A count that the bytes cannot hold raises badarg.
    assert!(matches!(
        decode_with(&atoms, &[105, 0, 0, 0, 3, 97, 1], false),
        Err(DecodeError::Badarg)
    ));
    // A bitstring of whole bytes is a binary.
    assert_eq!(
        decode_with(&atoms, &[77, 0, 0, 0, 1, 8, 5], false).unwrap(),
        Term::binary(vec![5])
    );
    // A bit count of 0 or 9, and a payload of no bytes, raise badarg
    // (`external.c:4895-4920`).
    assert!(matches!(
        decode_with(&atoms, &[77, 0, 0, 0, 1, 9, 5], false),
        Err(DecodeError::Badarg)
    ));
    assert!(matches!(
        decode_with(&atoms, &[77, 0, 0, 0, 1, 0, 5], false),
        Err(DecodeError::Badarg)
    ));
    assert!(matches!(
        decode_with(&atoms, &[77, 0, 0, 0, 0, 8], false),
        Err(DecodeError::Badarg)
    ));
    // An empty `LARGE_TUPLE_EXT`, an empty `MAP_EXT`, and a
    // `LIST_EXT` of no cells give `{}`, `#{}`, and the tail.
    assert_eq!(
        decode_with(&atoms, &[105, 0, 0, 0, 0], false).unwrap(),
        Term::tuple(vec![])
    );
    let empty = decode_with(&atoms, &[116, 0, 0, 0, 0], false).unwrap();
    assert!(matches!(&empty, Term::Map(map) if map.len() == 0));
    assert_eq!(
        decode_with(&atoms, &[108, 0, 0, 0, 0, 97, 1], false).unwrap(),
        Term::Integer(1)
    );
    // A small tuple whose count the bytes cannot hold raises badarg.
    assert!(matches!(
        decode_with(&atoms, &[104, 3, 97, 1], false),
        Err(DecodeError::Badarg)
    ));
    // A tuple above `ERTS_MAX_TUPLE_SIZE` raises badarg.
    assert!(matches!(
        decode_with(&atoms, &[105, 1, 0, 0, 0], false),
        Err(DecodeError::Badarg)
    ));
    // A duplicate map key raises badarg (`external.c:5069-5150`).
    assert!(matches!(
        decode_with(
            &atoms,
            &[116, 0, 0, 0, 2, 97, 1, 97, 2, 97, 1, 97, 3],
            false
        ),
        Err(DecodeError::Badarg)
    ));
    // A map and a list whose bytes do not hold their count raise badarg.
    assert!(matches!(
        decode_with(&atoms, &[116, 0, 0, 0, 2, 97, 1], false),
        Err(DecodeError::Badarg)
    ));
    assert!(matches!(
        decode_with(&atoms, &[108, 0, 0, 0, 2, 97, 1], false),
        Err(DecodeError::Badarg)
    ));
}

#[test]
fn decodes_each_identifier_tag_and_checks_the_node() {
    let atoms = AtomTable::new();
    let mut pid = vec![88];
    pid.extend(node());
    pid.extend(with_words(&[], &[5, 0, 0]));
    assert_eq!(
        decode_with(&atoms, &pid, false).unwrap(),
        Term::Pid(ProcessId(5))
    );
    let mut old_pid = vec![103];
    old_pid.extend(node());
    old_pid.extend(with_words(&[], &[5, 0]));
    old_pid.push(0);
    assert_eq!(
        decode_with(&atoms, &old_pid, false).unwrap(),
        Term::Pid(ProcessId(5))
    );
    let mut v4_port = vec![120];
    v4_port.extend(node());
    v4_port.extend(with_words(&[], &[0, 5, 0]));
    assert_eq!(
        decode_with(&atoms, &v4_port, false).unwrap(),
        Term::Port(PortId(5))
    );
    // A port number above 2^60 - 1 raises badarg
    // (`ERTS_MAX_INTERNAL_PORT_NUMBER`, `erl_node_container_utils.h:250`).
    let mut big_port = vec![120];
    big_port.extend(node());
    big_port.extend_from_slice(&u64::MAX.to_be_bytes());
    big_port.extend(with_words(&[], &[0]));
    assert!(matches!(
        decode_with(&atoms, &big_port, false),
        Err(DecodeError::Badarg)
    ));
    let mut new_port = vec![89];
    new_port.extend(node());
    new_port.extend(with_words(&[], &[5, 0]));
    assert_eq!(
        decode_with(&atoms, &new_port, false).unwrap(),
        Term::Port(PortId(5))
    );
    let mut old_port = vec![102];
    old_port.extend(node());
    old_port.extend(with_words(&[], &[5]));
    old_port.push(0);
    assert_eq!(
        decode_with(&atoms, &old_port, false).unwrap(),
        Term::Port(PortId(5))
    );
    let mut newer_ref = vec![90, 0, 3];
    newer_ref.extend(node());
    newer_ref.extend(with_words(&[], &[0, 5, 1, 0]));
    assert_eq!(
        decode_with(&atoms, &newer_ref, false).unwrap(),
        Term::Reference(Reference::ordinary(5).unwrap())
    );
    let mut new_ref = vec![114, 0, 3];
    new_ref.extend(node());
    new_ref.push(0);
    new_ref.extend(with_words(&[], &[5, 1, 0]));
    assert_eq!(
        decode_with(&atoms, &new_ref, false).unwrap(),
        Term::Reference(Reference::ordinary(5).unwrap())
    );
    // An old reference has the scheduler number 0, which PBeam cannot
    // hold (`dec_term`, `external.c:4786-4855`).
    let mut old_ref = vec![101];
    old_ref.extend(node());
    old_ref.extend(with_words(&[], &[5]));
    old_ref.push(0);
    assert!(matches!(
        decode_with(&atoms, &old_ref, false),
        Err(DecodeError::Unsupported(_))
    ));

    // `NIL_EXT` is a node only inside `LOCAL_EXT`, a sysname must be an
    // atom, and a creation that is not 0 belongs to another node
    // (`external.c:3110-3131`).
    let mut nil_pid = vec![88, 106];
    nil_pid.extend(with_words(&[], &[5, 0, 0]));
    assert!(matches!(
        decode_with(&atoms, &nil_pid, false),
        Err(DecodeError::Badarg)
    ));
    let mut bad_sysname = vec![88, 97, 1];
    bad_sysname.extend(with_words(&[], &[5, 0, 0]));
    assert!(matches!(
        decode_with(&atoms, &bad_sysname, false),
        Err(DecodeError::Badarg)
    ));
    let mut other = vec![119, 3, 102, 111, 111];
    let mut other_pid = vec![88];
    other_pid.append(&mut other);
    other_pid.extend(with_words(&[], &[5, 0, 1]));
    assert!(matches!(
        decode_with(&atoms, &other_pid, false),
        Err(DecodeError::Unsupported(_))
    ));
    // A port and a reference of another node, and an old reference of
    // another node, give the runtime error of the missing term.
    let foo = vec![119, 3, 102, 111, 111];
    let mut other_port = vec![120];
    other_port.extend(foo.clone());
    other_port.extend(with_words(&[], &[0, 5, 1]));
    assert!(matches!(
        decode_with(&atoms, &other_port, false),
        Err(DecodeError::Unsupported(_))
    ));
    let mut other_ref = vec![90, 0, 3];
    other_ref.extend(foo.clone());
    other_ref.extend(with_words(&[], &[1, 5, 1, 0]));
    assert!(matches!(
        decode_with(&atoms, &other_ref, false),
        Err(DecodeError::Unsupported(_))
    ));
    let mut other_old_ref = vec![101];
    other_old_ref.extend(foo);
    other_old_ref.extend(with_words(&[], &[5]));
    other_old_ref.push(0);
    assert!(matches!(
        decode_with(&atoms, &other_old_ref, false),
        Err(DecodeError::Unsupported(_))
    ));
    // An old reference whose number is 2^18 or more raises badarg.
    let mut big_old_ref = vec![101];
    big_old_ref.extend(node());
    big_old_ref.extend(with_words(&[], &[1 << 18]));
    big_old_ref.push(0);
    assert!(matches!(
        decode_with(&atoms, &big_old_ref, false),
        Err(DecodeError::Badarg)
    ));
    // The node atom of a pid takes each atom tag (`dec_sysname`,
    // `external.c:3110-3122`). Each name here is not the local node.
    for sysname in [&[100, 0, 1, 97][..], &[115, 1, 97], &[118, 0, 1, 97]] {
        let mut tagged = vec![88];
        tagged.extend_from_slice(sysname);
        tagged.extend(with_words(&[], &[5, 0, 0]));
        assert!(matches!(
            decode_with(&atoms, &tagged, false),
            Err(DecodeError::Unsupported(_))
        ));
    }
    // A legacy creation above 3 raises badarg before the node check
    // (`is_tiny_creation`, `external.c:71-72`, `3162`).
    let mut tiny_pid = vec![103];
    tiny_pid.extend(node());
    tiny_pid.extend(with_words(&[], &[5, 0]));
    tiny_pid.push(4);
    assert!(matches!(
        decode_with(&atoms, &tiny_pid, false),
        Err(DecodeError::Badarg)
    ));
    // A pid number above 2^28 - 1 raises badarg, and a reference whose
    // first number is 2^18 or more raises it too (`external.c:4810-4840`).
    let mut big_pid = vec![88];
    big_pid.extend(node());
    big_pid.extend(with_words(&[], &[1 << 28, 0, 0]));
    assert!(matches!(
        decode_with(&atoms, &big_pid, false),
        Err(DecodeError::Badarg)
    ));
    let mut big_ref = vec![90, 0, 3];
    big_ref.extend(node());
    big_ref.extend(with_words(&[], &[0, 1 << 18, 1, 0]));
    assert!(matches!(
        decode_with(&atoms, &big_ref, false),
        Err(DecodeError::Badarg)
    ));
    // A reference of another scheduler has other low bits in its second
    // reference number, which PBeam cannot hold.
    let mut other_scheduler = vec![90, 0, 3];
    other_scheduler.extend(node());
    other_scheduler.extend(with_words(&[], &[0, 5, 2, 0]));
    assert!(matches!(
        decode_with(&atoms, &other_scheduler, false),
        Err(DecodeError::Unsupported(_))
    ));
    // ERTS marks a pid reference in the second reference number
    // (`ERTS_REF1_PID_MARKER_BIT__`, `erl_bif_unique.h:106-109`), and the
    // alias reference holds a pid that PBeam does not keep.
    let mut pid_ref = vec![90, 0, 3];
    pid_ref.extend(node());
    pid_ref.extend(with_words(&[], &[0, 5, 0x4000_0001, 0]));
    assert!(matches!(
        decode_with(&atoms, &pid_ref, false),
        Err(DecodeError::Unsupported(_))
    ));
    // The high bit of the third number is not a marker in ERTS, and PBeam
    // cannot hold an ordinary reference with that bit.
    let mut high_ref = vec![90, 0, 3];
    high_ref.extend(node());
    high_ref.extend(with_words(&[], &[0, 5, 1, 0x8000_0000]));
    assert!(matches!(
        decode_with(&atoms, &high_ref, false),
        Err(DecodeError::Unsupported(_))
    ));
    // A zero count is a reference of the scheduler 0, which PBeam cannot
    // hold, and the decoder still reads its first number
    // (`external.c:4795-4796`).
    let mut zero_ref = vec![90, 0, 0];
    zero_ref.extend(node());
    zero_ref.extend(with_words(&[], &[0, 0]));
    assert!(matches!(
        decode_with(&atoms, &zero_ref, false),
        Err(DecodeError::Unsupported(_))
    ));
    // A count above five raises badarg (`ERTS_MAX_REF_NUMBERS`,
    // `external.c:4800`).
    let mut six_ref = vec![90, 0, 6];
    six_ref.extend(node());
    six_ref.extend(with_words(&[], &[0, 1, 1, 0, 0, 0]));
    assert!(matches!(
        decode_with(&atoms, &six_ref, false),
        Err(DecodeError::Badarg)
    ));
    // A count above three reads and skips the extra numbers.
    let mut four_ref = vec![90, 0, 4];
    four_ref.extend(node());
    four_ref.extend(with_words(&[], &[0, 5, 1, 0, 9]));
    assert_eq!(
        decode_with(&atoms, &four_ref, false).unwrap(),
        Term::Reference(Reference::ordinary(5).unwrap())
    );
}

#[test]
fn decodes_the_unsupported_forms_and_local_terms() {
    let atoms = AtomTable::new();
    // A truncated fun and the removed `FUN_EXT` raise badarg; a record
    // without its fields raises badarg (`decoded_size`, `external.c:6150-6500`).
    assert!(matches!(
        decode_with(&atoms, &[112], false),
        Err(DecodeError::Badarg)
    ));
    assert!(matches!(
        decode_with(&atoms, &[117], false),
        Err(DecodeError::Badarg)
    ));
    assert!(matches!(
        decode_with(&atoms, &[67], false),
        Err(DecodeError::Badarg)
    ));
    // A well-formed record passes the structure check, and the decoder
    // gives the documented runtime error. An exported flag above 1 and a
    // record without its fields raise badarg.
    let empty_record = vec![
        67, 0, 0, 0, 0, 0, 119, 6, 101, 114, 108, 97, 110, 103, 119, 1, 120,
    ];
    assert!(matches!(
        decode_with(&atoms, &empty_record, false),
        Err(DecodeError::Unsupported(_))
    ));
    assert!(matches!(
        decode_with(&atoms, &[67, 0, 0, 0, 0, 2], false),
        Err(DecodeError::Badarg)
    ));
    // A well-formed local fun of a module that is not loaded decodes to a
    // fun with a stub entry (`erl_fun.c:299-309`).
    let mut local_fun = vec![112, 0, 0, 0, 0, 0];
    local_fun.extend([0u8; 16]);
    local_fun.extend([0u8; 8]);
    local_fun.extend([119, 6, 101, 114, 108, 97, 110, 103, 97, 0, 97, 0]);
    local_fun.extend([88]);
    local_fun.extend(node());
    local_fun.extend(with_words(&[], &[5, 0, 0]));
    assert!(matches!(
        decode_with(&atoms, &local_fun, false),
        Ok(Term::Fun(_))
    ));
    // A free-variable count above 255 raises badarg
    // (`MAX_ARG`, `erl_vm.h:54`, `external.c:6392-6394`).
    let mut many_free = vec![112, 0, 0, 0, 0, 0];
    many_free.extend([0u8; 16]);
    many_free.extend([0u8; 4]);
    many_free.extend(256u32.to_be_bytes());
    assert!(matches!(
        decode_with(&atoms, &many_free, false),
        Err(DecodeError::Badarg)
    ));
    // A `LOCAL_EXT` term inside a term sets the local node, and its hash
    // covers the term that it wraps (`external.c:5250-5255`).
    let hash = external::block_hash(&[97, 1], SEED);
    let mut nested = vec![104, 2, 121];
    nested.extend(hash.to_be_bytes());
    nested.extend([97, 1, 97, 2]);
    assert_eq!(
        decode_with(&atoms, &nested, false).unwrap(),
        Term::tuple(vec![Term::Integer(1), Term::Integer(2)])
    );
    assert!(matches!(
        decode_with(&atoms, &[104, 2, 121, 0, 0, 0, 0, 97, 1, 97, 2], false),
        Err(DecodeError::Badarg)
    ));
    // An `EXPORT_EXT` term gives an external fun, and each other form of
    // it raises badarg (`external.c:5004-5047`).
    let export = vec![
        113, 119, 6, 101, 114, 108, 97, 110, 103, 119, 3, 97, 98, 115, 97, 1,
    ];
    let term = decode_with(&atoms, &export, false).unwrap();
    assert!(matches!(
        &term,
        Term::Fun(fun) if *fun.target()
            == Target::External {
                module: atoms.get("erlang").unwrap(),
                function: atoms.get("abs").unwrap(),
                arity: 1,
            }
    ));
    assert!(matches!(
        decode_with(&atoms, &[113, 97, 1], false),
        Err(DecodeError::Badarg)
    ));
    let bad_module = vec![113, 97, 1, 119, 3, 97, 98, 115, 97, 1];
    assert!(matches!(
        decode_with(&atoms, &bad_module, false),
        Err(DecodeError::Badarg)
    ));
    let bad_name = vec![113, 119, 6, 101, 114, 108, 97, 110, 103, 97, 1, 97, 1];
    assert!(matches!(
        decode_with(&atoms, &bad_name, false),
        Err(DecodeError::Badarg)
    ));
    let bad_function = vec![113, 119, 6, 101, 114, 108, 97, 110, 103, 97, 1];
    assert!(matches!(
        decode_with(&atoms, &bad_function, false),
        Err(DecodeError::Badarg)
    ));
    // A bignum that fits a small integer decodes to the integer.
    // ERTS takes an export arity that is a small integer above 2^32 - 1
    // (`external.c:5014-5030`). A PBeam fun holds its arity as a `u32`, so
    // the decoder gives the documented runtime error.
    let big_arity = vec![
        113, 119, 6, 101, 114, 108, 97, 110, 103, 119, 3, 97, 98, 115, 110, 5, 0, 0, 0, 0, 0, 1,
    ];
    assert!(matches!(
        decode_with(&atoms, &big_arity, false),
        Err(DecodeError::Unsupported(_))
    ));
    let bad_arity = vec![
        113, 119, 6, 101, 114, 108, 97, 110, 103, 119, 3, 97, 98, 115, 119, 1, 97,
    ];
    assert!(matches!(
        decode_with(&atoms, &bad_arity, false),
        Err(DecodeError::Badarg)
    ));
    let negative = vec![
        113, 119, 6, 101, 114, 108, 97, 110, 103, 119, 3, 97, 98, 115, 98, 255, 255, 255, 255,
    ];
    assert!(matches!(
        decode_with(&atoms, &negative, false),
        Err(DecodeError::Badarg)
    ));
    assert!(matches!(
        decode_with(&atoms, &export, true),
        Err(DecodeError::Unsupported(_))
    ));
    // A local pid holds `NIL_EXT` for the node, and its hash covers the
    // pid bytes (`external.c:3651-3654`, `5250-5255`).
    let options = Options {
        local: true,
        ..Options::default()
    };
    let pid_body = encode(&Term::Pid(ProcessId(5)), &atoms, &options).unwrap();
    let pid_hash = external::block_hash(&pid_body, SEED);
    let pid_input = [
        &[VERSION_MAGIC, LOCAL_EXT][..],
        &pid_hash.to_be_bytes(),
        &pid_body,
    ]
    .concat();
    assert_eq!(
        decode_full(&atoms, &pid_input, false).unwrap(),
        Term::Pid(ProcessId(5))
    );
    // The local node scope ends with the `LOCAL_EXT` term, so a sibling
    // pid cannot use `NIL_EXT` as its node (`decoded_size`,
    // `external.c:6150-6500`).
    let inner = encode(&Term::static_atom("ok"), &atoms, &Options::default()).unwrap();
    let inner_hash = external::block_hash(&inner, SEED);
    let mut siblings = vec![104, 2, 121];
    siblings.extend(inner_hash.to_be_bytes());
    siblings.extend(inner);
    siblings.push(88);
    siblings.push(106);
    siblings.extend(with_words(&[], &[5, 0, 0]));
    assert!(matches!(
        decode_with(&atoms, &siblings, false),
        Err(DecodeError::Badarg)
    ));
    // A local term with a different block hash raises badarg
    // (`external.c:5250-5255`).
    let body = encode(&Term::Integer(1), &atoms, &options).unwrap();
    let hash = external::block_hash(&body, SEED);
    let mut local = [&[VERSION_MAGIC, LOCAL_EXT][..], &hash.to_be_bytes(), &body].concat();
    assert_eq!(
        Decoder {
            input: &local,
            pos: 0,
            safe: false,
            local: false,
            atoms: &atoms,
            seed: SEED,
            code: &CodeTable::default(),
            ets: &crate::runtime::ets::NodeTables::default(),
        }
        .decode_input(SEED)
        .unwrap()
        .0,
        Term::Integer(1)
    );
    local[2] ^= 0xff;
    assert!(matches!(
        Decoder {
            input: &local,
            pos: 0,
            safe: false,
            local: false,
            atoms: &atoms,
            seed: SEED,
            code: &CodeTable::default(),
            ets: &crate::runtime::ets::NodeTables::default(),
        }
        .decode_input(SEED),
        Err(DecodeError::Badarg)
    ));
}

#[test]
fn validates_the_structure_before_it_adds_an_atom() {
    let atoms = AtomTable::new();
    // `decoded_size` validates the whole term before `dec_term` adds the
    // atoms (`external.c:6150-6500`, `2159-2198`), so a malformed tuple
    // leaves the atom table unchanged.
    assert!(matches!(
        decode_with(&atoms, &[104, 2, 119, 3, 120, 121, 122, 255], false),
        Err(DecodeError::Badarg)
    ));
    assert!(atoms.get("xyz").is_none());
    // The structure pass adds no atom, and the decode pass adds an atom
    // before a later malformed atom fails (`decoded_size` and `dec_term`,
    // `external.c:6150-6500`, `4525-4545`).
    assert!(matches!(
        decode_with(&atoms, &[104, 2, 119, 3, 97, 98, 99, 119, 1, 0xff], false),
        Err(DecodeError::Badarg)
    ));
    assert!(atoms.get("abc").is_some());
}

#[test]
fn decodes_and_rejects_the_compressed_form() {
    let atoms = AtomTable::new();
    // `COMPRESSED` with the length of the plain term and its zlib stream
    // (`external.c:1783-1843`).
    let mut valid = vec![131, 80, 0, 0, 0, 2];
    valid.extend(deflate(&[97, 1], 6).unwrap());
    assert_eq!(
        decode_full(&atoms, &valid, false).unwrap(),
        Term::Integer(1)
    );
    // A stream that inflates past the declared length, a stream that does
    // not start, a result that is shorter than the declared length, and a
    // stream that zlib rejects raise badarg.
    let mut past = vec![131, 80, 0, 0, 0, 0];
    past.extend(deflate(&[97, 1], 6).unwrap());
    assert!(matches!(
        decode_full(&atoms, &past, false),
        Err(DecodeError::Badarg)
    ));
    let mut stalled = vec![131, 80, 0, 0, 0, 10, 120];
    assert!(matches!(
        decode_full(&atoms, &stalled, false),
        Err(DecodeError::Badarg)
    ));
    let stalled_header = vec![131, 80, 0, 0, 0, 10, 120, 156];
    assert!(matches!(
        decode_full(&atoms, &stalled_header, false),
        Err(DecodeError::Badarg)
    ));
    let mut short = vec![131, 80, 0, 0, 0, 5];
    short.extend(deflate(&[97, 1], 6).unwrap());
    assert!(matches!(
        decode_full(&atoms, &short, false),
        Err(DecodeError::Badarg)
    ));
    stalled[6] = 0xff;
    stalled.push(0xff);
    assert!(matches!(
        decode_full(&atoms, &stalled, false),
        Err(DecodeError::Badarg)
    ));
}

#[test]
fn encodes_the_large_and_unsupported_forms() {
    let atoms = AtomTable::new();
    let options = Options::default();
    // A tuple of 256 elements takes `LARGE_TUPLE_EXT`.
    let elements = (0..256).map(Term::Integer).collect();
    let bytes = encode(&Term::tuple(elements), &atoms, &options).unwrap();
    assert_eq!(&bytes[..5], [105, 0, 0, 1, 0]);
    // An atom of 256 bytes takes `ATOM_UTF8_EXT`.
    let long = "a".repeat(256);
    let atom = atoms.put(&long).unwrap();
    let bytes = encode(&Term::Atom(atom.clone()), &atoms, &options).unwrap();
    assert_eq!(&bytes[..3], [118, 1, 0]);
    let latin1 = Options {
        minor: 1,
        ..Options::default()
    };
    let bytes = encode(&Term::Atom(atom.clone()), &atoms, &latin1).unwrap();
    assert_eq!(&bytes[..3], [100, 1, 0]);
    // A bignum of more than 255 magnitude bytes takes `LARGE_BIG_EXT`.
    let large = Term::BigInt(Arc::new(BigInt::from(1) << (256 * 8)));
    let bytes = encode(&large, &atoms, &options).unwrap();
    assert_eq!(&bytes[..5], [111, 0, 0, 1, 1]);
    // A match context has no bytes.
    let state = MatchState::new(Binary::from(vec![]));
    assert!(encode(&Term::MatchContext(Arc::new(state)), &atoms, &options).is_err());
    // A map above 32 pairs keeps the HAMT order of `Map::iter`.
    let pairs: Vec<(Term, Term)> = (0..40)
        .map(|value| (Term::Integer(value), Term::Integer(value)))
        .collect();
    let large = crate::term::Map::from_pairs(pairs, &atoms).unwrap();
    let bytes = encode(&Term::Map(large), &atoms, &options).unwrap();
    assert_eq!(&bytes[..5], [116, 0, 0, 0, 40]);
    // A bitstring at a bit offset is written from its first bit
    // (`external.c:3916-3935`).
    let partial = Term::Binary(Binary::from(vec![0x5A, 0xFF]).bits(1, 12));
    assert_eq!(
        encode(&partial, &atoms, &options).unwrap(),
        [77, 0, 0, 0, 2, 3, 0xB5, 0xE0]
    );
    // `deterministic` sorts map keys in exact term order, where an integer
    // is less than a float (`erl_utils.h:267-272`, `utils.c:2121-2130`).
    let pairs: Vec<(Term, Term)> = vec![
        (Term::Float(1.0), Term::static_atom("y")),
        (Term::Integer(2), Term::static_atom("x")),
    ];
    let map = crate::term::Map::from_pairs(pairs, &atoms).unwrap();
    let deterministic = Options {
        deterministic: true,
        ..Options::default()
    };
    let bytes = encode(&Term::Map(map), &atoms, &deterministic).unwrap();
    assert_eq!(&bytes[..9], [116, 0, 0, 0, 2, 97, 2, 119, 1]);
    assert_eq!(bytes[9], b'x');
}

#[test]
fn encodes_a_hashmap_in_the_erts_order() {
    // ERTS writes the reverse of the HAMT traversal order
    // (`enc_term`, `external.c:3819-3825`). The bytes are
    // `term_to_binary(maps:from_list([{I, I} || I <- lists:seq(1, 33)]))`
    // on the pinned ERTS.
    let atoms = AtomTable::new();
    let pairs = (1..=33)
        .map(|value| (Term::Integer(value), Term::Integer(value)))
        .collect();
    let map = crate::term::Map::from_pairs(pairs, &atoms).unwrap();
    let expected = [
        116, 0, 0, 0, 33, 97, 22, 97, 22, 97, 26, 97, 26, 97, 27, 97, 27, 97, 5, 97, 5, 97, 21, 97,
        21, 97, 30, 97, 30, 97, 16, 97, 16, 97, 3, 97, 3, 97, 33, 97, 33, 97, 14, 97, 14, 97, 24,
        97, 24, 97, 17, 97, 17, 97, 11, 97, 11, 97, 6, 97, 6, 97, 20, 97, 20, 97, 28, 97, 28, 97,
        25, 97, 25, 97, 1, 97, 1, 97, 32, 97, 32, 97, 15, 97, 15, 97, 9, 97, 9, 97, 23, 97, 23, 97,
        10, 97, 10, 97, 8, 97, 8, 97, 31, 97, 31, 97, 7, 97, 7, 97, 2, 97, 2, 97, 13, 97, 13, 97,
        29, 97, 29, 97, 19, 97, 19, 97, 12, 97, 12, 97, 4, 97, 4, 97, 18, 97, 18,
    ];
    assert_eq!(
        encode(&Term::Map(map), &atoms, &Options::default()).unwrap(),
        expected
    );
}

#[test]
fn rejects_a_pid_reference() {
    // ERTS writes the monitored pid in the alias reference, which PBeam
    // does not hold, so a decode and an encode give a runtime error
    // (`dec_term`, `external.c:4786-4855`).
    let reference = Reference::ordinary(5).unwrap().pid_reference();
    let mut input = vec![VERSION_MAGIC, 90, 0, 3];
    input.extend(node());
    input.extend(with_words(&[], &[0, 5, 0x4000_0001, 0]));
    let atoms = AtomTable::new();
    assert!(matches!(
        decode_full(&atoms, &input, false),
        Err(DecodeError::Unsupported(_))
    ));
    let atoms = AtomTable::new();
    assert!(encode(&Term::Reference(reference), &atoms, &Options::default()).is_err());
}

#[test]
fn binary_to_term_gives_the_error_of_a_full_atom_table() {
    // ERTS exits the emulator when the atom table is full
    // (`index_put_entry`); the decode returns the error of the table.
    use crate::runtime::scheduler::Scheduler;
    let atoms = AtomTable::new();
    let mut index = 0u32;
    while atoms.put(&format!("pbeam_fill_{index}")).is_ok() {
        index += 1;
    }
    let mut scheduler = Scheduler::new(atoms);
    let mut context = BifContext {
        scheduler: &mut scheduler,
    };
    let mut bytes = vec![131, 119, 10];
    bytes.extend_from_slice(b"pbeam_full");
    assert_eq!(
        binary_to_term(&mut context, &Term::binary(bytes), &Term::nil())
            .unwrap_err()
            .to_string(),
        "no more index entries in atom_tab (max=1048576)"
    );
}

#[test]
fn binary_to_term_checks_the_binary_and_the_options() {
    use crate::runtime::scheduler::Scheduler;
    let mut scheduler = Scheduler::new(AtomTable::new());
    let mut context = BifContext {
        scheduler: &mut scheduler,
    };
    // A bad option list gives `badarg` with the cause `badopt`
    // (`external.c:2165-2194`).
    assert!(matches!(
        binary_to_term(&mut context, &Term::Integer(1), &Term::nil()).unwrap(),
        BifResult::Raise { .. }
    ));
    let bad_options = Term::list_from(vec![Term::static_atom("info")]);
    assert!(matches!(
        binary_to_term(&mut context, &Term::binary(vec![131, 97, 1]), &bad_options).unwrap(),
        BifResult::ErrorWithCause { cause, .. } if cause == Term::static_atom("badopt")
    ));
    // A term that is not a byte-aligned binary raises `badarg`.
    assert!(matches!(
        binary_to_term(&mut context, &Term::Integer(1), &Term::nil()).unwrap(),
        BifResult::Raise { reason, .. } if reason == Term::static_atom("badarg")
    ));
    let bitstring = Term::Binary(Binary::from(vec![1]).bits(0, 1));
    assert!(matches!(
        binary_to_term(&mut context, &bitstring, &Term::nil()).unwrap(),
        BifResult::Raise { .. }
    ));
    // A truncated local fun raises `badarg`, and a well-formed one decodes.
    assert!(matches!(
        binary_to_term(&mut context, &Term::binary(vec![131, 112]), &Term::nil()).unwrap(),
        BifResult::Raise { reason, .. } if reason == Term::static_atom("badarg")
    ));
    let mut local_fun = vec![131, 112, 0, 0, 0, 0, 0];
    local_fun.extend([0u8; 16]);
    local_fun.extend([0u8; 8]);
    local_fun.extend([119, 6, 101, 114, 108, 97, 110, 103, 97, 0, 97, 0]);
    local_fun.extend([88]);
    local_fun.extend(node());
    local_fun.extend(with_words(&[], &[5, 0, 0]));
    assert!(matches!(
        binary_to_term(&mut context, &Term::binary(local_fun), &Term::nil()).unwrap(),
        BifResult::Value(Term::Fun(_))
    ));
}

#[test]
fn term_to_binary_checks_the_options_and_the_local_path() {
    use crate::runtime::scheduler::Scheduler;
    let mut scheduler = Scheduler::new(AtomTable::new());
    let mut context = BifContext {
        scheduler: &mut scheduler,
    };
    // A bad option list raises `badarg` (`external.c:1565-1590`).
    assert!(matches!(
        term_to_binary_2(
            &mut context,
            &Term::Integer(1),
            &Term::list_from(vec![Term::static_atom("info")]),
        )
        .unwrap(),
        BifResult::Raise { .. }
    ));
    // The `[local]` path of `erlang.rs` keeps its error for each other
    // term (`erlang.rs:5395-5420`).
    let local = Term::list_from(vec![Term::static_atom("local")]);
    assert!(super::erlang::term_to_binary_2(&mut context, &Term::Integer(1), &local).is_err());
}

#[test]
fn decodes_and_encodes_a_deep_term_without_a_rust_stack() {
    // ERTS decodes and encodes a term of any nesting depth (`dec_term` and
    // `enc_term` keep a work stack, `external.c:4270-4300`, `3280-3340`).
    let depth = 100_000;
    let mut bytes = vec![VERSION_MAGIC];
    for _ in 0..depth {
        bytes.extend([104, 1]);
    }
    bytes.push(106);
    let atoms = AtomTable::new();
    let term = decode_full(&atoms, &bytes, false).unwrap();
    assert_eq!(
        encode(&term, &atoms, &Options::default()).unwrap(),
        bytes[1..]
    );
}

#[test]
fn keeps_the_decoder_guards_behind_the_validation() {
    // `binary_to_term` validates the structure before `Decoder::term` adds
    // an atom (`external.c:6150-6500`, `2159-2198`). These bodies reach the
    // guard of `Decoder::term` itself, which keeps the same checks.
    let atoms = AtomTable::new();
    let term = |bytes: &[u8]| {
        Decoder {
            input: bytes,
            pos: 0,
            safe: false,
            local: false,
            atoms: &atoms,
            seed: SEED,
            code: &CodeTable::default(),
            ets: &crate::runtime::ets::NodeTables::default(),
        }
        .term()
    };
    for invalid in [
        &[108, 0, 0, 0, 5, 97, 1][..],
        &[104, 3, 97, 1],
        &[105, 0, 0, 0, 3, 97, 1],
        &[116, 0, 0, 0, 2, 97, 1],
        &[77, 0, 0, 0, 0, 8],
        &[255],
        &[121, 0, 0, 0, 0, 97, 1],
        &[112],
    ] {
        assert!(matches!(term(invalid), Err(DecodeError::Badarg)));
    }
    let mut long = vec![118, 1, 0];
    long.resize(3 + 256, b'a');
    assert!(matches!(term(&long), Err(DecodeError::Badarg)));
    let mut nil_pid = vec![88, 106];
    nil_pid.extend(with_words(&[], &[5, 0, 0]));
    assert!(matches!(term(&nil_pid), Err(DecodeError::Badarg)));
    let mut bad_sysname = vec![88, 97, 1];
    bad_sysname.extend(with_words(&[], &[5, 0, 0]));
    assert!(matches!(term(&bad_sysname), Err(DecodeError::Badarg)));
    for unsupported in [&[117][..], &[67]] {
        assert!(matches!(
            term(unsupported),
            Err(DecodeError::Unsupported(_))
        ));
    }
    let mut tiny_pid = vec![103];
    tiny_pid.extend(node());
    tiny_pid.extend(with_words(&[], &[5, 0]));
    tiny_pid.push(4);
    assert!(matches!(term(&tiny_pid), Err(DecodeError::Badarg)));
    let mut tiny_port = vec![102];
    tiny_port.extend(node());
    tiny_port.extend(with_words(&[], &[5]));
    tiny_port.push(4);
    assert!(matches!(term(&tiny_port), Err(DecodeError::Badarg)));
    let mut tiny_ref = vec![101];
    tiny_ref.extend(node());
    tiny_ref.extend(with_words(&[], &[5]));
    tiny_ref.push(4);
    assert!(matches!(term(&tiny_ref), Err(DecodeError::Badarg)));
    let mut six_ref = vec![90, 0, 6];
    six_ref.extend(node());
    six_ref.extend(with_words(&[], &[0, 1, 1, 0, 0, 0]));
    assert!(matches!(term(&six_ref), Err(DecodeError::Badarg)));
    // A legacy creation above 3 raises badarg for `NEW_REFERENCE_EXT` too
    // (`is_tiny_creation`, `external.c:4763-4777`).
    let mut tiny_new_ref = vec![114, 0, 3];
    tiny_new_ref.extend(node());
    tiny_new_ref.push(4);
    tiny_new_ref.extend(with_words(&[], &[5, 1, 0]));
    assert!(matches!(term(&tiny_new_ref), Err(DecodeError::Badarg)));
    assert!(matches!(
        decode_with(&atoms, &tiny_new_ref, false),
        Err(DecodeError::Badarg)
    ));
    // The validated path rejects a six-word `NEW_REFERENCE_EXT` too.
    let mut six_new_ref = vec![114, 0, 6];
    six_new_ref.extend(node());
    six_new_ref.extend(with_words(&[], &[0, 1, 1, 0, 0, 0]));
    assert!(matches!(
        decode_with(&atoms, &six_new_ref, false),
        Err(DecodeError::Badarg)
    ));
}

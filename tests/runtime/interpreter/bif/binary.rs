//! Unit tests of the search results of `src/runtime/interpreter/bif/binary.rs`.

use std::sync::Arc;

use super::*;

/// Return the bytes of a part and the array of its binary.
fn bytes(term: &Term) -> (&[u8], &Arc<Vec<u8>>) {
    let Term::Binary(binary) = term else {
        panic!("not a binary: {term:?}");
    };
    (binary.bit_bytes(), binary.array())
}

#[test]
fn gives_the_last_byte_of_an_append_binary_without_joining_its_chunks() {
    // `binary_last_1` reads the one byte (`erl_bif_binary.c:2311-2333`), so
    // the read of an append-built binary touches one chunk and joins none.
    let binary = Binary::from_chunks(vec![Arc::new(b"ab".to_vec()), Arc::new(b"c".to_vec())]);
    let result = last(&Term::Binary(binary.clone()));
    assert!(matches!(result, Value(Term::Integer(value)) if value == i64::from(b'c')));
    assert!(!binary.is_joined());
    // The empty binary has no last byte.
    assert!(matches!(
        last(&Term::Binary(Binary::default())),
        BifResult::Raise { .. }
    ));
}

#[test]
fn gives_a_not_found_result_that_shares_the_bytes_of_the_subject() {
    // `do_split_not_found_result` (`erl_bif_binary.c:1737-1751`) gives the
    // subject in a list, and `binary_split` writes sub-binaries of the
    // subject (`erl_bif_binary.c:1599-1645`), so the part shares the bytes
    // of the subject.
    let subject = Binary::from(b"abc".to_vec());
    let parts = not_found(0, &subject);
    let parts: Vec<Term> = parts
        .proper_list()
        .expect("the parts are a list")
        .cloned()
        .collect();
    assert_eq!(parts, [Term::Binary(subject.clone())]);
    let (part, array) = bytes(&parts[0]);
    assert_eq!(part, b"abc");
    assert!(Arc::ptr_eq(subject.array(), array));
    // An empty subject with `trim` gives no part.
    assert_eq!(not_found(TRIM, &Binary::from(Vec::new())), Term::nil());
}

#[test]
fn gives_split_parts_that_share_the_bytes_of_the_subject() {
    // `do_split_single_result` (`erl_bif_binary.c:1753-1833`) gives the part
    // before the match and the part after it, and each part keeps the offset
    // of its bytes in the array of the subject.
    let subject = Binary::from(b"abc,bcd".to_vec());
    let parts = split_once(0, &subject, 3, 1);
    let parts: Vec<Term> = parts
        .proper_list()
        .expect("the parts are a list")
        .cloned()
        .collect();
    assert_eq!(
        parts,
        [Term::binary(b"abc".to_vec()), Term::binary(b"bcd".to_vec())]
    );
    for part in &parts {
        let (_, array) = bytes(part);
        assert!(Arc::ptr_eq(subject.array(), array));
    }
    // The second part starts at the byte 4 of the subject.
    let (part, _) = bytes(&parts[1]);
    assert_eq!(
        part.as_ptr() as usize - subject.bit_bytes().as_ptr() as usize,
        4
    );
    // `trim` leaves an empty part at the end out.
    let parts = split_once(TRIM, &Binary::from(b"abc,".to_vec()), 3, 1);
    let parts: Vec<Term> = parts
        .proper_list()
        .expect("the parts are a list")
        .cloned()
        .collect();
    assert_eq!(parts, [Term::binary(b"abc".to_vec())]);
}

#[test]
fn gives_global_parts_that_share_the_bytes_of_the_subject() {
    // `do_split_global_result` (`erl_bif_binary.c:1835-1933`) builds the
    // parts from the last match to the first, and each part keeps the offset
    // of its bytes in the array of the subject.
    let subject = Binary::from(b"a,b,c".to_vec());
    let mut find = Find {
        flags: 0,
        end: 5,
        pos: 0,
        node: ROOT,
        candidate: None,
        matches: vec![(1, 1), (3, 1)],
        build: None,
    };
    let mut reds = 100;
    let parts = split_all(&mut find, &subject, &mut reds).expect("the parts are built");
    let parts: Vec<Term> = parts
        .proper_list()
        .expect("the parts are a list")
        .cloned()
        .collect();
    assert_eq!(
        parts,
        [
            Term::binary(b"a".to_vec()),
            Term::binary(b"b".to_vec()),
            Term::binary(b"c".to_vec())
        ]
    );
    for part in &parts {
        let (_, array) = bytes(part);
        assert!(Arc::ptr_eq(subject.array(), array));
    }
    // The parts start at the bytes 0, 2, and 4 of the subject.
    let offsets: Vec<usize> = parts
        .iter()
        .map(|part| bytes(part).0.as_ptr() as usize - subject.bit_bytes().as_ptr() as usize)
        .collect();
    assert_eq!(offsets, [0, 2, 4]);
}

#[test]
fn raises_badarg_for_a_bitstring_subject() {
    // The pinned ERTS raises `badarg` for `binary:at/2` and `binary:last/1`
    // of <<1,2,3:4>> (measured with `mise run erts-eval`).
    let term = Term::Binary(Binary::from(vec![1, 2, 3 << 4]).bits(0, 20));
    let badarg = |result: BifResult| {
        matches!(
            result,
            BifResult::Raise { class: ExceptionClass::Error, reason }
                if reason == Term::static_atom("badarg")
        )
    };
    assert!(badarg(at(&term, &Term::Integer(0))));
    assert!(badarg(last(&term)));
}

#[test]
fn limits_the_common_affix_list_to_the_tuple_arity() {
    // `MAX_ARITYVAL` (`erl_term.h:322`) binaries or more raise
    // `system_limit` (`erl_bif_binary.c:2212-2213`).
    let limit = |count| match list_error(count, &Term::Integer(1)) {
        Some(BifResult::Raise { reason, .. }) => reason == Term::static_atom("system_limit"),
        _ => false,
    };
    assert!(!limit((1 << 24) - 2));
    assert!(limit((1 << 24) - 1));
}

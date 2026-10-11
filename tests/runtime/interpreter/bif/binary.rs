//! Unit tests of the search results of `src/runtime/interpreter/bif/binary.rs`.

use std::sync::Arc;

use super::*;

/// Return the bytes of a part and the array of its binary.
fn bytes(term: &Term) -> (&[u8], &Arc<Vec<u8>>) {
    let Term::Binary(binary) = term else {
        panic!("not a binary: {term:?}");
    };
    (binary, binary.bytes())
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
    assert!(Arc::ptr_eq(subject.bytes(), array));
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
        assert!(Arc::ptr_eq(subject.bytes(), array));
    }
    // The second part starts at the byte 4 of the subject.
    let (part, _) = bytes(&parts[1]);
    assert_eq!(part.as_ptr() as usize - subject.as_ptr() as usize, 4);
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
        assert!(Arc::ptr_eq(subject.bytes(), array));
    }
    // The parts start at the bytes 0, 2, and 4 of the subject.
    let offsets: Vec<usize> = parts
        .iter()
        .map(|part| bytes(part).0.as_ptr() as usize - subject.as_ptr() as usize)
        .collect();
    assert_eq!(offsets, [0, 2, 4]);
}

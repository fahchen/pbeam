//! Unit tests of the binary newtype of `src/term/binary.rs`.

use super::*;

/// Return the shared array of bytes of a binary term. A sub-binary keeps the
/// array of its source, so the test compares this array.
fn array(term: &Term) -> &Arc<Vec<u8>> {
    let Term::Binary(bytes) = term else {
        panic!("not a binary: {term:?}");
    };
    bytes.bytes()
}

/// Return the bytes of a binary term as a vector.
fn bytes(term: &Term) -> Vec<u8> {
    let Term::Binary(bytes) = term else {
        panic!("not a binary: {term:?}");
    };
    bytes.to_vec()
}

#[test]
fn shares_the_bytes_between_the_copies_of_a_binary() {
    // A copy of a binary shares its bytes, as an ERTS copy of a refc binary
    // only increments the reference count (`copy.c:812-840`).
    let source = Term::binary(b"abc".to_vec());
    let copy = source.clone();
    assert!(Arc::ptr_eq(array(&source), array(&copy)));
    // A new binary of equal bytes is a different array.
    assert!(!Arc::ptr_eq(
        array(&source),
        array(&Term::binary(b"abc".to_vec()))
    ));
    assert_eq!(source, copy);
}

#[test]
fn shares_the_bytes_of_a_binary_between_a_sub_binary_and_its_source() {
    // `binary_part/3`, `bs_get_binary2`, `bs_get_tail`, and
    // `binary:split/2,3` give a view of the bytes of the source binary, as
    // an ERTS sub-binary refers to the original refc binary
    // (`erl_bif_binary.c:2508-2530`, `erl_bits.c:81-105`).
    let source = Term::binary(b"abcdef".to_vec());
    let Term::Binary(source_bytes) = &source else {
        unreachable!()
    };
    let part = Term::Binary(source_bytes.bits(8, 32));
    assert!(Arc::ptr_eq(array(&source), array(&part)));
    let Term::Binary(part_bytes) = &part else {
        unreachable!()
    };
    assert!(source_bytes.same_bytes(part_bytes));
    assert!(!source_bytes.same_bytes(&Binary::from(b"bcd".to_vec())));
    assert_eq!(part, Term::binary(b"bcd".to_vec()));
    // The sub-binary keeps the offset of its source
    // (`docs/portable-beam-spec.md:97`): the bytes after the view are the
    // bytes after `bcd`, and they are not at the start of the array.
    let Term::Binary(part_bytes) = &part else {
        unreachable!()
    };
    let tail = Term::Binary(part_bytes.bits(8, 24));
    assert!(Arc::ptr_eq(array(&source), array(&tail)));
    assert_eq!(bytes(&tail), b"cd".to_vec());
    assert_eq!(array(&tail).len(), 6);
}

#[test]
fn gives_the_bytes_of_a_binary_as_an_owned_vector() {
    // A trap state and a `bs_create_bin` append segment hold an owned vector
    // of the bytes of a binary.
    let raw = Binary::from(b"ab".to_vec());
    assert_eq!(raw.to_bytes(), b"ab".to_vec());
    // A sub-binary gives only its own bytes, and keeps the offset of its
    // array.
    let source = Binary::from(b"abc".to_vec());
    let part = source.bits(8, 16);
    assert_eq!(part.to_bytes(), b"b".to_vec());
    assert_eq!(source.to_bytes(), b"abc".to_vec());
}

#[test]
fn compares_binaries_by_their_bytes() {
    use std::cmp::Ordering::{Equal, Greater, Less};
    // The `Ord` implementation of a binary compares the bytes and then the
    // sizes, as `erts_cmp_compound` does (`utils.c:2100-2125`).
    let long = Binary::from(b"abc".to_vec());
    assert_eq!(long.cmp(&Binary::from(b"abc".to_vec())), Equal);
    assert_eq!(long.cmp(&Binary::from(b"b".to_vec())), Less);
    assert_eq!(long.cmp(&Binary::from(b"ab".to_vec())), Greater);
    assert_eq!(
        long.partial_cmp(&Binary::from(b"abc".to_vec())),
        Some(Equal)
    );
    assert!(Binary::from(b"ab".to_vec()) < Binary::from(b"abc".to_vec()));

    assert_eq!(Term::binary(vec![]), Term::binary(vec![]));
    assert_eq!(Term::binary(b"ab".to_vec()), Term::binary(b"ab".to_vec()));
    assert_ne!(Term::binary(b"ab".to_vec()), Term::binary(b"ac".to_vec()));
    assert_ne!(Term::binary(b"ab".to_vec()), Term::binary(b"a".to_vec()));
    assert_ne!(Term::binary(vec![]), Term::nil());
    // A view of a longer array is equal to a binary of the same bytes, as
    // ERTS compares the bytes and not the size of the array. The arrays are
    // different, so the comparison walks the bytes.
    let source = Binary::from(b"abc".to_vec());
    let view = source.bits(0, 16);
    let other = Binary::from(b"ab".to_vec());
    assert!(!view.same_bytes(&other));
    assert_eq!(view, other);
    assert_ne!(view, Binary::from(b"ac".to_vec()));
    // The same view of one array is equal with no walk, and two views with
    // different bytes are not equal.
    let source = Term::binary(b"abcd".to_vec());
    let Term::Binary(raw) = &source else {
        unreachable!()
    };
    assert_eq!(raw.bits(0, 16), raw.bits(0, 16));
    assert!(raw.bits(0, 16).same_bytes(&raw.bits(16, 32)));
    assert_ne!(raw.bits(0, 16), raw.bits(16, 32));
}

#[test]
fn orders_binaries_by_their_bytes() {
    // The term order compares two binaries byte by byte, and a shorter
    // binary is less when it is a prefix of the other
    // (`docs/portable-beam-spec.md`, term order).
    use std::cmp::Ordering::{Equal, Greater, Less};
    let less = Term::binary(b"ab".to_vec());
    let more = Term::binary(b"b".to_vec());
    assert_eq!(less.compare(&more).unwrap(), Less);
    assert_eq!(more.compare(&less).unwrap(), Greater);
    assert_eq!(
        Term::binary(b"a".to_vec())
            .compare(&Term::binary(b"a".to_vec()))
            .unwrap(),
        Equal
    );
    assert_eq!(
        Term::binary(b"a".to_vec())
            .compare(&Term::binary(b"ab".to_vec()))
            .unwrap(),
        Less
    );
}

#[test]
fn gives_no_byte_for_a_binary_of_no_bits() {
    // A binary of 0 bits at the end of its array has no byte, as an ERTS
    // bitstring of 0 bits is empty.
    let source = Binary::from(&[1, 2]);
    assert!(source.bits(16, 16).is_empty());
    assert_eq!(&source.bits(16, 16)[..], &[]);
    assert!(Binary::from(Vec::new()).bits(0, 0).is_empty());
}

#[test]
fn prints_a_binary_as_its_bytes() {
    // The `Debug` text of a binary term is the text that the `Term::Binary`
    // variant wrote before the shared array: `Binary([1, 2])`.
    assert_eq!(format!("{:?}", Term::binary(vec![1, 2])), "Binary([1, 2])");
    assert_eq!(
        format!("{:?}", Term::binary(b"ab".to_vec())),
        "Binary([97, 98])"
    );
    // The array of a binary formatter writes the same text.
    assert_eq!(format!("{:?}", Binary::from(vec![1, 2])), "[1, 2]");
}

#[test]
fn copies_a_view_whose_size_is_not_a_whole_number_of_bytes_in_o1_time() {
    // The bits of such a view are packed in one `Arc`, so a copy of the view
    // copies the `Arc` and not the bits. PBeam has no bitstring term, so
    // such a view comes only from a segment that PBeam does not run yet.
    let source = Binary::from(&[0xC3]);
    let view = source.bits(4, 8);
    assert_eq!(&view[..], &[0b0011_0000]);
    let copy = view.clone();
    assert_eq!(&copy[..], &[0b0011_0000]);
    assert!(view.same_bytes(&copy));
    let packed = view.packed().expect("the bits of the view are packed");
    let copied = copy.packed().expect("the bits of the copy are packed");
    assert!(Arc::ptr_eq(packed, copied));
}

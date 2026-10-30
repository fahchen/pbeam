//! Unit tests of the binary newtype of `src/term/binary.rs`.

use super::*;

/// Return the shared array of bytes of a binary term. A sub-binary keeps the
/// array of its source, so the test compares this array.
#[expect(
    clippy::rc_buffer,
    reason = "`Binary` shares byte arrays as `Arc<Vec<u8>>` because `append_in_place` grows the vector through `Arc::get_mut`; an `Arc<[u8]>` cannot grow"
)]
fn array(term: &Term) -> &Arc<Vec<u8>> {
    let Term::Binary(bytes) = term else {
        panic!("not a binary: {term:?}");
    };
    bytes.array()
}

/// Return the bytes of a binary term as a vector.
fn bytes(term: &Term) -> Vec<u8> {
    let Term::Binary(bytes) = term else {
        panic!("not a binary: {term:?}");
    };
    bytes.bit_bytes().to_vec()
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
    assert_eq!(
        less.compare(&more, &crate::term::test_atom_table())
            .unwrap(),
        Less
    );
    assert_eq!(
        more.compare(&less, &crate::term::test_atom_table())
            .unwrap(),
        Greater
    );
    assert_eq!(
        Term::binary(b"a".to_vec())
            .compare(
                &Term::binary(b"a".to_vec()),
                &crate::term::test_atom_table()
            )
            .unwrap(),
        Equal
    );
    assert_eq!(
        Term::binary(b"a".to_vec())
            .compare(
                &Term::binary(b"ab".to_vec()),
                &crate::term::test_atom_table()
            )
            .unwrap(),
        Less
    );
}

#[test]
fn gives_no_byte_for_a_binary_of_no_bits() {
    // A binary of 0 bits at the end of its array has no byte, as an ERTS
    // bitstring of 0 bits is empty.
    let source = Binary::from(&[1, 2]);
    assert_eq!(source.bits(16, 16).bit_bytes(), []);
    assert_eq!(source.bits(16, 16).bit_bytes(), &[]);
    assert_eq!(Binary::from(Vec::new()).bits(0, 0).bit_bytes(), []);
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
    assert_eq!(view.bit_bytes(), &[0b0011_0000]);
    let copy = view.clone();
    assert_eq!(copy.bit_bytes(), &[0b0011_0000]);
    assert!(view.same_bytes(&copy));
    let packed = view.packed().expect("the bits of the view are packed");
    let copied = copy.packed().expect("the bits of the copy are packed");
    assert!(Arc::ptr_eq(packed, copied));
}

#[test]
fn gives_the_last_byte_of_each_view_without_joining_it() {
    // An append-built view keeps its chunks, so the read of its last byte
    // touches one chunk and joins none.
    let binary = Binary::from_chunks(vec![Arc::new(b"ab".to_vec()), Arc::new(b"cd".to_vec())]);
    assert_eq!(binary.last_byte(), Some(b'd'));
    assert!(!binary.is_joined());
    // An empty chunk at the end holds no byte.
    let binary = Binary::from_chunks(vec![
        Arc::new(b"a".to_vec()),
        Arc::new(Vec::new()),
        Arc::new(Vec::new()),
    ]);
    assert_eq!(binary.last_byte(), Some(b'a'));
    // A view of one array reads the byte of the array.
    assert_eq!(Binary::from(b"ab".to_vec()).last_byte(), Some(b'b'));
    assert_eq!(Binary::default().last_byte(), None);
}

#[test]
fn takes_the_bytes_of_the_last_holder_with_no_copy() {
    // `Binary::take_bytes` moves the array out when the binary is its last
    // holder, and else it copies the view. The trap state of
    // `list_to_bitstring_continue` takes the bytes that a turn collected, so
    // a later turn does not copy them again.
    let binary = Binary::from(b"ab".to_vec());
    let array = binary.array().as_ptr();
    let bytes = binary.take_bytes();
    assert_eq!(bytes, b"ab".to_vec());
    assert_eq!(bytes.as_ptr(), array);
    // A shared binary copies its bytes, and the other holder keeps them.
    let binary = Binary::from(b"ab".to_vec());
    let copy = binary.clone();
    assert_eq!(binary.take_bytes(), b"ab".to_vec());
    assert_eq!(copy, Binary::from(b"ab".to_vec()));
    // An empty binary is the value of the state after the take.
    assert_eq!(Binary::default().bit_bytes(), []);
    assert_eq!(Binary::default(), Binary::from(Vec::new()));
    // A sub-binary that owns the array of its source gives only its own
    // bytes.
    let source = Binary::from(b"abc".to_vec());
    let part = source.bits(8, 16);
    drop(source);
    assert_eq!(part.take_bytes(), b"b".to_vec());
    // A view whose bytes are not whole bytes packs them, so it copies the
    // packed vector.
    let source = Binary::from(&[0xC3]);
    assert_eq!(source.bits(4, 8).take_bytes(), vec![0b0011_0000]);
}

#[test]
fn reports_the_bytes_of_each_chunk_of_an_append() {
    // A view that an append built keeps its bytes in several arrays, so the
    // accounting counts the capacity of each one and of the joined array.
    let mut first = Vec::with_capacity(64);
    first.extend_from_slice(&[1_u8; 5]);
    let mut second = Vec::with_capacity(4096);
    second.extend_from_slice(&[2_u8; 4095]);
    let array = Binary::from_chunks(vec![Arc::new(first), Arc::new(second)]);
    let chunks: usize = array
        .chunk_backing()
        .expect("a large append keeps its arrays")
        .iter()
        .map(|chunk| chunk.capacity())
        .sum();
    assert_eq!(array.allocated_bytes(), array.backing().capacity() + chunks);
    // The first read joins the arrays, and the account counts the joined
    // array as well.
    let joined = array.to_bytes();
    assert_eq!(joined.len(), 4100);
    assert_eq!(
        array.allocated_bytes(),
        array.backing().capacity() + chunks + array.flat_backing().capacity()
    );
    assert_eq!(array.take_bytes(), joined);
}

#[test]
fn reports_the_bytes_of_the_backing_and_the_packed_array() {
    let bytes = Binary::from(b"abcd".to_vec());
    assert!(bytes.allocated_bytes() >= 4);
    let packed = bytes.bits(4, 12);
    assert_eq!(
        packed.allocated_bytes(),
        bytes.allocated_bytes() + packed.packed().unwrap().capacity()
    );
}

#[test]
fn shares_the_prefix_array_of_a_large_append() {
    // `Binary::concat` is the result of an `append` segment that adds bits.
    // A later segment that does not fit in the last leaf with the prefix
    // keeps the array of the prefix, so the append copies no byte of the old
    // payload.
    let prefix = Binary::from(vec![7_u8; 100_000]);
    let result = Binary::concat(&prefix, &[9]);
    let arrays = result
        .chunk_backing()
        .expect("a large append keeps its bytes in arrays");
    assert_eq!(arrays.len(), 2);
    assert!(Arc::ptr_eq(&arrays[0], prefix.backing()));
    assert_eq!(arrays[0].len(), 100_000);
    assert_eq!(arrays[1][..], [9]);
    assert_eq!(result.bit_bytes().len(), 100_001);
    assert_eq!(result.bit_bytes()[0], 7);
    assert_eq!(result.bit_bytes()[100_000], 9);
    // A chunked view and a flat view never share their bytes.
    assert!(!result.same_bytes(&prefix));
    assert!(!prefix.same_bytes(&result));
    // An empty suffix shares the whole prefix handle.
    let whole = Binary::concat(&prefix, &[]);
    assert!(whole.same_bytes(&prefix));
}

#[test]
fn shares_every_array_when_a_binary_is_appended_to_itself() {
    // The doubling build appends a binary to itself. The later binary is
    // larger than one leaf, so the append shares each array of the prefix
    // and of the later binary, and copies no byte of the old payload.
    let full = Arc::new(vec![1_u8; APPEND_LEAF]);
    let partial = Arc::new(vec![2_u8; 7]);
    let half = Binary::from_chunks(vec![Arc::clone(&full), Arc::clone(&partial)]);
    let whole = Binary::concat_arrays(&half, half.chunks());
    let arrays = whole
        .chunk_backing()
        .expect("the doubling keeps its bytes in arrays");
    assert_eq!(arrays.len(), 4);
    assert!(Arc::ptr_eq(&arrays[0], &full));
    assert!(Arc::ptr_eq(&arrays[1], &partial));
    assert!(Arc::ptr_eq(&arrays[2], &full));
    assert!(Arc::ptr_eq(&arrays[3], &partial));
    assert_eq!(whole.bit_bytes().len(), 2 * APPEND_LEAF + 14);
}

#[test]
fn merges_a_small_append_in_the_last_leaf() {
    // A later segment that fits in one leaf with the last array of the
    // prefix merges in one leaf, so a repeated small append fills a leaf
    // instead of adding one small array for each step. The merge copies at
    // most that one leaf.
    let prefix = Binary::from(vec![7_u8; 100]);
    let result = Binary::concat(&prefix, &[9]);
    assert!(result.chunk_backing().is_none());
    assert!(!Arc::ptr_eq(result.backing(), prefix.backing()));
    assert_eq!(result.bit_bytes().len(), 101);
    assert_eq!(result.bit_bytes()[0], 7);
    assert_eq!(result.bit_bytes()[99], 7);
    assert_eq!(result.bit_bytes()[100], 9);
    // The merge stops at one leaf: a full leaf keeps its array and the byte
    // starts a new array.
    let full = Arc::new(vec![1_u8; APPEND_LEAF]);
    let merged = Binary::concat_arrays(
        &Binary::from_chunks(vec![Arc::clone(&full)]),
        vec![Arc::new(vec![9_u8])],
    );
    let arrays = merged
        .chunk_backing()
        .expect("a full leaf stays an array of the result");
    assert_eq!(arrays.len(), 2);
    assert!(Arc::ptr_eq(&arrays[0], &full));
    assert_eq!(arrays[1][..], [9]);
}

#[test]
fn drops_an_empty_array_of_an_append() {
    // An empty array holds no byte, so the result keeps no array for it.
    let empty = Binary::from(Vec::new());
    assert!(empty.chunk_backing().is_none());
    let result = Binary::concat_arrays(
        &empty,
        vec![Arc::new(vec![1_u8; APPEND_LEAF]), Arc::new(vec![2_u8])],
    );
    let arrays = result
        .chunk_backing()
        .expect("a large suffix keeps its bytes in arrays");
    assert_eq!(arrays.len(), 2);
    assert_eq!(arrays[0].len(), APPEND_LEAF);
    assert_eq!(arrays[1][..], [2]);
    let result = Binary::concat_arrays(
        &Binary::from(vec![1_u8; APPEND_LEAF + 1]),
        vec![Arc::new(Vec::new()), Arc::new(vec![2_u8])],
    );
    let arrays = result
        .chunk_backing()
        .expect("a large prefix keeps its bytes in arrays");
    assert_eq!(arrays.len(), 2);
    assert_eq!(arrays[0].len(), APPEND_LEAF + 1);
    assert_eq!(arrays[1][..], [2]);
}

#[test]
fn joins_the_arrays_of_a_chunked_prefix_in_one_array() {
    // A read of the contiguous bytes joins the arrays of an append at the
    // first read and keeps the joined array for each later read.
    let first = Binary::from_chunks(vec![Arc::new(vec![1_u8; 5]), Arc::new(vec![2_u8; 4095])]);
    let second = Binary::concat_arrays(&first, vec![Arc::new(vec![3_u8; 8])]);
    let arrays = second
        .chunk_backing()
        .expect("a large append keeps its bytes in arrays");
    assert_eq!(arrays.len(), 3);
    assert_eq!(second.bit_bytes().len(), 4108);
    assert_eq!(second.bit_bytes()[0], 1);
    assert_eq!(second.bit_bytes()[5], 2);
    assert_eq!(second.bit_bytes()[4100], 3);
    // The joined array is built once and shared by each later read.
    let joined = Arc::clone(second.backing());
    assert!(joined.is_empty());
    assert_eq!(second.backing().len(), 0);
    assert_eq!(second.to_bytes(), second.to_bytes());
    assert_eq!(second.to_bytes().len(), 4108);
    // A copy of the handle shares the joined array.
    let copy = second.clone();
    assert_eq!(copy.bit_bytes(), second.bit_bytes());
    assert!(second.same_bytes(&copy));
    // A sub-binary of an append keeps the joined array, so the two handles
    // keep their bytes in different arrays.
    let part = second.bits(32, 48);
    assert_eq!(part.bit_bytes(), &[1, 2]);
    assert!(!second.same_bytes(&part));
    assert_eq!(second.take_bytes().len(), 4108);
}

#[test]
fn merges_a_sub_binary_prefix_of_an_append_in_one_leaf() {
    // A prefix that is not the whole of its array has no single array of
    // bytes, so the append materializes its view and merges the later
    // segment in that array.
    let source = Binary::from(b"abcdef".to_vec());
    let prefix = source.bits(8, 24);
    let result = Binary::concat(&prefix, b"z");
    assert!(result.chunk_backing().is_none());
    assert_eq!(result.bit_bytes(), b"bcz");
    // A packed prefix packs its bits in one array.
    let packed = Binary::from(&[0xC3]).bits(4, 8);
    let result = Binary::concat(&packed, &[0xFF]);
    assert!(result.chunk_backing().is_none());
    assert_eq!(result.bit_bytes(), &[0b0011_0000, 0xFF]);
    // The take of an append copies its bytes.
    assert_eq!(result.take_bytes(), vec![0b0011_0000, 0xFF]);
}

#[test]
fn appends_in_place_only_to_a_binary_that_one_handle_owns() {
    // A `private_append` writes in the buffer of a writable binary
    // (`erl_bits.c:1963-1981`). PBeam writes in the array only when one
    // handle owns the view and the array, so no other term sees the bytes.
    let mut binary = Binary::from(b"ab".to_vec());
    let start = Arc::as_ptr(binary.array());
    assert!(binary.append_in_place(b"c"));
    assert_eq!(
        (binary.bit_bytes(), Arc::as_ptr(binary.array())),
        (&b"abc"[..], start)
    );
    assert_eq!(binary.bit_bytes().len(), 3);

    // A copy shares the view, so the append keeps both binaries unchanged.
    let copy = binary.clone();
    assert!(!binary.append_in_place(b"d"));
    assert_eq!(
        (binary.bit_bytes(), copy.bit_bytes()),
        (&b"abc"[..], &b"abc"[..])
    );
    drop(copy);

    // A sub-binary is a part of its array, so the append does not write in it.
    let mut part = Binary::from(b"abcd".to_vec()).bits(0, 16);
    assert!(!part.append_in_place(b"z"));
    assert_eq!(part.bit_bytes(), b"ab");
    let mut tail = Binary::from(b"abcd".to_vec()).bits(8, 32);
    assert!(!tail.append_in_place(b"z"));
    assert_eq!(tail.bit_bytes(), b"bcd");

    // A view that an append built joins its arrays in one array one time,
    // and then appends in that array.
    let large = Binary::from(vec![1; APPEND_LEAF]);
    let mut built = Binary::concat(&large, &[2]);
    assert!(built.append_in_place(&[3]));
    let joined = Arc::as_ptr(built.array());
    assert!(built.append_in_place(&[4]));
    assert_eq!(Arc::as_ptr(built.array()), joined);
    assert_eq!(built.bit_bytes().len(), APPEND_LEAF + 3);
    assert_eq!(&built.bit_bytes()[APPEND_LEAF..], &[2, 3, 4]);

    // A joined array that another handle holds stays unchanged.
    let shared = Binary::concat(&large, &[2]);
    let _flat = Arc::clone(shared.flat_backing());
    let mut shared = shared;
    assert!(!shared.append_in_place(&[3]));
    assert_eq!(shared.bit_bytes().len(), APPEND_LEAF + 1);
}

#[test]
fn reads_a_byte_range_of_an_appended_binary_from_the_chunks_of_the_range() {
    let chunks = (0..1000u32)
        .map(|index| Arc::new(index.to_be_bytes().to_vec()))
        .collect::<Vec<_>>();
    let binary = Binary::from_chunks(chunks);
    // The view builds the chunk index when it is made, and reads keep it.
    let index = binary.chunk_backing().unwrap().ends.as_ptr();
    assert_eq!(binary.chunk_backing().unwrap().ends[999], 4000);
    assert_eq!(binary.byte_runs(3999, 4000), vec![&[231u8][..]]);
    assert_eq!(binary.byte_runs(5, 10), vec![&[0u8, 0, 1][..], &[0, 0][..]]);
    assert_eq!(binary.chunk_backing().unwrap().ends.as_ptr(), index);
    assert!(!binary.is_joined());

    // A range near the end visits only the chunks of the range.
    let ends = &binary.chunk_backing().unwrap().ends;
    assert_eq!(chunk_span(ends, 3998, 3999), 999..1000);
    assert_eq!(chunk_span(ends, 3995, 4000), 998..1000);
    assert_eq!(chunk_span(ends, 4, 8), 1..2);
    assert_eq!(chunk_span(ends, 7, 7), 0..0);
    let no_runs: [&[u8]; 0] = [];
    assert_eq!(binary.byte_runs(7, 7), no_runs);
}

/// Return the bitstring of the first `size` bits of `bytes`, at offset 0.
fn bits(bytes: &[u8], size: usize) -> Binary {
    Binary::from(bytes).bits(0, size)
}

#[test]
fn compares_two_bitstrings_by_their_bits_and_then_their_sizes() {
    use std::cmp::Ordering;
    // ERTS compares the bits, and then the sizes (`utils.c:1209-1217`,
    // `utils.c:2325-2335`). Each result is from the pinned ERTS.
    let one_of_3 = bits(&[0x20], 3);
    let one_of_4 = bits(&[0x10], 4);
    let space = Binary::from(b" ".to_vec());
    // list_to_bitstring([<<1:3>>]) == <<32>> is false.
    assert_ne!(one_of_3, space);
    assert_eq!(one_of_3.cmp(&space), Ordering::Less);
    // lists:sort([<<32>>, <<1:3>>, <<1:4>>]) is [<<1:4>>,<<1:3>>,<<" ">>].
    let mut sorted = vec![space.clone(), one_of_3.clone(), one_of_4.clone()];
    sorted.sort();
    assert_eq!(sorted, vec![one_of_4, one_of_3.clone(), space]);
    // <<0:3>> < <<>> is false, and <<5:3>> < <<6:3>> is true.
    assert_eq!(bits(&[0], 3).cmp(&Binary::default()), Ordering::Greater);
    assert_eq!(bits(&[0xA0], 3).cmp(&bits(&[0xC0], 3)), Ordering::Less);
    // <<255,1:1>> < <<255,1:2>> is false: the 9th bit decides.
    assert_eq!(
        bits(&[0xFF, 0x80], 9).cmp(&bits(&[0xFF, 0x40], 10)),
        Ordering::Greater
    );
    // The bits after the size do not change the result.
    let one_of_3_in_0x3f = bits(&[0x3F], 3);
    assert_eq!(one_of_3_in_0x3f, one_of_3);
    assert_eq!(one_of_3_in_0x3f.cmp(&one_of_3), Ordering::Equal);
    assert_ne!(one_of_3_in_0x3f, bits(&[0x3F], 4));
}

#[test]
fn compares_a_view_at_a_bit_offset_by_its_bits() {
    use std::cmp::Ordering;
    // <<_:4, A:12/bits, _/bits>> = <<"abc">> gives A = <<22,2:4>> on the
    // pinned ERTS, and A > <<22>>.
    let view = Binary::from(b"abc".to_vec()).bits(4, 16);
    assert_eq!(view, bits(&[0x16, 0x2F], 12));
    assert_ne!(view, Binary::from(vec![0x16, 0x20]));
    assert_eq!(view.cmp(&bits(&[0x16, 0x30], 12)), Ordering::Less);
    assert_eq!(view.cmp(&Binary::from(vec![0x16])), Ordering::Greater);
    // <<1:3>> at bit offset 3 of <<4>> is equal to <<1:3>> at offset 0.
    assert_eq!(Binary::from(vec![0x04]).bits(3, 6), bits(&[0x20], 3));
}

#[test]
fn gives_whole_bytes_only_for_a_binary() {
    // A size that is not a whole number of bytes gives no bytes, as ERTS
    // `erts_get_aligned_binary_bytes_extra` gives NULL for it
    // (`erl_bits.h:605-635`). The bits stay readable with `bit_bytes`.
    let source = Binary::from(vec![0x3F, 0x80]);
    assert_eq!(source.bytes(), Some(&[0x3F, 0x80][..]));
    assert_eq!(source.bits(8, 16).bytes(), Some(&[0x80][..]));
    let bitstring = source.bits(0, 3);
    assert_eq!(bitstring.bytes(), None);
    assert_eq!(bitstring.bit_bytes(), &[0x3F]);
    assert_eq!(Term::Binary(bitstring).binary_bytes(), None);
    assert_eq!(Term::Binary(source).binary_bytes(), Some(&[0x3F, 0x80][..]));
    assert_eq!(Term::Integer(1).binary_bytes(), None);
}

#[test]
fn gives_the_referenced_size_of_the_erts_binary_kind() {
    // A view of at most 64 bytes is an ERTS heap binary and gives its own
    // size, and a larger view gives the size of the array that it refers to
    // (`binary_referenced_byte_size_1`, `erl_bif_binary.c:2508-2534`).
    let source = Binary::from(vec![0; 200]);
    assert_eq!(source.bits(8, 81).referenced_bytes(), 9);
    assert_eq!(source.bits(8, 9).referenced_bytes(), 0);
    assert_eq!(source.bits(3, 3 + 8 * 64).referenced_bytes(), 64);
    assert_eq!(source.bits(3, 4 + 8 * 64).referenced_bytes(), 200);
    assert_eq!(source.bits(8, 8 + 8 * 65).referenced_bytes(), 200);
    // A view marked as an off-heap sub-binary gives its source size at
    // each size, and a view made of it does not keep the mark.
    assert_eq!(source.bits(8, 81).off_heap().referenced_bytes(), 200);
    assert_eq!(
        source.bits(8, 81).off_heap().bits(0, 8).referenced_bytes(),
        1
    );
    // A view that an append built refers to its chunks.
    let chunks = Binary::from_chunks(vec![Arc::new(vec![1; 40]), Arc::new(vec![2; 40])]);
    assert_eq!(chunks.referenced_bytes(), 80);
}

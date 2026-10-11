use super::{Binary, MatchContext, segment_bits, segment_equals, segment_integer};

#[test]
fn reads_skips_and_gives_the_tail_at_each_bit_position() {
    let mut context = MatchContext::new(Binary::from(&[0xAB, 0xCD, 0xEF]));
    assert_eq!(context.read_bits(4), Some(vec![0xA0]));
    assert_eq!(context.read_bits(8), Some(vec![0xBC]));
    assert_eq!(context.read_bits(0), Some(vec![]));
    assert_eq!(context.position(), 12);
    assert_eq!(context.skip(4), Some(()));
    // The position is on a byte boundary, so the tail is a view of the bytes
    // after it.
    assert_eq!(context.tail(), Some(Binary::from(&[0xEF])));
    assert_eq!(context.position(), 16);
    assert_eq!(context.read_bits(8), Some(vec![0xEF]));
    assert_eq!(context.remaining_bits(), 0);
    // Too few bits keep the position.
    assert_eq!(context.read_bits(1), None);
    assert_eq!(context.skip(1), None);
    assert_eq!(context.position(), 24);
    assert_eq!(context.tail(), Some(Binary::from(&[])));
}

#[test]
fn reads_the_bytes_of_a_segment_as_a_view_of_the_binary() {
    // `bs_get_binary2` and the `binary` command of `bs_match` give a
    // sub-binary that shares the bytes of the binary
    // (`erts_bs_get_binary_2`, `erl_bits.c:400-424`).
    let mut context = MatchContext::new(Binary::from(&[1, 2, 3]));
    assert_eq!(context.read_bytes(16), Some(Binary::from(&[1, 2])));
    assert_eq!(context.position(), 16);
    // The view keeps the offset of its bits in the binary. The size of the
    // view is a number of bits, and the byte of a size that is not a whole
    // number of bytes holds the bits from the offset onwards, as the byte of
    // the binary holds them.
    // The high four bits of the byte 2 are `0b0000` and its low four bits
    // are `0b0011`, at the top of their byte.
    let high = context.read_bytes(4).expect("4 bits remain");
    assert_eq!(&high[..], &[0b0000_0011]);
    assert_eq!(context.position(), 20);
    let low = context.read_bytes(4).expect("4 bits remain");
    assert_eq!(&low[..], &[0b0011_0000]);
    assert_eq!(context.position(), 24);
    // A view keeps the offset of its bits in the array of its source.
    let source = Binary::from(&[0xC3]);
    assert!(source.same_bytes(&source.bits(4, 8)));
    assert_eq!(&source.bits(4, 8)[..], &[0b0011_0000]);
    // Too few bits give no view and keep the position.
    assert_eq!(context.read_bytes(16), None);
    assert_eq!(context.position(), 24);
    // A read from a position that is not on a byte boundary gives a view of
    // the bits from that position.
    let mut context = MatchContext::new(Binary::from(&[0xAB, 0xCD, 0xEF]));
    assert_eq!(context.read_bits(4), Some(vec![0xA0]));
    assert_eq!(context.read_bytes(8), Some(Binary::from(&[0xBC])));
    assert_eq!(context.position(), 12);
}

#[test]
fn shares_the_bytes_of_the_binary_that_it_matches() {
    // `erts_bs_start_match_3` writes the binary itself into the match
    // context, with no copy of its bytes (`erl_bits.c:81-105`).
    let source = Binary::from(&[1, 2, 3]);
    let context = MatchContext::new(source.clone());
    assert!(context.bytes().same_bytes(&source));
    let mut context = context;
    assert_eq!(
        context.read_bytes(16).map(|bytes| bytes.to_bytes()),
        Some(b"\x01\x02".to_vec())
    );
}

#[test]
fn gives_no_tail_view_at_a_position_that_is_not_on_a_byte_boundary() {
    // PBeam has no bitstring term, so the bits after a position that is not
    // on a byte boundary have no view.
    let mut context = MatchContext::new(Binary::from(&[0xAB, 0xCD]));
    assert_eq!(context.read_bits(4), Some(vec![0xA0]));
    assert_eq!(context.tail(), None);
    assert_eq!(context.position(), 4);
}

#[test]
fn takes_bits_at_aligned_and_unaligned_offsets() {
    assert_eq!(segment_bits(&[0xAB, 0xCD], 0, 16), vec![0xAB, 0xCD]);
    assert_eq!(segment_bits(&[0xAB, 0xCD], 0, 12), vec![0xAB, 0xC0]);
    assert_eq!(segment_bits(&[0xAB, 0xCD], 4, 12), vec![0xBC, 0xD0]);
    assert_eq!(segment_bits(&[0xAB, 0xCD], 7, 2), vec![0xC0]);
    assert_eq!(segment_bits(&[0xAB], 3, 0), Vec::<u8>::new());
}

#[test]
fn forgets_the_position_until_set_position() {
    let mut context = MatchContext::new(Binary::from(&[1]));
    assert!(context.position_known());
    context.forget_position();
    assert!(!context.position_known());
    context.set_position(8);
    assert!(context.position_known());
    assert_eq!(context.remaining_bits(), 0);
}

#[test]
fn gives_the_integer_of_each_endianness_and_sign() {
    assert_eq!(segment_integer(&[], 0, false, true), Some(0));
    assert_eq!(segment_integer(&[0xA0], 3, false, false), Some(5));
    assert_eq!(segment_integer(&[0xA0], 3, false, true), Some(-3));
    assert_eq!(
        segment_integer(&[0x01, 0x20], 12, false, false),
        Some(0x012)
    );
    // Little-endian: the last 4 bits are the most significant part.
    assert_eq!(segment_integer(&[0x23, 0x10], 12, true, false), Some(0x123));
    assert_eq!(
        segment_integer(&[0x23, 0x80], 12, true, true),
        Some(0x823 - 0x1000)
    );
    assert_eq!(
        segment_integer(&[0x01, 0x02], 16, true, false),
        Some(0x0201)
    );
    assert_eq!(segment_integer(&[0xFF, 0xFE], 16, false, true), Some(-2));
    assert_eq!(segment_integer(&[0xFE, 0xFF], 16, true, true), Some(-2));
}

#[test]
fn gives_none_for_an_integer_outside_the_64_bit_range() {
    let mut max = vec![0x7F];
    max.extend([0xFF; 7]);
    assert_eq!(segment_integer(&max, 64, false, false), Some(i64::MAX));
    let mut above = vec![0x80];
    above.extend([0; 7]);
    assert_eq!(segment_integer(&above, 64, false, false), None);
    assert_eq!(segment_integer(&above, 64, false, true), Some(i64::MIN));
    // 72 bits with 8 leading sign bits fit; one bit more does not.
    let mut wide = vec![0xFF, 0x80];
    wide.extend([0; 7]);
    assert_eq!(segment_integer(&wide, 72, false, true), Some(i64::MIN));
    let mut below = vec![0xFF, 0x7F];
    below.extend([0xFF; 7]);
    assert_eq!(segment_integer(&below, 72, false, true), None);
    let mut zero = vec![0; 9];
    zero[8] = 5;
    assert_eq!(segment_integer(&zero, 72, false, false), Some(5));
}

#[test]
fn compares_bits_with_an_unsigned_big_endian_value() {
    assert!(segment_equals(&[0x61, 0x60], 12, 0x616));
    assert!(!segment_equals(&[0x61, 0x70], 12, 0x616));
    assert!(segment_equals(&[], 0, 0));
    // A value of more bits than the segment does not match.
    assert!(!segment_equals(&[0x61], 8, 0x161));
    let mut wide = vec![0; 9];
    wide[8] = 0x80;
    assert!(segment_equals(&wide, 72, 0x80));
    wide[0] = 1;
    assert!(!segment_equals(&wide, 72, 0x80));
    assert!(segment_equals(&[0xFF; 8], 64, u64::MAX));
}

#[test]
fn reads_utf8_at_a_position_that_is_not_byte_aligned() {
    let mut context = MatchContext::new(Binary::from(&[0x0C, 0x3A, 0x90]));
    context.skip(4);
    assert_eq!(context.get_utf8(), Some(233));
    assert_eq!(context.position(), 20);
    // 4 bits remain: no byte to read, and the position stays.
    let mut context = MatchContext::new(Binary::from(&[0x04]));
    context.skip(4);
    assert_eq!(context.get_utf8(), None);
    assert_eq!(context.position(), 4);
    // 12 bits remain: a first byte of 2 bytes, but no second byte.
    let mut context = MatchContext::new(Binary::from(&[0x0C, 0x3A]));
    context.skip(4);
    assert_eq!(context.get_utf8(), None);
    assert_eq!(context.position(), 4);
}

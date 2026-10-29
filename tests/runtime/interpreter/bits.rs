use super::BitWriter;
use crate::term::{Binary, Endian};

/// The result of a writer that wrote the whole bytes `bytes`.
fn whole(bytes: Vec<u8>) -> (Vec<u8>, u64) {
    let bits = bytes.len() as u64 * 8;
    (bytes, bits)
}

#[test]
fn writes_bits_across_byte_boundaries() {
    // <<16#A:4, 16#BC:8, 16#D:4>> is <<16#AB, 16#CD>>. Only the low
    // `count` bits of a chunk are written.
    let mut writer = BitWriter::default();
    writer.push_bits(0xFA, 4);
    writer.push_bits(0xBC, 8);
    writer.push_bits(0x0D, 4);
    assert_eq!(writer.into_bytes(), whole(vec![0xAB, 0xCD]));
    // <<1:3, 2:3, 3:2>> fills one byte from the most significant bit.
    let mut writer = BitWriter::default();
    writer.push_bits(1, 3);
    writer.push_bits(2, 3);
    writer.push_bits(3, 2);
    assert_eq!(writer.into_bytes(), whole(vec![0b0010_1011]));
}

#[test]
fn writes_bytes_at_a_bit_offset() {
    let mut writer = BitWriter::default();
    writer.push_bytes(&[0x01]);
    writer.push_bits(0x2, 4);
    writer.push_bytes(&[0x34, 0x56]);
    writer.push_bits(0x7, 4);
    assert_eq!(writer.into_bytes(), whole(vec![0x01, 0x23, 0x45, 0x67]));
}

#[test]
fn writes_integers_in_each_byte_order_as_erts() {
    // The values of the pinned ERTS 29.1.1 run of
    // `fixtures/bs_create_bin/bs_create_bin_integer_probe.erl`.
    let bytes = |parts: &[(i64, u64, Endian)]| {
        let mut writer = BitWriter::default();
        for &(value, bits, little) in parts {
            writer.push_integer(value, bits, little);
        }
        writer.into_bytes()
    };
    assert_eq!(bytes(&[(0x1234, 16, Endian::Big)]), whole(vec![0x12, 0x34]));
    assert_eq!(
        bytes(&[(0x1234, 16, Endian::Little)]),
        whole(vec![0x34, 0x12])
    );
    assert_eq!(bytes(&[(263, 8, Endian::Big)]), whole(vec![7]));
    assert_eq!(bytes(&[(-1, 72, Endian::Big)]), whole(vec![0xFF; 9]));
    assert_eq!(
        bytes(&[(0x0102_0304_0506_0708, 72, Endian::Big)]),
        whole(vec![0, 1, 2, 3, 4, 5, 6, 7, 8])
    );
    assert_eq!(
        bytes(&[(-2, 72, Endian::Little)]),
        whole(vec![0xFE, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF])
    );
    assert_eq!(
        bytes(&[(0xABC, 12, Endian::Little), (0xD, 4, Endian::Big)]),
        whole(vec![0xBC, 0xAD])
    );
    assert_eq!(
        bytes(&[(5, 3, Endian::Big), (0x1234, 13, Endian::Little)]),
        whole(vec![166, 146])
    );
    assert_eq!(bytes(&[(5, 0, Endian::Big)]), whole(Vec::new()));
    // <<1:4>> keeps its 4 bits in the high bits of one byte.
    assert_eq!(bytes(&[(1, 4, Endian::Big)]), (vec![0x10], 4));
}

#[test]
fn writes_the_bits_of_a_binary() {
    // The pinned ERTS gives <<(<<1:4>>)/bitstring, (<<2:4>>)/bitstring>> =
    // <<18>>: the writer copies the 4 bits of each bitstring.
    let mut writer = BitWriter::default();
    writer.push_binary(&Binary::from(vec![0x10]).bits(0, 4), 4);
    writer.push_binary(&Binary::from(vec![0x20]).bits(0, 4), 4);
    writer.push_binary(&Binary::from(vec![0xAB]), 8);
    assert_eq!(writer.into_bytes(), whole(vec![0x12, 0xAB]));
}

#[test]
fn writes_a_prefix_of_the_bits_of_a_binary() {
    // `<<(<<255, 1>>):3/bits, 1:1>>` copies the first 3 bits of the
    // binary (`erts_bs_put_binary`, erl_bits.c:1285-1308).
    let mut writer = BitWriter::default();
    writer.push_binary(&Binary::from(vec![0xFF, 0x01]), 3);
    writer.push_bits(1, 1);
    assert_eq!(writer.into_bytes(), (vec![0xF0], 4));
}

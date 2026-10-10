use super::BitWriter;

#[test]
fn writes_bits_across_byte_boundaries() {
    // <<16#A:4, 16#BC:8, 16#D:4>> is <<16#AB, 16#CD>>. Only the low
    // `count` bits of a chunk are written.
    let mut writer = BitWriter::default();
    writer.push_bits(0xFA, 4);
    writer.push_bits(0xBC, 8);
    writer.push_bits(0x0D, 4);
    assert_eq!(writer.into_bytes(), Ok(vec![0xAB, 0xCD]));
    // <<1:3, 2:3, 3:2>> fills one byte from the most significant bit.
    let mut writer = BitWriter::default();
    writer.push_bits(1, 3);
    writer.push_bits(2, 3);
    writer.push_bits(3, 2);
    assert_eq!(writer.into_bytes(), Ok(vec![0b0010_1011]));
}

#[test]
fn writes_bytes_at_a_bit_offset() {
    let mut writer = BitWriter::default();
    writer.push_bytes(&[0x01]);
    writer.push_bits(0x2, 4);
    writer.push_bytes(&[0x34, 0x56]);
    writer.push_bits(0x7, 4);
    assert_eq!(writer.into_bytes(), Ok(vec![0x01, 0x23, 0x45, 0x67]));
}

#[test]
fn writes_integers_in_each_byte_order_as_erts() {
    // The values of the pinned ERTS 29.1.1 run of
    // `fixtures/bs_create_bin/bs_create_bin_integer_probe.erl`.
    let bytes = |parts: &[(i64, u64, bool)]| {
        let mut writer = BitWriter::default();
        for &(value, bits, little) in parts {
            writer.push_integer(value, bits, little);
        }
        writer.into_bytes()
    };
    assert_eq!(bytes(&[(0x1234, 16, false)]), Ok(vec![0x12, 0x34]));
    assert_eq!(bytes(&[(0x1234, 16, true)]), Ok(vec![0x34, 0x12]));
    assert_eq!(bytes(&[(263, 8, false)]), Ok(vec![7]));
    assert_eq!(bytes(&[(-1, 72, false)]), Ok(vec![0xFF; 9]));
    assert_eq!(
        bytes(&[(0x0102_0304_0506_0708, 72, false)]),
        Ok(vec![0, 1, 2, 3, 4, 5, 6, 7, 8])
    );
    assert_eq!(
        bytes(&[(-2, 72, true)]),
        Ok(vec![0xFE, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF])
    );
    assert_eq!(
        bytes(&[(0xABC, 12, true), (0xD, 4, false)]),
        Ok(vec![0xBC, 0xAD])
    );
    assert_eq!(
        bytes(&[(5, 3, false), (0x1234, 13, true)]),
        Ok(vec![166, 146])
    );
    assert_eq!(bytes(&[(5, 0, false)]), Ok(Vec::new()));
    assert_eq!(bytes(&[(1, 4, false)]), Err(4));
}

use super::*;

#[test]
fn gives_the_block_hash_of_erts() {
    // `erlang:phash2(Binary)` is the low 27 bits of the block hash of the
    // bytes with the seed `HCONST_13` (erl_term_hashing.c:878, 1268-1292);
    // the values come from the pinned ERTS.
    let expected = [
        (1, 76691218),
        (4, 39890977),
        (5, 81915243),
        (8, 85689312),
        (9, 46387738),
        (11, 21612627),
        (12, 103762329),
        (13, 92101820),
        (25, 10118065),
    ];
    for (length, phash2) in expected {
        let bytes: Vec<u8> = (1..=length).collect();
        assert_eq!(
            block_hash(&bytes, 0x08d1_2e65) & 0x7ff_ffff,
            phash2,
            "{length}"
        );
    }
}

#[test]
fn encodes_a_local_reference_as_erts() {
    // `#Ref<0.3420218054.3052142593.51154>` on the pinned ERTS gives these
    // bytes after its hash.
    let reference = Reference((3_420_218_054u64 << 32) | (3_052_142_593 & 0xFFFC_0000) | 51154);
    let bytes = local_reference(reference, 7);
    assert_eq!(bytes[..2], [131, 121]);
    assert_eq!(
        bytes[6..],
        [
            90, 0, 3, 106, 0, 0, 0, 0, 0, 0, 199, 210, 181, 236, 0, 1, 203, 220, 98, 198
        ]
    );
    assert_eq!(bytes[2..6], block_hash(&bytes[6..], 7).to_be_bytes());
}

#[test]
fn decodes_only_its_own_local_reference() {
    let reference = Reference((7u64 << 32) | (1 << 18) | 5);
    let bytes = local_reference(reference, 3);
    assert_eq!(decode_local_reference(&bytes, 3), Some(reference));
    assert_eq!(decode_local_reference(&bytes, 4), None);
    assert_eq!(decode_local_reference(&bytes[..20], 3), None);
    assert_eq!(decode_local_reference(&[], 3), None);
    let mut other = bytes.clone();
    other[15] ^= 1;
    assert_eq!(decode_local_reference(&other, 3), None);
}

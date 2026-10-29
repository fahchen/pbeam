use super::*;

#[test]
fn gives_the_block_hash_of_erts() {
    // `erlang:phash2(Binary)` is the low 27 bits of the block hash of the
    // bytes with the seed `HCONST_13` (erl_term_hashing.c:878, 1268-1292);
    // the values come from the pinned ERTS.
    let expected = [
        (1, 76_691_218),
        (4, 39_890_977),
        (5, 81_915_243),
        (8, 85_689_312),
        (9, 46_387_738),
        (11, 21_612_627),
        (12, 103_762_329),
        (13, 92_101_820),
        (25, 10_118_065),
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
    // `#Ref<0.198992582.3052142593.51154>` on the pinned ERTS gives these
    // bytes after its hash.
    let reference = Reference((198_992_582u64 << 32) | (3_052_142_593 & 0xFFFC_0000) | 51154);
    let bytes = local_reference(reference, 7);
    assert_eq!(bytes[..2], [131, 121]);
    assert_eq!(
        bytes[6..],
        [
            90, 0, 3, 106, 0, 0, 0, 0, 0, 0, 199, 210, 181, 236, 0, 1, 11, 220, 98, 198
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
    let mut other = bytes;
    other[15] ^= 1;
    assert_eq!(decode_local_reference(&other, 3), None);
    let magic = reference.magic_reference();
    let bytes = local_reference(magic, 3);
    assert_eq!(decode_local_reference(&bytes, 3), Some(magic));
}

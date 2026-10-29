//! The external term format of `erlang:term_to_binary/2` with the option
//! `local` for a reference, as in ERTS OTP 29.1.1 (`external.c`).
//!
//! PBeam encodes only an ordinary reference with `[local]`, which
//! `prim_inet:send/3` puts in the header of each send (`prim_inet.erl:581`):
//!
//! | Bytes | Value |
//! | --- | --- |
//! | `131` | `VERSION_MAGIC` |
//! | `121` | `LOCAL_EXT` (external.c:3305-3306) |
//! | `H3, H2, H1, H0` | the block hash of the bytes after it with the seed of the node (external.c:4046-4090) |
//! | `90` | `NEWER_REFERENCE_EXT` |
//! | `0, 3` | 3 reference numbers |
//! | `106` | `NIL_EXT`, the local node (`INTERNAL_LOCAL_NODE`, external.c:3651-3654) |
//! | `0, 0, 0, 0` | creation 0 |
//! | `N0`, `N1`, `N2` | each reference number, big-endian 32 bits |
//!
//! For example, `#Ref<0.3420218054.3052142593.51154>` gives `<<131, 121,
//! H3, H2, H1, H0, 90, 0, 3, 106, 0, 0, 0, 0, 0, 0, 199, 210, 181, 236, 0,
//! 1, 203, 220, 98, 198>>`. The seed changes from node to node
//! (`erts_late_init_external`, external.c:136-198), so the hash bytes do.

use super::Reference;

/// `VERSION_MAGIC`, the first byte of the external term format.
const VERSION_MAGIC: u8 = 131;
/// `LOCAL_EXT`, the tag of a term that only the local node decodes.
const LOCAL_EXT: u8 = 121;
/// `NEWER_REFERENCE_EXT`, the tag of a reference.
const NEWER_REFERENCE_EXT: u8 = 90;
/// `NIL_EXT`, which stands for the local node in a `LOCAL_EXT` term.
const NIL_EXT: u8 = 106;

/// Return the bytes of `term_to_binary(Reference, [local])` for an
/// ordinary reference, with the seed `seed` of the node (see the module
/// documentation).
///
/// # Examples
///
/// ```rust,ignore
/// let bytes = local_reference(Reference(0), 0);
/// assert_eq!(bytes[..2], [131, 121]);
/// assert_eq!(bytes[6..], [90, 0, 3, 106, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 1, 0, 0, 0, 0]);
/// ```
pub(crate) fn local_reference(reference: Reference, seed: u32) -> Vec<u8> {
    let mut term = vec![NEWER_REFERENCE_EXT, 0, 3, NIL_EXT, 0, 0, 0, 0];
    for number in reference.numbers() {
        term.extend(number.to_be_bytes());
    }
    let hash = block_hash(&term, seed);
    [&[VERSION_MAGIC, LOCAL_EXT][..], &hash.to_be_bytes(), &term].concat()
}

/// Return the reference of `bytes` when they are the bytes of
/// [`local_reference`] with the seed `seed`, else `None`. The driver of a
/// port decodes the header of `prim_inet:send/3` with them
/// (`ERL_DRV_EXT2TERM`, inet_drv.c:2642-2666).
///
/// # Examples
///
/// ```rust,ignore
/// let bytes = local_reference(Reference(5), 9);
/// assert_eq!(decode_local_reference(&bytes, 9), Some(Reference(5)));
/// assert_eq!(decode_local_reference(&bytes, 8), None);
/// ```
pub(crate) fn decode_local_reference(bytes: &[u8], seed: u32) -> Option<Reference> {
    let numbers: [u8; 12] = bytes.get(14..)?.try_into().ok()?;
    let number = |index: usize| {
        let word: [u8; 4] = numbers[4 * index..4 * index + 4]
            .try_into()
            .expect("4 bytes");
        u64::from(u32::from_be_bytes(word))
    };
    let value = (number(2) << 32) | (number(1) & 0xFFFC_0000) | number(0);
    let reference = Reference::ordinary(value)?;
    // Bit 17 of `N1` marks a magic reference (`ERTS_REF1_MAGIC_MARKER_BIT__`,
    // erl_bif_unique.h:102-105).
    let reference = if number(1) & (1 << 17) != 0 {
        reference.magic_reference()
    } else {
        reference
    };
    (local_reference(reference, seed) == bytes).then_some(reference)
}

/// The golden ratio, the start value of `a` and `b` (`HCONST`,
/// `erl_term_hashing.c:440`).
const HCONST: u32 = 0x9e37_79b9;

/// Return the block hash of `bytes` with the start value `seed`, as
/// `erts_block_hash` gives it (erl_term_hashing.c:440-575): the Bob
/// Jenkins hash of 1996, 12 bytes for each mix, with the total length in
/// `c` before the last mix.
///
/// # Examples
///
/// ```rust,ignore
/// assert_eq!(block_hash(&[], 0), block_hash(&[], 0));
/// assert_ne!(block_hash(&[1], 0), block_hash(&[1], 1));
/// ```
pub(crate) fn block_hash(bytes: &[u8], seed: u32) -> u32 {
    iov_block_hash(bytes, bytes.len(), seed)
}

/// Return the block hash of the bytes of an I/O vector whose last element
/// holds `last` bytes, as `erts_iov_block_hash` gives it
/// (erl_term_hashing.c:605-705). When the bytes after the last whole
/// 12-byte block start in an earlier element, the function adds only the
/// first bytes of them, as many as the last element gives to them:
/// `no_final_bytes` takes the size of the last piece and not the count in
/// `buf` (erl_term_hashing.c:655-673).
///
/// # Examples
///
/// ```rust,ignore
/// assert_eq!(iov_block_hash(&[1, 2], 2, 0), block_hash(&[1, 2], 0));
/// assert_eq!(iov_block_hash(&[1, 2], 1, 0), iov_block_hash(&[1, 3], 1, 0));
/// ```
pub(crate) fn iov_block_hash(bytes: &[u8], last: usize, seed: u32) -> u32 {
    let word = |chunk: &[u8]| {
        chunk.iter().enumerate().fold(0u32, |sum, (index, &byte)| {
            sum.wrapping_add(u32::from(byte) << (8 * index))
        })
    };
    let (mut a, mut b, mut c) = (HCONST, HCONST, seed);
    let (blocks, rest) = bytes.as_chunks::<12>();
    for block in blocks {
        a = a.wrapping_add(word(&block[0..4]));
        b = b.wrapping_add(word(&block[4..8]));
        c = c.wrapping_add(word(&block[8..12]));
        (a, b, c) = mix(a, b, c);
    }
    let rest = &rest[..rest.len().min(last)];
    // The low byte of `c` holds the length (erl_term_hashing.c:484-503).
    c = c.wrapping_add(bytes.len() as u32);
    a = a.wrapping_add(word(&rest[..rest.len().min(4)]));
    b = b.wrapping_add(word(rest.get(4..rest.len().min(8)).unwrap_or(&[])));
    c = c.wrapping_add(word(rest.get(8..).unwrap_or(&[])) << 8);
    mix(a, b, c).2
}

/// The `MIX` step of the block hash (erl_term_hashing.c:428-439).
fn mix(mut a: u32, mut b: u32, mut c: u32) -> (u32, u32, u32) {
    for (left, right, mid) in [(13, 8, 13), (12, 16, 5), (3, 10, 15)] {
        a = a.wrapping_sub(b).wrapping_sub(c) ^ (c >> left);
        b = b.wrapping_sub(c).wrapping_sub(a) ^ (a << right);
        c = c.wrapping_sub(a).wrapping_sub(b) ^ (b >> mid);
    }
    (a, b, c)
}

#[cfg(test)]
#[path = "../../tests/term/external.rs"]
mod tests;

//! The layout of a BEAM binary.
//!
//! A binary holds its bytes in one array of `Arc<Vec<u8>>`, with the bit
//! offset and the size of the view of this binary in that array. Copies of
//! a binary share the array, as an ERTS copy of a reference-counted binary
//! only increments the reference count (`copy.c:812-840`). A sub-binary
//! keeps the array of its source and changes only its offset and size, as
//! an ERTS sub-binary refers to the original binary and keeps its offset
//! (`binary_referenced_byte_size_1`, `erl_bif_binary.c:2508-2530`). The
//! array is immutable, so a view needs no copy.
//!
//! `Binary` derefs to `[u8]`, so each reader uses the byte slice operations
//! and cannot see the sharing. The size is a number of bits, because an
//! ERTS sub-bitstring can start and end at any bit
//! (`erts_bs_get_binary_2`, `erl_bits.c:400-424`). PBeam has no bitstring
//! term, so a size that is not a whole number of bytes comes only from a
//! segment that PBeam does not run yet, and each such path stops with a
//! runtime error (row "Bitstring terms" of `docs/erts-differences.md`).
//! [`Binary::to_bytes`] gives an owned vector to the code that changes
//! bytes, such as a `bs_create_bin` append segment. The bytes of a binary
//! contain no term, so the drop of a deep term does not walk them, and a
//! recursive drop needs no work stack.
//!
//! A sub-binary keeps the offset (`docs/portable-beam-spec.md:97`). PBeam
//! does not implement `binary:referenced_byte_size/1`, which reports the
//! size of the whole array of a sub-binary, so no code reads the size of
//! the array.

use std::{fmt, ops::Deref, sync::Arc};

use super::Term;

/// The bytes of a BEAM binary: the array that holds them, the bit offset of
/// the first bit of this binary in the array, and its size in bits. All
/// copies of the binary own one reference to the array. The array holds the
/// bytes of the source binary of the view, so a sub-binary keeps the offset
/// of its bits. The last copy that drops releases the array.
#[derive(Clone)]
pub struct Binary {
    /// The array of the bytes of the source binary, from its first byte.
    bytes: Arc<Vec<u8>>,
    /// The offset in bits of the first bit of this binary in the array.
    offset: usize,
    /// The number of bits of this binary.
    size: usize,
    /// The bytes of the view, with the bits of an offset that is not on a
    /// byte boundary packed as [`Binary::deref`] gives them. `None` when the
    /// offset is on a byte boundary, because the view is then a slice of the
    /// array with no copy. The `Arc` keeps a copy of such a binary O(1), as
    /// a copy of a byte-aligned binary is.
    packed: Option<Arc<Vec<u8>>>,
}

impl Binary {
    /// Return the shared array of the bytes. Two binaries that return one
    /// array keep their bytes in one place.
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// let bytes = Binary::from(b"ab".to_vec());
    /// assert_eq!(bytes.bytes().len(), 2);
    /// ```
    #[cfg(test)]
    pub(crate) fn bytes(&self) -> &Arc<Vec<u8>> {
        &self.bytes
    }

    /// Return the packed bits of a view whose size is not a whole number of
    /// bytes, or `None` for each other view.
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// let view = Binary::from(&[0xC3]).bits(4, 8);
    /// assert!(view.packed().is_some());
    /// ```
    #[cfg(test)]
    pub(crate) fn packed(&self) -> Option<&Arc<Vec<u8>>> {
        self.packed.as_ref()
    }

    /// Return the bits from the bit position `start` to the bit position
    /// `end` of the array of this binary as a view of the same array, as an
    /// ERTS sub-bitstring refers to the original binary. The bit positions
    /// are from the first bit of this binary. The view keeps the offset of
    /// its bits, so it shares the array with its source.
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// let bytes = Binary::from(b"abc".to_vec());
    /// assert_eq!(bytes.bits(4, 20).to_vec(), vec![0x1B, 0xC0]);
    /// ```
    ///
    /// # Panics
    ///
    /// Panic when a bit position is outside the bits of this binary.
    pub(crate) fn bits(&self, start: usize, end: usize) -> Binary {
        assert!(
            start <= end && end <= self.size,
            "the bit range {start}..{end} is outside the binary"
        );
        let offset = self.offset + start;
        let size = end - start;
        let packed = (!offset.is_multiple_of(BITS_PER_BYTE)).then(|| {
            let first = byte(offset);
            // The segment can end in the byte after its last whole byte, so
            // the bits include that byte. `pack_bits` reads one byte past the
            // slice, and the last byte of the array has no byte after it.
            let end = (first + size.div_ceil(BITS_PER_BYTE) + 1).min(self.bytes.len());
            Arc::new(pack_bits(
                &self.bytes[first..end],
                offset % BITS_PER_BYTE,
                size,
            ))
        });
        Binary {
            bytes: Arc::clone(&self.bytes),
            offset,
            size,
            packed,
        }
    }

    /// Return whether the two binaries keep their bytes in one array, as two
    /// ERTS binary terms that refer to one binary keep their bytes in one
    /// place.
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// let bytes = Binary::from(b"ab".to_vec());
    /// assert!(bytes.same_bytes(&bytes.clone()));
    /// assert!(!bytes.same_bytes(&Binary::from(b"ab".to_vec())));
    /// ```
    pub fn same_bytes(&self, other: &Binary) -> bool {
        Arc::ptr_eq(&self.bytes, &other.bytes)
    }

    /// Return the bytes of this binary as an owned vector. A size that is
    /// not a whole number of bytes gives the extra bits in the high bits of
    /// the last byte, as [`MatchContext::read_bits`] packs them. The
    /// function copies the bytes, and leaves the binary and its array
    /// unchanged.
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// let bytes = Binary::from(b"ab".to_vec());
    /// assert_eq!(bytes.to_bytes(), b"ab".to_vec());
    /// assert_eq!(bytes.len(), 2);
    /// ```
    pub(crate) fn to_bytes(&self) -> Vec<u8> {
        // The bytes are copied, because a term cannot give up the array of a
        // binary: `Term` has a `Drop`, and the binary lives until the term
        // drops. No supported path needs the array of the source.
        self[..].to_vec()
    }
}

impl PartialEq for Binary {
    /// Compare the two binaries by their bytes. Two binaries that share one
    /// array, one offset, and one size have the same bytes. The size of the
    /// array that holds the bytes does not change the result: two binaries
    /// with equal bytes are equal, also when one array is longer. ERTS
    /// compares two bitstrings by their bits and then by their sizes
    /// (`erts_cmp_compound`, `utils.c:2100-2125`). PBeam has no bitstring
    /// term, and no supported path compares two binaries with a size that is
    /// not a whole number of bytes.
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// let source = Binary::from(b"abc".to_vec());
    /// assert_eq!(source.bits(0, 16), Binary::from(b"ab".to_vec()));
    /// assert_ne!(source.bits(0, 16), Binary::from(b"ac".to_vec()));
    /// ```
    fn eq(&self, other: &Self) -> bool {
        // A view of one array with the same offset and length has the same
        // bytes. Each other pair compares the bytes, so the size of the
        // array does not change the result.
        (self.same_bytes(other) && self.offset == other.offset && self.size == other.size)
            || self[..] == other[..]
    }
}

impl Eq for Binary {}

/// The number of bits of a byte.
const BITS_PER_BYTE: usize = 8;

/// Return the index of the byte of the bit `bit`.
///
/// # Examples
///
/// ```rust,ignore
/// assert_eq!(byte(8), 1);
/// ```
fn byte(bit: usize) -> usize {
    bit / BITS_PER_BYTE
}

/// Return the `size` bits of `bytes` from the bit `shift`, packed in bytes
/// as [`MatchContext::read_bits`] gives them: the first bit is the high bit
/// of the first byte, and the bits after the size in the last byte are 0.
///
/// # Examples
///
/// ```rust,ignore
/// assert_eq!(pack_bits(&[0xC3], 4, 4), vec![0b0011_0000]);
/// ```
fn pack_bits(bytes: &[u8], shift: usize, size: usize) -> Vec<u8> {
    let mut packed: Vec<u8> = (0..size.div_ceil(BITS_PER_BYTE))
        .map(|index| {
            let next = bytes.get(index + 1).copied().unwrap_or(0);
            (bytes[index] << shift) | (next >> (BITS_PER_BYTE - shift))
        })
        .collect();
    if let Some(last) = packed.last_mut() {
        *last &= 0xFF_u8 << ((BITS_PER_BYTE - size % BITS_PER_BYTE) % BITS_PER_BYTE);
    }
    packed
}

impl Deref for Binary {
    type Target = [u8];

    /// Return the bytes of the binary. The bytes from the byte of the
    /// offset, and of a size that is not a whole number of bytes the bits
    /// from the offset onward in the high bits of the last byte, as an ERTS
    /// bitstring holds them (`erl_bits.h`). No supported path forms such a
    /// binary yet.
    fn deref(&self) -> &[u8] {
        match &self.packed {
            Some(packed) => packed,
            // The offset is on a byte boundary, so the view is the bytes from
            // the byte of the offset to the last byte of the size. A size of
            // 0 gives no byte, also at the end of the array.
            None if self.size == 0 => &[],
            None => {
                let first = byte(self.offset);
                let end = self.bytes.len().min(byte(self.offset + self.size - 1) + 1);
                &self.bytes[first..end]
            }
        }
    }
}

impl<const N: usize> From<&[u8; N]> for Binary {
    /// Make a binary of a byte array. The array of the binary holds a copy
    /// of the bytes, and copies of the binary share it.
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// let binary = Binary::from(&[1, 2]);
    /// assert_eq!(&*binary, &[1, 2]);
    /// ```
    fn from(bytes: &[u8; N]) -> Self {
        Binary::from(bytes.as_slice())
    }
}

impl From<&[u8]> for Binary {
    /// Make a binary of a byte slice. The array of the binary holds a copy
    /// of the bytes, and copies of the binary share it.
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// let binary = Binary::from(&[1, 2][..]);
    /// assert_eq!(&*binary, &[1, 2]);
    /// ```
    fn from(bytes: &[u8]) -> Self {
        Binary::from(bytes.to_vec())
    }
}

impl From<Vec<u8>> for Binary {
    /// Make a binary of `bytes`. The array of the binary holds the bytes, and
    /// copies of the binary share it.
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// let binary = Binary::from(vec![1, 2]);
    /// assert_eq!(&*binary, &[1, 2]);
    /// ```
    fn from(bytes: Vec<u8>) -> Self {
        let size = bytes.len() * BITS_PER_BYTE;
        Binary {
            bytes: Arc::new(bytes),
            offset: 0,
            size,
            packed: None,
        }
    }
}

impl fmt::Debug for Binary {
    /// Write the bytes of the binary, for example `Binary([1, 2])`, as the
    /// derived `Debug` of the `Term::Binary` variant wrote them.
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// assert_eq!(format!("{:?}", Binary::from(vec![1])), "Binary([1])");
    /// ```
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "{:?}", &**self)
    }
}

impl PartialOrd for Binary {
    /// Compare the bytes, then the sizes, as `erts_cmp_compound` compares two
    /// bitstrings (`utils.c:2100-2125`).
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// assert!(Binary::from(vec![1]) < Binary::from(vec![2]));
    /// ```
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for Binary {
    /// Compare the bytes, then the sizes, as `erts_cmp_compound` compares two
    /// bitstrings: a shorter binary is less when it is a prefix of the other
    /// (`utils.c:2100-2125`).
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// use std::cmp::Ordering;
    /// assert_eq!(Binary::from(vec![1]).cmp(&Binary::from(vec![1])), Ordering::Equal);
    /// assert_eq!(Binary::from(vec![1]).cmp(&Binary::from(vec![1, 2])), Ordering::Less);
    /// ```
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        self[..].cmp(&other[..])
    }
}

impl Term {
    /// Build a binary of `bytes`. The array of the binary holds the bytes,
    /// and copies of the binary share it.
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// let binary = Term::binary(vec![1, 2]);
    /// assert_eq!(binary, Term::binary(vec![1, 2]));
    /// ```
    pub fn binary(bytes: Vec<u8>) -> Term {
        Term::Binary(bytes.into())
    }
}

#[cfg(test)]
#[path = "../../tests/term/binary.rs"]
mod tests;

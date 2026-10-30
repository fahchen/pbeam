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
//! [`Binary::bytes`] gives the bytes only for a whole number of bytes, so
//! each reader states what it does for a bitstring, and no reader sees the
//! sharing. [`Binary::bit_bytes`] gives the bytes that hold the bits to a
//! reader of bits. The size is a number of bits, because an
//! ERTS sub-bitstring can start and end at any bit
//! (`erts_bs_get_binary_2`, `erl_bits.c:400-424`). [`Binary::bits`] copies
//! no byte when the first bit is on a byte boundary. For a first bit that
//! is not on a byte boundary, it packs the bits of the view in a vector at
//! that call, and the vector is in one `Arc`, so a copy of such a view is
//! O(1). The `binary` command of `bs_match` reads a segment of any size at
//! a bit position (`GET_BITSTRING`, jit/arm/instr_bs.cpp:3136-3176, and
//! `EXTRACT_BITSTRING`, instr_bs.cpp:3047-3054, for 64 bits or fewer), so
//! the packed view of that command is a supported path. A view whose size
//! is not a whole number of bytes is a bitstring.
//! [`Binary::to_bytes`] gives an owned vector to the code that changes
//! bytes, such as a `bs_create_bin` append segment. The bytes of a binary
//! contain no term, so the drop of a deep term does not walk them, and a
//! recursive drop needs no work stack.
//!
//! A sub-binary keeps the offset (`docs/portable-beam-spec.md:97`).
//! `binary:referenced_byte_size/1` reads the size of the whole array of a
//! view of more than 64 bytes ([`Binary::referenced_bytes`]).
//!
//! [`Binary`] is one `Arc` to a [`BinaryView`], so a binary term is one
//! pointer and holds no descriptor of its own. A copy of a term then copies
//! one pointer instead of the four words of the view, and a read of the
//! offset, the size, or the packed bytes follows one dependent load.
//! `Term::Binary` is 8 bytes, so every compound arm of `Term` is at most 8
//! bytes (`docs/term-layout-design.md` section 4).

use std::{
    fmt,
    ops::Deref,
    sync::{Arc, OnceLock},
};

use super::Term;

/// The bytes of a BEAM binary: the array that holds them, the bit offset of
/// the first bit of this binary in the array, and its size in bits. All
/// copies of the binary own one reference to the array. The array holds the
/// bytes of the source binary of the view, so a sub-binary keeps the offset
/// of its bits. The last copy that drops releases the array.
struct BinaryView {
    /// The array of the bytes of the source binary, from its first byte.
    /// The array is empty when the view keeps its bytes in `chunks`.
    #[expect(
        clippy::rc_buffer,
        reason = "`Binary` shares byte arrays as `Arc<Vec<u8>>` because `append_in_place` grows the vector through `Arc::get_mut`; an `Arc<[u8]>` cannot grow"
    )]
    bytes: Arc<Vec<u8>>,
    /// The chunks of a view that an append built, or `None` for each other
    /// view. Each chunk is an immutable array, and an append shares the
    /// chunks of its prefix, so an append copies no old payload byte. The
    /// view keeps the chunks in order, and their bytes are the bytes of the
    /// view.
    chunks: Option<Arc<Chunks>>,
    /// The bytes of `chunks` joined in one array, built at the first read
    /// that needs the contiguous bytes. `None` until such a read.
    #[expect(
        clippy::rc_buffer,
        reason = "`Binary` shares byte arrays as `Arc<Vec<u8>>` because `append_in_place` grows the vector through `Arc::get_mut`; an `Arc<[u8]>` cannot grow"
    )]
    flat: OnceLock<Arc<Vec<u8>>>,
    /// The offset in bits of the first bit of this binary in the array.
    offset: usize,
    /// The number of bits of this binary.
    size: usize,
    /// The bytes of the view, with the bits of an offset that is not on a
    /// byte boundary packed as [`Binary::bit_bytes`] gives them. `None` when the
    /// offset is on a byte boundary, because the view is then a slice of the
    /// array with no copy. The `Arc` keeps a copy of such a binary O(1), as
    /// a copy of a byte-aligned binary is.
    #[expect(
        clippy::rc_buffer,
        reason = "`Binary` shares byte arrays as `Arc<Vec<u8>>` because `append_in_place` grows the vector through `Arc::get_mut`; an `Arc<[u8]>` cannot grow"
    )]
    packed: Option<Arc<Vec<u8>>>,
    /// Whether the view is a sub-binary of an off-heap binary even when it
    /// holds at most 512 bits, as each element of ERTS `term_to_iovec/1,2`
    /// is (`external.c:2689-2719`, `3958`). Only [`Binary::off_heap`] sets it.
    off_heap: bool,
}

/// The chunks of a view that an append built, in order, and their index. The
/// index is in this allocation and not in [`BinaryView`], so a view that no
/// append built keeps its descriptor size.
struct Chunks {
    /// The chunks, each one an immutable array.
    #[expect(
        clippy::rc_buffer,
        reason = "`Binary` shares byte arrays as `Arc<Vec<u8>>` because `append_in_place` grows the vector through `Arc::get_mut`; an `Arc<[u8]>` cannot grow"
    )]
    arrays: Vec<Arc<Vec<u8>>>,
    /// The byte offset of the end of each chunk, in order. The view builds
    /// the index with the chunk list, in the same walk of the chunks that
    /// sums their size, so a ranged read does no O(chunks) work.
    ends: Vec<usize>,
}

impl Deref for Chunks {
    type Target = Vec<Arc<Vec<u8>>>;

    /// Return the chunks in order.
    fn deref(&self) -> &Self::Target {
        &self.arrays
    }
}

/// One BEAM binary term: one `Arc` to the [`BinaryView`] that holds its
/// bytes, its bit offset, its size, and its packed bytes. The handle is one
/// pointer, so a binary term is 8 bytes and a copy of the term increments
/// one reference count. A read of a field of the view follows the pointer,
/// which is the one dependent load that the boxed descriptor adds.
#[derive(Clone)]
pub struct Binary(Arc<BinaryView>);

/// The size of one leaf of an append-built binary. An append merges a small
/// later segment in the last leaf of its prefix, so the merge copies at most
/// this many bytes. The size is an internal storage choice of the append
/// tree, not an ERTS binary-size limit
/// (`docs/term-performance-design-codex-v2.md`, the append-tree paragraph).
const APPEND_LEAF: usize = 4096;

impl Binary {
    /// Return the shared array of the bytes. Two binaries that return one
    /// array keep their bytes in one place.
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// let bytes = Binary::from(b"ab".to_vec());
    /// assert_eq!(bytes.array().len(), 2);
    /// ```
    #[cfg(test)]
    #[expect(
        clippy::rc_buffer,
        reason = "`Binary` shares byte arrays as `Arc<Vec<u8>>` because `append_in_place` grows the vector through `Arc::get_mut`; an `Arc<[u8]>` cannot grow"
    )]
    pub(crate) fn array(&self) -> &Arc<Vec<u8>> {
        &self.0.bytes
    }

    /// Return the bytes of this binary when its size is a whole number of
    /// bytes, or `None` for a bitstring whose size is not. Each caller
    /// states what it does for a bitstring: most BIFs that take a binary
    /// raise `badarg` for it, as ERTS `erts_get_aligned_binary_bytes_extra`
    /// gives no bytes for a bitstring whose size is not a multiple of 8
    /// (`erl_bits.h:605-635`).
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// assert_eq!(Binary::from(b"ab".to_vec()).bytes(), Some(&b"ab"[..]));
    /// assert_eq!(Binary::from(vec![32]).bits(0, 3).bytes(), None);
    /// ```
    pub fn bytes(&self) -> Option<&[u8]> {
        self.0
            .size
            .is_multiple_of(BITS_PER_BYTE)
            .then(|| self.0.bytes_of_view())
    }

    /// Return the bytes that hold the bits of this binary, for a caller
    /// that reads bits and not whole bytes, such as a binary match or the
    /// order of two bitstrings. The bytes are from the byte of the offset.
    /// A size that is not a whole number of bytes gives its last bits in the
    /// high bits of the last byte, as an ERTS bitstring holds them
    /// (`erl_bits.h`), and [`Binary::bit_size`] gives the number of bits
    /// to read. The low bits of that last byte are not part of the binary.
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// assert_eq!(Binary::from(vec![0x3F]).bits(0, 3).bit_bytes(), &[0x3F]);
    /// ```
    pub(crate) fn bit_bytes(&self) -> &[u8] {
        self.0.bytes_of_view()
    }

    /// Return the shared array of the bytes for the live-byte walk. Two
    /// binaries that return one array keep their bytes in one place. A view
    /// that an append built returns its empty source array and keeps its
    /// bytes in the chunks of [`Binary::chunk_backing`].
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// let bytes = Binary::from(b"ab".to_vec());
    /// assert_eq!(bytes.backing().len(), 2);
    /// ```
    #[expect(
        clippy::rc_buffer,
        reason = "`Binary` shares byte arrays as `Arc<Vec<u8>>` because `append_in_place` grows the vector through `Arc::get_mut`; an `Arc<[u8]>` cannot grow"
    )]
    pub(super) fn backing(&self) -> &Arc<Vec<u8>> {
        &self.0.bytes
    }

    /// Return the chunks of a view that an append built, or `None` for each
    /// other view, for the live-byte walk. Each chunk is one allocation that
    /// the view keeps.
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// assert!(Binary::from(b"ab".to_vec()).chunk_backing().is_none());
    /// ```
    #[cfg(test)]
    fn chunk_backing(&self) -> Option<&Arc<Chunks>> {
        self.0.chunks.as_ref()
    }

    /// Return the array that joins the chunks of a view that an append
    /// built, or an empty array for each other view, for the accounting and
    /// the tests.
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// assert!(Binary::from(b"ab".to_vec()).flat_backing().is_empty());
    /// ```
    #[cfg(test)]
    #[expect(
        clippy::rc_buffer,
        reason = "`Binary` shares byte arrays as `Arc<Vec<u8>>` because `append_in_place` grows the vector through `Arc::get_mut`; an `Arc<[u8]>` cannot grow"
    )]
    pub(super) fn flat_backing(&self) -> &Arc<Vec<u8>> {
        self.0.flat_backing()
    }

    /// Return whether the two handles own one view. Two handles that share
    /// one view share one descriptor allocation, so the live-byte walk
    /// counts that allocation once.
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// let bytes = Binary::from(b"ab".to_vec());
    /// assert!(bytes.same_view(&bytes.clone()));
    /// assert!(!bytes.same_view(&Binary::from(b"ab".to_vec())));
    /// ```
    pub(super) fn same_view(&self, other: &Binary) -> bool {
        Arc::ptr_eq(&self.0, &other.0)
    }

    /// Return the bytes of the view descriptor that this handle owns: the
    /// size of the fields behind the pointer. The number excludes the header
    /// of the `Arc`, as the accounting of task
    /// [#559](https://github.com/fahchen/pbeam/issues/559) excludes it.
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// let bytes = Binary::from(b"abc".to_vec());
    /// assert_eq!(Binary::view_bytes(), std::mem::size_of::<usize>() * 4);
    /// ```
    pub(super) fn view_bytes() -> usize {
        std::mem::size_of::<BinaryView>()
    }

    /// Return the result of `binary:referenced_byte_size/1` for this binary
    /// (`binary_referenced_byte_size_1`, `erl_bif_binary.c:2508-2534`). ERTS
    /// gives the whole bytes of a heap binary, which holds at most 512
    /// bits (`ERL_ONHEAP_BINARY_LIMIT`, `erl_bits.h:166-167`), and the size
    /// of the whole refc binary that a larger view refers to. PBeam keeps no
    /// heap binary, so a view of at most 512 bits gives its whole bytes, and a
    /// larger view gives the size of its source array, or of its chunks.
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// let bytes = Binary::from(vec![0; 200]);
    /// assert_eq!(bytes.bits(8, 81).referenced_bytes(), 9);
    /// assert_eq!(bytes.bits(8, 1600).referenced_bytes(), 200);
    /// ```
    pub(crate) fn referenced_bytes(&self) -> usize {
        // `BYTE_SIZE` rounds down (`erl_bits.h:47`), and a heap bitstring
        // holds at most `ERL_ONHEAP_BITS_LIMIT` bits (`erl_bits.h:168`).
        if !self.0.off_heap && self.0.size <= 64 * BITS_PER_BYTE {
            return self.0.size / BITS_PER_BYTE;
        }
        match &self.0.chunks {
            Some(chunks) => chunks.iter().map(|chunk| chunk.len()).sum(),
            None => self.0.bytes.len(),
        }
    }

    /// Return this view as a sub-binary of an off-heap binary, so that
    /// [`Binary::referenced_bytes`] gives the size of its source array or
    /// chunks at each size, as for an element of ERTS `term_to_iovec/1,2`
    /// (`external.c:2689-2719`, `3958`). A view that [`Binary::bits`] makes
    /// of it does not keep the mark.
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// let bytes = Binary::from(vec![0; 8]);
    /// assert_eq!(bytes.bits(0, 8).off_heap().referenced_bytes(), 8);
    /// ```
    pub(crate) fn off_heap(&self) -> Binary {
        let view = &*self.0;
        Binary(Arc::new(BinaryView {
            bytes: Arc::clone(&view.bytes),
            chunks: view.chunks.clone(),
            flat: view.flat.clone(),
            offset: view.offset,
            size: view.size,
            packed: view.packed.clone(),
            off_heap: true,
        }))
    }

    /// Return whether the first bit of this binary is on a byte boundary of
    /// its source array, as `BIT_OFFSET(offset) == 0` tells for an ERTS
    /// bitstring (`external.c:3864-3867`).
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// let bytes = Binary::from(b"abc".to_vec());
    /// assert!(!bytes.bits(4, 12).starts_on_byte());
    /// ```
    pub(crate) fn starts_on_byte(&self) -> bool {
        self.0.offset.is_multiple_of(BITS_PER_BYTE)
    }

    /// Return the number of bits of this binary. The size comes from the
    /// view, so a binary that an append built keeps its chunks and does not
    /// join them, as [`Binary::bit_bytes`] does at its first read.
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// let bytes = Binary::from(b"abc".to_vec());
    /// assert_eq!(bytes.bit_size(), 24);
    /// assert_eq!(bytes.bits(4, 12).bit_size(), 8);
    /// ```
    pub(crate) fn bit_size(&self) -> usize {
        self.0.size
    }

    /// Return the bytes of the backing array of this binary, and of the
    /// packed array of a view whose offset is not on a byte boundary: the
    /// capacity of each array. The number excludes the header of the `Arc`
    /// and the header of the `Vec`, as the accounting of task
    /// [#559](https://github.com/fahchen/pbeam/issues/559) excludes them.
    /// A view counts the whole backing array of its source, because the
    /// array is one allocation and the view keeps it.
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// let bytes = Binary::from(b"abc".to_vec());
    /// assert!(bytes.allocated_bytes() >= 3);
    /// assert!(bytes.bits(4, 12).allocated_bytes() > bytes.allocated_bytes());
    /// ```
    pub(crate) fn allocated_bytes(&self) -> usize {
        let chunks = self.0.chunks.as_ref().map_or(0, |chunks| {
            chunks.iter().map(|chunk| chunk.capacity()).sum()
        });
        self.0.bytes.capacity()
            + chunks
            + self.0.flat.get().map_or(0, |flat| flat.capacity())
            + self.0.packed.as_ref().map_or(0, |packed| packed.capacity())
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
    #[expect(
        clippy::rc_buffer,
        reason = "`Binary` shares byte arrays as `Arc<Vec<u8>>` because `append_in_place` grows the vector through `Arc::get_mut`; an `Arc<[u8]>` cannot grow"
    )]
    pub(crate) fn packed(&self) -> Option<&Arc<Vec<u8>>> {
        self.0.packed.as_ref()
    }

    /// Return the packed array of a view whose offset is not on a byte
    /// boundary, or `None` for each other view, for the live-byte walk.
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// assert!(Binary::from(&[0xC3]).bits(4, 8).packed_backing().is_some());
    /// ```
    #[expect(
        clippy::rc_buffer,
        reason = "`Binary` shares byte arrays as `Arc<Vec<u8>>` because `append_in_place` grows the vector through `Arc::get_mut`; an `Arc<[u8]>` cannot grow"
    )]
    pub(super) fn packed_backing(&self) -> Option<&Arc<Vec<u8>>> {
        self.0.packed.as_ref()
    }

    /// Return the bits from the bit position `start` to the bit position
    /// `end` of the array of this binary as a view of the same array, as an
    /// ERTS sub-bitstring refers to the original binary. The bit positions
    /// are from the first bit of this binary. The view keeps the offset of
    /// its bits, so it shares the array with its source.
    ///
    /// The view copies no byte when the bit position `start` is on a byte
    /// boundary. Else the bits are packed in a vector of one `Arc` at this
    /// call, so a copy of the view is O(1), and the bits after the size in
    /// the last byte are 0, as an ERTS bitstring has them (`erl_bits.h`).
    /// The `binary` command of `bs_match` reads a whole number of bytes at
    /// such a position (`GET_BITSTRING`, jit/arm/instr_bs.cpp:3136-3176).
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// let bytes = Binary::from(b"abc".to_vec());
    /// assert_eq!(bytes.bits(4, 20).bytes(), Some(&[0x16, 0x26][..]));
    /// ```
    ///
    /// # Panics
    ///
    /// Panic when a bit position is outside the bits of this binary.
    pub(crate) fn bits(&self, start: usize, end: usize) -> Binary {
        let view = &self.0;
        assert!(
            start <= end && end <= view.size,
            "the bit range {start}..{end} is outside the binary"
        );
        let offset = view.offset + start;
        let size = end - start;
        let flat = view.flat_backing();
        let packed = (!offset.is_multiple_of(BITS_PER_BYTE)).then(|| {
            let first = byte(offset);
            // The segment can end in the byte after its last whole byte, so
            // the bits include that byte. `pack_bits` reads one byte past the
            // slice, and the last byte of the array has no byte after it.
            let end = (first + size.div_ceil(BITS_PER_BYTE) + 1).min(flat.len());
            Arc::new(pack_bits(&flat[first..end], offset % BITS_PER_BYTE, size))
        });
        Binary(Arc::new(BinaryView {
            bytes: Arc::clone(flat),
            chunks: None,
            flat: OnceLock::new(),
            offset,
            size,
            packed,
            off_heap: false,
        }))
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
        match (&self.0.chunks, &other.0.chunks) {
            (Some(left), Some(right)) => Arc::ptr_eq(left, right),
            (None, None) => Arc::ptr_eq(&self.0.bytes, &other.0.bytes),
            (Some(_), None) | (None, Some(_)) => false,
        }
    }

    /// Take the bytes of this binary as an owned vector. The function moves
    /// the array out when this binary was its last holder, and else it
    /// copies the bytes of the view. A view with packed bits copies its
    /// packed vector, because those bits are not whole bytes.
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// let bytes = Binary::from(b"ab".to_vec());
    /// assert_eq!(bytes.take_bytes(), b"ab".to_vec());
    /// ```
    pub(crate) fn take_bytes(self) -> Vec<u8> {
        // The view moves out only when this binary owns the descriptor, and
        // the array of the view moves out only when the view owns it. A
        // shared descriptor copies the bytes of the view, because another
        // holder keeps the descriptor and its array.
        match Arc::try_unwrap(self.0) {
            Ok(view) => view.take_bytes(),
            Err(view) => view.bytes_of_view().to_vec(),
        }
    }

    /// Return the bytes of this binary as an owned vector. A size that is
    /// not a whole number of bytes gives the extra bits in the high bits of
    /// the last byte, as [`MatchState::read_bits`] packs them. The
    /// function copies the bytes, and leaves the binary and its array
    /// unchanged.
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// let bytes = Binary::from(b"ab".to_vec());
    /// assert_eq!(bytes.to_bytes(), b"ab".to_vec());
    /// assert_eq!(bytes.bit_size(), 16);
    /// ```
    pub(crate) fn to_bytes(&self) -> Vec<u8> {
        // The bytes are copied, because a term cannot give up the array of a
        // binary: `Term` has a `Drop`, and the binary lives until the term
        // drops. `take_bytes` moves the array out when the caller owns the
        // binary.
        self.bit_bytes().to_vec()
    }

    /// Build the binary of `prefix` followed by `suffix`, the result of an
    /// `append` segment that adds bits. The result shares each array of
    /// `prefix` and copies no byte of the old payload, as an ERTS append to
    /// a binary that is not writable shares the old binary
    /// (`erts_bs_append_checked`, `erl_bits.c:1851-1921`). A prefix view
    /// that is not the whole of its array, or that keeps packed bits, is
    /// materialized in one array at this call, because its bytes are not one
    /// array.
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// let prefix = Binary::from(b"ab".to_vec());
    /// assert_eq!(Binary::concat(&prefix, b"cd"), Binary::from(b"abcd".to_vec()));
    /// ```
    pub(crate) fn concat(prefix: &Binary, suffix: &[u8]) -> Binary {
        if suffix.is_empty() {
            return prefix.clone();
        }
        Binary::concat_arrays(prefix, vec![Arc::new(suffix.to_vec())])
    }

    /// Build the binary of `prefix` followed by the arrays of `suffix`, in
    /// order, the result of an `append` segment whose later segments are
    /// whole byte ranges. The result shares each array of `prefix` and of
    /// `suffix`, so the append copies no byte of the old payload.
    ///
    /// When the arrays of `suffix` fit in one leaf with the last array of
    /// `prefix`, the result merges them in one leaf of at most
    /// [`APPEND_LEAF`] bytes. A repeated small append then fills a leaf
    /// instead of adding one small array for each step, and the merge copies
    /// at most that one leaf.
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// let prefix = Binary::from(b"ab".to_vec());
    /// let suffix = vec![Arc::new(b"cd".to_vec())];
    /// assert_eq!(Binary::concat_arrays(&prefix, suffix), Binary::from(b"abcd".to_vec()));
    /// ```
    #[expect(
        clippy::rc_buffer,
        reason = "`Binary` shares byte arrays as `Arc<Vec<u8>>` because `append_in_place` grows the vector through `Arc::get_mut`; an `Arc<[u8]>` cannot grow"
    )]
    pub(crate) fn concat_arrays(prefix: &Binary, suffix: Vec<Arc<Vec<u8>>>) -> Binary {
        let mut chunks = prefix.chunks();
        let appended: usize = suffix.iter().map(|chunk| chunk.len()).sum();
        let merge = chunks
            .last()
            .is_some_and(|chunk| chunk.len() + appended <= APPEND_LEAF);
        if merge {
            let mut leaf = chunks
                .pop()
                .expect("the merge keeps the last array of the prefix")
                .to_vec();
            for chunk in &suffix {
                leaf.extend_from_slice(chunk);
            }
            if !leaf.is_empty() {
                chunks.push(Arc::new(leaf));
            }
        } else {
            chunks.retain(|chunk| !chunk.is_empty());
            chunks.extend(suffix.into_iter().filter(|chunk| !chunk.is_empty()));
        }
        Binary::from_chunks(chunks)
    }

    /// Append `suffix` to this binary in place and return true, when this
    /// handle is the only owner of its view and of its array, as an ERTS
    /// `private_append` to a writable binary writes in its buffer
    /// (`erts_bs_private_append_checked`, `erl_bits.c:1963-1981`). The
    /// array grows as a `Vec` grows, so a loop of appends copies each byte
    /// a constant number of times on average. A unique view that an append
    /// built joins its arrays in one array one time, and later appends then
    /// write in that array. Return false, and keep the bytes, when another
    /// handle shares the view or the array, or when the view is a part of
    /// its array. The caller then builds a new binary.
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// let mut bytes = Binary::from(b"ab".to_vec());
    /// assert!(bytes.append_in_place(b"c"));
    /// let copy = bytes.clone();
    /// assert!(!bytes.append_in_place(b"d"));
    /// assert_eq!(copy.bytes(), Some(&b"abc"[..]));
    /// ```
    pub(crate) fn append_in_place(&mut self, suffix: &[u8]) -> bool {
        let Some(view) = Arc::get_mut(&mut self.0) else {
            return false;
        };
        if view.offset != 0 || view.packed.is_some() {
            return false;
        }
        if view.chunks.is_some() {
            let joined = Arc::clone(view.flat_backing());
            view.chunks = None;
            view.flat = OnceLock::new();
            view.bytes = joined;
        }
        if view.size != view.bytes.len() * BITS_PER_BYTE {
            return false;
        }
        let Some(bytes) = Arc::get_mut(&mut view.bytes) else {
            return false;
        };
        bytes.extend_from_slice(suffix);
        view.size = bytes.len() * BITS_PER_BYTE;
        true
    }

    /// Return the arrays that hold the bytes of this binary, in order. The
    /// function shares the arrays of a view that an append built, and the
    /// whole array of a view that is all of it. It copies the bytes of a
    /// sub-binary or of a packed view into one array, because those bytes
    /// are not one whole array.
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// let binary = Binary::from(b"ab".to_vec());
    /// assert_eq!(binary.chunks().len(), 1);
    /// ```
    #[expect(
        clippy::rc_buffer,
        reason = "`Binary` shares byte arrays as `Arc<Vec<u8>>` because `append_in_place` grows the vector through `Arc::get_mut`; an `Arc<[u8]>` cannot grow"
    )]
    pub(crate) fn chunks(&self) -> Vec<Arc<Vec<u8>>> {
        let view = &self.0;
        match &view.chunks {
            Some(chunks) => chunks.iter().cloned().collect(),
            None if view.offset == 0
                && view.packed.is_none()
                && view.size == view.bytes.len() * BITS_PER_BYTE =>
            {
                vec![Arc::clone(&view.bytes)]
            }
            None => vec![Arc::new(view.bytes_of_view().to_vec())],
        }
    }

    /// Return the last byte of this binary, or `None` for a binary with no
    /// byte. The call reads the last chunk of an append-built binary and
    /// joins no chunk, and each empty chunk at the end costs one step.
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// assert_eq!(Binary::from(b"ab".to_vec()).last_byte(), Some(b'b'));
    /// assert_eq!(Binary::default().last_byte(), None);
    /// ```
    pub(crate) fn last_byte(&self) -> Option<u8> {
        match &self.0.chunks {
            Some(chunks) => chunks.iter().rev().find_map(|chunk| chunk.last().copied()),
            None => self.0.bytes_of_view().last().copied(),
        }
    }

    /// Return the runs of the bytes from byte `start` to byte `end` of this
    /// binary, in order, for a reader of one byte range. A view that an
    /// append built gives the part of each chunk in the range and does not
    /// join its chunks, as [`Binary::bit_bytes`] does at its first read. The call
    /// copies no byte. It finds the first and the last chunk of the range
    /// with binary searches of the chunk index (`Chunks::ends`),
    /// so it does O(log chunks) work to find the range. It then does O(1)
    /// work for each chunk in the range and one allocation of the list of
    /// runs, and touches only the chunks and the bytes of the range. The view
    /// builds the chunk index when it is made, so no read builds it. Each
    /// other view gives one run of its bytes.
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// let binary = Binary::from_chunks(vec![Arc::new(b"ab".to_vec()), Arc::new(b"cd".to_vec())]);
    /// assert_eq!(binary.byte_runs(1, 3), vec![&b"b"[..], &b"c"[..]]);
    /// ```
    pub(crate) fn byte_runs(&self, start: usize, end: usize) -> Vec<&[u8]> {
        let Some(chunks) = &self.0.chunks else {
            return vec![&self.bit_bytes()[start..end]];
        };
        let ends = &chunks.ends;
        chunk_span(ends, start, end)
            .map(|index| {
                let chunk = &chunks[index];
                let last = ends[index];
                let first = last - chunk.len();
                &chunk[start.max(first) - first..end.min(last) - first]
            })
            .collect()
    }

    /// Return whether a view that an append built joined its chunks in one
    /// array, for the tests of the readers that must not join them.
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// assert!(!Binary::from(b"ab".to_vec()).is_joined());
    /// ```
    #[cfg(test)]
    pub(crate) fn is_joined(&self) -> bool {
        self.0.flat.get().is_some()
    }

    /// Build the binary of `chunks` in order. The view keeps the arrays and
    /// copies no byte of them, so a later append can share each one. One
    /// chunk gives a view of that array with no chunk list.
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// let chunk = Arc::new(b"ab".to_vec());
    /// assert_eq!(Binary::from_chunks(vec![chunk]), Binary::from(b"ab".to_vec()));
    /// ```
    #[expect(
        clippy::rc_buffer,
        reason = "`Binary` shares byte arrays as `Arc<Vec<u8>>` because `append_in_place` grows the vector through `Arc::get_mut`; an `Arc<[u8]>` cannot grow"
    )]
    pub(crate) fn from_chunks(chunks: Vec<Arc<Vec<u8>>>) -> Binary {
        if let [chunk] = chunks.as_slice() {
            let size = chunk.len() * BITS_PER_BYTE;
            return Binary(Arc::new(BinaryView {
                bytes: Arc::clone(chunk),
                chunks: None,
                flat: OnceLock::new(),
                offset: 0,
                size,
                packed: None,
                off_heap: false,
            }));
        }
        // The index adds one word for each chunk to the walk that sums the
        // size of the chunks, so it adds no walk of the chunks.
        let mut ends = Vec::with_capacity(chunks.len());
        let mut end = 0;
        for chunk in &chunks {
            end += chunk.len();
            ends.push(end);
        }
        let size = ends.last().copied().unwrap_or(0) * BITS_PER_BYTE;
        Binary(Arc::new(BinaryView {
            bytes: Arc::new(Vec::new()),
            chunks: Some(Arc::new(Chunks {
                arrays: chunks,
                ends,
            })),
            flat: OnceLock::new(),
            offset: 0,
            size,
            packed: None,
            off_heap: false,
        }))
    }
}

impl BinaryView {
    /// Take the bytes of this view as an owned vector. The array moves out
    /// only when this view is all of it, and only when this view is its last
    /// holder. A view that an append built copies its bytes, because its
    /// bytes are in several arrays.
    fn take_bytes(mut self) -> Vec<u8> {
        if self.chunks.is_some() {
            return self.bytes_of_view().to_vec();
        }
        // A sub-binary gives only its own bytes, also when it owns the array
        // of its source.
        let whole = self.offset == 0 && self.size == self.bytes.len() * BITS_PER_BYTE;
        if self.packed.is_none()
            && whole
            && Arc::strong_count(&self.bytes) == 1
            && let Some(bytes) = Arc::into_inner(std::mem::take(&mut self.bytes))
        {
            return bytes;
        }
        self.bytes_of_view().to_vec()
    }

    /// Return the one array that holds the bytes of this view. A view that
    /// an append built joins its chunks in one array at the first call, so
    /// the reader of the contiguous bytes pays one copy of the chunks.
    #[expect(
        clippy::rc_buffer,
        reason = "`Binary` shares byte arrays as `Arc<Vec<u8>>` because `append_in_place` grows the vector through `Arc::get_mut`; an `Arc<[u8]>` cannot grow"
    )]
    fn flat_backing(&self) -> &Arc<Vec<u8>> {
        match &self.chunks {
            None => &self.bytes,
            Some(chunks) => self.flat.get_or_init(|| {
                let mut bytes = Vec::with_capacity(self.size.div_ceil(BITS_PER_BYTE));
                for chunk in chunks.iter() {
                    bytes.extend_from_slice(chunk);
                }
                Arc::new(bytes)
            }),
        }
    }

    /// Return the bytes of this view, as [`Binary::bit_bytes`] gives them.
    fn bytes_of_view(&self) -> &[u8] {
        match &self.packed {
            Some(packed) => packed,
            // The offset is on a byte boundary, so the view is the bytes from
            // the byte of the offset to the last byte of the size. A size of
            // 0 gives no byte, also at the end of the array.
            None if self.size == 0 => &[],
            None => {
                let flat = self.flat_backing();
                let first = byte(self.offset);
                let end = flat.len().min(byte(self.offset + self.size - 1) + 1);
                &flat[first..end]
            }
        }
    }
}

impl Default for Binary {
    /// Make a binary with no byte, for the trap state that takes its bytes
    /// out and leaves the state empty.
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// assert!(Binary::default().is_empty());
    /// ```
    fn default() -> Self {
        Binary::from(Vec::new())
    }
}

impl PartialEq for Binary {
    /// Compare the two binaries by their bits. Two binaries are equal when
    /// their sizes in bits are equal and their bits are equal, as ERTS `eq`
    /// compares two bitstrings (`utils.c:1209-1217`). Thus `<<1:3>>` is not
    /// equal to `<<32>>`. Two binaries that share one array, one offset, and
    /// one size have the same bits. The size of the array that holds the
    /// bytes does not change the result.
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// let source = Binary::from(b"abc".to_vec());
    /// assert_eq!(source.bits(0, 16), Binary::from(b"ab".to_vec()));
    /// assert_ne!(source.bits(0, 16), Binary::from(b"ac".to_vec()));
    /// assert_ne!(Binary::from(vec![32]).bits(0, 3), Binary::from(vec![32]));
    /// ```
    fn eq(&self, other: &Self) -> bool {
        // A view of one array with the same offset and size has the same
        // bits. Each other pair of one size compares the bits, so the size
        // of the array does not change the result.
        self.0.size == other.0.size
            && ((self.same_bytes(other) && self.0.offset == other.0.offset)
                || cmp_bits(self, other, self.0.size).is_eq())
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
/// as [`MatchState::read_bits`] gives them: the first bit is the high bit
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

impl<const N: usize> From<&[u8; N]> for Binary {
    /// Make a binary of a byte array. The array of the binary holds a copy
    /// of the bytes, and copies of the binary share it.
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// let binary = Binary::from(&[1, 2]);
    /// assert_eq!(binary.bytes(), Some(&[1, 2][..]));
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
    /// assert_eq!(binary.bytes(), Some(&[1, 2][..]));
    /// ```
    fn from(bytes: &[u8]) -> Self {
        Binary::from(bytes.to_vec())
    }
}

/// Return the indexes of the chunks that hold the bytes from byte `start` to
/// byte `end`, from the byte offset of the end of each chunk, in order. Two
/// binary searches find the first and the last chunk, so the call is
/// O(log chunks). An empty range gives no chunk.
fn chunk_span(ends: &[usize], start: usize, end: usize) -> std::ops::Range<usize> {
    if start >= end {
        return 0..0;
    }
    let first = ends.partition_point(|&last| last <= start);
    let last = ends.partition_point(|&last| last < end) + 1;
    first..last
}

impl From<Vec<u8>> for Binary {
    /// Make a binary of `bytes`. The array of the binary holds the bytes, and
    /// copies of the binary share it.
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// let binary = Binary::from(vec![1, 2]);
    /// assert_eq!(binary.bytes(), Some(&[1, 2][..]));
    /// ```
    fn from(bytes: Vec<u8>) -> Self {
        let size = bytes.len() * BITS_PER_BYTE;
        Binary(Arc::new(BinaryView {
            bytes: Arc::new(bytes),
            chunks: None,
            flat: OnceLock::new(),
            offset: 0,
            size,
            packed: None,
            off_heap: false,
        }))
    }
}

impl fmt::Debug for Binary {
    /// Write the bytes of the binary as `[1, 2]`, so the derived `Debug` of
    /// the `Term::Binary` variant writes `Binary([1, 2])`.
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// assert_eq!(format!("{:?}", Binary::from(vec![1])), "[1]");
    /// ```
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "{:?}", self.bit_bytes())
    }
}

impl PartialOrd for Binary {
    /// Compare the bits, then the sizes in bits, as `erts_cmp` compares two
    /// bitstrings (`utils.c:2325-2335`).
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
    /// Compare the bits that the two binaries both have, then the sizes in
    /// bits, as `erts_cmp` compares two bitstrings: a shorter binary is less
    /// when its bits are a prefix of the other (`utils.c:2325-2335`).
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// use std::cmp::Ordering;
    /// assert_eq!(Binary::from(vec![1]).cmp(&Binary::from(vec![1])), Ordering::Equal);
    /// assert_eq!(Binary::from(vec![1]).cmp(&Binary::from(vec![1, 2])), Ordering::Less);
    /// // <<1:3>> < <<32>>: the 3 bits are equal and the size is less.
    /// assert_eq!(Binary::from(vec![32]).bits(0, 3).cmp(&Binary::from(vec![32])), Ordering::Less);
    /// ```
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        cmp_bits(self, other, self.0.size.min(other.0.size)).then(self.0.size.cmp(&other.0.size))
    }
}

/// Compare the first `size` bits of the two binaries, as `erts_cmp_bits`
/// does (`erl_bits.h:559-582`): the whole bytes, then the high bits of the
/// next byte. A mask removes the bits after `size` in that byte, because a
/// view at a byte offset keeps those bits of its array. `size` is at most
/// the size of each binary.
///
/// # Examples
///
/// ```rust,ignore
/// let left = Binary::from(vec![0x3F]);
/// let right = Binary::from(vec![0x20]);
/// assert!(cmp_bits(&left, &right, 3).is_eq());
/// assert!(cmp_bits(&left, &right, 4).is_gt());
/// ```
fn cmp_bits(left: &Binary, right: &Binary, size: usize) -> std::cmp::Ordering {
    let whole = byte(size);
    let tail = size % BITS_PER_BYTE;
    let (left, right) = (left.bit_bytes(), right.bit_bytes());
    left[..whole].cmp(&right[..whole]).then_with(|| {
        if tail == 0 {
            return std::cmp::Ordering::Equal;
        }
        let mask = 0xFF_u8 << (BITS_PER_BYTE - tail);
        (left[whole] & mask).cmp(&(right[whole] & mask))
    })
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

    /// Return the bytes of a binary term whose size is a whole number of
    /// bytes, or `None` for a bitstring whose size is not and for each term
    /// that is not a binary. A BIF that takes a binary raises `badarg` for
    /// each `None`, as ERTS does for a term that
    /// `erts_get_aligned_binary_bytes_extra` gives no bytes for
    /// (`erl_bits.h:605-635`).
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// assert_eq!(Term::binary(vec![1]).binary_bytes(), Some(&[1][..]));
    /// assert_eq!(Term::Binary(Binary::from(vec![1]).bits(0, 3)).binary_bytes(), None);
    /// assert_eq!(Term::Integer(1).binary_bytes(), None);
    /// ```
    pub fn binary_bytes(&self) -> Option<&[u8]> {
        match self {
            Term::Binary(binary) => binary.bytes(),
            _ => None,
        }
    }
}

#[cfg(test)]
#[path = "../../tests/term/binary.rs"]
mod tests;

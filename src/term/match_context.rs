//! A binary match context, as in ERTS OTP 29.1.1: an `ErlSubBits` with the
//! flag `ERL_SUB_BITS_FLAGS_MATCH_CONTEXT` (`erts_bs_start_match_3`,
//! `erl_bits.c:81-105`).
//!
//! `bs_start_match3` makes a match context for a binary and writes it to a
//! register. The binary-match instructions read the bits after its position
//! and move the position. The BEAM validator lets no instruction read a
//! match context as a term (`get_term_type`, beam_validator.erl:2903-2908).
//! It lets `move` copy a match context, and `byte_size/1` read it
//! (beam_validator.erl:403-404, 1649-1654). A register holds a reference to
//! the match context, as an ERTS register holds a boxed pointer to the
//! `ErlSubBits`, so each copy shares the position. PBeam lets a match
//! context reach only the binary-match instructions, `move`, `byte_size/1`,
//! and the argument registers of a local call, and gives a runtime
//! invariant error at each other instruction.

use super::Binary;

/// The panic message of a term operation, such as printing or hashing,
/// that gets a match context. The interpreter gives a runtime error before
/// a match context can reach such an operation.
pub(crate) const MATCH_CONTEXT_ESCAPE: &str =
    "a match context reached a term operation: only binary-match instructions read it";

/// The state of a binary match: the binary and the bit position of the
/// next segment (`sb->start`). The match ends at the end of the binary
/// (`sb->end`). ERTS makes a match context of the binary itself, with no
/// copy (`erts_bs_start_match_3`, `erl_bits.c:81-105`), so a PBeam match
/// context shares the bytes of the binary.
///
/// # Examples
///
/// ```rust,ignore
/// let mut context = MatchContext::new(Binary::from(&[b'A', 195, 169][..]));
/// assert_eq!(context.get_utf8(), Some(65));
/// assert_eq!(context.position(), 8);
/// assert_eq!(context.remaining_bits(), 16);
/// ```
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MatchContext {
    /// The bytes of the binary. A clone of the context shares them, as an
    /// ERTS sub-bitstring refers to its binary.
    bytes: Binary,
    /// The bit offset of the next segment from the start of the binary.
    /// The `integer`, `binary`, `skip`, and `=:=` commands of `bs_match`
    /// can move it by any number of bits.
    position: usize,
    /// `false` after a failed `=:=` command of `bs_match` and until
    /// `bs_set_position`. ERTS then has the position at the end of the
    /// read that holds the command, which PBeam does not model (row
    /// "Binary matching" of `docs/erts-differences.md`).
    position_known: bool,
}

impl MatchContext {
    /// Make a match context at the start of a binary, as
    /// `erts_bs_start_match_3` does (`erl_bits.c:81-105`).
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// let context = MatchContext::new(Binary::from(&[1, 2][..]));
    /// assert_eq!((context.position(), context.remaining_bits()), (0, 16));
    /// ```
    pub(crate) fn new(bytes: Binary) -> Self {
        Self {
            bytes,
            position: 0,
            position_known: true,
        }
    }

    /// Return the bytes of the binary of this match context, as the match
    /// context of ERTS refers to its binary. Two match contexts that return
    /// one array of bytes share it.
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// let context = MatchContext::new(Binary::from(&[1, 2][..]));
    /// assert_eq!(context.bytes().len(), 2);
    /// ```
    #[cfg(test)]
    pub(crate) fn bytes(&self) -> &Binary {
        &self.bytes
    }

    /// Return the bit position of the next segment, as `bs_get_position`
    /// reads `sb->start` (emu `bs_instrs.tab:1125-1129`).
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// assert_eq!(MatchContext::new(Binary::from(&[1][..])).position(), 0);
    /// ```
    pub(crate) fn position(&self) -> usize {
        self.position
    }

    /// Set the bit position of the next segment, as `bs_set_position`
    /// writes `sb->start` (emu `bs_instrs.tab:1120-1123`). The position is
    /// a value that `position` gave for this context.
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// let mut context = MatchContext::new(Binary::from(&[1][..]));
    /// context.get_utf8();
    /// context.set_position(0);
    /// assert_eq!(context.remaining_bits(), 8);
    /// ```
    pub(crate) fn set_position(&mut self, position: usize) {
        self.position = position;
        self.position_known = true;
    }

    /// Mark the position as unknown after a failed `=:=` command of
    /// `bs_match`. The ERTS JIT reads up to 64 bits of segments at a time
    /// and writes the position after the read before it compares a value
    /// (`beam_jit_opt_bsm_segments`, jit/beam_jit_bs.cpp:366-521; `READ`
    /// and `EQ` in `emit_i_bs_match_test_heap`, jit/arm/instr_bs.cpp:
    /// 2995-3045). Thus after a failed `=:=` the position is at the end of
    /// that read. The compiler restores the position before it reads the
    /// context again (`bs_restores`, beam_ssa_pre_codegen.erl:308-437).
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// let mut context = MatchContext::new(Binary::from(&[1][..]));
    /// context.forget_position();
    /// assert!(!context.position_known());
    /// context.set_position(0);
    /// assert!(context.position_known());
    /// ```
    pub(crate) fn forget_position(&mut self) {
        self.position_known = false;
    }

    /// Return `false` after a failed `=:=` command and until
    /// `set_position`.
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// assert!(MatchContext::new(Binary::from(&[1][..])).position_known());
    /// ```
    pub(crate) fn position_known(&self) -> bool {
        self.position_known
    }

    /// Move the position by `size` bits, as the `skip` command of
    /// `bs_match` does (`SKIP` in `emit_i_bs_match_test_heap`,
    /// jit/arm/instr_bs.cpp:3187-3197). Return `None` and keep the
    /// position when fewer than `size` bits remain.
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// let mut context = MatchContext::new(Binary::from(&[1, 2][..]));
    /// assert_eq!(context.skip(12), Some(()));
    /// assert_eq!(context.remaining_bits(), 4);
    /// assert_eq!(context.skip(5), None);
    /// ```
    pub(crate) fn skip(&mut self, size: u64) -> Option<()> {
        let size = usize::try_from(size)
            .ok()
            .filter(|&size| size <= self.remaining_bits())?;
        self.position += size;
        Some(())
    }

    /// Read `size` bits after the position and move the position past
    /// them, as the `integer`, `binary`, and `=:=` commands of `bs_match`
    /// do (`emit_read_bits` and `emit_i_bs_match_test_heap`,
    /// jit/arm/instr_bs.cpp:2496-2748, 3024-3176). Return the bits first
    /// bit first, packed in bytes: the last byte holds the last `size % 8`
    /// bits in its high bits, and its other bits are 0. Return `None` and
    /// keep the position when fewer than `size` bits remain.
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// let mut context = MatchContext::new(Binary::from(&[0xAB, 0xCD][..]));
    /// assert_eq!(context.read_bits(4), Some(vec![0xA0]));
    /// assert_eq!(context.read_bits(8), Some(vec![0xBC]));
    /// assert_eq!(context.read_bits(8), None);
    /// ```
    pub(crate) fn read_bits(&mut self, size: u64) -> Option<Vec<u8>> {
        let start = self.position;
        self.skip(size)?;
        Some(segment_bits(&self.bytes, start, self.position - start))
    }

    /// Read `size` bits after the position and move the position past them,
    /// as [`MatchContext::read_bits`] does, and return the bits as a view of
    /// the bytes of the binary. The view shares the bytes of the binary, as
    /// an ERTS `bs_get_binary2` or `bs_match` binary command writes a
    /// sub-bitstring that refers to the original binary
    /// (`erts_bs_get_binary_2`, `erl_bits.c:400-424`). Return `None` and
    /// keep the position when fewer than `size` bits remain. A size that is
    /// not a whole number of bytes is a bitstring, which PBeam does not
    /// have: each caller checks the size before it uses the view.
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// let mut context = MatchContext::new(Binary::from(&[1, 2, 3][..]));
    /// assert_eq!(context.read_bytes(16), Some(Binary::from(&[1, 2][..])));
    /// let bits = context.read_bytes(4).expect("4 bits remain");
    /// assert_eq!(&bits[..], &[0b0000_0000]);
    /// ```
    pub(crate) fn read_bytes(&mut self, size: u64) -> Option<Binary> {
        let start = self.position;
        self.skip(size)?;
        Some(self.bytes.bits(start, self.position))
    }

    /// Return the bits between the bit positions `start` and `end` as a view
    /// of the bytes of the binary, as an ERTS sub-bitstring refers to its
    /// binary. Return `None` when the size is not a whole number of bytes,
    /// because PBeam has no bitstring term. The `start` position stays with
    /// the size, so the view keeps the bit offset.
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// let context = MatchContext::new(Binary::from(&[1, 2, 3][..]));
    /// assert_eq!(context.bits_at(0, 16), Some(Binary::from(&[1, 2][..])));
    /// assert_eq!(context.bits_at(0, 4), None);
    /// ```
    pub(crate) fn bits_at(&self, start: usize, end: usize) -> Option<Binary> {
        ((end - start).is_multiple_of(8)).then(|| self.bytes.bits(start, end))
    }

    /// Return the bits after the position as a view of the bytes of the
    /// binary, and keep the position, as the `get_tail` command of
    /// `bs_match` does (`emit_bs_get_tail_shared`, jit/arm/instr_bs.cpp:
    /// 407-439). The view shares the bytes of the binary, as an ERTS
    /// sub-bitstring refers to its binary. Return `None` when the position
    /// is not on a byte boundary, because PBeam has no bitstring term (row
    /// "Bitstring terms" of `docs/erts-differences.md`).
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// let mut context = MatchContext::new(Binary::from(&[1, 2, 3][..]));
    /// context.skip(8);
    /// assert_eq!(context.tail(), Some(Binary::from(&[2, 3][..])));
    /// assert_eq!(context.position(), 8);
    /// ```
    pub(crate) fn tail(&self) -> Option<Binary> {
        let size = self.bytes.len() * 8;
        (self.position.is_multiple_of(8)).then(|| self.bytes.bits(self.position, size))
    }

    /// Return the number of bits after the position (`sb->end -
    /// sb->start`).
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// assert_eq!(MatchContext::new(Binary::from(&[1, 2][..])).remaining_bits(), 16);
    /// assert_eq!(MatchContext::new(Binary::from(&[][..])).remaining_bits(), 0);
    /// ```
    pub(crate) fn remaining_bits(&self) -> usize {
        self.bytes.len() * 8 - self.position
    }

    /// Read one UTF-8 character at the position and move the position
    /// past it, as `erts_bs_get_utf8` does (`erl_bits.c:2128-2221`). The
    /// position can be at any bit. Return `None` and keep the position for
    /// short input, a first byte that does not start a character
    /// (`0x80..=0xC1` and `0xF8..=0xFF`), a bad continuation byte, an
    /// overlong form of 3 or 4 bytes, a surrogate, or a value above
    /// 0x10FFFF.
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// assert_eq!(MatchContext::new(Binary::from(&[195, 169][..])).get_utf8(), Some(233));
    /// assert_eq!(MatchContext::new(Binary::from(&[195][..])).get_utf8(), None);
    /// assert_eq!(MatchContext::new(Binary::from(&[192, 128][..])).get_utf8(), None);
    /// assert_eq!(MatchContext::new(Binary::from(&[237, 160, 128][..])).get_utf8(), None);
    /// assert_eq!(MatchContext::new(Binary::from(&[244, 144, 128, 128][..])).get_utf8(), None);
    /// let mut context = MatchContext::new(Binary::from(&[0x04, 0x10][..]));
    /// context.skip(4);
    /// assert_eq!(context.get_utf8(), Some(65));
    /// ```
    pub(crate) fn get_utf8(&mut self) -> Option<u32> {
        // The whole bytes after the position, up to 4, aligned as
        // `erts_align_utf8_bytes` copies them (`erl_bits.c:2108-2126`).
        // A missing byte is short input, as the `remaining_bits` checks of
        // `erts_bs_get_utf8` give.
        let bits = (self.remaining_bits() / 8).min(4) * 8;
        let rest = segment_bits(&self.bytes, self.position, bits);
        let first = u32::from(*rest.first()?);
        // A byte of the character that is a continuation byte `10xxxxxx`.
        let continuation = |index: usize| {
            rest.get(index)
                .map(|&byte| u32::from(byte))
                .filter(|byte| byte & 0xC0 == 0x80)
        };
        // The first byte gives the number of trailing bytes
        // (`erts_trailing_bytes_for_utf8`). The constants remove the tag
        // bits of each byte.
        let (code_point, size) = match first {
            0x00..=0x7F => (first, 1),
            0xC2..=0xDF => ((first << 6) + continuation(1)? - 0x3080, 2),
            0xE0..=0xEF => {
                let (a, b) = (continuation(1)?, continuation(2)?);
                if first == 0xE0 && a < 0xA0 {
                    return None;
                }
                let code_point = (((first << 6) + a) << 6) + b - 0x000E_2080;
                if (0xD800..=0xDFFF).contains(&code_point) {
                    return None;
                }
                (code_point, 3)
            }
            0xF0..=0xF7 => {
                let (a, b, c) = (continuation(1)?, continuation(2)?, continuation(3)?);
                if first == 0xF0 && a < 0x90 {
                    return None;
                }
                let code_point = (((((first << 6) + a) << 6) + b) << 6) + c - 0x03C8_2080;
                if code_point > 0x0010_FFFF {
                    return None;
                }
                (code_point, 4)
            }
            _ => return None,
        };
        self.position += size * 8;
        Some(code_point)
    }
}

/// Return `size` bits of `bytes` from the bit offset `start`, packed in
/// bytes first bit first. The last byte holds the last `size % 8` bits in
/// its high bits, and its other bits are 0. The bits are in `bytes`.
///
/// # Examples
///
/// ```rust,ignore
/// assert_eq!(segment_bits(&[0xAB, 0xCD], 4, 8), vec![0xBC]);
/// assert_eq!(segment_bits(&[0xAB, 0xCD], 0, 12), vec![0xAB, 0xC0]);
/// assert_eq!(segment_bits(&[0xAB], 3, 0), Vec::<u8>::new());
/// ```
fn segment_bits(bytes: &[u8], start: usize, size: usize) -> Vec<u8> {
    let first = start / 8;
    let shift = start % 8;
    let mut bits: Vec<u8> = (first..first + size.div_ceil(8))
        .map(|index| {
            let next = bytes.get(index + 1).copied().unwrap_or(0);
            // A shift of 0 takes no bit of the next byte.
            (bytes[index] << shift) | (u16::from(next) >> (8 - shift)) as u8
        })
        .collect();
    if let Some(last) = bits.last_mut() {
        *last &= 0xFF_u8 << ((8 - size % 8) % 8);
    }
    bits
}

/// Return the value of an integer segment of `size` bits that `read_bits`
/// gave, as the ERTS JIT extracts it (`emit_extract_integer`,
/// jit/arm/instr_bs.cpp:2750-2879) and `erts_bs_get_integer_2` reads a
/// segment above 64 bits (`erl_bits.c:170-398`). A big-endian segment has its
/// bytes in order and then its last `size % 8` bits. A little-endian
/// segment has its last `size % 8` bits as the most significant part, and
/// then its bytes from the last byte to the first. A signed segment uses
/// two's complement. Return `None` for a value outside the 64-bit signed
/// range, which ERTS gives as a bignum.
///
/// # Examples
///
/// ```rust,ignore
/// assert_eq!(segment_integer(&[0x01, 0x20], 12, false, false), Some(0x012));
/// assert_eq!(segment_integer(&[0x23, 0x10], 12, true, false), Some(0x123));
/// assert_eq!(segment_integer(&[0xFF], 8, false, true), Some(-1));
/// assert_eq!(segment_integer(&[0x80, 0, 0, 0, 0, 0, 0, 0], 64, false, false), None);
/// assert_eq!(segment_integer(&[], 0, false, true), Some(0));
/// ```
pub(crate) fn segment_integer(bits: &[u8], size: usize, little: bool, signed: bool) -> Option<i64> {
    let (full, partial) = (size / 8, size % 8);
    // The parts of the segment, each a value and its width in bits, from
    // the most significant part.
    let last = (partial > 0).then(|| (bits[full] >> (8 - partial), partial));
    let bytes = bits[..full].iter().map(|&byte| (byte, 8));
    let parts: Vec<(u8, usize)> = if little {
        last.into_iter().chain(bytes.rev()).collect()
    } else {
        bytes.chain(last).collect()
    };
    let Some((&(first, width), rest)) = parts.split_first() else {
        return Some(0);
    };
    let negative = signed && first >> (width - 1) == 1;
    let mut value = i64::from(first) - if negative { 1 << width } else { 0 };
    // Each step moves the value away from 0 or keeps it, so a value that
    // leaves the 64-bit range does not come back.
    for &(part, width) in rest {
        value = value
            .checked_mul(1 << width)?
            .checked_add(i64::from(part))?;
    }
    Some(value)
}

/// Return `true` when the `size` bits that `read_bits` gave are the
/// unsigned big-endian integer `value`, as the `=:=` command of
/// `bs_match` compares them (`EQ` in `emit_i_bs_match_test_heap`,
/// jit/arm/instr_bs.cpp:2995-3016). A `value` of `size` or more bits does
/// not match.
///
/// # Examples
///
/// ```rust,ignore
/// assert!(segment_equals(&[0x61, 0x60], 12, 0x616));
/// assert!(!segment_equals(&[0x61], 8, 0x161));
/// assert!(segment_equals(&[0, 0, 0, 0, 0, 0, 0, 0, 0x80], 72, 0x80));
/// ```
pub(crate) fn segment_equals(bits: &[u8], size: usize, value: u64) -> bool {
    (size >= 64 || value >> size == 0)
        && (0..size).all(|index| {
            let bit = u64::from(bits[index / 8] >> (7 - index % 8) & 1);
            let shift = size - 1 - index;
            bit == if shift < 64 { value >> shift & 1 } else { 0 }
        })
}

#[cfg(test)]
#[path = "../../tests/term/match_context.rs"]
mod tests;

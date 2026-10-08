//! A binary match context, as in ERTS OTP 29.1.1: an `ErlSubBits` with the
//! flag `ERL_SUB_BITS_FLAGS_MATCH_CONTEXT` (`erts_bs_start_match_3`,
//! `erl_bits.c:81-105`).
//!
//! `bs_start_match3` makes a match context for a binary and writes it to a
//! register. The binary-match instructions read the bits after its position
//! and move the position. The BEAM validator lets no instruction read a
//! match context as a term (`get_term_type`, beam_validator.erl:2903-2908).
//! PBeam lets a match context reach only the binary-match instructions and
//! the argument registers of a local call, and gives a runtime invariant
//! error at each other instruction.

use std::sync::Arc;

/// The panic message of a term operation, such as printing or hashing,
/// that gets a match context. The interpreter gives a runtime error before
/// a match context can reach such an operation.
pub(crate) const MATCH_CONTEXT_ESCAPE: &str =
    "a match context reached a term operation: only binary-match instructions read it";

/// The state of a binary match: the binary and the bit position of the
/// next segment (`sb->start`). The match ends at the end of the binary
/// (`sb->end`).
///
/// # Examples
///
/// ```rust,ignore
/// let mut context = MatchContext::new(&[b'A', 195, 169]);
/// assert_eq!(context.get_utf8(), Some(65));
/// assert_eq!(context.position(), 8);
/// assert_eq!(context.remaining_bits(), 16);
/// ```
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MatchContext {
    /// The bytes of the binary. A copy of the context shares them, as an
    /// ERTS sub-bitstring refers to its binary.
    bytes: Arc<[u8]>,
    /// The bit offset of the next segment from the start of the binary.
    /// PBeam has only binaries, and each match instruction moves the
    /// position by whole bytes, so the offset is a multiple of 8.
    position: usize,
}

impl MatchContext {
    /// Make a match context at the start of a binary, as
    /// `erts_bs_start_match_3` does (`erl_bits.c:81-105`).
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// let context = MatchContext::new(&[1, 2]);
    /// assert_eq!((context.position(), context.remaining_bits()), (0, 16));
    /// ```
    pub(crate) fn new(bytes: &[u8]) -> Self {
        Self {
            bytes: bytes.into(),
            position: 0,
        }
    }

    /// Return the bit position of the next segment, as `bs_get_position`
    /// reads `sb->start` (emu `bs_instrs.tab:1125-1129`).
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// assert_eq!(MatchContext::new(&[1]).position(), 0);
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
    /// let mut context = MatchContext::new(&[1]);
    /// context.get_utf8();
    /// context.set_position(0);
    /// assert_eq!(context.remaining_bits(), 8);
    /// ```
    pub(crate) fn set_position(&mut self, position: usize) {
        self.position = position;
    }

    /// Return the number of bits after the position (`sb->end -
    /// sb->start`).
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// assert_eq!(MatchContext::new(&[1, 2]).remaining_bits(), 16);
    /// assert_eq!(MatchContext::new(&[]).remaining_bits(), 0);
    /// ```
    pub(crate) fn remaining_bits(&self) -> usize {
        self.bytes.len() * 8 - self.position
    }

    /// Read one UTF-8 character at the position and move the position
    /// past it, as `erts_bs_get_utf8` does (`erl_bits.c:2128-2221`).
    /// Return `None` and keep the position for short input, a first byte
    /// that does not start a character (`0x80..=0xC1` and `0xF8..=0xFF`),
    /// a bad continuation byte, an overlong form of 3 or 4 bytes, a
    /// surrogate, or a value above 0x10FFFF.
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// assert_eq!(MatchContext::new(&[195, 169]).get_utf8(), Some(233));
    /// assert_eq!(MatchContext::new(&[195]).get_utf8(), None);
    /// assert_eq!(MatchContext::new(&[192, 128]).get_utf8(), None);
    /// assert_eq!(MatchContext::new(&[237, 160, 128]).get_utf8(), None);
    /// assert_eq!(MatchContext::new(&[244, 144, 128, 128]).get_utf8(), None);
    /// ```
    pub(crate) fn get_utf8(&mut self) -> Option<u32> {
        let rest = &self.bytes[self.position / 8..];
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

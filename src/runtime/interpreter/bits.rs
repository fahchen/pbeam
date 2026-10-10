//! The bit buffer that `bs_create_bin` builds a binary in.
//!
//! ERTS writes each segment at the bit offset of the binary under
//! construction (`erts_bin_offset` of `ErlBitsState`, erl_bits.h:84-96). An
//! `integer` segment can end inside a byte, so the next segment starts at a
//! bit offset that is not a multiple of 8. `BitWriter` keeps the bytes and
//! the number of bits written, and fills the bits of each byte from the
//! most significant bit, as ERTS does.

/// A byte buffer that grows by any number of bits.
///
/// `bytes` holds the written bits from the most significant bit of the
/// first byte. The unused low bits of the last byte are 0. `bits` is the
/// number of written bits, so `bytes.len()` is `bits / 8` rounded up.
#[derive(Debug, Default)]
pub(super) struct BitWriter {
    bytes: Vec<u8>,
    bits: u64,
}

impl BitWriter {
    /// Write the low `count` bits of `chunk`, from the most significant of
    /// them. `count` is 1 to 8.
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// let mut writer = BitWriter::default();
    /// writer.push_bits(0xA, 4);
    /// writer.push_bits(0xBC, 8);
    /// writer.push_bits(0xD, 4);
    /// assert_eq!(writer.into_bytes(), Ok(vec![0xAB, 0xCD]));
    /// ```
    pub(super) fn push_bits(&mut self, chunk: u8, count: u32) {
        let chunk = chunk & (u8::MAX >> (8 - count));
        let free = (8 - self.bits % 8) as u32 % 8;
        match self.bytes.last_mut() {
            Some(last) if free >= count => *last |= chunk << (free - count),
            Some(last) if free > 0 => {
                *last |= chunk >> (count - free);
                self.bytes.push(chunk << (8 - (count - free)));
            }
            _ => self.bytes.push(chunk << (8 - count)),
        }
        self.bits += u64::from(count);
    }

    /// Write `bytes` at the current bit offset.
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// let mut writer = BitWriter::default();
    /// writer.push_bits(0x1, 4);
    /// writer.push_bytes(&[0x23]);
    /// writer.push_bits(0x4, 4);
    /// assert_eq!(writer.into_bytes(), Ok(vec![0x12, 0x34]));
    /// ```
    pub(super) fn push_bytes(&mut self, bytes: &[u8]) {
        if self.bits.is_multiple_of(8) {
            self.bytes.extend_from_slice(bytes);
            self.bits += bytes.len() as u64 * 8;
        } else {
            for &byte in bytes {
                self.push_bits(byte, 8);
            }
        }
    }

    /// Write the integer `value` in `bits` bits, as `erts_bs_put_integer_be`
    /// and `erts_bs_put_integer_le` do (erl_bits.c:926-1167). The result is
    /// `value` modulo 2^`bits` in two's complement, so a size above 64
    /// repeats the sign bit. Big-endian order writes the most significant
    /// bits first. Little-endian order writes the bytes from the least
    /// significant byte, and then the `bits % 8` most significant bits.
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// let mut writer = BitWriter::default();
    /// writer.push_integer(0xABC, 12, true); // <<16#ABC:12/little>>
    /// writer.push_integer(0xD, 4, false);
    /// assert_eq!(writer.into_bytes(), Ok(vec![0xBC, 0xAD]));
    /// ```
    pub(super) fn push_integer(&mut self, value: i64, bits: u64, little: bool) {
        let byte_at = |shift: u64| (value >> shift.min(63)) as u8;
        let (whole, rest) = (bits / 8, (bits % 8) as u32);
        if little {
            for index in 0..whole {
                self.push_bits(byte_at(index * 8), 8);
            }
        }
        if rest > 0 {
            self.push_bits(byte_at(whole * 8), rest);
        }
        if !little {
            for index in (0..whole).rev() {
                self.push_bits(byte_at(index * 8), 8);
            }
        }
    }

    /// Return the bytes when the number of written bits is a multiple of 8.
    /// Else return the number of bits: the result is a bitstring that is
    /// not a binary, which PBeam has no term for.
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// let mut writer = BitWriter::default();
    /// writer.push_bits(1, 4);
    /// assert_eq!(writer.into_bytes(), Err(4));
    /// ```
    pub(super) fn into_bytes(self) -> Result<Vec<u8>, u64> {
        if self.bits.is_multiple_of(8) {
            Ok(self.bytes)
        } else {
            Err(self.bits)
        }
    }
}

#[cfg(test)]
#[path = "../../../tests/runtime/interpreter/bits.rs"]
mod tests;

//! The `asn1rt_nif` NIF library (`lib/asn1/c_src/asn1_erl_nif.c`), which
//! `asn1rt_nif:load_nif/0` loads with the load info 1
//! (`asn1rt_nif.erl:37-41`, `67`).
//!
//! The bindings port the C functions of the library. The C code reads and
//! writes with pointers and `int` values, so the port keeps its counters,
//! its `int` shifts, and its order of steps. The C code can read the
//! length bytes of an opcode after the end of its input; the check of
//! `counter` that follows then gives `ASN1_ERROR` for each value, so PBeam
//! reads the byte 0 there. The opcode 10 has no such check, and PBeam
//! stops the run for it (see "NIF stubs" in `docs/erts-differences.md`).

use super::bif::iodata_bytes;
use super::*;
use std::cmp::Ordering;

/// The functions of the `asn1rt_nif` NIF library, as `(name, arity)` in
/// the order of `nif_funcs` (`asn1_erl_nif.c:1343-1347`).
/// `erlang:load_nif/2` checks each of them in this order. The entries have
/// no flags field, so their flags are 0.
///
/// # Examples
///
/// ```rust,ignore
/// assert_eq!(FUNCTIONS[1], ("decode_ber_tlv_raw", 1));
/// ```
pub(super) const FUNCTIONS: [(&str, u32); 3] = [
    ("encode_per_complete", 1),
    ("decode_ber_tlv_raw", 1),
    ("encode_ber_tlv", 1),
];

/// The registered bindings of `asn1rt_nif`, sorted by name, then by arity.
///
/// # Examples
///
/// ```rust,ignore
/// assert_eq!(BINDINGS[0].name, "decode_ber_tlv_raw");
/// ```
pub(super) const BINDINGS: &[Binding] = &[
    Binding {
        name: "decode_ber_tlv_raw",
        arity: 1,
        job: Job::Normal,
        run: |scheduler, arguments| decode_ber_tlv_raw(scheduler, &arguments[0]),
    },
    Binding {
        name: "encode_ber_tlv",
        arity: 1,
        job: Job::Normal,
        run: |scheduler, arguments| encode_ber_tlv(scheduler, &arguments[0]),
    },
    Binding {
        name: "encode_per_complete",
        arity: 1,
        job: Job::Normal,
        run: |scheduler, arguments| encode_per_complete(scheduler, &arguments[0]),
    },
];

/// Run the load callback of the `asn1rt_nif` library with `load_info`, as
/// the ERTS `load` does, and return its veto: 0 for the integer 1, and -1
/// for each other term (`asn1_erl_nif.c:1320-1329`).
///
/// # Examples
///
/// ```rust,ignore
/// assert_eq!(load(&Term::Integer(1), &atoms), 0);
/// assert_eq!(load(&Term::Integer(2), &atoms), -1);
/// ```
pub(super) fn load(load_info: &Term, _scheduler: &Scheduler) -> i32 {
    if matches!(load_info, Term::Integer(1)) {
        0
    } else {
        -1
    }
}

/// Return `{error, Code}` with the atom `error`.
fn error_tuple(scheduler: &mut Scheduler, code: Term) -> Result<BifResult, BeamError> {
    let error = Term::Atom(scheduler.atoms.put("error")?);
    Ok(BifResult::Value(Term::tuple(vec![error, code])))
}

/// Run `asn1rt_nif:encode_per_complete(Data)` as the ERTS NIF
/// (`asn1_erl_nif.c:1202-1231`): iodata that is not iodata raises
/// `badarg`, empty iodata gives `<<>>`, an error of `per_complete` gives
/// `{error, $1}`, and else the completed PER bytes.
///
/// # Examples
///
/// ```rust,ignore
/// let data = Term::list_from([1, 0, 1].map(Term::Integer));
/// assert!(matches!(encode_per_complete(&mut scheduler, &data)?, BifResult::Value(_)));
/// ```
fn encode_per_complete(scheduler: &mut Scheduler, data: &Term) -> Result<BifResult, BeamError> {
    let Some(input) = iodata_bytes(data) else {
        return Ok(BifResult::badarg());
    };
    if input.is_empty() {
        return Ok(BifResult::Value(Term::binary(Vec::new())));
    }
    let mut truncated = false;
    match per_complete(&input, &mut truncated) {
        Some(bytes) => Ok(BifResult::Value(Term::binary(bytes))),
        None if truncated => Err(error(
            "asn1rt_nif:encode_per_complete/1 reads after the end of its input at the opcode 10, which PBeam does not support",
        )),
        None => error_tuple(scheduler, Term::Integer(i64::from(b'1'))),
    }
}

/// The state of `per_complete` (`asn1_erl_nif.c:119-493`): the input and
/// the index `ip` of the byte that `in_ptr` points at, the output and the
/// index `p` of the byte that `ptr` points at, and `unused`, the count of
/// the free low bits of that byte.
struct Per<'a> {
    input: &'a [u8],
    ip: usize,
    out: Vec<u8>,
    p: usize,
    unused: i32,
}

impl Per<'_> {
    /// Move `in_ptr` to the next byte and read it, as `*++in_ptr` does.
    /// A read after the end of the input gives 0.
    fn next(&mut self) -> i32 {
        self.ip += 1;
        i32::from(self.input.get(self.ip).copied().unwrap_or(0))
    }

    /// Return the output byte at `ptr`.
    fn get(&self) -> i32 {
        i32::from(self.out[self.p])
    }

    /// Set the output byte at `ptr` to the low byte of `value`.
    fn set(&mut self, value: i32) {
        self.out[self.p] = value as u8;
    }

    /// Set the output byte at `ptr` to itself or `value`.
    fn or(&mut self, value: i32) {
        self.set(self.get() | value);
    }

    /// Move `ptr` to the next byte and set it to 0, as `*++ptr = 0x00`.
    fn advance(&mut self) {
        self.p += 1;
        if self.out.len() <= self.p {
            self.out.resize(self.p + 1, 0);
        }
        self.set(0);
    }

    /// `per_insert_most_sign_bits` (`asn1_erl_nif.c:508-527`).
    fn insert_most_sign_bits(&mut self, no_bits: i32, val: i32) {
        let unused = self.unused;
        self.or(shr(val, 8 - unused));
        match no_bits.cmp(&unused) {
            Ordering::Less => self.unused -= no_bits,
            Ordering::Equal => {
                self.unused = 8;
                self.advance();
            }
            Ordering::Greater => {
                self.advance();
                self.or(shl(val, unused));
                self.unused = 8 - (no_bits - unused);
            }
        }
    }

    /// `per_insert_least_sign_bits` (`asn1_erl_nif.c:529-553`).
    fn insert_least_sign_bits(&mut self, no_bits: i32, val: i32) -> i32 {
        let unused = self.unused;
        match no_bits.cmp(&unused) {
            Ordering::Less => {
                self.or(shl(val, unused - no_bits));
                self.unused -= no_bits;
                0
            }
            Ordering::Equal => {
                self.or(val);
                self.unused = 8;
                self.advance();
                1
            }
            Ordering::Greater => {
                self.or(shr(val, no_bits - unused));
                self.advance();
                self.or(shl(val, 8 - (no_bits - unused)));
                self.unused = 8 - (no_bits - unused);
                1
            }
        }
    }

    /// Add one bit, as the cases 0 and 1 of `per_complete` and of
    /// `per_insert_octets_as_bits` do. Return whether a new byte starts.
    fn insert_bit(&mut self, bit: bool) -> bool {
        if self.unused == 1 {
            if bit {
                self.or(1);
            }
            self.unused = 8;
            self.advance();
            true
        } else {
            if bit {
                self.or(shl(1, self.unused - 1));
            }
            self.unused -= 1;
            false
        }
    }

    /// `per_pad_bits` (`asn1_erl_nif.c:558-573`).
    fn pad_bits(&mut self, mut no_bits: i32) -> i32 {
        let mut ret = 0;
        while no_bits > 0 {
            if self.insert_bit(false) {
                ret += 1;
            }
            no_bits -= 1;
        }
        ret
    }

    /// `per_insert_bits_as_bits` (`asn1_erl_nif.c:581-618`). It reads
    /// only the bytes that it inserts, so the next opcode can be a byte of
    /// `Bin`.
    fn insert_bits_as_bits(&mut self, desired_no: i32, no_bytes: i32) -> i32 {
        let unused = self.unused;
        match desired_no.cmp(&(no_bytes * 8)) {
            Ordering::Equal => self.insert_octets_unaligned(no_bytes, unused),
            Ordering::Less => {
                self.insert_octets_unaligned(desired_no / 8, unused);
                let val = self.next();
                self.insert_most_sign_bits(desired_no % 8, val);
                ceil(desired_no, 8)
            }
            Ordering::Greater => {
                self.insert_octets_unaligned(no_bytes, unused);
                self.pad_bits(desired_no - no_bytes * 8);
                ceil(desired_no, 8)
            }
        }
    }

    /// `per_insert_octets_as_bits_exact_len` (`asn1_erl_nif.c:621-651`).
    fn insert_octets_as_bits_exact_len(&mut self, desired_len: i32, in_len: i32) -> Option<i32> {
        match desired_len.cmp(&in_len) {
            Ordering::Equal => self.insert_octets_as_bits(in_len),
            Ordering::Greater => {
                let ret = self.insert_octets_as_bits(in_len)?;
                Some(ret + self.pad_bits(desired_len - in_len))
            }
            Ordering::Less => {
                let ret = self.insert_octets_as_bits(desired_len)?;
                self.ip += (in_len - desired_len) as usize;
                Some(ret)
            }
        }
    }

    /// `per_insert_octets_as_bits` (`asn1_erl_nif.c:659-693`): each byte
    /// must be 0 or 1. The C function returns `(used_bits + no_bytes) / 8`
    /// after its loop counts `no_bytes` down; with `unused` from 16 to 23,
    /// that value is -1, which the callers take as `ASN1_ERROR`.
    fn insert_octets_as_bits(&mut self, mut no_bytes: i32) -> Option<i32> {
        let used_bits = 8 - self.unused;
        while no_bytes > 0 {
            match self.next() {
                0 => self.insert_bit(false),
                1 => self.insert_bit(true),
                _ => return None,
            };
            no_bytes -= 1;
        }
        checked((used_bits + no_bytes) / 8)
    }

    /// `per_insert_octets` (`asn1_erl_nif.c:699-719`).
    fn insert_octets(&mut self, mut no_bytes: i32) -> i32 {
        let mut ret = 0;
        if self.unused != 8 {
            self.advance();
            ret += 1;
            self.unused = 8;
        }
        while no_bytes > 0 {
            let byte = self.next();
            self.set(byte);
            self.advance();
            no_bytes -= 1;
        }
        ret + no_bytes
    }

    /// `per_insert_octets_unaligned` (`asn1_erl_nif.c:724-746`), with the
    /// `unused` value that the caller passes.
    fn insert_octets_unaligned(&mut self, no_bytes: i32, unused: i32) -> i32 {
        for _ in 0..no_bytes.max(0) {
            let val = self.next();
            if unused == 8 {
                self.set(val);
                self.advance();
            } else {
                self.or(shr(val, 8 - unused));
                self.advance();
                self.set(shl(val, unused));
            }
        }
        no_bytes
    }

    /// `per_insert_octets_except_unused` (`asn1_erl_nif.c:748-789`). With
    /// `in_unused` other than 0 and `no_bytes` 0, the inner call returns
    /// -1, which is `ASN1_ERROR`.
    fn insert_octets_except_unused(&mut self, no_bytes: i32, in_unused: i32) -> Option<i32> {
        let unused = self.unused;
        if in_unused == 0 {
            return checked(self.insert_octets_unaligned(no_bytes, unused));
        }
        let mut ret = checked(self.insert_octets_unaligned(no_bytes - 1, unused))?;
        let val = self.next();
        let no_bits = 8 - in_unused;
        self.or(shr(val, 8 - unused));
        match no_bits.cmp(&unused) {
            Ordering::Less => self.unused = unused - no_bits,
            Ordering::Equal => {
                self.advance();
                ret += 1;
                self.unused = 8;
            }
            Ordering::Greater => {
                self.advance();
                ret += 1;
                self.or(shl(val, unused));
                self.unused = 8 - (no_bits - unused);
            }
        }
        Some(ret)
    }
}

/// The C value `ASN1_ERROR` (`asn1_erl_nif.c:31`).
const ASN1_ERROR: i32 = -1;

/// Return `None` for a helper result that is `ASN1_ERROR`, as each
/// `== ASN1_ERROR` check of the C code does. A count can equal -1, and the
/// C code then takes it as the error.
fn checked(ret: i32) -> Option<i32> {
    (ret != ASN1_ERROR).then_some(ret)
}

/// The C `int` shift `value << amount`. The shift instructions of the
/// pinned ERTS hosts use the amount modulo 32.
fn shl(value: i32, amount: i32) -> i32 {
    value.wrapping_shl(amount as u32)
}

/// The C `int` shift `value >> amount`, with the amount modulo 32.
fn shr(value: i32, amount: i32) -> i32 {
    value.wrapping_shr(amount as u32)
}

/// The C macro `CEIL(X,Y)` (`asn1_erl_nif.c:57`).
fn ceil(x: i32, y: i32) -> i32 {
    (x - 1) / y + 1
}

/// Read a length of `wide` bytes, big endian, as the cases of
/// `per_complete` do.
fn read_length(per: &mut Per, wide: bool) -> i32 {
    let high = per.next();
    if wide { (high << 8) | per.next() } else { high }
}

/// Run `per_complete` (`asn1_erl_nif.c:119-493`) on the non-empty `input`
/// and return the bytes of the binary of `encode_per_complete`, or `None`
/// for `ASN1_ERROR`. The binary has the size of the input, plus each
/// reallocation of the cases 40 to 47; `encode_per_complete` shrinks it to
/// the completed length and does not grow it
/// (`asn1_erl_nif.c:1227-1228`).
///
/// The opcode 10 reads its two bytes before a check of `counter`
/// (`asn1_erl_nif.c:186-199`), so in the last two bytes ERTS reads memory
/// outside the term and gives no stable result. Set `truncated` and return
/// `None` for that input.
fn per_complete(input: &[u8], truncated: &mut bool) -> Option<Vec<u8>> {
    let in_len = input.len() as i64;
    let mut per = Per {
        input,
        ip: 0,
        out: vec![0],
        p: 0,
        unused: 8,
    };
    let mut counter = in_len;
    let mut buf_space = in_len;
    let mut buf_size = in_len;
    while counter > 0 {
        counter -= 1;
        let opcode = input.get(per.ip).copied().unwrap_or(0);
        match opcode {
            0 | 1 => {
                if per.insert_bit(opcode == 1) {
                    buf_space -= 1;
                }
            }
            2 => {
                if per.unused != 8 {
                    per.advance();
                    buf_space -= 1;
                    per.unused = 8;
                }
            }
            10 => {
                if counter < 2 {
                    *truncated = true;
                    return None;
                }
                let no_bits = per.next();
                let val = per.next();
                counter -= 2;
                buf_space -= i64::from(per.insert_least_sign_bits(no_bits, val));
            }
            20 | 21 => {
                let no_bytes = read_length(&mut per, opcode == 21);
                counter -= i64::from(no_bytes) + if opcode == 21 { 2 } else { 1 };
                if counter < 0 {
                    return None;
                }
                buf_space -= i64::from(per.insert_octets(no_bytes));
            }
            30 | 31 => {
                let in_unused = per.next();
                let no_bytes = read_length(&mut per, opcode == 31);
                counter -= i64::from(no_bytes) + if opcode == 31 { 3 } else { 2 };
                if counter < 0 {
                    return None;
                }
                buf_space -= i64::from(per.insert_octets_except_unused(no_bytes, in_unused)?);
            }
            40..=43 | 45..=47 => {
                let wide_desired = matches!(opcode, 42 | 43 | 46 | 47);
                let wide_bytes = matches!(opcode, 41 | 43 | 47);
                let desired_len = read_length(&mut per, wide_desired);
                let no_bytes = read_length(&mut per, wide_bytes);
                let saved_mem = buf_space - counter;
                let needed = if opcode == 40 {
                    let pad_bits = desired_len - no_bytes - per.unused;
                    if pad_bits > 0 { ceil(pad_bits, 8) } else { 0 }
                } else {
                    ceil(desired_len - per.unused, 8) - no_bytes
                };
                if saved_mem < i64::from(needed) {
                    buf_size += i64::from(needed);
                    buf_space += i64::from(needed);
                }
                let header = 2 + i64::from(wide_desired) + i64::from(wide_bytes);
                counter -= header + i64::from(no_bytes);
                if counter < 0 {
                    return None;
                }
                let ret = if opcode < 45 {
                    checked(per.insert_octets_as_bits_exact_len(desired_len, no_bytes)?)?
                } else {
                    per.insert_bits_as_bits(desired_len, no_bytes)
                };
                buf_space -= i64::from(ret);
            }
            _ => return None,
        }
        per.ip += 1;
    }
    let complete_len = if per.unused == 8 && per.p != 0 {
        per.p
    } else {
        per.p + 1
    };
    per.out
        .truncate(usize::try_from(buf_size).unwrap_or(0).min(complete_len));
    Some(per.out)
}

/// The deepest nesting of `ber_decode_value` that PBeam decodes. The C
/// function stops at a stack distance of 8192 pointer words
/// (`asn1_erl_nif.c:954-958`), so the depth of the `unknown` error depends
/// on the stack frame size of the ERTS build: the pinned ERTS on
/// aarch64-apple-darwin decodes 454 calls and stops at 455, and the pinned
/// ERTS on x86_64 Linux decodes more. PBeam decodes the depth that each
/// build decodes and stops the run for a deeper input.
const MAX_DECODE_DEPTH: u32 = 454;

/// The reason of [`BerError`] for an input that nests deeper than
/// [`MAX_DECODE_DEPTH`].
const TOO_DEEP: &str = "too_deep";

/// A BER decode error: the return code and the index `ib_index` at the
/// error (`asn1_erl_nif.c:848-853`).
struct BerError {
    reason: &'static str,
    position: usize,
}

/// The state of `ber_decode_begin` (`asn1_erl_nif.c:841-861`).
struct Ber<'a> {
    input: &'a [u8],
    ib: usize,
}

impl Ber<'_> {
    /// Return the error `reason` at the current index.
    fn fail<T>(&self, reason: &'static str) -> Result<T, BerError> {
        Err(BerError {
            reason,
            position: self.ib,
        })
    }

    /// `ber_decode` (`asn1_erl_nif.c:863-889`): one `{Tag, Value}` up to
    /// the end index `end`.
    fn decode(&mut self, end: usize, depth: u32) -> Result<Term, BerError> {
        if self.ib + 2 > end {
            return self.fail("invalid_value");
        }
        let (tag, constructed) = self.decode_tag(end)?;
        if self.ib >= end {
            return self.fail("invalid_tag");
        }
        let value = self.decode_value(constructed, end, depth + 1)?;
        Ok(Term::tuple(vec![Term::Integer(tag), value]))
    }

    /// `ber_decode_tag` (`asn1_erl_nif.c:895-939`): the tag number with
    /// the class in bits 16 and 17, and whether the form is constructed.
    fn decode_tag(&mut self, end: usize) -> Result<(i64, bool), BerError> {
        let first = self.input[self.ib];
        let class = i64::from(first & 0xc0) << 10;
        let constructed = first & 0x20 != 0;
        let number = first & 0x1f;
        if number < 31 {
            self.ib += 1;
            return Ok((class | i64::from(number), constructed));
        }
        if self.ib + 3 > end {
            return self.fail("invalid_value");
        }
        self.ib += 1;
        let mut tag = class;
        if self.input[self.ib] >= 128 {
            tag |= i64::from(self.input[self.ib] & 0x7f) << 7;
            self.ib += 1;
        }
        if self.input[self.ib] >= 128 {
            return self.fail("invalid_tag");
        }
        tag |= i64::from(self.input[self.ib]);
        self.ib += 1;
        Ok((tag, constructed))
    }

    /// `ber_decode_value` (`asn1_erl_nif.c:946-1018`) at the call depth
    /// `depth`.
    fn decode_value(
        &mut self,
        constructed: bool,
        end: usize,
        depth: u32,
    ) -> Result<Term, BerError> {
        if depth > MAX_DECODE_DEPTH {
            return self.fail(TOO_DEEP);
        }
        let first = self.input[self.ib];
        let mut len = 0usize;
        if first & 0x80 == 0 {
            len = usize::from(first);
        } else if first == 0x80 {
            self.ib += 1;
            if self.ib + 1 >= end || !constructed {
                return self.fail("invalid_length");
            }
            let mut elements = Vec::new();
            while !(self.input[self.ib] == 0 && self.input[self.ib + 1] == 0) {
                elements.push(self.decode(end, depth)?);
                if self.ib + 1 >= end {
                    return self.fail("invalid_length");
                }
            }
            self.ib += 2;
            return Ok(Term::list_from(elements));
        } else {
            let lenoflen = usize::from(first & 0x7f);
            if lenoflen > end - (self.ib + 1) {
                return self.fail("invalid_length");
            }
            for _ in 0..lenoflen {
                self.ib += 1;
                if len >= 1 << 24 {
                    return self.fail("invalid_length");
                }
                len = (len << 8) + usize::from(self.input[self.ib]);
            }
        }
        if len > end - (self.ib + 1) {
            return self.fail("invalid_value");
        }
        self.ib += 1;
        let value_end = self.ib + len;
        if constructed {
            let mut elements = Vec::new();
            while self.ib < value_end {
                elements.push(self.decode(value_end, depth)?);
            }
            Ok(Term::list_from(elements))
        } else {
            self.ib = value_end;
            Ok(Term::binary(
                self.input[value_end - len..value_end].to_vec(),
            ))
        }
    }
}

/// Run `asn1rt_nif:decode_ber_tlv_raw(Data)` as the ERTS NIF
/// (`asn1_erl_nif.c:1259-1274`): data that is not iodata raises `badarg`,
/// a decode error gives `{error, {Reason, Position}}`
/// (`asn1_erl_nif.c:1233-1257`), and else `{{Tag, Value}, Rest}`.
///
/// # Examples
///
/// ```rust,ignore
/// let data = Term::binary(vec![0x30, 3, 2, 1, 5, 9]);
/// assert!(matches!(decode_ber_tlv_raw(&mut scheduler, &data)?, BifResult::Value(_)));
/// ```
fn decode_ber_tlv_raw(scheduler: &mut Scheduler, data: &Term) -> Result<BifResult, BeamError> {
    let Some(input) = iodata_bytes(data) else {
        return Ok(BifResult::badarg());
    };
    let mut ber = Ber {
        input: &input,
        ib: 0,
    };
    match ber.decode(input.len(), 0) {
        Ok(term) => {
            let rest = Term::binary(input[ber.ib..].to_vec());
            Ok(BifResult::Value(Term::tuple(vec![term, rest])))
        }
        Err(BerError {
            reason: TOO_DEEP, ..
        }) => Err(error(
            "asn1rt_nif:decode_ber_tlv_raw/1 nests more than 453 values, where the ERTS result depends on the C stack frame size of the build, which PBeam does not support",
        )),
        Err(BerError { reason, position }) => {
            let reason = Term::Atom(scheduler.atoms.put(reason)?);
            let detail = Term::tuple(vec![reason, Term::Integer(position as i64)]);
            error_tuple(scheduler, detail)
        }
    }
}

/// Run `asn1rt_nif:encode_ber_tlv(Tlv)` as the ERTS NIF
/// (`asn1_erl_nif.c:1276-1318`): each error of `ber_encode` gives
/// `{error, -1}`, and else the BER bytes.
///
/// # Examples
///
/// ```rust,ignore
/// let tlv = Term::tuple(vec![Term::Integer(2), Term::binary(vec![5])]);
/// assert!(matches!(encode_ber_tlv(&mut scheduler, &tlv)?, BifResult::Value(_)));
/// ```
fn encode_ber_tlv(scheduler: &mut Scheduler, tlv: &Term) -> Result<BifResult, BeamError> {
    let mut reversed = Vec::new();
    match ber_encode(tlv, &mut reversed) {
        Some(()) => {
            reversed.reverse();
            Ok(BifResult::Value(Term::binary(reversed)))
        }
        None => error_tuple(scheduler, Term::Integer(-1)),
    }
}

/// `ber_encode` (`asn1_erl_nif.c:1027-1100`): encode `{Tag, Value}`, where
/// a list `Value` is constructed and a binary `Value` is primitive. As the
/// C function writes each chunk from its end, the bytes go to `reversed`
/// in reverse order: the value, the length, then the tag, and the elements
/// of a list from the last one. The C function reads the first two
/// elements of a tuple without a check of the arity; PBeam rejects a tuple
/// of fewer than two elements.
fn ber_encode(tlv: &Term, reversed: &mut Vec<u8>) -> Option<()> {
    let Term::Tuple(elements) = tlv else {
        return None;
    };
    let (tag, value) = (elements.first()?, elements.get(1)?);
    let start = reversed.len();
    let form = if value.is_nil() || value.cons_cell().is_some() {
        let items: Vec<&Term> = value.proper_list()?.collect();
        for element in items.into_iter().rev() {
            ber_encode(element, reversed)?;
        }
        0x20
    } else {
        reversed.extend(value.binary_bytes()?.iter().rev());
        0
    };
    let Term::Integer(tag) = tag else {
        return None;
    };
    let tag = u32::try_from(*tag).ok()?;
    ber_encode_length(reversed.len() - start, reversed);
    ber_encode_tag(tag, form, reversed);
    Some(())
}

/// `ber_encode_tag` (`asn1_erl_nif.c:1102-1135`): the class from bits 16
/// and 17 of `tag`, and the tag number from its low 16 bits, in reverse
/// order.
fn ber_encode_tag(tag: u32, form: u8, reversed: &mut Vec<u8>) {
    let head = form | ((tag & 0x30000) >> 10) as u8;
    let mut number = tag & 0xffff;
    if number <= 30 {
        reversed.push(head | number as u8);
        return;
    }
    reversed.push((number & 127) as u8);
    number >>= 7;
    while number > 0 {
        reversed.push((number & 127) as u8 | 0x80);
        number >>= 7;
    }
    reversed.push(head | 0x1f);
}

/// `ber_encode_length` (`asn1_erl_nif.c:1137-1163`), in reverse order.
fn ber_encode_length(mut size: usize, reversed: &mut Vec<u8>) {
    if size < 128 {
        reversed.push(size as u8);
        return;
    }
    let mut chunks = 0u8;
    while size > 0 {
        reversed.push((size & 0xff) as u8);
        size >>= 8;
        chunks += 1;
    }
    reversed.push(chunks | 0x80);
}

#[cfg(test)]
#[path = "../../../../tests/runtime/interpreter/native/asn1rt_nif.rs"]
mod tests;

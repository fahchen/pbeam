//! The `erlang` md5 and phash BIFs: `md5/1`, `md5_init/0`, `md5_update/2`,
//! `md5_final/1`, `phash/2`, `phash2/1`, and `phash2/2`, which the
//! `plug_hello` release imports.
//!
//! The md5 BIFs run the state machine of `erl_md5.c` (`erts_md5_init`,
//! `erts_md5_update`, and `erts_md5_finish`, `76-144`) over the iodata walk
//! `do_chksum` (`erl_bif_chksum.c:53-268`). The context of `md5_init/0` is
//! the state as a binary of 96 bytes (`erts_md5_state`, `erl_md5.h:31-36`):
//! four 32-bit words, the length of the whole input, the length of the
//! buffered part, and a 64-byte buffer. A walk reads at most four bytes for
//! each reduction of the caller, charges one reduction for each byte read,
//! and traps with the rest of the iodata when the budget ends
//! (`erl_bif_chksum.c:455-610`).
//!
//! `phash/2` is the old term hash [`make_hash`] and `phash2/1,2` is the term
//! hash [`make_hash2`], both in the `crate::term::hash` module. `phash2/1,2`
//! charge one reduction for each 64 iterations of the hash
//! (`erl_term_hashing.c:858-861`, `1565-1570`). ERTS saves the state of a
//! `phash2` hash when its budget ends and continues in the next turn
//! (`erl_term_hashing.c:817-855`, `943`); PBeam completes the value in one
//! turn (see `docs/erts-differences.md`).

use super::BifResult::{TrapBif, Value};
use super::*;
use crate::term::{make_hash, make_hash2};
use num_bigint::Sign;

/// The size of the md5 context, which is the state as a binary
/// (`sizeof(erts_md5_state)`, `erl_md5.h:31-36`).
const CONTEXT_BYTES: usize = 96;

/// The size of an md5 digest (`MD5_SIZE`, `erl_bif_chksum.c:485`).
const DIGEST_BYTES: usize = 16;

/// The state of an md5 hash, as the 96 bytes of `erts_md5_state`
/// (`erl_md5.h:31-36`).
struct Md5 {
    /// The first word of the hash (`a`).
    a: u32,
    /// The second word of the hash (`b`).
    b: u32,
    /// The third word of the hash (`c`).
    c: u32,
    /// The fourth word of the hash (`d`).
    d: u32,
    /// The length of the whole input (`tot_len`).
    total: u64,
    /// The length of the buffered part (`len`), which is at most 64.
    buffered: u64,
    /// The bytes of the part that is not hashed yet (`buf`).
    buffer: [u8; 64],
}

impl Md5 {
    /// Return the state of a new hash, as `erts_md5_init`
    /// (`erl_md5.c:76-85`) gives it.
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// let state = Md5::init();
    /// assert_eq!(state.a, 0x67452301);
    /// assert_eq!(state.buffered, 0);
    /// ```
    fn init() -> Md5 {
        Md5 {
            a: 0x6745_2301,
            b: 0xefcd_ab89,
            c: 0x98ba_dcfe,
            d: 0x1032_5476,
            total: 0,
            buffered: 0,
            buffer: [0; 64],
        }
    }

    /// Return the state of the 96 bytes `bytes` of a context binary, or
    /// `None` when the buffered length of the state is above 64. ERTS keeps
    /// a buffered length of up to 64 (`erl_md5.c:96-108`), and reads and
    /// writes outside its buffer above 64 (`90-116`, `222-243`), which is
    /// undefined behavior; `docs/erts-differences.md` records that PBeam
    /// treats the term as a term that is not a context.
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// let state = Md5::from_binary(&Md5::init().to_binary()).unwrap();
    /// assert_eq!(state.a, 0x67452301);
    /// ```
    fn from_binary(bytes: &[u8]) -> Option<Md5> {
        let word = |at: usize| {
            u32::from_le_bytes([bytes[at], bytes[at + 1], bytes[at + 2], bytes[at + 3]])
        };
        let long = |at: usize| {
            let mut value = [0; 8];
            value.copy_from_slice(&bytes[at..at + 8]);
            u64::from_le_bytes(value)
        };
        let buffered = long(24);
        if buffered > 64 {
            return None;
        }
        let mut buffer = [0; 64];
        buffer.copy_from_slice(&bytes[32..96]);
        Some(Md5 {
            a: word(0),
            b: word(4),
            c: word(8),
            d: word(12),
            total: long(16),
            buffered,
            buffer,
        })
    }

    /// Return the state as the 96 bytes of a context binary.
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// assert_eq!(Md5::init().to_binary().len(), CONTEXT_BYTES);
    /// ```
    fn to_binary(&self) -> Vec<u8> {
        let mut bytes = Vec::with_capacity(CONTEXT_BYTES);
        for word in [self.a, self.b, self.c, self.d] {
            bytes.extend_from_slice(&word.to_le_bytes());
        }
        bytes.extend_from_slice(&self.total.to_le_bytes());
        bytes.extend_from_slice(&self.buffered.to_le_bytes());
        bytes.extend_from_slice(&self.buffer);
        bytes
    }

    /// Add `bytes` to the state, as `erts_md5_update` (`erl_md5.c:87-123`)
    /// does: the buffered part first, then whole 64-byte blocks, then the
    /// new buffered part.
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// let mut state = Md5::init();
    /// state.update(b"abc");
    /// assert_eq!(state.buffered, 3);
    /// ```
    fn update(&mut self, bytes: &[u8]) {
        let mut part = bytes;
        if self.buffered > 0 {
            let buffered = self.buffered as usize;
            let copy = 64 - buffered;
            if copy < part.len() {
                self.buffer[buffered..].copy_from_slice(&part[..copy]);
                part = &part[copy..];
                let block = self.buffer;
                self.hash_block(&block);
                self.buffered = 0;
                self.total = self.total.wrapping_add(64);
            } else {
                self.buffer[buffered..buffered + part.len()].copy_from_slice(part);
                self.buffered += part.len() as u64;
                return;
            }
        }
        while part.len() >= 64 {
            let (block, rest) = part.split_at(64);
            let block: [u8; 64] = block.try_into().expect("a block of 64 bytes");
            self.hash_block(&block);
            part = rest;
            self.total = self.total.wrapping_add(64);
        }
        if !part.is_empty() {
            self.buffer[..part.len()].copy_from_slice(part);
            self.buffered = part.len() as u64;
        }
    }

    /// Return the digest of the state, as `erts_md5_finish`
    /// (`erl_md5.c:125-144`) does: pad the buffered part with one bit, then
    /// zeros, then the bit length of the whole input, and hash the blocks.
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// assert_eq!(Md5::init().finish(), [212, 29, 140, 217, 143, 0, 178, 4, 233, 128, 9, 152, 236, 248, 66, 126]);
    /// ```
    fn finish(&mut self) -> [u8; DIGEST_BYTES] {
        let size = self.buffered as usize;
        let mut pad = [0u8; 128];
        pad[..size].copy_from_slice(&self.buffer[..size]);
        pad[size] = 0x80;
        // The padding fills the rest of the block, except the 8 bytes of the
        // bit length (`pad`, `erl_md5.c:222-243`).
        let filled = (size + 1) % 64;
        // `size_t` arithmetic wraps for a buffered part of 56 bytes or more,
        // as `pad` does (`erl_md5.c:227-231`).
        let zeroes = (56_u64.wrapping_sub(filled as u64) & 63) as usize;
        let bits = self.total.wrapping_add(self.buffered).wrapping_mul(8);
        let length = size + 1 + zeroes + 8;
        pad[length - 8..length].copy_from_slice(&bits.to_le_bytes());
        for block in pad[..length].chunks(64) {
            let block: [u8; 64] = block.try_into().expect("a block of 64 bytes");
            self.hash_block(&block);
        }
        let mut digest = [0; DIGEST_BYTES];
        for (at, word) in [self.a, self.b, self.c, self.d].into_iter().enumerate() {
            digest[at * 4..at * 4 + 4].copy_from_slice(&word.to_le_bytes());
        }
        digest
    }

    /// Hash one 64-byte block into the state, as `hash_part`
    /// (`erl_md5.c:156-215`) does.
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// let mut state = Md5::init();
    /// state.hash_block(&[0; 64]);
    /// assert_eq!(state.a, 0xd98c1dd4);
    /// ```
    fn hash_block(&mut self, block: &[u8]) {
        /// The shift and the added constant of each of the 64 rounds
        /// (`sk`, `erl_md5.c:157-178`).
        const SK: [(u32, u32); 64] = [
            (7, 0xd76a_a478),
            (12, 0xe8c7_b756),
            (17, 0x2420_70db),
            (22, 0xc1bd_ceee),
            (7, 0xf57c_0faf),
            (12, 0x4787_c62a),
            (17, 0xa830_4613),
            (22, 0xfd46_9501),
            (7, 0x6980_98d8),
            (12, 0x8b44_f7af),
            (17, 0xffff_5bb1),
            (22, 0x895c_d7be),
            (7, 0x6b90_1122),
            (12, 0xfd98_7193),
            (17, 0xa679_438e),
            (22, 0x49b4_0821),
            (5, 0xf61e_2562),
            (9, 0xc040_b340),
            (14, 0x265e_5a51),
            (20, 0xe9b6_c7aa),
            (5, 0xd62f_105d),
            (9, 0x0244_1453),
            (14, 0xd8a1_e681),
            (20, 0xe7d3_fbc8),
            (5, 0x21e1_cde6),
            (9, 0xc337_07d6),
            (14, 0xf4d5_0d87),
            (20, 0x455a_14ed),
            (5, 0xa9e3_e905),
            (9, 0xfcef_a3f8),
            (14, 0x676f_02d9),
            (20, 0x8d2a_4c8a),
            (4, 0xfffa_3942),
            (11, 0x8771_f681),
            (16, 0x6d9d_6122),
            (23, 0xfde5_380c),
            (4, 0xa4be_ea44),
            (11, 0x4bde_cfa9),
            (16, 0xf6bb_4b60),
            (23, 0xbebf_bc70),
            (4, 0x289b_7ec6),
            (11, 0xeaa1_27fa),
            (16, 0xd4ef_3085),
            (23, 0x0488_1d05),
            (4, 0xd9d4_d039),
            (11, 0xe6db_99e5),
            (16, 0x1fa2_7cf8),
            (23, 0xc4ac_5665),
            (6, 0xf429_2244),
            (10, 0x432a_ff97),
            (15, 0xab94_23a7),
            (21, 0xfc93_a039),
            (6, 0x655b_59c3),
            (10, 0x8f0c_cc92),
            (15, 0xffef_f47d),
            (21, 0x8584_5dd1),
            (6, 0x6fa8_7e4f),
            (10, 0xfe2c_e6e0),
            (15, 0xa301_4314),
            (21, 0x4e08_11a1),
            (6, 0xf753_7e82),
            (10, 0xbd3a_f235),
            (15, 0x2ad7_d2bb),
            (21, 0xeb86_d391),
        ];
        let (mut a, mut b, mut c, mut d) = (self.a, self.b, self.c, self.d);
        for (index, (shift, constant)) in SK.into_iter().enumerate() {
            let (function, word) = match index {
                0..=15 => (d ^ (b & (c ^ d)), index),
                16..=31 => (c ^ (d & (b ^ c)), (5 * index + 1) & 0xF),
                32..=47 => (b ^ c ^ d, (3 * index + 5) & 0xF),
                _ => (c ^ (b | !d), (7 * index) & 0xF),
            };
            let value = block[word * 4..word * 4 + 4]
                .try_into()
                .map(u32::from_le_bytes)
                .expect("a word of the block");
            let mut value = function
                .wrapping_add(a)
                .wrapping_add(constant)
                .wrapping_add(value);
            value = value.rotate_left(shift);
            (a, b, c, d) = (d, b.wrapping_add(value), b, c);
        }
        self.a = self.a.wrapping_add(a);
        self.b = self.b.wrapping_add(b);
        self.c = self.c.wrapping_add(c);
        self.d = self.d.wrapping_add(d);
    }
}

/// Give the bytes of a pending run to `sum` as one update call, and empty
/// the run. ERTS reads a flat level of byte elements with one call of its
/// sum function, and a nil element or another element ends the run
/// (`erl_bif_chksum.c:120-160`), so the md5 state of
/// `erlang:md5_update/2` keeps the same block boundaries.
///
/// # Examples
///
/// ```rust,ignore
/// let mut run = vec![1, 2];
/// let mut bytes = Vec::new();
/// flush(&mut run, &mut |part| bytes.extend_from_slice(part));
/// assert_eq!(bytes, [1, 2]);
/// ```
fn flush(run: &mut Vec<u8>, sum: &mut impl FnMut(&[u8])) {
    if !run.is_empty() {
        sum(run);
        run.clear();
    }
}

/// Return the rest of a walk that ended inside `term`: the term and then the
/// tail of each list around it, innermost first, as `do_chksum` builds the
/// rest from its stack (`erl_bif_chksum.c:247-259`).
///
/// # Examples
///
/// ```rust,ignore
/// assert_eq!(rest_term(Term::Integer(1), Vec::new()), Term::Integer(1));
/// ```
fn rest_term(mut term: Term, mut tails: Vec<Term>) -> Term {
    while let Some(tail) = tails.pop() {
        term = Term::cons(term, tail);
    }
    term
}

/// Read at most `budget` bytes of the iodata `term` and give them to `sum`,
/// as `do_chksum` (`erl_bif_chksum.c:53-268`) reads them. Return `None` when
/// the walk read all of the iodata, and the rest of the iodata when the
/// budget ended before it did. `Err(())` marks a term that is not iodata: a
/// term that is not a byte, a list, a nil, or a byte-aligned binary.
///
/// The walk keeps the tails of the lists around the current level on a work
/// list, so a deep term does not recurse.
///
/// # Examples
///
/// ```rust,ignore
/// let mut budget = 2;
/// let mut bytes = Vec::new();
/// let list = Term::list_from([Term::Integer(1), Term::Integer(2), Term::Integer(3)]);
/// assert!(read(&list, &mut budget, &mut |part| bytes.extend_from_slice(part)).unwrap().is_some());
/// assert_eq!(bytes, [1, 2]);
/// ```
fn read(term: &Term, budget: &mut u64, sum: &mut impl FnMut(&[u8])) -> Result<Option<Term>, ()> {
    if term.is_nil() {
        return Ok(None);
    }
    if *budget == 0 {
        return Ok(Some(term.clone()));
    }
    let mut run: Vec<u8> = Vec::new();
    // The tails of the lists above the current level, innermost last.
    let mut tails: Vec<Term> = Vec::new();
    let mut current = term.clone();
    loop {
        // The walk of ERTS stops at the budget before it reads the next
        // element, also when that element is an empty list or the budget ends
        // at the end of a list level (`erl_bif_chksum.c:118-160`, `205-212`).
        if *budget == 0 {
            flush(&mut run, sum);
            let rest = rest_term(current.clone(), tails);
            // An empty rest ends the walk: ERTS returns the nil term, which
            // `md5/1` does not trap on (`erl_bif_chksum.c:205-268`).
            return Ok((!rest.is_nil()).then_some(rest));
        }
        match &current {
            Term::Nil => {}
            Term::Binary(binary) => {
                flush(&mut run, sum);
                let bit_size = binary.bit_size();
                // `erts_get_aligned_binary_bytes` gives NULL for a bitstring
                // that is not a whole number of bytes (`erl_bits.h:621-628`).
                if !bit_size.is_multiple_of(8) {
                    return Err(());
                }
                let size = bit_size / 8;
                let taken = (size as u64).min(*budget) as usize;
                // One update call reads the binary, as `md5_wrap` reads one
                // bitstring, so the state does not depend on the parts of a
                // binary that an append built (`erl_bif_chksum.c:74-92`).
                let runs = binary.byte_runs(0, taken);
                match runs.as_slice() {
                    [run] => sum(run),
                    runs => {
                        let mut bytes = Vec::with_capacity(taken);
                        for run in runs {
                            bytes.extend_from_slice(run);
                        }
                        sum(&bytes);
                    }
                }
                *budget -= taken as u64;
                if taken < size {
                    let rest = Term::Binary(binary.bits(taken * 8, bit_size));
                    return Ok(Some(rest_term(rest, tails)));
                }
            }
            Term::Cons(_) => {
                let (head, rest) = current.cons_cell().expect("the term is a list");
                let (head, rest) = (head.clone(), rest.clone());
                match head {
                    // The walk skips an empty list in a list, and the empty
                    // list ends a run of bytes (`erl_bif_chksum.c:145-160`).
                    Term::Nil => {
                        flush(&mut run, sum);
                        current = rest;
                        continue;
                    }
                    Term::Integer(value @ 0..=255) => {
                        run.push(value as u8);
                        *budget -= 1;
                        current = rest;
                        continue;
                    }
                    Term::Cons(_) | Term::Binary(_) => {
                        flush(&mut run, sum);
                        tails.push(rest);
                        current = head;
                        continue;
                    }
                    _ => return Err(()),
                }
            }
            _ => return Err(()),
        }
        // The current term is read: continue with the tail of the list above
        // it, or end the walk.
        match tails.pop() {
            Some(tail) => {
                // ERTS reads each flat list level with its own update call,
                // so the run ends with the level (`erl_bif_chksum.c:120-160`).
                flush(&mut run, sum);
                current = tail;
            }
            None => {
                flush(&mut run, sum);
                return Ok(None);
            }
        }
    }
}

/// Return the bytes of the context `term` when it is a byte-aligned binary
/// of the size of an md5 state, as the checks of `md5_update_2` and
/// `md5_final_1` read it (`erl_bif_chksum.c:556-563`, `594-601`).
///
/// # Examples
///
/// ```rust,ignore
/// let context = Term::Binary(Binary::from(Md5::init().to_binary()));
/// assert!(context_bytes(&context).is_some());
/// assert!(context_bytes(&Term::binary(vec![0])).is_none());
/// ```
fn context_bytes(term: &Term) -> Option<&[u8]> {
    match term {
        Term::Binary(binary) if binary.bit_size() == CONTEXT_BYTES * 8 => binary.bytes(),
        _ => None,
    }
}

/// The value of a checksum BIF, which selects the BIF that a walk that uses
/// its whole budget traps to (`erl_bif_chksum.c:481`, `527`, `582`).
#[derive(Clone, Copy)]
enum Checksum {
    /// `erlang:md5/1`: the value is the digest, and a trap continues in the
    /// hidden export `erlang:md5_trap/2` (`erl_bif_chksum.c:42-52`).
    Digest,
    /// `erlang:md5_update/2`: the value is the new context, and a trap calls
    /// `erlang:md5_update/2` again.
    Context,
}

/// Charge the bytes that a walk read as reductions,
/// `BUMP_REDS(BIF_P, res)` (`erl_bif_chksum.c:463-489`): one reduction for
/// each byte. A charge that uses more reductions than the caller has stops at
/// 0, as `BUMP_REDS` does (`bif.h:129-141`).
///
/// # Examples
///
/// ```rust,ignore
/// process.fcalls = 3;
/// charge(&mut scheduler, 5);
/// assert_eq!(scheduler.current_process_mut().fcalls, 0);
/// ```
fn charge(scheduler: &mut Scheduler, bytes: u64) {
    let reductions = i32::try_from(bytes).unwrap_or(i32::MAX);
    scheduler.current_process_mut().bump_reductions(reductions);
}

/// Run the walk of `erlang:md5/1`, `erlang:md5_trap/2`, or
/// `erlang:md5_update/2`: mix the bytes of `iolist` into `state` with the
/// budget of four bytes for each reduction of the caller
/// (`reds = (4 * ERTS_BIF_REDS_LEFT(BIF_P)) & ~0x3f`,
/// `erl_bif_chksum.c:465-467`). When the walk does not end, use all
/// reductions of the caller, return the state as a context binary, and trap
/// to the BIF of `checksum` with the rest of the iodata
/// (`erl_bif_chksum.c:477-483`, `523-529`, `578-584`).
///
/// # Examples
///
/// ```rust,ignore
/// let result = checksum(&mut scheduler, Md5::init(), &Term::binary(b"abc".to_vec()), Checksum::Digest)?;
/// assert!(matches!(result, BifResult::Value(Term::Binary(bytes)) if bytes.len() == DIGEST_BYTES));
/// ```
fn checksum(
    scheduler: &mut Scheduler,
    mut state: Md5,
    iolist: &Term,
    kind: Checksum,
) -> Result<BifResult, BeamError> {
    // `do_chksum` returns the whole term when `left <= 0`, so a negative
    // budget traps at once, as a budget of 0 does (`erl_bif_chksum.c:64-67`).
    let reds = scheduler.current_process_mut().bif_reductions_or_zero();
    let mut budget = (reds as u64 * 4) & !0x3f;
    let walk = read(iolist, &mut budget, &mut |bytes| state.update(bytes));
    let rest = match walk {
        Err(()) => {
            // A walk that found a term that is not iodata charges the bytes
            // that it read and raises `badarg` (`erl_bif_chksum.c:470-473`).
            charge(scheduler, ((reds as u64 * 4) & !0x3f) - budget);
            return Ok(BifResult::badarg());
        }
        Ok(rest) => rest,
    };
    let bytes = ((reds as u64 * 4) & !0x3f) - budget;
    charge(scheduler, bytes);
    let Some(rest) = rest else {
        return Ok(Value(match kind {
            Checksum::Digest => Term::Binary(Binary::from(state.finish().to_vec())),
            Checksum::Context => Term::Binary(Binary::from(state.to_binary())),
        }));
    };
    // `BUMP_ALL_REDS` before a trap (`erl_bif_chksum.c:479`, `525`, `581`).
    scheduler.current_process_mut().bump_all_reductions();
    let function = match kind {
        Checksum::Digest => "md5_trap",
        Checksum::Context => "md5_update",
    };
    Ok(TrapBif {
        module: "erlang",
        function,
        arguments: vec![Term::Binary(Binary::from(state.to_binary())), rest],
    })
}

/// Run `erlang:md5/1` as ERTS `md5_1` does (`erl_bif_chksum.c:455-490`).
///
/// # Examples
///
/// ```rust,ignore
/// let result = md5(&mut scheduler, &Term::binary(b"abc".to_vec()))?;
/// assert!(matches!(result, BifResult::Value(Term::Binary(bytes)) if bytes.len() == DIGEST_BYTES));
/// ```
pub(super) fn md5(scheduler: &mut Scheduler, iolist: &Term) -> Result<BifResult, BeamError> {
    checksum(scheduler, Md5::init(), iolist, Checksum::Digest)
}

/// Run `erlang:md5_init/0` as ERTS `md5_init_0` does
/// (`erl_bif_chksum.c:539-547`): a binary of the new state. The state is 96
/// bytes, so the binary is not a small binary.
///
/// # Examples
///
/// ```rust,ignore
/// assert!(matches!(md5_init(), BifResult::Value(Term::Binary(bytes)) if bytes.len() == CONTEXT_BYTES));
/// ```
pub(super) fn md5_init() -> BifResult {
    Value(Term::Binary(Binary::from(Md5::init().to_binary())))
}

/// Continue `erlang:md5/1` after a trap, as the hidden export
/// `erlang:md5_trap/2` does (`md5_2`, `erl_bif_chksum.c:494-536`): the
/// argument is the state of the walk and the rest of the iodata.
///
/// # Examples
///
/// ```rust,ignore
/// let context = Term::Binary(Binary::from(Md5::init().to_binary()));
/// let result = md5_trap(&mut scheduler, &context, &Term::binary(b"abc".to_vec()))?;
/// assert!(matches!(result, BifResult::Value(Term::Binary(bytes)) if bytes.len() == DIGEST_BYTES));
/// ```
pub(super) fn md5_trap(
    scheduler: &mut Scheduler,
    context: &Term,
    iolist: &Term,
) -> Result<BifResult, BeamError> {
    // Only the checksum BIF of `erlang:md5/1` makes this trap, and it always
    // passes a state that `Md5::to_binary` wrote.
    let state = context_bytes(context)
        .and_then(Md5::from_binary)
        .expect("the trap state of erlang:md5_trap/2 is an md5 state");
    checksum(scheduler, state, iolist, Checksum::Digest)
}

/// Run `erlang:md5_update/2` as ERTS `md5_update_2` does
/// (`erl_bif_chksum.c:551-588`): the first argument is the state as a
/// binary of 96 bytes, and the value is the new state. The call does not
/// change the argument, which is immutable.
///
/// # Examples
///
/// ```rust,ignore
/// let context = md5_init();
/// let BifResult::Value(context) = context else { unreachable!() };
/// let result = md5_update(&mut scheduler, &context, &Term::binary(b"abc".to_vec()))?;
/// assert!(matches!(result, BifResult::Value(Term::Binary(bytes)) if bytes.len() == CONTEXT_BYTES));
/// ```
pub(super) fn md5_update(
    scheduler: &mut Scheduler,
    context: &Term,
    iolist: &Term,
) -> Result<BifResult, BeamError> {
    let Some(state) = context_bytes(context).and_then(Md5::from_binary) else {
        return Ok(BifResult::badarg());
    };
    checksum(scheduler, state, iolist, Checksum::Context)
}

/// Run `erlang:md5_final/1` as ERTS `md5_final_1` does
/// (`erl_bif_chksum.c:590-610`): the argument is the state as a binary of 96
/// bytes, and the value is the 16-byte digest. The call does not change the
/// argument.
///
/// # Examples
///
/// ```rust,ignore
/// let context = Term::Binary(Binary::from(Md5::init().to_binary()));
/// assert!(matches!(md5_final(&context), BifResult::Value(Term::Binary(bytes)) if bytes.len() == DIGEST_BYTES));
/// assert!(matches!(md5_final(&Term::nil()), BifResult::Raise { .. }));
/// ```
pub(super) fn md5_final(context: &Term) -> BifResult {
    let Some(mut state) = context_bytes(context).and_then(Md5::from_binary) else {
        return BifResult::badarg();
    };
    Value(Term::Binary(Binary::from(state.finish().to_vec())))
}

/// Return the `Range` argument of `phash/2` and `phash2/2`, as ERTS
/// `phash_2` reads it (`bif.c:5324-5334`): `Some(0)` for the range 2^32,
/// which ERTS treats as no range, `Some(range)` for the range 1 to 2^32 - 1,
/// and `None` for the range 0, a negative range, a range above 2^32, and a
/// term that is not an integer, which are all `badarg`.
///
/// # Examples
///
/// ```rust,ignore
/// assert_eq!(parse_range(&Term::Integer(1)), Some(1));
/// assert_eq!(parse_range(&Term::Integer(4294967296)), Some(0));
/// assert_eq!(parse_range(&Term::Integer(0)), None);
/// ```
fn parse_range(term: &Term) -> Option<u32> {
    match term {
        Term::Integer(value) if *value == 1 << 32 => Some(0),
        Term::Integer(value) => u32::try_from(*value).ok().filter(|range| *range != 0),
        Term::BigInt(value) if value.sign() != Sign::Minus => u32::try_from(value.as_ref())
            .ok()
            .filter(|range| *range != 0),
        _ => None,
    }
}

/// Run `erlang:phash/2` as ERTS `phash_2` does (`bif.c:5319-5348`): the old
/// term hash of `term` in the range 1 to `Range`, or `hash + 1` for the
/// range 2^32. The value of a range of 2^32 does not fit a 32-bit word when
/// the hash is 2^32 - 1, so ERTS makes a bignum; PBeam makes the integer
/// 2^32, which is a small integer.
///
/// # Examples
///
/// ```rust,ignore
/// assert!(matches!(phash(&mut scheduler, &Term::static_atom("a"), &Term::Integer(2)), BifResult::Value(Term::Integer(2))));
/// assert!(matches!(phash(&mut scheduler, &Term::static_atom("a"), &Term::Integer(0)), BifResult::Raise { .. }));
/// ```
pub(super) fn phash(scheduler: &mut Scheduler, term: &Term, range: &Term) -> BifResult {
    let Some(range) = parse_range(range) else {
        return BifResult::badarg();
    };
    let hash = make_hash(&scheduler.atoms, term);
    let value = match range {
        0 => u64::from(hash) + 1,
        range => 1 + u64::from(hash) % u64::from(range),
    };
    Value(Term::Integer(value as i64))
}

/// Charge the reductions of a `phash2` call: one reduction for each 64
/// iterations of the term hash, as `BUMP_REDS` with
/// `(max_iterations - iterations_until_trap) / ITERATIONS_PER_RED` charges
/// them (`erl_term_hashing.c:860-861`, `1565-1570`).
///
/// # Examples
///
/// ```rust,ignore
/// process.fcalls = 10;
/// charge_iterations(&mut scheduler, 128);
/// assert_eq!(scheduler.current_process_mut().fcalls, 8);
/// ```
fn charge_iterations(scheduler: &mut Scheduler, iterations: u64) {
    let reductions = i32::try_from(iterations / 64).unwrap_or(i32::MAX);
    scheduler.current_process_mut().bump_reductions(reductions);
}

/// Run `erlang:phash2/1` as ERTS `phash2_1` does (`bif.c:5350-5360`): the
/// 27 low bits of the term hash, which give a value in 0 to 2^27 - 1.
///
/// # Examples
///
/// ```rust,ignore
/// assert!(matches!(phash2_1(&mut scheduler, &Term::static_atom("abc")), BifResult::Value(Term::Integer(26499))));
/// ```
pub(super) fn phash2_1(scheduler: &mut Scheduler, term: &Term) -> BifResult {
    let (hash, iterations) = make_hash2(&scheduler.atoms, term);
    charge_iterations(scheduler, iterations);
    Value(Term::Integer(i64::from(hash & ((1 << 27) - 1))))
}

/// Run `erlang:phash2/2` as ERTS `phash2_2` does (`bif.c:5362-5402`): the
/// term hash modulo `Range`, which gives a value in 0 to `Range` - 1, or the
/// hash itself for the range 2^32. ERTS reads the range before the hash, so
/// a range that is `badarg` charges no reduction.
///
/// # Examples
///
/// ```rust,ignore
/// assert!(matches!(phash2_2(&mut scheduler, &Term::static_atom("a"), &Term::Integer(2)), BifResult::Value(Term::Integer(1))));
/// assert!(matches!(phash2_2(&mut scheduler, &Term::static_atom("a"), &Term::Integer(0)), BifResult::Raise { .. }));
/// ```
pub(super) fn phash2_2(scheduler: &mut Scheduler, term: &Term, range: &Term) -> BifResult {
    let Some(range) = parse_range(range) else {
        return BifResult::badarg();
    };
    let (hash, iterations) = make_hash2(&scheduler.atoms, term);
    charge_iterations(scheduler, iterations);
    let value = match range {
        0 => hash,
        range => hash % range,
    };
    Value(Term::Integer(i64::from(value)))
}

#[cfg(test)]
#[path = "../../../../tests/runtime/interpreter/bif/hash.rs"]
mod tests;

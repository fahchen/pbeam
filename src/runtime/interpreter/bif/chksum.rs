//! The `erlang` checksum BIFs `adler32/1,2`, `adler32_combine/3`,
//! `crc32/1,2`, and `crc32_combine/3` (`erl_bif_chksum.c:266-451`).
//!
//! A walk reads the iodata with the walk of the md5 BIFs ([`hash::read`],
//! `do_chksum`, `erl_bif_chksum.c:53-264`) and gives each part to the zlib
//! function, as `adler32_wrap` and `crc32_wrap` do
//! (`erl_bif_chksum.c:266-278`). A walk reads at most
//! `CHUNK_PER_SCHEDULE` bytes, charges one reduction for each 10 bytes, and
//! traps to the two-argument BIF with the sum and the rest of the iodata
//! when the budget ends (`erl_bif_chksum.c:286-309`).
//!
//! The adler32 sum is a port of zlib `adler32_z` and `adler32_combine_`. An
//! initial sum whose halves are not below 65521 gives a value that depends
//! on the length of each part, so the port keeps each branch of zlib. The
//! crc32 sum uses the `crc32fast` crate, which gives the zlib values.

use super::BifResult::{TrapBif, Value};
use super::*;
use crate::runtime::scheduler::CONTEXT_REDS;

/// `BYTES_PER_REDUCTION` (`erl_bif_chksum.c:286`).
const BYTES_PER_REDUCTION: u64 = 10;
/// `CHUNK_PER_SCHEDULE` (`erl_bif_chksum.c:287`).
const CHUNK_PER_SCHEDULE: u64 = BYTES_PER_REDUCTION * CONTEXT_REDS as u64;
/// The adler32 modulus `BASE` of zlib `adler32.c`.
const BASE: u64 = 65521;
/// The adler32 `NMAX` of zlib `adler32.c`: the longest run that does not
/// overflow before the modulus.
const NMAX: usize = 5552;

/// The sum of a checksum BIF.
#[derive(Clone, Copy)]
pub(super) enum Sum {
    /// `erlang:adler32/1,2`.
    Adler32,
    /// `erlang:crc32/1,2`.
    Crc32,
}

/// Return the zlib `adler32_z(adler, buf, len)` of a buffer that is not
/// NULL, with each branch of zlib `adler32.c`.
///
/// # Examples
///
/// ```rust,ignore
/// assert_eq!(adler32(1, b"abc"), 0x024d0127);
/// ```
fn adler32(adler: u32, buf: &[u8]) -> u32 {
    let mut sum2 = u64::from(adler >> 16);
    let mut adler = u64::from(adler & 0xffff);
    if let [byte] = buf {
        adler += u64::from(*byte);
        if adler >= BASE {
            adler -= BASE;
        }
        sum2 += adler;
        if sum2 >= BASE {
            sum2 -= BASE;
        }
        return (adler | (sum2 << 16)) as u32;
    }
    if buf.len() < 16 {
        for byte in buf {
            adler += u64::from(*byte);
            sum2 += adler;
        }
        if adler >= BASE {
            adler -= BASE;
        }
        sum2 %= BASE;
        return (adler | (sum2 << 16)) as u32;
    }
    for block in buf.chunks(NMAX) {
        for byte in block {
            adler += u64::from(*byte);
            sum2 += adler;
        }
        adler %= BASE;
        sum2 %= BASE;
    }
    (adler | (sum2 << 16)) as u32
}

/// Return the zlib `adler32_combine_(adler1, adler2, len2)` for a length
/// that is not negative, truncated to 32 bits as `adler32_combine_3` does
/// (`erl_bif_chksum.c:446`).
///
/// # Examples
///
/// ```rust,ignore
/// assert_eq!(adler32_combine(1, 2, 0), 3);
/// ```
fn adler32_combine(adler1: u32, adler2: u32, len2: u32) -> u32 {
    let (adler1, adler2) = (u64::from(adler1), u64::from(adler2));
    let rem = u64::from(len2) % BASE;
    let mut sum1 = adler1 & 0xffff;
    let mut sum2 = (rem * sum1) % BASE;
    sum1 += (adler2 & 0xffff) + BASE - 1;
    sum2 += ((adler1 >> 16) & 0xffff) + ((adler2 >> 16) & 0xffff) + BASE - rem;
    if sum1 >= BASE {
        sum1 -= BASE;
    }
    if sum1 >= BASE {
        sum1 -= BASE;
    }
    if sum2 >= BASE << 1 {
        sum2 -= BASE << 1;
    }
    if sum2 >= BASE {
        sum2 -= BASE;
    }
    (sum1 | (sum2 << 16)) as u32
}

/// Return a 32-bit argument, as `term_to_Uint(term, &u)` and
/// `((u >> 16) >> 16) == 0` check it (`erl_bif_chksum.c:319`): an integer
/// from 0 to 2^32 - 1. A bignum is above that range.
///
/// # Examples
///
/// ```rust,ignore
/// assert_eq!(word(&Term::Integer(1)), Some(1));
/// assert_eq!(word(&Term::Integer(-1)), None);
/// ```
fn word(term: &Term) -> Option<u32> {
    match term {
        Term::Integer(value) => u32::try_from(*value).ok(),
        _ => None,
    }
}

/// Run `erlang:adler32/1,2` or `erlang:crc32/1,2` as `adler32_1`,
/// `adler32_2`, `crc32_1`, and `crc32_2` do (`erl_bif_chksum.c:289-337`,
/// `369-417`). `initial` is `None` for the one-argument BIF, whose initial
/// sum is `adler32(0, NULL, 0)` (1) or `crc32(0, NULL, 0)` (0).
///
/// # Examples
///
/// ```rust,ignore
/// let result = checksum(&mut scheduler, Sum::Crc32, None, &Term::binary(b"abc".to_vec()))?;
/// assert!(matches!(result, BifResult::Value(Term::Integer(891568578))));
/// ```
pub(super) fn checksum(
    scheduler: &mut Scheduler,
    kind: Sum,
    initial: Option<&Term>,
    iolist: &Term,
) -> Result<BifResult, BeamError> {
    let mut sum = match (initial, kind) {
        (None, Sum::Adler32) => 1,
        (None, Sum::Crc32) => 0,
        (Some(initial), _) => match word(initial) {
            Some(sum) => sum,
            None => return Ok(BifResult::badarg()),
        },
    };
    let mut budget = CHUNK_PER_SCHEDULE;
    let walk = hash::read(iolist, &mut budget, &mut |bytes| {
        sum = match kind {
            Sum::Adler32 => adler32(sum, bytes),
            Sum::Crc32 => {
                let mut hasher = crc32fast::Hasher::new_with_initial(sum);
                hasher.update(bytes);
                hasher.finalize()
            }
        }
    });
    // `BUMP_REDS(BIF_P, res / BYTES_PER_REDUCTION)` before the `badarg`
    // check (`erl_bif_chksum.c:300-303`).
    hash::charge(
        scheduler,
        (CHUNK_PER_SCHEDULE - budget) / BYTES_PER_REDUCTION,
    );
    let Ok(rest) = walk else {
        return Ok(BifResult::badarg());
    };
    let Some(rest) = rest else {
        return Ok(Value(Term::Integer(i64::from(sum))));
    };
    // `BUMP_ALL_REDS` and a trap to the two-argument BIF
    // (`erl_bif_chksum.c:305-308`).
    scheduler.current_process_mut().bump_all_reductions();
    Ok(TrapBif {
        module: "erlang",
        function: match kind {
            Sum::Adler32 => "adler32",
            Sum::Crc32 => "crc32",
        },
        arguments: vec![Term::Integer(i64::from(sum)), rest],
    })
}

/// Run `erlang:adler32_combine/3` or `erlang:crc32_combine/3` as
/// `adler32_combine_3` and `crc32_combine_3` do (`erl_bif_chksum.c:339-367`,
/// `419-451`). Each argument is a 32-bit word. A length of 0 gives the first
/// sum for adler32 (`erl_bif_chksum.c:443-444`).
///
/// # Examples
///
/// ```rust,ignore
/// assert!(matches!(combine(Sum::Adler32, &Term::Integer(1), &Term::Integer(2), &Term::Integer(0)), BifResult::Value(Term::Integer(1))));
/// ```
pub(super) fn combine(kind: Sum, sum1: &Term, sum2: &Term, length: &Term) -> BifResult {
    let (Some(sum1), Some(sum2), Some(length)) = (word(sum1), word(sum2), word(length)) else {
        return BifResult::badarg();
    };
    let sum = match kind {
        Sum::Adler32 if length == 0 => sum1,
        Sum::Adler32 => adler32_combine(sum1, sum2, length),
        // zlib 1.2.12 and later `crc32_combine` multiplies `crc1` by x^0 for
        // a length of 0, so the value is `crc1 ^ crc2` (zlib 1.2.11 and
        // earlier return `crc1`). ERTS links the host zlib; both pinned
        // hosts have zlib 1.2.12 or later. `crc32fast` gives `crc1`.
        Sum::Crc32 if length == 0 => sum1 ^ sum2,
        Sum::Crc32 => {
            let mut first = crc32fast::Hasher::new_with_initial(sum1);
            first.combine(&crc32fast::Hasher::new_with_initial_len(
                sum2,
                u64::from(length),
            ));
            first.finalize()
        }
    };
    Value(Term::Integer(i64::from(sum)))
}

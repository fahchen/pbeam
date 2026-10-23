//! The random NIFs of `crypto` (`rand.c:26-155` of OTP 29.1.1):
//! `strong_rand_bytes_nif/1`, `strong_rand_range_nif/1`,
//! `rand_uniform_nif/2`, and `rand_seed_nif/1`. As ERTS, PBeam takes the
//! bytes from the OpenSSL RAND generator, through the safe `openssl` crate.

use super::*;
use openssl::bn::BigNum;
use openssl::error::ErrorStack;

/// Run `crypto:strong_rand_bytes_nif(Bytes)` (`rand.c:26-52`). A `Bytes`
/// that is not an unsigned C `int` raises `error:badarg`, and the result
/// is a binary of `Bytes` bytes of `RAND_bytes`. A failed `RAND_bytes`
/// gives `false`, which `crypto:strong_rand_bytes/1` changes to
/// `error:low_entropy`.
///
/// # Examples
///
/// ```rust,ignore
/// assert!(matches!(strong_rand_bytes_nif(&Term::Integer(16)), BifResult::Value(Term::Binary(_))));
/// assert!(matches!(strong_rand_bytes_nif(&Term::Integer(-1)), BifResult::Raise { .. }));
/// ```
pub(super) fn strong_rand_bytes_nif(size: &Term) -> BifResult {
    // `enif_get_uint` and the `INT_MAX` check: an integer in 0..=i32::MAX.
    let size = match *size {
        Term::Integer(size) => i32::try_from(size)
            .ok()
            .and_then(|size| usize::try_from(size).ok()),
        _ => None,
    };
    let Some(size) = size else {
        return BifResult::badarg();
    };
    let mut bytes = vec![0; size];
    match openssl::rand::rand_bytes(&mut bytes) {
        Ok(()) => BifResult::Value(Term::binary(bytes)),
        Err(_) => BifResult::Value(Term::static_atom("false")),
    }
}

/// Run `crypto:strong_rand_range_nif(Range)` (`rand.c:54-85`). `Range` is a
/// big-endian unsigned binary (`get_bn_from_bin`), and the result is the
/// value of `BN_rand_range` with no leading zero byte (`bin_from_bn`):
/// `<<>>` is the value 0. A range of 0 fails in `BN_rand_range` and gives
/// `false`. A term that is not a binary raises `error:badarg`.
///
/// # Examples
///
/// ```rust,ignore
/// assert!(matches!(strong_rand_range_nif(&Term::binary(vec![10])), BifResult::Value(Term::Binary(_))));
/// assert!(matches!(strong_rand_range_nif(&Term::binary(vec![0])), BifResult::Value(Term::Atom(_))));
/// ```
pub(super) fn strong_rand_range_nif(range: &Term) -> BifResult {
    let Some(range) = range.binary_bytes() else {
        return BifResult::badarg();
    };
    let value = BigNum::from_slice(range).and_then(|range| random_below(&range));
    match value {
        Ok(value) => BifResult::Value(Term::binary(value.to_vec())),
        Err(_) => BifResult::Value(Term::static_atom("false")),
    }
}

/// Return the value of `BN_rand_range(Range)`, which fails for a range
/// that is not above 0.
fn random_below(range: &BigNum) -> Result<BigNum, ErrorStack> {
    let mut value = BigNum::new()?;
    range.rand_range(&mut value)?;
    Ok(value)
}

/// Run `crypto:rand_uniform_nif(From, To)` (`rand.c:87-137`). `From` and
/// `To` are mpints (`get_bn_from_mpint`, `bn.c:26-51`): a 4-byte signed
/// big-endian length, then that many bytes of an unsigned number. The
/// result is the mpint of `From + BN_rand_range(To - From)`, with the
/// length of `BN_num_bytes` and no sign byte. A bad mpint and a failed
/// OpenSSL call (a `To` that is not above `From`) raise `error:badarg`.
///
/// # Examples
///
/// ```rust,ignore
/// let mpint = |byte| Term::binary(vec![0, 0, 0, 1, byte]);
/// assert!(matches!(rand_uniform_nif(&[mpint(1), mpint(9)]), BifResult::Value(Term::Binary(_))));
/// assert!(matches!(rand_uniform_nif(&[mpint(9), mpint(9)]), BifResult::Raise { .. }));
/// ```
pub(super) fn rand_uniform_nif(arguments: &[Term]) -> BifResult {
    rand_uniform(arguments).map_or_else(BifResult::badarg, |bytes| {
        BifResult::Value(Term::binary(bytes))
    })
}

/// Compute the result of `rand_uniform_nif/2` as `rand.c:96-122` does.
fn rand_uniform(arguments: &[Term]) -> Option<Vec<u8>> {
    let from = mpint(&arguments[0])?;
    let to = mpint(&arguments[1])?;
    let mut span = BigNum::new().ok()?;
    span.checked_sub(&to, &from).ok()?;
    let below = random_below(&span).ok()?;
    let mut value = BigNum::new().ok()?;
    value.checked_add(&below, &from).ok()?;
    let bytes = value.to_vec();
    let mut result = u32::try_from(bytes.len()).ok()?.to_be_bytes().to_vec();
    result.extend(bytes);
    Some(result)
}

/// Return the number of the mpint binary `term`, as `get_bn_from_mpint`
/// (`bn.c:26-51`) does: the length in the first 4 bytes, read as a signed
/// 32-bit integer, must be the number of the bytes that follow.
fn mpint(term: &Term) -> Option<BigNum> {
    let bytes = term.binary_bytes()?;
    let (length, number) = bytes.split_first_chunk::<4>()?;
    let length = usize::try_from(i32::from_be_bytes(*length)).ok()?;
    (length == number.len())
        .then(|| BigNum::from_slice(number).ok())
        .flatten()
}

/// Run `crypto:rand_seed_nif(Seed)` (`rand.c:139-155`): give `ok` for a
/// binary and raise `error:badarg` for another term. ERTS mixes the bytes
/// into the OpenSSL generator with `RAND_seed`. The safe `openssl` crate
/// has no `RAND_seed`, so PBeam does not mix the bytes in (row "NIF stubs"
/// of `docs/erts-differences.md`). OpenSSL seeds its generator from the
/// host, so the result of each later call keeps its shape.
///
/// # Examples
///
/// ```rust,ignore
/// assert!(matches!(rand_seed_nif(&Term::binary(vec![1])), BifResult::Value(Term::Atom(_))));
/// assert!(matches!(rand_seed_nif(&Term::Integer(1)), BifResult::Raise { .. }));
/// ```
pub(super) fn rand_seed_nif(seed: &Term) -> BifResult {
    match seed.binary_bytes() {
        Some(_) => BifResult::Value(Term::static_atom("ok")),
        None => BifResult::badarg(),
    }
}

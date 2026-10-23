//! The big number NIFs of `crypto`: `mod_exp_nif/4` (`bn.c:101-165`) and
//! the SRP NIFs `srp_value_B_nif/5`, `srp_user_secret_nif/7`, and
//! `srp_host_secret_nif/5` (`srp.c:26-300`) of OTP 29.1.1. The safe
//! `openssl` crate makes each OpenSSL call.

use super::*;
use openssl::bn::{BigNum, BigNumContext};
use openssl::error::ErrorStack;

/// Return the unsigned big-endian number of the binary `term`, as
/// `get_bn_from_bin` (`bn.c:53-78`) does.
fn number(term: &Term) -> Option<BigNum> {
    BigNum::from_slice(term.binary_bytes()?).ok()
}

/// Return the numbers of the binaries `terms`, or `None` when one of them
/// is not a binary.
fn numbers<const N: usize>(terms: &[Term]) -> Option<[BigNum; N]> {
    let numbers = terms[..N].iter().map(number).collect::<Option<Vec<_>>>()?;
    numbers.try_into().ok()
}

/// Run `mod_exp_nif/4` (`bn.c:101-165`): give `Base^Exponent mod Modulo`
/// as a plain binary (`BinHdr` 0) or as an mpint (`BinHdr` 4). Each failed
/// check and each failed OpenSSL call give `badarg` (`bn.c:147-149`).
pub(super) fn mod_exp_nif(arguments: &[Term]) -> BifResult {
    mod_exp(arguments).map_or_else(BifResult::badarg, |bytes| {
        BifResult::Value(Term::binary(bytes))
    })
}

/// Compute the result of `mod_exp_nif/4` as `bn.c:111-144` does.
fn mod_exp(arguments: &[Term]) -> Option<Vec<u8>> {
    let [base, exponent, modulo] = numbers::<3>(arguments)?;
    let header = match &arguments[3] {
        Term::Integer(0) => 0,
        Term::Integer(4) => 4,
        _ => return None,
    };
    let mut result = BigNum::new().ok()?;
    let mut context = BigNumContext::new().ok()?;
    result
        .mod_exp(&base, &exponent, &modulo, &mut context)
        .ok()?;
    let bytes = result.to_vec();
    if header == 0 {
        return Some(bytes);
    }
    // An mpint has a zero byte first when the high bit of the number is set
    // (`bn.c:133-142`).
    let extra = usize::from(bytes.first().is_some_and(|byte| byte & 0x80 != 0));
    let length = u32::try_from(bytes.len() + extra).ok()?;
    let mut mpint = length.to_be_bytes().to_vec();
    mpint.resize(4 + extra, 0);
    mpint.extend(bytes);
    Some(mpint)
}

/// Give the result of an SRP NIF: the bytes of the number, `badarg` for an
/// argument that is not a binary (`bad_arg`), or the atom `error` for a
/// failed OpenSSL call or a value that is 0 modulo the prime (`err`).
fn srp_result(result: Option<Result<Option<BigNum>, ErrorStack>>) -> BifResult {
    match result {
        None => BifResult::badarg(),
        Some(Ok(Some(number))) => BifResult::Value(Term::binary(number.to_vec())),
        Some(_) => BifResult::Value(Term::static_atom("error")),
    }
}

/// Return `true` when `value mod prime` is 0, as `BN_nnmod` and
/// `BN_is_zero` do in `srp.c:71-75`, `152-155`, and `258-261`.
fn zero_modulo(
    value: &BigNum,
    prime: &BigNum,
    context: &mut BigNumContext,
) -> Result<bool, ErrorStack> {
    let mut remainder = BigNum::new()?;
    remainder.nnmod(value, prime, context)?;
    Ok(remainder.num_bits() == 0)
}

/// Run `srp_value_B_nif/5` (`srp.c:26-111`): give
/// `B = (k * v + g^b) mod N` of `[k, v, g, b, N]`.
pub(super) fn srp_value_b_nif(arguments: &[Term]) -> BifResult {
    srp_result(numbers::<5>(arguments).map(
        |[multiplier, verifier, generator, mut exponent, prime]| {
            let mut context = BigNumContext::new()?;
            let mut product = BigNum::new()?;
            product.mod_mul(&multiplier, &verifier, &prime, &mut context)?;
            exponent.set_const_time();
            let mut power = BigNum::new()?;
            power.mod_exp(&generator, &exponent, &prime, &mut context)?;
            let mut result = BigNum::new()?;
            result.mod_add(&power, &product, &prime, &mut context)?;
            if zero_modulo(&result, &prime, &mut context)? {
                return Ok(None);
            }
            Ok(Some(result))
        },
    ))
}

/// Run `srp_user_secret_nif/7` (`srp.c:113-223`): give
/// `(B - k * g^x) ^ (a + u * x) mod N` of `[a, u, B, k, g, x, N]`.
pub(super) fn srp_user_secret_nif(arguments: &[Term]) -> BifResult {
    srp_result(
        numbers::<7>(arguments).map(|[a, u, b, multiplier, generator, mut x, prime]| {
            let mut context = BigNumContext::new()?;
            if zero_modulo(&b, &prime, &mut context)? {
                return Ok(None);
            }
            x.set_const_time();
            let mut power = BigNum::new()?;
            power.mod_exp(&generator, &x, &prime, &mut context)?;
            let mut product = BigNum::new()?;
            product.mod_mul(&multiplier, &power, &prime, &mut context)?;
            let mut base = BigNum::new()?;
            base.mod_sub(&b, &product, &prime, &mut context)?;
            let mut ux = BigNum::new()?;
            ux.checked_mul(&u, &x, &mut context)?;
            let mut exponent = BigNum::new()?;
            exponent.checked_add(&a, &ux)?;
            exponent.set_const_time();
            let mut result = BigNum::new()?;
            result.mod_exp(&base, &exponent, &prime, &mut context)?;
            Ok(Some(result))
        }),
    )
}

/// Run `srp_host_secret_nif/5` (`srp.c:225-313`): give
/// `(A * v^u) ^ b mod N` of `[v, b, u, A, N]`.
pub(super) fn srp_host_secret_nif(arguments: &[Term]) -> BifResult {
    srp_result(
        numbers::<5>(arguments).map(|[verifier, mut b, mut u, a, prime]| {
            let mut context = BigNumContext::new()?;
            if zero_modulo(&a, &prime, &mut context)? {
                return Ok(None);
            }
            u.set_const_time();
            let mut power = BigNum::new()?;
            power.mod_exp(&verifier, &u, &prime, &mut context)?;
            let mut base = BigNum::new()?;
            base.mod_mul(&a, &power, &prime, &mut context)?;
            b.set_const_time();
            let mut result = BigNum::new()?;
            result.mod_exp(&base, &b, &prime, &mut context)?;
            Ok(Some(result))
        }),
    )
}

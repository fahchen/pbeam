//! The key agreement NIFs of `crypto`: `dh_generate_key_nif/4` and
//! `dh_compute_key_nif/3` (`dh.c:44-271`), `ecdh_compute_key_nif/3`
//! (`ecdh.c:38-141`), and `evp_compute_key_nif/3` (`evp.c:131-218`) of
//! OTP 29.1.1 with the OpenSSL 3 API. The safe `openssl` crate makes each
//! OpenSSL call.
//!
//! A DH private key length, and an EC curve over a characteristic-two
//! field, give a runtime error (row "NIF stubs" of
//! `docs/erts-differences.md`).

use super::keygen::is_undefined;
use super::pkey::{Failure, badarg, ec_group, failed, finish_in};
use super::{BeamError, BifResult, Id, PKey, Scheduler, Term, atom_name};
use openssl::bn::{BigNum, BigNumContext, BigNumRef};
use openssl::derive::Deriver;
use openssl::dh::Dh;
use openssl::ec::{EcGroupRef, EcKey, EcPoint};

/// Run `dh_generate_key_nif/4` (`dh.c:44-179`): give `{Pub, Priv}` of a
/// new DH key of the group `[P, G]`.
///
/// # Errors
///
/// A runtime error for a private key length that is not 0.
pub(super) fn dh_generate_key_nif(
    scheduler: &mut Scheduler,
    arguments: &[Term],
) -> Result<BifResult, BeamError> {
    let result = dh_generate_key(scheduler, arguments)
        .map(|(public, private)| Term::tuple(vec![Term::binary(public), Term::binary(private)]));
    finish_in(scheduler, "dh.c", "dh_generate_key_nif", 4, result)
}

/// Give the numbers of the binaries `P` and `G` of the list `[P, G]`, or
/// the failure at `p_line` or `g_line` for the argument `argument`.
fn dh_group(
    list: &Term,
    argument: i64,
    (p_line, g_line): (i64, i64),
) -> Result<(BigNum, BigNum), Failure> {
    // `map_ensure_int_as_bin` gives the NIF a list of two elements
    // (`crypto.erl:3671`, `3816`), so the check of the empty tail
    // (`dh.c:82-83`, `211-212`) cannot fail.
    let (items, _) = list.list_elements().unwrap_or_default();
    let number = |index: usize| {
        items
            .get(index)
            .and_then(Term::binary_bytes)
            .and_then(|bytes| BigNum::from_slice(bytes).ok())
    };
    let p = number(0).ok_or(badarg(p_line, argument, "Bad value of 'p'"))?;
    let g = number(1).ok_or(badarg(g_line, argument, "Bad value of 'g'"))?;
    Ok((p, g))
}

/// Generate the DH key as `dh.c:61-167` does.
fn dh_generate_key(
    scheduler: &Scheduler,
    arguments: &[Term],
) -> Result<(Vec<u8>, Vec<u8>), Failure> {
    // ERTS checks that the private key is a binary (`dh.c:61-65`), but the
    // key generation of OpenSSL takes only the group of the template key
    // (`dh.c:121-136`), so the new key does not depend on it.
    if !is_undefined(scheduler, &arguments[0]) && arguments[0].binary_bytes().is_none() {
        return Err(badarg(63, 0, "PrivKeyIn"));
    }
    let (p, g) = dh_group(&arguments[1], 1, (72, 77))?;
    // `enif_get_uint64` and the limit `LONG_MAX` (`dh.c:93-97`).
    let length = match &arguments[3] {
        Term::Integer(length) if *length >= 0 => *length,
        _ => return Err(badarg(95, 3, "Bad value of length element")),
    };
    if length != 0 {
        // ERTS gives OpenSSL the parameter `priv_len` (`dh.c:98-103`),
        // which the `openssl` crate cannot set.
        return Err(Failure::Unsupported("a DH private key length".to_owned()));
    }
    let key = Dh::from_pqg(p, None, g)
        .and_then(Dh::generate_key)
        .map_err(|_| failed(134, "Can't generate DH key pair"))?;
    // `BN_bn2bin` of `BN_num_bytes` bytes (`dh.c:144-162`).
    Ok((key.public_key().to_vec(), key.private_key().to_vec()))
}

/// Run `dh_compute_key_nif/3` (`dh.c:182-271`): give the shared secret of
/// the peer public key and the own private key in the group `[P, G]`.
///
/// # Errors
///
/// None besides the exceptions of the checks.
pub(super) fn dh_compute_key_nif(
    scheduler: &mut Scheduler,
    arguments: &[Term],
) -> Result<BifResult, BeamError> {
    let result = dh_compute_key(arguments).map(Term::binary);
    finish_in(scheduler, "dh.c", "dh_compute_key_nif", 3, result)
}

/// Compute the DH secret as `dh.c:197-258` does.
fn dh_compute_key(arguments: &[Term]) -> Result<Vec<u8>, Failure> {
    let number = |term: &Term| {
        term.binary_bytes()
            .and_then(|bytes| BigNum::from_slice(bytes).ok())
    };
    let public =
        number(&arguments[0]).ok_or(badarg(198, 0, "Bad peer public key; binary expected"))?;
    let (p, g) = dh_group(&arguments[2], 2, (205, 208))?;
    let group = |p: &BigNum, g: &BigNum| {
        Dh::from_pqg(BigNumRef::to_owned(p)?, None, BigNumRef::to_owned(g)?)
    };
    let peer = group(&p, &g)
        .and_then(|dh| dh.set_public_key(BigNumRef::to_owned(&public)?))
        .and_then(PKey::from_dh)
        .map_err(|_| failed(222, "Can't do fromdata"))?;
    // ERTS checks the private key with the error of the public key
    // (`dh.c:225-226`).
    let private =
        number(&arguments[1]).ok_or(badarg(226, 0, "Bad peer public key; binary expected"))?;
    // ERTS builds the own key from the private key alone (`dh.c:225-234`);
    // the derivation reads only the private key of the own key.
    let own = group(&p, &g)
        .and_then(|dh| dh.set_key(public, private))
        .and_then(PKey::from_dh)
        .map_err(|_| failed(234, "Can't do fromdata"))?;
    let mut deriver = Deriver::new(&own).map_err(|_| failed(241, "Can't EVP_PKEY_derive_init"))?;
    // `EVP_PKEY_derive_set_peer` gives -1 for a public key that fails the
    // check, which the test `!` of ERTS takes as a success (`dh.c:243`).
    // The peer is then not set, and the size query fails (`dh.c:246-247`).
    let size = deriver
        .set_peer(&peer)
        .and_then(|()| deriver.len())
        .map_err(|_| failed(247, "Can't get result size"))?;
    let mut secret = vec![0; size];
    let size = deriver
        .derive(&mut secret)
        .map_err(|_| failed(254, "Can't get result"))?;
    secret.truncate(size);
    Ok(secret)
}

/// Run `ecdh_compute_key_nif/3` (`ecdh.c:38-141`): give the shared secret
/// of the peer public point and the own private key on the curve.
///
/// # Errors
///
/// A runtime error for a curve over a characteristic-two field.
pub(super) fn ecdh_compute_key_nif(
    scheduler: &mut Scheduler,
    arguments: &[Term],
) -> Result<BifResult, BeamError> {
    let result = ecdh_compute_key(scheduler, arguments).map(Term::binary);
    finish_in(scheduler, "ecdh.c", "ecdh_compute_key_nif", 3, result)
}

/// Give the failure of the first check of `get_curve_definition` that the
/// definition of a curve without a name fails (`ec.c:103-218`). The
/// failures name the file `ec.c`, which raises them.
fn curve_check(scheduler: &Scheduler, definition: &[Term]) -> Option<(i64, &'static str)> {
    let binary = |term: Option<&Term>| term.and_then(Term::binary_bytes).is_some();
    let prime = definition[1].tuple_elements().unwrap_or_default();
    let field = definition[0].tuple_elements();
    let checks = [
        (binary(definition.get(2)), 104, "Bad Generator (Point)"),
        (binary(definition.get(3)), 107, "Bad order"),
        (
            atom_name(scheduler, &definition[4]) != "none",
            110,
            "Cofactor must be != none",
        ),
        (binary(definition.get(4)), 113, "Bad cofactor"),
        // `nif_curve_params` gives a tuple `{A, B, Seed}`
        // (`crypto.erl:4725-4747`), so the check of `ec.c:116-117` cannot
        // fail, and a binary seed passes `ec.c:125-127`.
        (binary(prime.first()), 120, "Bad a"),
        (binary(prime.get(1)), 123, "Bad b"),
        (field.is_some(), 131, "Bad Field"),
    ];
    if let Some((_, line, message)) = checks.into_iter().find(|(passed, _, _)| !passed) {
        return Some((line, message));
    }
    match field.unwrap_or_default() {
        [tag, p] if atom_name(scheduler, tag) == "prime_field" => {
            (!binary(Some(p))).then_some((138, "Bad p (Prime)"))
        }
        // `ec_group` stops on a characteristic-two field.
        [tag, _, _] if atom_name(scheduler, tag) == "characteristic_two_field" => None,
        _ => Some((218, "Bad field-type")),
    }
}

/// Give a private key that the derivation of the `openssl` crate takes.
///
/// ERTS gives OpenSSL a private key longer than the order as it is
/// (`ecdh.c:91-100`), and the multiplication of the peer point takes it as
/// it is. The export of an `EcKey` to the provider of the derivation
/// rejects such a key. The private key modulo the number of points of the
/// curve, the order times the cofactor, gives the same multiple of each
/// point of the curve.
fn fit_private(
    group: &EcGroupRef,
    private: BigNum,
    context: &mut BigNumContext,
) -> Result<BigNum, Failure> {
    let fail = |_| failed(100, "Can't do fromdata");
    let mut order = BigNum::new().map_err(fail)?;
    group.order(&mut order, context).map_err(fail)?;
    if private.num_bytes() <= order.num_bytes() {
        return Ok(private);
    }
    let mut cofactor = BigNum::new().map_err(fail)?;
    group.cofactor(&mut cofactor, context).map_err(fail)?;
    let mut points = BigNum::new().map_err(fail)?;
    points
        .checked_mul(&order, &cofactor, context)
        .map_err(fail)?;
    let mut fitted = BigNum::new().map_err(fail)?;
    fitted.nnmod(&private, &points, context).map_err(fail)?;
    Ok(fitted)
}

/// Compute the ECDH secret as `ecdh.c:54-130` does.
fn ecdh_compute_key(scheduler: &Scheduler, arguments: &[Term]) -> Result<Vec<u8>, Failure> {
    let public =
        arguments[0]
            .binary_bytes()
            .ok_or(badarg(57, 0, "Bad peer public key; binary expected"))?;
    // `nif_curve_params` gives the NIF a tuple `{CurveDef, CurveName}`
    // (`crypto.erl:4725-4747`), so the check of `ec.c:78-79` cannot fail.
    // An unknown curve name gives `{undefined, Name}`.
    let parts = arguments[1].tuple_elements().unwrap_or_default();
    let definition = match parts.first().and_then(Term::tuple_elements) {
        Some(definition) if definition.len() == 5 => definition,
        _ => {
            return Err(Failure::CheckIn(
                "ec.c",
                "error",
                84,
                1,
                "Bad curve def. Expect 5-tuple.",
            ));
        }
    };
    // ERTS loads a named curve by name; PBeam builds the same curve from
    // its definition. ERTS checks the definition of a curve without a name
    // (`ec.c:86-101`), and of a named curve after a failed load of the
    // public key (`ecdh.c:74-79`); the definition of a named curve passes
    // each check.
    let named =
        matches!(parts.get(1), Some(name @ Term::Atom(_)) if !is_undefined(scheduler, name));
    if !named && let Some((line, message)) = curve_check(scheduler, definition) {
        return Err(Failure::CheckIn("ec.c", "error", line, 1, message));
    }
    let do_fromdata = || failed(80, "Can't do fromdata");
    let group = ec_group(scheduler, parts)?.ok_or_else(do_fromdata)?;
    let mut context = BigNumContext::new().map_err(|_| do_fromdata())?;
    let point = EcPoint::from_bytes(&group, public, &mut context).map_err(|_| do_fromdata())?;
    let peer = EcKey::from_public_key(&group, &point)
        .and_then(PKey::from_ec_key)
        .map_err(|_| do_fromdata())?;
    let private = arguments[2]
        .binary_bytes()
        .and_then(|bytes| BigNum::from_slice(bytes).ok())
        .ok_or(badarg(92, 0, "Bad peer public key; binary expected"))?;
    let private = fit_private(&group, private, &mut context)?;
    // ERTS builds the own key from the private key alone (`ecdh.c:91-100`);
    // the derivation reads only the private key of the own key.
    let own = EcKey::from_private_components(&group, &private, &point)
        .and_then(PKey::from_ec_key)
        .map_err(|_| failed(100, "Can't do fromdata"))?;
    let mut deriver = Deriver::new(&own).map_err(|_| failed(111, "Can't EVP_PKEY_derive_init"))?;
    // `EVP_PKEY_derive_set_peer_ex` with no check of the peer
    // (`ecdh.c:113-114`).
    deriver
        .set_peer_ex(&peer, false)
        .map_err(|_| failed(114, "Can't derive secret or set peer"))?;
    let size = deriver
        .len()
        .map_err(|_| failed(117, "Can't get result size"))?;
    let mut secret = vec![0; size];
    let size = deriver
        .derive(&mut secret)
        .map_err(|_| failed(124, "Can't get result"))?;
    secret.truncate(size);
    Ok(secret)
}

/// Run `evp_compute_key_nif/3` (`evp.c:131-218`): give the X25519 or X448
/// shared secret of the raw peer public key and the raw own private key.
///
/// # Errors
///
/// None besides the exceptions of the checks.
pub(super) fn evp_compute_key_nif(
    scheduler: &mut Scheduler,
    arguments: &[Term],
) -> Result<BifResult, BeamError> {
    let result = evp_compute_key(scheduler, arguments).map(Term::binary);
    finish_in(scheduler, "evp.c", "evp_compute_key_nif", 3, result)
}

/// Compute the secret as `evp.c:145-196` does.
fn evp_compute_key(scheduler: &Scheduler, arguments: &[Term]) -> Result<Vec<u8>, Failure> {
    // `compute_key/4` calls the NIF only with `x25519` or `x448`
    // (`crypto.erl:3853-3855`), so the check of `evp.c:152-153` cannot
    // fail.
    let id = if atom_name(scheduler, &arguments[0]) == "x25519" {
        Id::X25519
    } else {
        Id::X448
    };
    let mine = arguments[2]
        .binary_bytes()
        .ok_or(badarg(157, 2, "Binary expected"))?;
    // The peer key outlives the deriver that borrows it.
    let peer;
    let mine = PKey::private_key_from_raw_bytes(mine, id)
        .map_err(|_| badarg(160, 2, "Not a valid raw private key"))?;
    let mut deriver = Deriver::new(&mine).map_err(|_| failed(166, "Can't EVP_PKEY_derive_init"))?;
    let bytes = arguments[1]
        .binary_bytes()
        .ok_or(badarg(170, 1, "Binary expected"))?;
    peer = PKey::public_key_from_raw_bytes(bytes, id)
        .map_err(|_| badarg(173, 1, "Not a raw public peer key"))?;
    deriver
        .set_peer(&peer)
        .map_err(|_| Failure::Check("error", 176, 1, "Can't EVP_PKEY_derive_set_peer"))?;
    let size = deriver
        .len()
        .map_err(|_| Failure::Check("error", 180, 1, "Can't get max size"))?;
    let mut secret = vec![0; size];
    let size = deriver
        .derive(&mut secret)
        .map_err(|_| failed(189, "Can't EVP_PKEY_derive"))?;
    secret.truncate(size);
    Ok(secret)
}

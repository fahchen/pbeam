//! The key generation NIFs of `crypto`: `rsa_generate_key_nif/2`
//! (`rsa.c:453-543`), `ec_generate_key_nif/2` (`ec.c:353-517`), and
//! `evp_generate_key_nif/2` (`evp.c:221-300`) of OTP 29.1.1 with the
//! OpenSSL 3 API. The safe `openssl` crate makes each OpenSSL call.
//!
//! An EC curve over a characteristic-two field, and an EC curve that
//! PBeam cannot build from its definition when ERTS can load it by name,
//! give a runtime error (row "NIF stubs" of `docs/erts-differences.md`).

use super::pkey::{Failure, PKEY_TYPES, badarg, ec_group, failed, finish_in, pkey_type_id};
use super::{BeamError, BifResult, Id, PKey, Scheduler, Term, atom_name};
use openssl::bn::{BigNum, BigNumContext};
use openssl::ec::{EcKey, EcPoint, PointConversionForm};
use openssl::pkey_ctx::PkeyCtx;

/// Run `rsa_generate_key_nif/2` (`rsa.c:453-543`): generate an RSA key of
/// `ModulusSize` bits with the public exponent `E`, and give
/// `[E, N, D, P, Q, DMP1, DMQ1, IQMP]`.
///
/// # Errors
///
/// None besides the exceptions of the checks.
pub(super) fn rsa_generate_key_nif(
    scheduler: &mut Scheduler,
    arguments: &[Term],
) -> Result<BifResult, BeamError> {
    let result = rsa_generate_key(&arguments[0], &arguments[1])
        .map(|numbers| Term::list_from(numbers.into_iter().map(Term::binary)));
    finish_in(scheduler, "rsa.c", "rsa_generate_key_nif", 2, result)
}

/// Generate the RSA key as `rsa.c:462-527` does.
fn rsa_generate_key(bits: &Term, exponent: &Term) -> Result<Vec<Vec<u8>>, Failure> {
    let bits = match bits {
        Term::Integer(value) => u32::try_from(*value).ok(),
        _ => None,
    }
    .ok_or(badarg(463, 0, "Can't get unsigned int"))?;
    if bits < 256 {
        return Err(badarg(467, 0, "Can't be < 256"));
    }
    let exponent =
        exponent
            .binary_bytes()
            .ok_or(badarg(472, 1, "Can't get binary public exponent"))?;
    // `OSSL_PARAM_construct_BN` takes the bytes in the native order
    // (`rsa.c:485`), which is little-endian on each PBeam target.
    let mut big_endian = exponent.to_vec();
    big_endian.reverse();
    let exponent = BigNum::from_slice(&big_endian).map_err(|_| failed(489, "Can't set params"))?;
    let mut context =
        PkeyCtx::new_id(Id::RSA).map_err(|_| failed(480, "Can't init RSA generation"))?;
    context
        .keygen_init()
        .map_err(|_| failed(480, "Can't init RSA generation"))?;
    context
        .set_rsa_keygen_bits(bits)
        .map_err(|_| failed(489, "Can't set params"))?;
    // The crate rejects the exponent 0, which ERTS gives to
    // `EVP_PKEY_generate`, which then fails (`rsa.c:493-494`).
    context
        .set_rsa_keygen_pubexp(&exponent)
        .map_err(|_| failed(494, "Can't generate RSA key-pair"))?;
    let key = context
        .keygen()
        .map_err(|_| failed(494, "Can't generate RSA key-pair"))?;
    let rsa = key.rsa().map_err(|_| failed(524, "Can't get RSA keys"))?;
    let parts = [rsa.p(), rsa.q(), rsa.dmp1(), rsa.dmq1(), rsa.iqmp()];
    let mut numbers = vec![rsa.e().to_vec(), rsa.n().to_vec(), rsa.d().to_vec()];
    for part in parts {
        numbers.push(part.ok_or(failed(524, "Can't get RSA keys"))?.to_vec());
    }
    Ok(numbers)
}

/// Run `ec_generate_key_nif/2` (`ec.c:353-440`): give `{Pub, Priv}` of a
/// new key on the curve, or of the given private key, with the private
/// key padded to the size of the order.
///
/// # Errors
///
/// A runtime error for a curve that PBeam cannot build.
pub(super) fn ec_generate_key_nif(
    scheduler: &mut Scheduler,
    arguments: &[Term],
) -> Result<BifResult, BeamError> {
    let result = ec_generate_key(scheduler, &arguments[0], &arguments[1])
        .map(|(public, private)| Term::tuple(vec![Term::binary(public), Term::binary(private)]));
    finish_in(scheduler, "ec.c", "ec_generate_key_nif", 2, result)
}

/// Return whether the curve definition passes the checks of
/// `ec.c:103-139`: a binary generator, order, and cofactor, a tuple
/// `{A, B, Seed}` with binary `A` and `B`, and a field `{prime_field, P}`
/// with a binary `P`.
fn well_formed(scheduler: &Scheduler, definition: &[Term]) -> bool {
    let binary = |term: &Term| term.binary_bytes().is_some();
    let prime = matches!(&definition[1], Term::Tuple(prime)
        if prime.len() == 3 && binary(&prime[0]) && binary(&prime[1]));
    let field = matches!(&definition[0], Term::Tuple(field)
        if field.len() == 2 && atom_name(scheduler, &field[0]) == "prime_field" && binary(&field[1]));
    binary(&definition[2]) && binary(&definition[4]) && prime && field
}

/// Generate or compute the EC key as `ec.c:365-432` does.
fn ec_generate_key(
    scheduler: &Scheduler,
    curve: &Term,
    private: &Term,
) -> Result<(Vec<u8>, Vec<u8>), Failure> {
    let given = !is_undefined(scheduler, private);
    let private = if given {
        // `get_ec_private_key_2` (`ec.c:292-293`).
        Some(
            private
                .binary_bytes()
                .ok_or(badarg(293, 0, "Bad private key"))?,
        )
    } else {
        None
    };
    // `get_curve_definition` (`ec.c:78-100`); without a private key, the
    // failure is the one of `ec.c:391`.
    let definition_failed = |failure| {
        if given {
            failure
        } else {
            badarg(391, 0, "Couldn't get Curve definition")
        }
    };
    // `nif_curve_params` gives the NIF a tuple `{CurveDef, CurveName}`
    // (`crypto.erl:4725-4747`), so the check of `ec.c:78-79` cannot fail.
    let parts = curve.tuple_elements().unwrap_or_default();
    let (parts, definition, name) = match parts {
        [Term::Tuple(definition), name] if definition.len() == 5 => (parts, definition, name),
        _ => {
            return Err(definition_failed(Failure::Check(
                "error",
                84,
                1,
                "Bad curve def. Expect 5-tuple.",
            )));
        }
    };
    let named = matches!(name, Term::Atom(_)) && !is_undefined(scheduler, name);
    if given && !named {
        // ERTS gives the curve to OpenSSL by its parameters, and then needs
        // the name of the curve that OpenSSL matches (`ec.c:465-482`).
        return Err(Failure::Unsupported(
            "a private key on an EC curve without a name".to_owned(),
        ));
    }
    // ERTS checks only the order of a named curve (`ec.c:86-100`); each
    // check of a curve without a name gives the failure of `ec.c:391`.
    let Some(order) = definition[3].binary_bytes() else {
        return Err(definition_failed(Failure::Check(
            "error",
            95,
            1,
            "Bad order",
        )));
    };
    // The checks of `ec.c:103-139`; without a private key, each failure
    // gives the failure of `ec.c:391`.
    if !named && !well_formed(scheduler, definition) {
        return Err(badarg(391, 0, "Couldn't get Curve definition"));
    }
    // ERTS loads a named curve by name; PBeam builds the same curve from
    // its definition. OpenSSL rejects a well-formed definition at
    // `ec.c:405-407`.
    let group =
        ec_group(scheduler, parts)?.ok_or_else(|| failed(407, "Couldn't generate EC key"))?;
    let mut context = BigNumContext::new().map_err(|_| failed(492, "Couldn't multiply POINT"))?;
    let (point, private) = if let Some(bytes) = private {
        let private =
            BigNum::from_slice(bytes).map_err(|_| failed(492, "Couldn't multiply POINT"))?;
        // `EVP_PKEY_get_bn_param` of `priv` fails for a number longer
        // than the order (`ec.c:488-489`).
        let mut order_number = BigNum::new().map_err(|_| failed(492, "Couldn't multiply POINT"))?;
        group
            .order(&mut order_number, &mut context)
            .map_err(|_| failed(492, "Couldn't multiply POINT"))?;
        if private.num_bytes() > order_number.num_bytes() {
            return Err(badarg(489, 1, "Couldn't get peer priv key bytes"));
        }
        let mut point = EcPoint::new(&group).map_err(|_| failed(486, "Couldn't create POINT"))?;
        point
            .mul_generator(&group, &private, &context)
            .map_err(|_| failed(492, "Couldn't multiply POINT"))?;
        (point, private)
    } else {
        let key = EcKey::generate(&group).map_err(|_| failed(407, "Couldn't generate EC key"))?;
        let point = key
            .public_key()
            .to_owned(&group)
            .map_err(|_| failed(407, "Couldn't generate EC key"))?;
        let private = key
            .private_key()
            .to_owned()
            .map_err(|_| failed(407, "Couldn't generate EC key"))?;
        (point, private)
    };
    let public = point
        .to_bytes(&group, PointConversionForm::UNCOMPRESSED, &mut context)
        .map_err(|_| failed(500, "Couldn't get pub key bytes"))?;
    // `bn2term(order_size, priv)` (`ec.c:428-432`, `bn.c:166-195`).
    let size = i32::try_from(order.len()).map_err(|_| failed(492, "Couldn't multiply POINT"))?;
    let private = private
        .to_vec_padded(size)
        .map_err(|_| failed(492, "Couldn't multiply POINT"))?;
    Ok((public, private))
}

/// Run `evp_generate_key_nif/2` (`evp.c:221-300`): give the raw
/// `{Pub, Priv}` of a new key of the type, or of the given raw private
/// key.
///
/// # Errors
///
/// None besides the exceptions of the checks.
pub(super) fn evp_generate_key_nif(
    scheduler: &mut Scheduler,
    arguments: &[Term],
) -> Result<BifResult, BeamError> {
    let result = evp_generate_key(scheduler, &arguments[0], &arguments[1])
        .map(|(public, private)| Term::tuple(vec![Term::binary(public), Term::binary(private)]));
    finish_in(scheduler, "evp.c", "evp_generate_key_nif", 2, result)
}

/// Generate or load the raw key as `evp.c:232-290` does.
fn evp_generate_key(
    scheduler: &Scheduler,
    kind: &Term,
    private: &Term,
) -> Result<(Vec<u8>, Vec<u8>), Failure> {
    let name = atom_name(scheduler, kind);
    let id = match name {
        _ if PKEY_TYPES.contains(&name) => pkey_type_id(name),
        "x25519" => Id::X25519,
        "x448" => Id::X448,
        "ed25519" => Id::ED25519,
        "ed448" => Id::ED448,
        // The NIDs of `obj_mac.h` of OpenSSL 3.5.
        "mlkem512" => Id::from_raw(1454),
        "mlkem768" => Id::from_raw(1455),
        "mlkem1024" => Id::from_raw(1456),
        _ => return Err(badarg(259, 0, "Bad key type")),
    };
    let key = if is_undefined(scheduler, private) {
        let mut context = PkeyCtx::new_id(id).map_err(|_| failed(264, "Can't make context"))?;
        context
            .keygen_init()
            .map_err(|_| failed(266, "Can't EVP_PKEY_keygen_init"))?;
        context
            .keygen()
            .map_err(|_| failed(268, "Can't EVP_PKEY_keygen"))?
    } else {
        let bytes =
            private
                .binary_bytes()
                .ok_or(Failure::Check("error", 271, 1, "Can't get max size"))?;
        PKey::private_key_from_raw_bytes(bytes, id)
            .map_err(|_| Failure::Check("error", 273, 1, "Can't EVP_PKEY_new_raw_private_key"))?
    };
    let public = key
        .raw_public_key()
        .map_err(|_| failed(281, "Can't EVP_PKEY_get_raw_public_key"))?;
    let private = key
        .raw_private_key()
        .map_err(|_| failed(288, "Can't EVP_PKEY_get_raw_private_key"))?;
    Ok((public, private))
}

/// Return whether `term` is the atom `undefined`.
pub(super) fn is_undefined(scheduler: &Scheduler, term: &Term) -> bool {
    matches!(term, Term::Atom(_)) && atom_name(scheduler, term) == "undefined"
}

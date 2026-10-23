//! The KEM NIFs of `crypto`: `encapsulate_key_nif/2` and
//! `decapsulate_key_nif/3` (`evp.c:28-129`) of OTP 29.1.1.
//!
//! PBeam makes the checks of the arguments and builds the ML-KEM key as
//! `get_pkey_from_octet_string` (`pkey.c:587-665`) does. The safe
//! `openssl` crate 0.10.75 has no `EVP_PKEY_encapsulate` and no
//! `EVP_PKEY_decapsulate`, so a valid key then gives a runtime error (row
//! "NIF stubs" of `docs/erts-differences.md`).

use super::pkey::{Failure, finish_in, fromdata_failed, octet_string, octet_string_key};
use super::*;
use openssl::pkey::{Private, Public};

/// The names that `EVP_PKEY_CTX_new_from_name` loads ML-KEM from in
/// OpenSSL 3.5, with the NID of each (`obj_mac.h`).
const ML_KEM_NAMES: [(&str, i32); 12] = [
    ("2.16.840.1.101.3.4.4.1", 1454),
    ("id-alg-ml-kem-512", 1454),
    ("ML-KEM-512", 1454),
    ("MLKEM512", 1454),
    ("2.16.840.1.101.3.4.4.2", 1455),
    ("id-alg-ml-kem-768", 1455),
    ("ML-KEM-768", 1455),
    ("MLKEM768", 1455),
    ("2.16.840.1.101.3.4.4.3", 1456),
    ("id-alg-ml-kem-1024", 1456),
    ("ML-KEM-1024", 1456),
    ("MLKEM1024", 1456),
];

/// Return the key type of the ML-KEM name of the atom `name`, or the
/// failure of `pkey.c:610-617` for another name.
fn kem_id(scheduler: &Scheduler, name: &Term) -> Result<Id, Failure> {
    let algorithm = match name {
        Term::Atom(_) => atom_name(scheduler, name),
        _ => "",
    };
    ML_KEM_NAMES
        .iter()
        .find(|(known, _)| known.eq_ignore_ascii_case(algorithm))
        .map(|&(_, nid)| Id::from_raw(nid))
        .ok_or_else(|| octet_string_key(scheduler, algorithm, name, None))
}

/// Return the failure of the operation that the safe `openssl` crate does
/// not wrap.
fn unsupported(operation: &str) -> Failure {
    Failure::Unsupported(format!(
        "an ML-KEM key: the safe openssl crate has no {operation}"
    ))
}

/// Run `encapsulate_key_nif/2` (`evp.c:30-76`) up to the encapsulation.
/// Each check that fails is in `pkey.c`.
///
/// # Errors
///
/// A runtime error for a valid key.
pub(super) fn encapsulate_key_nif(
    scheduler: &mut Scheduler,
    arguments: &[Term],
) -> Result<BifResult, BeamError> {
    let result = public_key(scheduler, &arguments[0], &arguments[1])
        .and_then(|_| Err(unsupported("EVP_PKEY_encapsulate")));
    finish_in(scheduler, "pkey.c", "encapsulate_key_nif", 2, result)
}

/// Run `decapsulate_key_nif/3` (`evp.c:80-129`) up to the decapsulation.
///
/// # Errors
///
/// A runtime error for a valid key.
pub(super) fn decapsulate_key_nif(
    scheduler: &mut Scheduler,
    arguments: &[Term],
) -> Result<BifResult, BeamError> {
    if arguments[2].binary_bytes().is_none() {
        return c_error(
            scheduler,
            "error",
            "evp.c",
            92,
            2,
            "Invalid encapsulated secret",
        );
    }
    let result = private_key(scheduler, &arguments[0], &arguments[1])
        .and_then(|_| Err(unsupported("EVP_PKEY_decapsulate")));
    finish_in(scheduler, "pkey.c", "decapsulate_key_nif", 3, result)
}

/// Build the public key of `get_pkey_from_octet_string` with `PKEY_PUB`.
fn public_key(scheduler: &Scheduler, name: &Term, key: &Term) -> Result<PKey<Public>, Failure> {
    let id = kem_id(scheduler, name)?;
    PKey::public_key_from_raw_bytes(octet_string(key)?, id).map_err(|_| fromdata_failed())
}

/// Build the private key of `get_pkey_from_octet_string` with `PKEY_PRIV`.
fn private_key(scheduler: &Scheduler, name: &Term, key: &Term) -> Result<PKey<Private>, Failure> {
    let id = kem_id(scheduler, name)?;
    PKey::private_key_from_raw_bytes(octet_string(key)?, id).map_err(|_| fromdata_failed())
}

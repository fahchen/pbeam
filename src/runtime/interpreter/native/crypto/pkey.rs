//! The sign, verify, encrypt, and decrypt NIFs of `crypto`:
//! `pkey_sign_nif/5`, `pkey_sign_heavy_nif/5`, `pkey_verify_nif/6`, and
//! `pkey_crypt_nif/6` (`pkey.c:212-1581` of OTP 29.1.1, with the OpenSSL 3
//! API and engine support). The safe
//! `openssl` crate makes each OpenSSL call.
//!
//! PBeam signs with the algorithms `rsa`, `dss`, `ecdsa`, `eddsa`, and the
//! ML-DSA and SLH-DSA algorithms of `pkey_types`. Each other algorithm that
//! ERTS gives to `EVP_PKEY_CTX_new_from_name`, and the ECDSA curves over a
//! characteristic-two field give a runtime error (row "NIF stubs" of
//! `docs/erts-differences.md`).

use super::{
    BeamError, BifResult, Hash, Hasher, Md, PKey, Scheduler, Term, atom_name, c_error, error,
    iodata_bytes,
};
use openssl::bn::{BigNum, BigNumContext};
use openssl::dsa::Dsa;
use openssl::ec::{EcGroup, EcKey, EcPoint};
use openssl::error::ErrorStack;
use openssl::pkey::{Id, Private, Public};
use openssl::pkey_ctx::PkeyCtx;
use openssl::rsa::{Padding, Rsa, RsaPrivateKeyBuilder};
use openssl::sign::{RsaPssSaltlen, Signer, Verifier};

/// The algorithms of `pkey_types` (`pkey.c:91-172`), for which the digest
/// type `none` gives no digest (`pkey.c:263-270`).
pub(super) const PKEY_TYPES: [&str; 15] = [
    "mldsa44",
    "mldsa65",
    "mldsa87",
    "slh_dsa_shake_128s",
    "slh_dsa_shake_128f",
    "slh_dsa_sha2_128s",
    "slh_dsa_sha2_128f",
    "slh_dsa_shake_192s",
    "slh_dsa_shake_192f",
    "slh_dsa_sha2_192s",
    "slh_dsa_sha2_192f",
    "slh_dsa_shake_256s",
    "slh_dsa_shake_256f",
    "slh_dsa_sha2_256s",
    "slh_dsa_sha2_256f",
];

/// The names and aliases of the key managers of the OpenSSL 3.5 default
/// provider (`providers/defltprov.c` of the vendored 3.5.5) and 5 aliases of
/// the OBJ database, which
/// `EVP_PKEY_CTX_new_from_name` matches without case (`pkey.c:614`).
const OPENSSL_KEY_NAMES: [&str; 107] = [
    "1.2.840.113549.1.1.1",
    "2.5.8.1.1",
    "RSA",
    "rsaEncryption",
    "1.2.840.113549.1.1.10",
    "RSA-PSS",
    "RSASSA-PSS",
    "rsassaPss",
    "1.2.840.113549.1.3.1",
    "DH",
    "dhKeyAgreement",
    "1.2.840.10046.2.1",
    "dhpublicnumber",
    "DHX",
    "X9.42 DH",
    "1.2.840.10040.4.1",
    "1.3.14.3.2.12",
    "DSA",
    "DSA-old",
    "dsaEncryption",
    "dsaEncryption-old",
    "1.2.840.10045.2.1",
    "EC",
    "id-ecPublicKey",
    "1.3.101.110",
    "X25519",
    "1.3.101.111",
    "X448",
    "1.3.101.112",
    "ED25519",
    "1.3.101.113",
    "ED448",
    "1.2.156.10197.1.301",
    "SM2",
    "TLS1-PRF",
    "HKDF",
    "1.3.6.1.4.1.11591.4.11",
    "id-scrypt",
    "SCRYPT",
    "HMAC",
    "SIPHASH",
    "POLY1305",
    "CMAC",
    "X25519MLKEM768",
    "X448MLKEM1024",
    "SecP256r1MLKEM768",
    "SecP384r1MLKEM1024",
    "2.16.840.1.101.3.4.4.1",
    "id-alg-ml-kem-512",
    "ML-KEM-512",
    "MLKEM512",
    "2.16.840.1.101.3.4.4.2",
    "id-alg-ml-kem-768",
    "ML-KEM-768",
    "MLKEM768",
    "2.16.840.1.101.3.4.4.3",
    "id-alg-ml-kem-1024",
    "ML-KEM-1024",
    "MLKEM1024",
    "2.16.840.1.101.3.4.3.17",
    "id-ml-dsa-44",
    "ML-DSA-44",
    "MLDSA44",
    "2.16.840.1.101.3.4.3.18",
    "id-ml-dsa-65",
    "ML-DSA-65",
    "MLDSA65",
    "2.16.840.1.101.3.4.3.19",
    "id-ml-dsa-87",
    "ML-DSA-87",
    "MLDSA87",
    "2.16.840.1.101.3.4.3.20",
    "id-slh-dsa-sha2-128s",
    "SLH-DSA-SHA2-128s",
    "2.16.840.1.101.3.4.3.21",
    "id-slh-dsa-sha2-128f",
    "SLH-DSA-SHA2-128f",
    "2.16.840.1.101.3.4.3.22",
    "id-slh-dsa-sha2-192s",
    "SLH-DSA-SHA2-192s",
    "2.16.840.1.101.3.4.3.23",
    "id-slh-dsa-sha2-192f",
    "SLH-DSA-SHA2-192f",
    "2.16.840.1.101.3.4.3.24",
    "id-slh-dsa-sha2-256s",
    "SLH-DSA-SHA2-256s",
    "2.16.840.1.101.3.4.3.25",
    "id-slh-dsa-sha2-256f",
    "SLH-DSA-SHA2-256f",
    "2.16.840.1.101.3.4.3.26",
    "id-slh-dsa-shake-128s",
    "SLH-DSA-SHAKE-128s",
    "2.16.840.1.101.3.4.3.27",
    "id-slh-dsa-shake-128f",
    "SLH-DSA-SHAKE-128f",
    "2.16.840.1.101.3.4.3.28",
    "id-slh-dsa-shake-192s",
    "SLH-DSA-SHAKE-192s",
    "2.16.840.1.101.3.4.3.29",
    "id-slh-dsa-shake-192f",
    "SLH-DSA-SHAKE-192f",
    "2.16.840.1.101.3.4.3.30",
    "id-slh-dsa-shake-256s",
    "SLH-DSA-SHAKE-256s",
    "2.16.840.1.101.3.4.3.31",
    "id-slh-dsa-shake-256f",
    "SLH-DSA-SHAKE-256f",
];

/// The OpenSSL padding values of the option `rsa_padding`
/// (`pkey.c:437-458`).
const RSA_PKCS1_PADDING: i32 = 1;
const RSA_NO_PADDING: i32 = 3;
const RSA_X931_PADDING: i32 = 5;
const RSA_PKCS1_PSS_PADDING: i32 = 6;

/// The cause of a failed sign or verify.
pub(super) enum Failure {
    /// The exception `{Tag, #{c_file_name => "pkey.c", ...}, Message}` of
    /// the check at a line of `pkey.c`.
    Check(&'static str, i64, i64, &'static str),
    /// The exception of a check in a helper of another C file, such as the
    /// curve checks of `ec.c` that `ecdh.c` calls: the file, then the
    /// fields of [`Failure::Check`].
    CheckIn(&'static str, &'static str, i64, i64, &'static str),
    /// A part that PBeam does not implement, which gives a runtime error.
    Unsupported(String),
}

/// Return the [`Failure::Check`] of the `badarg` check at `line` for the
/// argument `argument`.
pub(super) fn badarg(line: i64, argument: i64, message: &'static str) -> Failure {
    Failure::Check("badarg", line, argument, message)
}

/// Return the [`Failure::Check`] of the `error` check at `line`, which
/// names no argument (`EXCP_ERROR`).
pub(super) fn failed(line: i64, message: &'static str) -> Failure {
    Failure::Check("error", line, -1, message)
}

/// The values of the options of a sign or verify (`PKeySignOptions`).
struct SignOptions {
    padding: i32,
    mgf1: Option<Hash>,
    salt_length: i32,
}

/// The input of a sign or verify after the checks of the digest, the data,
/// and the options.
struct Input {
    algorithm: String,
    digest: Option<Hash>,
    data: Vec<u8>,
    options: SignOptions,
}

/// Run `pkey_sign_nif/5` (`pkey.c:762-904`) as the NIF `name`: sign the
/// data or the digest with the private key, and give the signature.
///
/// # Errors
///
/// A runtime error for an algorithm or a curve that PBeam does not
/// implement.
pub(super) fn pkey_sign_nif(
    scheduler: &mut Scheduler,
    name: &str,
    arguments: &[Term],
) -> Result<BifResult, BeamError> {
    let result = sign(scheduler, arguments).map(Term::binary);
    finish(scheduler, name, 5, result)
}

/// Run `pkey_verify_nif/6` (`pkey.c:1004-1122`): give `true` when the
/// signature of the data or the digest is valid for the public key, else
/// `false`.
///
/// # Errors
///
/// A runtime error for an algorithm or a curve that PBeam does not
/// implement.
pub(super) fn pkey_verify_nif(
    scheduler: &mut Scheduler,
    arguments: &[Term],
) -> Result<BifResult, BeamError> {
    let result = verify(scheduler, arguments)
        .map(|valid| Term::static_atom(if valid { "true" } else { "false" }));
    finish(scheduler, "pkey_verify_nif", 6, result)
}

/// Give the value of `result`, or the exception or runtime error of its
/// failure.
fn finish(
    scheduler: &mut Scheduler,
    name: &str,
    arity: u32,
    result: Result<Term, Failure>,
) -> Result<BifResult, BeamError> {
    finish_in(scheduler, "pkey.c", name, arity, result)
}

/// Give the value of `result`, or the exception of its failed check at a
/// line of the C file `file`, or the runtime error of its failure.
pub(super) fn finish_in(
    scheduler: &mut Scheduler,
    file: &str,
    name: &str,
    arity: u32,
    result: Result<Term, Failure>,
) -> Result<BifResult, BeamError> {
    match result {
        Ok(value) => Ok(BifResult::Value(value)),
        Err(Failure::Check(tag, line, argument, message)) => {
            c_error(scheduler, tag, file, line, argument, message)
        }
        Err(Failure::CheckIn(file, tag, line, argument, message)) => {
            c_error(scheduler, tag, file, line, argument, message)
        }
        Err(Failure::Unsupported(part)) => Err(error(format!(
            "crypto:{name}/{arity} NIF is not supported for {part}"
        ))),
    }
}

/// Sign as `pkey_sign_nif` does with the OpenSSL 3 API (`pkey.c:789-904`).
fn sign(scheduler: &Scheduler, arguments: &[Term]) -> Result<Vec<u8>, Failure> {
    let input = sign_input(scheduler, arguments, 4)?;
    let pkey = private_key(scheduler, &input.algorithm, &arguments[0], &arguments[3], 3)?;
    if input.algorithm == "eddsa" {
        let mut signer = Signer::new_without_digest(&pkey)
            .map_err(|_| failed(811, "Can't EVP_DigestSignInit"))?;
        return signer
            .sign_oneshot_to_vec(&input.data)
            .map_err(|_| failed(813, "Can't EVP_DigestSign"));
    }
    if PKEY_TYPES.contains(&input.algorithm.as_str()) {
        // `EVP_DigestSignInit` without a digest starts the same message
        // signature as `EVP_PKEY_sign_message_init` with the algorithm name
        // and no parameters (`pkey.c:835-837`). ERTS then sets the digest
        // (`pkey.c:847-850`), which ML-DSA and SLH-DSA do not read.
        let mut signer = Signer::new_without_digest(&pkey)
            .map_err(|_| failed(837, "Can't EVP_PKEY_sign_message_init"))?;
        return signer
            .sign_oneshot_to_vec(&input.data)
            .map_err(|_| failed(874, "Can't EVP_PKEY_sign"));
    }
    let mut context =
        PkeyCtx::new(&pkey).map_err(|_| failed(832, "Can't allocate new EVP_PKEY_CTX"))?;
    context
        .sign_init()
        .map_err(|_| failed(843, "Can't EVP_PKEY_sign_init"))?;
    configure(&mut context, &input, (849, 854, 860, 867), 4)?;
    let length = context
        .sign(&input.data, None)
        .map_err(|_| failed(874, "Can't EVP_PKEY_sign"))?;
    let mut signature = vec![0; length];
    let length = context
        .sign(&input.data, Some(&mut signature))
        .map_err(|_| failed(884, "Can't EVP_PKEY_sign"))?;
    signature.truncate(length);
    Ok(signature)
}

/// Verify as `pkey_verify_nif` does with the OpenSSL 3 API
/// (`pkey.c:1029-1122`).
fn verify(scheduler: &Scheduler, arguments: &[Term]) -> Result<bool, Failure> {
    let input = sign_input(scheduler, arguments, 5)?;
    let signature = arguments[3]
        .binary_bytes()
        .ok_or_else(|| badarg(1041, 3, "Expected a binary"))?;
    let pkey = public_key(scheduler, &input.algorithm, &arguments[0], &arguments[4], 4)?;
    if input.algorithm == "eddsa" {
        let mut verifier = Verifier::new_without_digest(&pkey)
            .map_err(|_| failed(1055, "Can't EVP_DigestVerifyInit"))?;
        return Ok(matches!(
            verifier.verify_oneshot(signature, &input.data),
            Ok(true)
        ));
    }
    if PKEY_TYPES.contains(&input.algorithm.as_str()) {
        // As in `sign`, for `EVP_PKEY_verify_message_init`
        // (`pkey.c:1070-1072`).
        let mut verifier = Verifier::new_without_digest(&pkey)
            .map_err(|_| failed(1072, "Can't EVP_PKEY_verify_message_init"))?;
        return Ok(matches!(
            verifier.verify_oneshot(signature, &input.data),
            Ok(true)
        ));
    }
    let mut context =
        PkeyCtx::new(&pkey).map_err(|_| failed(1067, "Can't allocate new EVP_PKEY_CTX"))?;
    context
        .verify_init()
        .map_err(|_| failed(1078, "Can't EVP_PKEY_verify_init"))?;
    configure(&mut context, &input, (1083, 1088, 1094, 1101), 5)?;
    Ok(matches!(context.verify(&input.data, signature), Ok(true)))
}

/// Set the digest and the RSA options of `context`, as `pkey.c:847-871`
/// and `pkey.c:1081-1105` do. `lines` gives the line of each failed call,
/// and `options` the argument of the options.
fn configure<T>(
    context: &mut PkeyCtx<T>,
    input: &Input,
    lines: (i64, i64, i64, i64),
    options: i64,
) -> Result<(), Failure> {
    if let Some(digest) = input.digest {
        context
            .set_signature_md(&fetch(digest))
            .map_err(|_| failed(lines.0, "Can't EVP_PKEY_CTX_set_signature_md"))?;
    }
    if input.algorithm != "rsa" {
        return Ok(());
    }
    let options_of = &input.options;
    context
        .set_rsa_padding(Padding::from_raw(options_of.padding))
        .map_err(|_| failed(lines.1, "Can't EVP_PKEY_CTX_set_rsa_padding"))?;
    if options_of.padding != RSA_PKCS1_PSS_PADDING {
        return Ok(());
    }
    if let Some(mgf1) = options_of.mgf1 {
        context
            .set_rsa_mgf1_md(&fetch(mgf1))
            .map_err(|_| failed(lines.2, "Can't EVP_PKEY_CTX_set_rsa_mgf1_md"))?;
    }
    if options_of.salt_length > -2 {
        context
            .set_rsa_pss_saltlen(RsaPssSaltlen::custom(options_of.salt_length))
            .map_err(|_| badarg(lines.3, options, "Bad rsa_pss_saltlen"))?;
    }
    Ok(())
}

/// Check the algorithm, the digest type, the data, and the options of a
/// sign or verify, as `get_pkey_sign_digest` (`pkey.c:293-385`) and
/// `get_pkey_sign_options` (`pkey.c:387-474`) do. `options` is the
/// argument of the options.
fn sign_input(scheduler: &Scheduler, arguments: &[Term], options: usize) -> Result<Input, Failure> {
    let algorithm = atom_name(scheduler, &arguments[0]).to_owned();
    let digest = digest_type(scheduler, &algorithm, &arguments[1], 1)?;
    let data = match &arguments[2] {
        Term::Tuple(items) => {
            let [tag, value] = &items[..] else {
                return Err(badarg(329, 2, "Bad list"));
            };
            if atom_name(scheduler, tag) != "digest" || !matches!(tag, Term::Atom(_)) {
                return Err(badarg(331, 2, "Expected 'digest' as head"));
            }
            let value =
                iodata_bytes(value).ok_or_else(|| badarg(333, 2, "Bad 2nd element in list"))?;
            if digest.is_some_and(|hash| fetch(hash).size() != value.len()) {
                return Err(badarg(338, 2, "Bad binary size for the algorithm"));
            }
            value
        }
        data => {
            let data = iodata_bytes(data);
            match &digest {
                None => data.ok_or_else(|| badarg(346, 2, "Expected a binary or a list"))?,
                Some(hash) => {
                    let data = data.ok_or_else(|| badarg(353, 2, "Expected a binary or a list"))?;
                    let mut hasher =
                        Hasher::new(hash.message_digest()).expect("OpenSSL starts each digest");
                    hasher.update(&data).expect("OpenSSL updates each digest");
                    hasher
                        .finish()
                        .map_err(|_| failed(367, "Can't create EVP_DigestFinal_ex"))?
                        .to_vec()
                }
            }
        }
    };
    let options = sign_options(scheduler, &algorithm, digest, &arguments[options], options)?;
    Ok(Input {
        algorithm,
        digest,
        data,
        options,
    })
}

/// Return the digest of the digest type `term` for `algorithm`, as
/// `get_pkey_digest_type` (`pkey.c:253-291`): `None` for no digest. The
/// errors name the argument `argument`.
fn digest_type(
    scheduler: &Scheduler,
    algorithm: &str,
    term: &Term,
    argument: i64,
) -> Result<Option<Hash>, Failure> {
    let name = atom_name(scheduler, term);
    let atom = matches!(term, Term::Atom(_));
    if atom && name == "none" && (algorithm == "rsa" || PKEY_TYPES.contains(&algorithm)) {
        return Ok(None);
    }
    if algorithm == "eddsa" {
        return Ok(None);
    }
    match Hash::from_name(name).filter(|_| atom) {
        Some(hash) => Ok(Some(hash)),
        // PBeam links OpenSSL without the legacy provider, so md4 has no
        // digest (digest.c:186-188).
        None if atom && name == "md4" => Err(badarg(284, argument, "Digest type not supported")),
        None => Err(badarg(278, argument, "Bad digest type")),
    }
}

/// Return the options of a sign or verify, as `get_pkey_sign_options`
/// (`pkey.c:387-474`) gives them. `argument` is the argument of the
/// options.
fn sign_options(
    scheduler: &Scheduler,
    algorithm: &str,
    digest: Option<Hash>,
    term: &Term,
    argument: usize,
) -> Result<SignOptions, Failure> {
    let argument = argument as i64;
    if !matches!(term, Term::Nil) && term.cons_cell().is_none() {
        return Err(badarg(400, argument, "Expected a list"));
    }
    let rsa = algorithm == "rsa";
    let mut options = SignOptions {
        padding: if rsa { RSA_PKCS1_PADDING } else { 0 },
        mgf1: None,
        salt_length: if rsa { -2 } else { 0 },
    };
    if matches!(term, Term::Nil) {
        return Ok(options);
    }
    if !rsa {
        return Err(badarg(417, argument, "Only RSA supports Options"));
    }
    let mut tail = term;
    while let Some((head, rest)) = tail.cons_cell() {
        tail = rest;
        let pair = match head {
            Term::Tuple(items) if items.len() == 2 => items,
            _ => return Err(badarg(423, argument, "Expects only two-tuples in the list")),
        };
        let value = &pair[1];
        match (
            matches!(pair[0], Term::Atom(_)),
            atom_name(scheduler, &pair[0]),
        ) {
            (true, "rsa_mgf1_md") => {
                if !matches!(value, Term::Atom(_)) {
                    return Err(badarg(
                        427,
                        argument,
                        "Atom expected as argument to option rsa_mgf1_md",
                    ));
                }
                options.mgf1 = digest_type(scheduler, algorithm, value, argument)?;
            }
            (true, "rsa_padding") => {
                options.padding = match atom_name(scheduler, value) {
                    "rsa_pkcs1_padding" => RSA_PKCS1_PADDING,
                    "rsa_pkcs1_pss_padding" => {
                        if options.mgf1.is_none() {
                            options.mgf1 = digest;
                        }
                        RSA_PKCS1_PSS_PADDING
                    }
                    "rsa_x931_padding" => RSA_X931_PADDING,
                    "rsa_no_padding" => RSA_NO_PADDING,
                    _ => return Err(badarg(457, argument, "Bad value in option rsa_padding")),
                };
            }
            (true, "rsa_pss_saltlen") => {
                options.salt_length = match value {
                    Term::Integer(length) => i32::try_from(*length).ok(),
                    _ => None,
                }
                .filter(|length| *length >= -2)
                .ok_or_else(|| badarg(463, argument, "Bad value in option rsa_pss_saltlen"))?;
            }
            _ => return Err(badarg(466, argument, "Bad option")),
        }
    }
    Ok(options)
}

/// Return the private key `term` of `algorithm`, as `get_pkey_private_key`
/// (`pkey.c:476-584`) gives it. The errors name the argument `argument`.
fn private_key(
    scheduler: &Scheduler,
    algorithm: &str,
    name: &Term,
    term: &Term,
    argument: i64,
) -> Result<PKey<Private>, Failure> {
    // PBeam has no engine resource, so each map fails
    // `get_engine_and_key_id` (engine.c:62-82).
    if matches!(term, Term::Map(_)) {
        return Err(badarg(494, argument, "Couldn't get engine and/or key id"));
    }
    let (key, line, message) = match algorithm {
        "rsa" => (rsa_private_key(term), 506, "Couldn't get RSA private key"),
        "ecdsa" => {
            let key = ec_parts(scheduler, term)?.and_then(|(group, key)| {
                let value = BigNum::from_slice(key).ok()?;
                let mut point = EcPoint::new(&group).ok()?;
                let context = BigNumContext::new().ok()?;
                point.mul_generator(&group, &value, &context).ok()?;
                let ec = EcKey::from_private_components(&group, &value, &point).ok()?;
                PKey::from_ec_key(ec).ok()
            });
            (key, 511, "Couldn't get ECDSA private key")
        }
        "eddsa" => {
            let key = eddsa_parts(scheduler, term)
                .and_then(|(bytes, id)| PKey::private_key_from_raw_bytes(bytes, id).ok());
            (key, 520, "Couldn't get EDDSA private key")
        }
        "dss" => {
            let key = dss_parts(term).and_then(|[p, q, g, value]| {
                // ERTS gives no public value with a private key (dss.c:30-68);
                // PBeam gives `G^Priv mod P`, or 0 for a modulus of 0, which a
                // signature does not read.
                let mut public = BigNum::new().ok()?;
                let mut context = BigNumContext::new().ok()?;
                let _ = public.mod_exp(&g, &value, &p, &mut context);
                PKey::from_dsa(Dsa::from_private_components(p, q, g, value, public).ok()?).ok()
            });
            (key, 530, "Couldn't get DSA private key")
        }
        _ if PKEY_TYPES.contains(&algorithm) => {
            return pkey_type_private_key(scheduler, algorithm, term, argument);
        }
        _ => return Err(octet_string_key(scheduler, algorithm, name, Some(term))),
    };
    key.ok_or_else(|| badarg(line, argument, message))
}

/// Return the public key `term` of `algorithm`, as `get_pkey_public_key`
/// (`pkey.c:669-754`) gives it. The errors name the argument `argument`.
fn public_key(
    scheduler: &Scheduler,
    algorithm: &str,
    name: &Term,
    term: &Term,
    argument: i64,
) -> Result<PKey<Public>, Failure> {
    if matches!(term, Term::Map(_)) {
        return Err(badarg(686, argument, "Couldn't get engine and/or key id"));
    }
    let (key, line, message) = match algorithm {
        "rsa" => (rsa_public_key(term), 698, "Couldn't get RSA public key"),
        "ecdsa" => {
            let key = ec_parts(scheduler, term)?.and_then(|(group, key)| {
                let mut context = BigNumContext::new().ok()?;
                let point = EcPoint::from_bytes(&group, key, &mut context).ok()?;
                PKey::from_ec_key(EcKey::from_public_key(&group, &point).ok()?).ok()
            });
            (key, 703, "Couldn't get ECDSA public key")
        }
        "eddsa" => {
            let key = eddsa_parts(scheduler, term)
                .and_then(|(bytes, id)| PKey::public_key_from_raw_bytes(bytes, id).ok());
            (key, 712, "Couldn't get EDDSA public key")
        }
        "dss" => {
            let key = dss_parts(term).and_then(|[p, q, g, value]| {
                PKey::from_dsa(Dsa::from_public_components(p, q, g, value).ok()?).ok()
            });
            (key, 722, "Couldn't get DSA public key")
        }
        _ if PKEY_TYPES.contains(&algorithm) => {
            let bytes = octet_string(term)?;
            return PKey::public_key_from_raw_bytes(bytes, pkey_type_id(algorithm))
                .map_err(|_| fromdata_failed());
        }
        _ => return Err(octet_string_key(scheduler, algorithm, name, None)),
    };
    key.ok_or_else(|| badarg(line, argument, message))
}

/// Return the failure of the key of an algorithm outside `rsa`, `ecdsa`,
/// `eddsa`, `dss`, and `pkey_types`, which ERTS gives to
/// `get_pkey_from_octet_string` (`pkey.c:536-565`, `728-731`). `private` is
/// the private key term, which passes the key tuple checks first
/// (`pkey.c:544-559`). A name that OpenSSL loads a key type from
/// (`pkey.c:614`) gives a runtime error: ERTS then reads the key bytes.
pub(super) fn octet_string_key(
    scheduler: &Scheduler,
    algorithm: &str,
    name: &Term,
    private: Option<&Term>,
) -> Failure {
    if let Some([tag, _]) = private.and_then(Term::tuple_elements) {
        match tag {
            Term::Atom(_) if atom_name(scheduler, tag) == "seed" => {
                return badarg(552, 3, "Seed key not allowed");
            }
            Term::Atom(_) if atom_name(scheduler, tag) == "expandedkey" => {}
            _ => return badarg(556, 3, "Invalid key tuple format"),
        }
    }
    // `enif_get_atom` fails for a term that is not an atom, or for a name of
    // 40 bytes or more (`pkey.c:610-613`).
    if !matches!(name, Term::Atom(_)) || algorithm.len() >= 40 {
        return Failure::Check("error", 612, 0, "Invalid key type");
    }
    if OPENSSL_KEY_NAMES
        .iter()
        .any(|name| name.eq_ignore_ascii_case(algorithm))
    {
        return unsupported_algorithm(algorithm);
    }
    failed(616, "Can't create PKEY_CTX from name")
}

/// Return the private key `term` of the algorithm `algorithm` of
/// `pkey_types`, as `get_pkey_private_key` (`pkey.c:539-565`) and
/// `get_pkey_from_octet_string` (`pkey.c:587-660`) give it: the tuple
/// `{seed, Seed}` of ML-DSA or `{expandedkey, Key}`, or the key bytes.
fn pkey_type_private_key(
    scheduler: &Scheduler,
    algorithm: &str,
    term: &Term,
    argument: i64,
) -> Result<PKey<Private>, Failure> {
    let mut key = term;
    let mut seed = false;
    if let Some([tag, value]) = term.tuple_elements() {
        match tag {
            Term::Atom(_) if atom_name(scheduler, tag) == "seed" => {
                // Only ML-DSA sets `allow_seed` (`pkey.c:90-110`).
                if !algorithm.starts_with("mldsa") {
                    return Err(badarg(552, argument, "Seed key not allowed"));
                }
                seed = true;
            }
            Term::Atom(_) if atom_name(scheduler, tag) == "expandedkey" => {}
            _ => return Err(badarg(556, argument, "Invalid key tuple format")),
        }
        key = value;
    }
    let bytes = octet_string(key)?;
    let id = pkey_type_id(algorithm);
    if !seed {
        return PKey::private_key_from_raw_bytes(bytes, id).map_err(|_| fromdata_failed());
    }
    // The safe `openssl` crate has no `EVP_PKEY_fromdata`, so PBeam gives
    // the seed as the seed-only PKCS #8 form of RFC 9881. OpenSSL takes only
    // a seed of 32 bytes (`ml_dsa_kmgmt.c:219` of OpenSSL 3.5).
    if bytes.len() != 32 {
        return Err(fromdata_failed());
    }
    let mut der = vec![
        0x30, 0x34, 0x02, 0x01, 0x00, 0x30, 0x0b, 0x06, 0x09, 0x60, 0x86, 0x48, 0x01, 0x65, 0x03,
        0x04, 0x03,
    ];
    der.push(match algorithm {
        "mldsa44" => 0x11,
        "mldsa65" => 0x12,
        _ => 0x13,
    });
    der.extend([0x04, 0x22, 0x80, 0x20]);
    der.extend_from_slice(bytes);
    PKey::private_key_from_pkcs8(&der).map_err(|_| fromdata_failed())
}

/// Return the bytes of the key binary `term`, as `pkey.c:620-622` does.
pub(super) fn octet_string(term: &Term) -> Result<&[u8], Failure> {
    term.binary_bytes()
        .ok_or(Failure::Check("error", 621, 1, "Invalid public key"))
}

/// Return the failure of `EVP_PKEY_fromdata` (`pkey.c:648-650`).
pub(super) fn fromdata_failed() -> Failure {
    failed(649, "Can't create PKEY fromdata")
}

/// Return the key type of the algorithm `algorithm` of `pkey_types`
/// (`pkey.c:90-172`). `EVP_PKEY_new_raw_private_key` and
/// `EVP_PKEY_new_raw_public_key` give the key bytes to `EVP_PKEY_fromdata`
/// as the parameter `priv` or `pub`, as `pkey.c:630-637` does.
pub(super) fn pkey_type_id(algorithm: &str) -> Id {
    // The NIDs of `obj_mac.h` of OpenSSL 3.5.
    Id::from_raw(match algorithm {
        "mldsa44" => 1457,
        "mldsa65" => 1458,
        "mldsa87" => 1459,
        "slh_dsa_sha2_128s" => 1460,
        "slh_dsa_sha2_128f" => 1461,
        "slh_dsa_sha2_192s" => 1462,
        "slh_dsa_sha2_192f" => 1463,
        "slh_dsa_sha2_256s" => 1464,
        "slh_dsa_sha2_256f" => 1465,
        "slh_dsa_shake_128s" => 1466,
        "slh_dsa_shake_128f" => 1467,
        "slh_dsa_shake_192s" => 1468,
        "slh_dsa_shake_192f" => 1469,
        "slh_dsa_shake_256s" => 1470,
        _ => 1471,
    })
}

/// Return the failure of an algorithm that PBeam does not sign with.
fn unsupported_algorithm(algorithm: &str) -> Failure {
    Failure::Unsupported(format!("the algorithm {algorithm}"))
}

/// Take the binary at the head of the list `cell` as a big-endian unsigned
/// number, as `get_ossl_param_from_bin_in_list` (`bn.c:254-263`) does, and
/// move `cell` to the tail.
fn next_number(cell: &mut &Term) -> Option<BigNum> {
    let (head, tail) = cell.cons_cell()?;
    *cell = tail;
    BigNum::from_slice(head.binary_bytes()?).ok()
}

/// Return the RSA private key `[E, N, D]` or `[E, N, D, P1, P2, E1, E2, C]`
/// (`rsa.c:364-411`).
fn rsa_private_key(term: &Term) -> Option<PKey<Private>> {
    let mut cell = term;
    let e = next_number(&mut cell)?;
    let n = next_number(&mut cell)?;
    let d = next_number(&mut cell)?;
    let rsa = if matches!(cell, Term::Nil) {
        RsaPrivateKeyBuilder::new(n, e, d).ok()?.build()
    } else {
        let p = next_number(&mut cell)?;
        let q = next_number(&mut cell)?;
        let dmp1 = next_number(&mut cell)?;
        let dmq1 = next_number(&mut cell)?;
        let iqmp = next_number(&mut cell)?;
        if !matches!(cell, Term::Nil) {
            return None;
        }
        Rsa::from_private_components(n, e, d, p, q, dmp1, dmq1, iqmp).ok()?
    };
    PKey::from_rsa(rsa).ok()
}

/// Return the RSA public key `[E, N]` (`rsa.c:414-451`).
fn rsa_public_key(term: &Term) -> Option<PKey<Public>> {
    let mut cell = term;
    let e = next_number(&mut cell)?;
    let n = next_number(&mut cell)?;
    if !matches!(cell, Term::Nil) {
        return None;
    }
    PKey::from_rsa(Rsa::from_public_components(n, e).ok()?).ok()
}

/// Return the numbers of the DSA key `[P, Q, G, Priv]` or `[P, Q, G, Pub]`
/// (`dss.c:30-108`).
fn dss_parts(term: &Term) -> Option<[BigNum; 4]> {
    let mut cell = term;
    let parts = [
        next_number(&mut cell)?,
        next_number(&mut cell)?,
        next_number(&mut cell)?,
        next_number(&mut cell)?,
    ];
    matches!(cell, Term::Nil).then_some(parts)
}

/// Return the bytes and the type of the `EdDSA` key `[Key, Curve]`
/// (`eddsa.c:26-66`).
fn eddsa_parts<'a>(scheduler: &Scheduler, term: &'a Term) -> Option<(&'a [u8], Id)> {
    let (head, tail) = term.cons_cell()?;
    let bytes = head.binary_bytes()?;
    let (curve, rest) = tail.cons_cell()?;
    if !matches!(rest, Term::Nil) || !matches!(curve, Term::Atom(_)) {
        return None;
    }
    match atom_name(scheduler, curve) {
        "ed25519" => Some((bytes, Id::ED25519)),
        "ed448" => Some((bytes, Id::ED448)),
        _ => None,
    }
}

/// Return the curve and the key bytes of the ECDSA key
/// `{{CurveDef, CurveName}, Key}` (`ec.c:228-345`): `Ok(None)` for a key
/// that fails a check. PBeam builds the curve from its definition, as ERTS
/// does after OpenSSL fails to load the curve by name (`ec.c:250-267`);
/// the two curves give the same signatures.
fn ec_parts<'a>(
    scheduler: &Scheduler,
    term: &'a Term,
) -> Result<Option<(EcGroup, &'a [u8])>, Failure> {
    let Term::Tuple(items) = term else {
        return Ok(None);
    };
    let [Term::Tuple(curve), key] = &items[..] else {
        return Ok(None);
    };
    let Some(key) = key.binary_bytes() else {
        return Ok(None);
    };
    Ok(ec_group(scheduler, curve)?.map(|group| (group, key)))
}

/// Return the curve of `{CurveDef, CurveName}` (`ec.c:31-226`) with the
/// field `{prime_field, P}`: `Ok(None)` for a definition that fails a
/// check.
pub(super) fn ec_group(scheduler: &Scheduler, curve: &[Term]) -> Result<Option<EcGroup>, Failure> {
    let [Term::Tuple(definition), _] = curve else {
        return Ok(None);
    };
    let [
        Term::Tuple(field),
        Term::Tuple(prime),
        point,
        order,
        cofactor,
    ] = &definition[..]
    else {
        return Ok(None);
    };
    match (
        &field[..],
        field.first().map(|tag| atom_name(scheduler, tag)),
    ) {
        ([_, p], Some("prime_field")) => Ok(prime_group(p, prime, point, order, cofactor)),
        ([_, _, _], Some("characteristic_two_field")) => Err(Failure::Unsupported(
            "an ECDSA curve over a characteristic-two field".to_owned(),
        )),
        _ => Ok(None),
    }
}

/// Return the curve over the prime field `p` with the coefficients of
/// `{A, B, Seed}`, the generator, the order, and the cofactor.
fn prime_group(
    p: &Term,
    prime: &[Term],
    point: &Term,
    order: &Term,
    cofactor: &Term,
) -> Option<EcGroup> {
    let number = |term: &Term| BigNum::from_slice(term.binary_bytes()?).ok();
    let (a, b) = (number(prime.first()?)?, number(prime.get(1)?)?);
    let (order, cofactor) = (number(order)?, number(cofactor)?);
    let mut context = BigNumContext::new().ok()?;
    let mut group = EcGroup::from_components(number(p)?, a, b, &mut context).ok()?;
    let generator = EcPoint::from_bytes(&group, point.binary_bytes()?, &mut context).ok()?;
    group.set_generator(generator, order, cofactor).ok()?;
    Some(group)
}

/// Return the OpenSSL digest of `hash`, as `get_digest_type` fetches it
/// (`digest.c:181-200`).
fn fetch(hash: Hash) -> Md {
    Md::fetch(None, hash.fetch_name(), None).expect("OpenSSL fetches each `Hash`")
}

/// The OpenSSL value of the padding `rsa_pkcs1_oaep_padding`
/// (`pkey.c:1223-1224`).
const RSA_PKCS1_OAEP_PADDING: i32 = 4;

/// The values of the options of an encrypt or decrypt
/// (`PKeyCryptOptions`).
struct CryptOptions {
    padding: i32,
    signature_md: Option<Hash>,
    mgf1: Option<Hash>,
    oaep_md: Option<Hash>,
    label: Vec<u8>,
}

/// Run `pkey_crypt_nif/6` (`pkey.c:1313-1581`): encrypt or decrypt the
/// binary with the public or the private key, and give the result. The
/// public and private encrypt and decrypt functions of `crypto.erl`
/// (`crypto.erl:3476-3599`) call it.
///
/// # Errors
///
/// None: each failure is an exception.
pub(super) fn pkey_crypt_nif(
    scheduler: &mut Scheduler,
    arguments: &[Term],
) -> Result<BifResult, BeamError> {
    let result = crypt(scheduler, arguments).map(Term::binary);
    finish(scheduler, "pkey_crypt_nif", 6, result)
}

/// Encrypt or decrypt as `pkey_crypt_nif` does with the OpenSSL 3 API
/// (`pkey.c:1335-1474`).
fn crypt(scheduler: &Scheduler, arguments: &[Term]) -> Result<Vec<u8>, Failure> {
    let is_private = is_true(scheduler, &arguments[4]);
    let is_encrypt = is_true(scheduler, &arguments[5]);
    // `check_pkey_algorithm_type` without unknown algorithms
    // (`pkey.c:212-250`).
    let algorithm = atom_name(scheduler, &arguments[0]).to_owned();
    if !matches!(arguments[0], Term::Atom(_))
        || !["rsa", "dss", "ecdsa", "eddsa"].contains(&algorithm.as_str())
    {
        return Err(badarg(244, 0, "Bad algorithm"));
    }
    let data = arguments[1]
        .binary_bytes()
        .ok_or_else(|| badarg(1342, 1, "Binary expected"))?;
    let name = &arguments[0];
    if is_private {
        let pkey = private_key(scheduler, &algorithm, name, &arguments[2], 2)?;
        let options = crypt_options(scheduler, &algorithm, &arguments[3])?;
        let mut context = new_context(&pkey)?;
        let init = if is_encrypt {
            context.sign_init()
        } else {
            context.decrypt_init()
        };
        run_crypt(context, init, &algorithm, &options, |context, out| {
            if is_encrypt {
                context.sign(data, out)
            } else {
                context.decrypt(data, out)
            }
        })
    } else {
        let pkey = public_key(scheduler, &algorithm, name, &arguments[2], 2)?;
        let options = crypt_options(scheduler, &algorithm, &arguments[3])?;
        let mut context = new_context(&pkey)?;
        let init = if is_encrypt {
            context.encrypt_init()
        } else {
            context.verify_recover_init()
        };
        run_crypt(context, init, &algorithm, &options, |context, out| {
            if is_encrypt {
                context.encrypt(data, out)
            } else {
                context.verify_recover(data, out)
            }
        })
    }
}

/// Return `true` for the atom `true`, as `argv[N] == atom_true` does
/// (`pkey.c:1335-1336`).
fn is_true(scheduler: &Scheduler, term: &Term) -> bool {
    matches!(term, Term::Atom(_)) && atom_name(scheduler, term) == "true"
}

/// Return a new `EVP_PKEY_CTX` of `pkey` (`pkey.c:1364-1365`).
fn new_context<T>(pkey: &PKey<T>) -> Result<PkeyCtx<T>, Failure> {
    PkeyCtx::new(pkey).map_err(|_| failed(1365, "Can't allocate new EVP_PKEY_CTX"))
}

/// Check the result `init` of the init call, set the RSA options of
/// `context`, and run `operation` once for the size and once for the
/// result (`pkey.c:1380-1474`).
fn run_crypt<T>(
    mut context: PkeyCtx<T>,
    init: Result<(), ErrorStack>,
    algorithm: &str,
    options: &CryptOptions,
    mut operation: impl FnMut(&mut PkeyCtx<T>, Option<&mut [u8]>) -> Result<usize, ErrorStack>,
) -> Result<Vec<u8>, Failure> {
    if init.is_err() {
        return Err(Failure::Check(
            "notsup",
            1381,
            -1,
            "Can't initiate encrypt/decrypt",
        ));
    }
    if algorithm == "rsa" {
        if let Some(digest) = options.signature_md {
            context
                .set_signature_md(&fetch(digest))
                .map_err(|_| failed(1387, "Can't EVP_PKEY_CTX_set_signature_md"))?;
        }
        context
            .set_rsa_padding(Padding::from_raw(options.padding))
            .map_err(|_| failed(1410, "Couldn't set rsa padding"))?;
        if options.padding == RSA_PKCS1_OAEP_PADDING {
            if let Some(digest) = options.oaep_md {
                context
                    .set_rsa_oaep_md(&fetch(digest))
                    .map_err(|_| failed(1416, "Couldn't EVP_PKEY_CTX_set_rsa_oaep_md"))?;
            }
            if let Some(digest) = options.mgf1 {
                context
                    .set_rsa_mgf1_md(&fetch(digest))
                    .map_err(|_| failed(1421, "Couldn't EVP_PKEY_CTX_set_rsa_mgf1_md"))?;
            }
            if !options.label.is_empty() {
                context
                    .set_rsa_oaep_label(&options.label)
                    .map_err(|_| failed(1435, "Couldn't set RSA oaep label"))?;
            }
        }
    }
    let length =
        operation(&mut context, None).map_err(|_| failed(1455, "Couldn't get size of result"))?;
    let mut out = vec![0; length];
    let length = operation(&mut context, Some(&mut out))
        .map_err(|_| failed(1474, "Couldn't get the result"))?;
    out.truncate(length);
    Ok(out)
}

/// Return the options of an encrypt or decrypt, as
/// `get_pkey_crypt_options` (`pkey.c:1173-1294`) gives them from the
/// argument 3.
fn crypt_options(
    scheduler: &Scheduler,
    algorithm: &str,
    term: &Term,
) -> Result<CryptOptions, Failure> {
    if !matches!(term, Term::Nil) && term.cons_cell().is_none() {
        return Err(badarg(1185, 3, "Expected a list"));
    }
    let rsa = algorithm == "rsa";
    let mut options = CryptOptions {
        padding: if rsa { RSA_PKCS1_PADDING } else { 0 },
        signature_md: None,
        mgf1: None,
        oaep_md: None,
        label: Vec::new(),
    };
    if matches!(term, Term::Nil) {
        return Ok(options);
    }
    if !rsa {
        return Err(badarg(1208, 3, "Only RSA supports Options"));
    }
    let mut tail = term;
    while let Some((head, rest)) = tail.cons_cell() {
        tail = rest;
        let pair = match head {
            Term::Tuple(items) if items.len() == 2 => items,
            _ => return Err(badarg(1214, 3, "Expect only two-tuples in the list")),
        };
        let value = &pair[1];
        let atom = matches!(value, Term::Atom(_));
        let key = if matches!(pair[0], Term::Atom(_)) {
            atom_name(scheduler, &pair[0])
        } else {
            ""
        };
        match key {
            "rsa_padding" | "rsa_pad" => {
                options.padding = match atom.then(|| atom_name(scheduler, value)) {
                    Some("rsa_pkcs1_padding") => RSA_PKCS1_PADDING,
                    Some("rsa_pkcs1_oaep_padding") => RSA_PKCS1_OAEP_PADDING,
                    Some("rsa_x931_padding") => RSA_X931_PADDING,
                    Some("rsa_no_padding") => RSA_NO_PADDING,
                    _ => return Err(badarg(1239, 3, "Bad padding type in option rsa_padding")),
                };
            }
            "signature_md" => {
                if !atom {
                    return Err(badarg(
                        1243,
                        3,
                        "Atom expected as argument to option signature_md",
                    ));
                }
                options.signature_md = digest_type(scheduler, algorithm, value, 3)?;
            }
            "rsa_mgf1_md" => {
                if !atom {
                    return Err(badarg(
                        1254,
                        3,
                        "Atom expected as argument to option rsa_mgf1_md",
                    ));
                }
                options.mgf1 = digest_type(scheduler, algorithm, value, 3)?;
            }
            "rsa_oaep_label" => {
                options.label = value
                    .binary_bytes()
                    .ok_or_else(|| badarg(1268, 3, "Binary expected for option rsa_oaep_label"))?
                    .to_vec();
            }
            "rsa_oaep_md" => {
                if !atom {
                    return Err(Failure::Check(
                        "notsup",
                        1276,
                        3,
                        "Atom expected as argument to option rsa_oaep_md",
                    ));
                }
                options.oaep_md = digest_type(scheduler, algorithm, value, 3)?;
            }
            _ => return Err(badarg(1287, 3, "Unknown option")),
        }
    }
    Ok(options)
}

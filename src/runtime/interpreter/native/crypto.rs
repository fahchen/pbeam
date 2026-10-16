//! Native bindings of the `crypto` NIFs (`lib/crypto/c_src/crypto.c`).
//!
//! PBeam links only `strong_rand_bytes_nif/1`, which reads the entropy of
//! the host from `/dev/urandom`. ERTS reads it with the OpenSSL
//! `RAND_bytes` (`rand.c:26-50`). Each other function of the library gives
//! the runtime error `crypto:F/A NIF is not supported` (row "NIF stubs" of
//! `docs/erts-differences.md`).

use super::*;
use std::{fs::File, io::Read};

/// The functions of the ERTS `crypto` NIF library, as `(name, arity)` in
/// the order of `nif_funcs` (`crypto.c:68-145`). `erlang:load_nif/2`
/// checks each of them in this order. PBeam has a binding for only
/// `strong_rand_bytes_nif/1`; `BINDINGS` lists it.
///
/// # Examples
///
/// ```rust,ignore
/// assert_eq!(FUNCTIONS[0], ("info_nif", 0));
/// assert_eq!(FUNCTIONS[26], ("strong_rand_bytes_nif", 1));
/// ```
pub(super) const FUNCTIONS: [(&str, u32); 70] = [
    ("info_nif", 0),
    ("info_lib", 0),
    ("info_fips", 0),
    ("enable_fips_mode_nif", 1),
    ("hash_algorithms", 0),
    ("pubkey_algorithms", 0),
    ("cipher_algorithms", 0),
    ("mac_algorithms", 0),
    ("curve_algorithms", 0),
    ("rsa_opts_algorithms", 0),
    ("hash_info", 1),
    ("hash_nif", 2),
    ("hash_init_nif", 1),
    ("hash_update_nif", 2),
    ("hash_final_nif", 1),
    ("hash_final_xof_nif", 2),
    ("mac_nif", 4),
    ("mac_init_nif", 3),
    ("mac_update_nif", 2),
    ("mac_final_nif", 1),
    ("cipher_info_nif", 1),
    ("ng_crypto_init_nif", 4),
    ("ng_crypto_update_nif", 2),
    ("ng_crypto_final_nif", 1),
    ("ng_crypto_get_data_nif", 1),
    ("ng_crypto_one_time_nif", 5),
    ("strong_rand_bytes_nif", 1),
    ("strong_rand_range_nif", 1),
    ("rand_uniform_nif", 2),
    ("mod_exp_nif", 4),
    ("do_exor", 2),
    ("hash_equals_nif", 2),
    ("pbkdf2_hmac_nif", 5),
    ("pkey_sign_nif", 5),
    ("pkey_sign_heavy_nif", 5),
    ("pkey_verify_nif", 6),
    ("pkey_crypt_nif", 6),
    ("encapsulate_key_nif", 2),
    ("decapsulate_key_nif", 3),
    ("kem_algorithms_nif", 0),
    ("rsa_generate_key_nif", 2),
    ("dh_generate_key_nif", 4),
    ("dh_compute_key_nif", 3),
    ("evp_compute_key_nif", 3),
    ("evp_generate_key_nif", 2),
    ("privkey_to_pubkey_nif", 2),
    ("srp_value_B_nif", 5),
    ("srp_user_secret_nif", 7),
    ("srp_host_secret_nif", 5),
    ("ec_generate_key_nif", 2),
    ("ecdh_compute_key_nif", 3),
    ("rand_seed_nif", 1),
    ("aead_cipher_nif", 7),
    ("aead_cipher_nif", 4),
    ("aead_cipher_init_nif", 4),
    ("engine_by_id_nif", 1),
    ("engine_init_nif", 1),
    ("engine_free_nif", 1),
    ("engine_load_dynamic_nif", 0),
    ("engine_ctrl_cmd_strings_nif", 3),
    ("engine_register_nif", 2),
    ("engine_unregister_nif", 2),
    ("engine_add_nif", 1),
    ("engine_remove_nif", 1),
    ("engine_get_first_nif", 0),
    ("engine_get_next_nif", 1),
    ("engine_get_id_nif", 1),
    ("engine_get_name_nif", 1),
    ("engine_get_all_methods_nif", 0),
    ("ensure_engine_loaded_nif", 2),
];

/// The registered bindings of `crypto`, sorted by name, then by arity.
///
/// # Examples
///
/// ```rust,ignore
/// assert_eq!((BINDINGS[0].name, BINDINGS[0].arity), ("strong_rand_bytes_nif", 1));
/// ```
pub(super) const BINDINGS: &[Binding] = &[Binding {
    name: "strong_rand_bytes_nif",
    arity: 1,
    run: |_, arguments| Ok(strong_rand_bytes_nif(&arguments[0])),
}];

/// Run the load callback of the `crypto` library on `load_info`, as
/// `initialize` (`crypto.c:182-357`) does, and return its veto: 0 when the
/// load passes, else the `crypto.c` line of the failed check. ERTS then
/// gives `{error, {load, "Library load-call unsuccessful (Veto)."}}`
/// (`beam/erl_nif.c:4980-4986`). The load info is `{302, LibPath,
/// FipsMode}` (`crypto.c:205-218`). ERTS enters FIPS mode only for
/// `FipsMode` `true` (`crypto.c:286-288`, `enable_fips_mode` in `info.c`),
/// and a `FipsMode` that is not a boolean gives the veto 287.
///
/// # Examples
///
/// ```rust,ignore
/// let info = Term::tuple(vec![Term::Integer(302), Term::binary(vec![]), Term::static_atom("false")]);
/// assert_eq!(load(&info)?, 0);
/// assert_eq!(load(&Term::Integer(302))?, 205);
/// ```
///
/// # Errors
///
/// `FipsMode` `true` gives a runtime error: PBeam has no FIPS mode.
pub(super) fn load(load_info: &Term) -> Result<i32, BeamError> {
    let Term::Tuple(elements) = load_info else {
        return Ok(205);
    };
    let [version, path, fips_mode] = &elements[..] else {
        return Ok(208);
    };
    let version = match version {
        Term::Integer(version) => i32::try_from(*version).ok(),
        _ => None,
    };
    let Some(version) = version else {
        return Ok(211);
    };
    if version != 302 {
        return Ok(214);
    }
    if !matches!(path, Term::Binary(path) if path.bit_size() % 8 == 0) {
        return Ok(217);
    }
    match fips_mode {
        Term::Atom(atom) if atom.is_static("false") => Ok(0),
        Term::Atom(atom) if atom.is_static("true") => Err(error(
            "crypto FIPS mode is not supported: erlang:load_nif/2 of crypto has FipsMode true",
        )),
        _ => Ok(287),
    }
}

/// Run `crypto:strong_rand_bytes_nif(Bytes)`, as the ERTS NIF
/// `strong_rand_bytes_nif` (`rand.c:26-50`) does. A `Bytes` that is not an
/// unsigned C `int` raises `error:badarg`, and the result is a binary of
/// `Bytes` random bytes. ERTS returns `false` when it cannot make the
/// bytes; PBeam returns `false` when it cannot read them from
/// `/dev/urandom`.
///
/// # Examples
///
/// ```rust,ignore
/// assert!(matches!(strong_rand_bytes_nif(&Term::Integer(16)), BifResult::Value(Term::Binary(_))));
/// assert!(matches!(strong_rand_bytes_nif(&Term::Integer(-1)), BifResult::Raise { .. }));
/// ```
fn strong_rand_bytes_nif(size: &Term) -> BifResult {
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
    let read = File::open("/dev/urandom").and_then(|mut file| file.read_exact(&mut bytes));
    let bytes = read.map(|()| Term::binary(bytes));
    BifResult::Value(bytes.unwrap_or(Term::static_atom("false")))
}

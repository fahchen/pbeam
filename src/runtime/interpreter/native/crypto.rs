//! Native bindings of the `crypto` NIFs (`lib/crypto/c_src/crypto.c`).
//!
//! PBeam links `strong_rand_bytes_nif/1`, which reads the entropy of
//! the host from `/dev/urandom`, which it opens once, and the NIFs that the
//! `plug_hello` release calls: `hash_nif/2`, `pbkdf2_hmac_nif/5`,
//! `strong_rand_range_nif/1`, `info_nif/0`, and `info_lib/0`. ERTS computes
//! the digests with OpenSSL; PBeam uses the RustCrypto `sha1` and `sha2`
//! implementations, because the repository has no hash implementation.
//! Each other function of the library gives the runtime error
//! `crypto:F/A NIF is not supported` (row "NIF stubs" of
//! `docs/erts-differences.md`).

use super::*;
use num_traits::ToPrimitive;
use sha1::Sha1;
use sha2::{Digest, Sha224, Sha256, Sha384, Sha512, Sha512_224, Sha512_256};
use std::{fs::File, io::Read, sync::OnceLock};

/// The functions of the ERTS `crypto` NIF library, as `(name, arity)` in
/// the order of `nif_funcs` (`crypto.c:68-145`). `erlang:load_nif/2`
/// checks each of them in this order. `BINDINGS` lists the bindings.
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
/// assert_eq!((BINDINGS[0].name, BINDINGS[0].arity), ("hash_nif", 2));
/// ```
pub(super) const BINDINGS: &[Binding] = &[
    Binding {
        name: "hash_nif",
        arity: 2,
        run: |scheduler, arguments| hash_nif(scheduler, &arguments[0], &arguments[1]),
    },
    Binding {
        name: "info_lib",
        arity: 0,
        run: |_, _| Ok(info_lib()),
    },
    Binding {
        name: "info_nif",
        arity: 0,
        run: |scheduler, _| info_nif(scheduler),
    },
    Binding {
        name: "pbkdf2_hmac_nif",
        arity: 5,
        run: |scheduler, arguments| {
            pbkdf2_hmac_nif(
                scheduler,
                &arguments[0],
                &arguments[1],
                &arguments[2],
                &arguments[3],
                &arguments[4],
            )
        },
    },
    Binding {
        name: "strong_rand_bytes_nif",
        arity: 1,
        run: |_, arguments| Ok(strong_rand_bytes_nif(&arguments[0])),
    },
    Binding {
        name: "strong_rand_range_nif",
        arity: 1,
        run: |_, arguments| Ok(strong_rand_range_nif(&arguments[0])),
    },
];

/// The OpenSSL version that the pinned ERTS 29.1.1 links, which
/// `crypto:info/0` and `crypto:info_lib/0` report (`info.c:110-204`). The
/// pinned ERTS links OpenSSL statically, so the version is constant.
///
/// # Examples
///
/// ```rust,ignore
/// assert_eq!(OPENSSL_VERSION, "OpenSSL 3.5.8 25 Aug 2026");
/// ```
const OPENSSL_VERSION: &str = "OpenSSL 3.5.8 25 Aug 2026";

/// The `OPENSSL_VERSION_NUMBER` of the pinned ERTS: `0x30500080`
/// (`openssl/opensslv.h`), as `crypto:info_lib/0` reports it.
///
/// # Examples
///
/// ```rust,ignore
/// assert_eq!(OPENSSL_VERSION_NUMBER, 810549376);
/// ```
const OPENSSL_VERSION_NUMBER: i64 = 810549376;

/// Run the load callback of the `crypto` library on `load_info`, as
/// `initialize` (`crypto.c:182-357`) does, and return its veto: 0 when the
/// load passes, else the `crypto.c` line of the failed check. ERTS then
/// gives `{error, {load, "Library load-call unsuccessful (Veto)."}}`
/// (`beam/erl_nif.c:4980-4986`). The load info is `{302, LibPath,
/// FipsMode}` (`crypto.c:205-218`). The check of `FipsMode` passes only
/// for `false` (`crypto.c:286-288`): the pinned ERTS has no FIPS support,
/// so `enable_fips_mode` gives `false` for `true` (`fips.c:59-60`), and
/// `true` or a term that is not a boolean gives the veto 287.
///
/// # Examples
///
/// ```rust,ignore
/// let info = Term::tuple(vec![Term::Integer(302), Term::binary(vec![]), Term::static_atom("false")]);
/// assert_eq!(load(&info), 0);
/// assert_eq!(load(&Term::Integer(302)), 205);
/// ```
pub(super) fn load(load_info: &Term) -> i32 {
    let Term::Tuple(elements) = load_info else {
        return 205;
    };
    let [version, path, fips_mode] = &elements[..] else {
        return 208;
    };
    let version = match version {
        Term::Integer(version) => i32::try_from(*version).ok(),
        _ => None,
    };
    let Some(version) = version else {
        return 211;
    };
    if version != 302 {
        return 214;
    }
    if !matches!(path, Term::Binary(path) if path.bit_size() % 8 == 0) {
        return 217;
    }
    match fips_mode {
        Term::Atom(atom) if atom.is_static("false") => 0,
        _ => 287,
    }
}

/// A digest that `hash_nif/2` and `pbkdf2_hmac_nif/5` compute.
///
/// # Examples
///
/// ```rust,ignore
/// assert!(Hash::from_name("sha256").is_some());
/// assert!(Hash::from_name("md5").is_none());
/// ```
#[derive(Clone, Copy)]
enum Hash {
    Sha,
    Sha224,
    Sha256,
    Sha384,
    Sha512,
    Sha512_224,
    Sha512_256,
}

impl Hash {
    /// Return the digest of `name`, or `None` when PBeam does not compute
    /// it. The names and their OpenSSL digests are those of `digest.c:25-179`.
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// assert!(Hash::from_name("sha").is_some());
    /// assert!(Hash::from_name("sha3_256").is_none());
    /// ```
    fn from_name(name: &str) -> Option<Self> {
        match name {
            "sha" => Some(Hash::Sha),
            "sha224" => Some(Hash::Sha224),
            "sha256" => Some(Hash::Sha256),
            "sha384" => Some(Hash::Sha384),
            "sha512" => Some(Hash::Sha512),
            "sha512_224" => Some(Hash::Sha512_224),
            "sha512_256" => Some(Hash::Sha512_256),
            _ => None,
        }
    }

    /// Return the digest bytes of `data`. The values match the OpenSSL
    /// digests of the pinned ERTS.
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// assert_eq!(digest_bytes(Hash::Sha, b"abc").len(), 20);
    /// ```
    fn digest(self, data: &[u8]) -> Vec<u8> {
        match self {
            Hash::Sha => Sha1::digest(data).to_vec(),
            Hash::Sha224 => Sha224::digest(data).to_vec(),
            Hash::Sha256 => Sha256::digest(data).to_vec(),
            Hash::Sha384 => Sha384::digest(data).to_vec(),
            Hash::Sha512 => Sha512::digest(data).to_vec(),
            Hash::Sha512_224 => Sha512_224::digest(data).to_vec(),
            Hash::Sha512_256 => Sha512_256::digest(data).to_vec(),
        }
    }
}

/// Return whether `name` is a digest of the pinned libcrypto
/// (`digest.c:25-179`). A known digest that PBeam does not compute stops
/// the run with a clear runtime error, where ERTS gives the digest.
///
/// # Examples
///
/// ```rust,ignore
/// assert!(known_digest("md5"));
/// assert!(!known_digest("nosuch"));
/// ```
fn known_digest(name: &str) -> bool {
    matches!(
        name,
        "md4"
            | "md5"
            | "ripemd160"
            | "sha"
            | "sha224"
            | "sha256"
            | "sha384"
            | "sha512"
            | "sha512_224"
            | "sha512_256"
            | "sha3_224"
            | "sha3_256"
            | "sha3_384"
            | "sha3_512"
            | "shake128"
            | "shake256"
            | "sm3"
            | "blake2b"
            | "blake2s"
    )
}

/// Run `crypto:hash_nif(Type, Data)` as the ERTS NIF `hash_nif`
/// (`hash.c:101-164`) does. `Data` is a binary: the only caller,
/// `crypto:hash/2`, converts the iodata first (`crypto.erl:1092-1095`).
/// `crypto:hash/2` sends an input of more than 20000 bytes through
/// `hash_init_nif/1` (`crypto.erl:4535-4540`), which PBeam does not link, so
/// that input stops the run with a runtime error. A digest type that PBeam
/// does not compute, such as `md5`, stops the run with a clear runtime
/// error. A name that is not a digest type raises the
/// ERTS error `{badarg, #{c_file_name => "hash.c", c_file_line_num => 111,
/// c_function_arg_num => 0}, "Bad digest type"}`, which `crypto:hash/2`
/// gives as `error:{badarg, {"hash.c", 111}, "Bad digest type"}`
/// (`crypto.erl:761-775`).
///
/// # Examples
///
/// ```rust,ignore
/// let term = Term::static_atom("sha");
/// let data = Term::binary(b"abc".to_vec());
/// assert!(matches!(hash_nif(&mut scheduler, &term, &data)?, BifResult::Value(Term::Binary(_))));
/// ```
fn hash_nif(
    scheduler: &mut Scheduler,
    digest_type: &Term,
    data: &Term,
) -> Result<BifResult, BeamError> {
    let name = match digest_type {
        Term::Atom(atom) => scheduler.atoms.name_of(atom),
        _ => "",
    };
    let Some(hash) = Hash::from_name(name) else {
        if known_digest(name) {
            return Err(error(format!(
                "crypto:hash_nif/2 does not support the digest type {name}"
            )));
        }
        return c_error(scheduler, "badarg", "hash.c", 111, 0, "Bad digest type");
    };
    let data = match data {
        Term::Binary(data) if data.bit_size() % 8 == 0 => data,
        _ => return c_error(scheduler, "badarg", "hash.c", 118, 1, "Not iolist"),
    };
    Ok(BifResult::Value(Term::binary(hash.digest(data))))
}

/// Run `crypto:pbkdf2_hmac_nif(Digest, Pass, Salt, Iter, KeyLen)` as the
/// ERTS NIF `pbkdf2_hmac_nif` (`pbkdf2_hmac.c:70-100`) does. ERTS checks
/// `Iter` and `KeyLen` before the digest and the two binaries, and it
/// computes only an eligible digest (`PBKDF2_ELIGIBLE_DIGEST`,
/// `digest.c:51-101`): `sha` and the SHA-2 family. A known digest that is
/// not eligible, such as `md5`, raises `{badarg,
/// #{c_file_name => "pbkdf2_hmac.c", c_file_line_num => 40,
/// c_function_arg_num => 0}, "Not eligible digest type"}`. ERTS passes
/// `Iter` as a C `int` to `PKCS5_PBKDF2_HMAC`, so a value above the
/// `INT_MAX` range keeps its low 32 bits, and a value that gives 0 or less
/// raises `{error, #{c_file_name => "pbkdf2_hmac.c", c_file_line_num => 63,
/// c_function_arg_num => -1}, "Low-level call failed"}`.
///
/// # Examples
///
/// ```rust,ignore
/// let digest = Term::static_atom("sha");
/// let pass = Term::binary(b"password".to_vec());
/// let salt = Term::binary(b"salt".to_vec());
/// let result = pbkdf2_hmac_nif(&mut scheduler, &digest, &pass, &salt, &Term::Integer(1), &Term::Integer(16))?;
/// assert!(matches!(result, BifResult::Value(Term::Binary(_))));
/// ```
fn pbkdf2_hmac_nif(
    scheduler: &mut Scheduler,
    digest_type: &Term,
    password: &Term,
    salt: &Term,
    iterations: &Term,
    key_length: &Term,
) -> Result<BifResult, BeamError> {
    let Some(iterations) = unsigned(iterations) else {
        return c_error(scheduler, "badarg", "pbkdf2_hmac.c", 77, 3, "Not integer");
    };
    if iterations < 1 {
        return c_error(scheduler, "badarg", "pbkdf2_hmac.c", 79, 3, "Must be > 0");
    }
    let Some(key_length) = unsigned(key_length) else {
        return c_error(scheduler, "badarg", "pbkdf2_hmac.c", 82, 4, "Not integer");
    };
    if key_length < 1 {
        return c_error(scheduler, "badarg", "pbkdf2_hmac.c", 84, 4, "Must be > 0");
    }
    let name = match digest_type {
        Term::Atom(atom) => scheduler.atoms.name_of(atom),
        _ => "",
    };
    let Some(hash) = Hash::from_name(name) else {
        return if known_digest(name) {
            c_error(
                scheduler,
                "badarg",
                "pbkdf2_hmac.c",
                40,
                0,
                "Not eligible digest type",
            )
        } else {
            c_error(
                scheduler,
                "badarg",
                "pbkdf2_hmac.c",
                36,
                0,
                "Bad digest type",
            )
        };
    };
    let password = match password {
        Term::Binary(password) if password.bit_size() % 8 == 0 => password,
        _ => return c_error(scheduler, "badarg", "pbkdf2_hmac.c", 43, 1, "Not binary"),
    };
    let salt = match salt {
        Term::Binary(salt) if salt.bit_size() % 8 == 0 => salt,
        _ => return c_error(scheduler, "badarg", "pbkdf2_hmac.c", 46, 2, "Not binary"),
    };
    // `PKCS5_PBKDF2_HMAC` takes the iterations as a C `int`
    // (pbkdf2_hmac.c:58-61), so the high 32 bits do not count.
    let iterations = (iterations as u32) as i32;
    if iterations <= 0 {
        return c_error(
            scheduler,
            "error",
            "pbkdf2_hmac.c",
            63,
            -1,
            "Low-level call failed",
        );
    }
    // The C code takes the key length as a `size_t`, so a 32-bit host
    // gives the allocation failure below for a larger value.
    let key_length = usize::try_from(key_length).unwrap_or(usize::MAX);
    let mut output = Vec::new();
    if output.try_reserve_exact(key_length).is_err() {
        return c_error(
            scheduler,
            "error",
            "pbkdf2_hmac.c",
            56,
            -1,
            "Can't allocate binary",
        );
    }
    hash.pbkdf2(password, salt, iterations, key_length, &mut output);
    Ok(BifResult::Value(Term::binary(output)))
}

impl Hash {
    /// Append the PBKDF2-HMAC bytes of `PKCS5_PBKDF2_HMAC`
    /// (`pbkdf2_hmac.c:58-61`) to `output`: the RFC 2898 blocks of the
    /// digest, each one XOR of the HMAC chain, up to `key_length` bytes.
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// let mut output = Vec::new();
    /// Hash::Sha.pbkdf2(b"password", b"salt", 1, 16, &mut output);
    /// assert_eq!(output.len(), 16);
    /// ```
    fn pbkdf2(
        self,
        password: &[u8],
        salt: &[u8],
        iterations: i32,
        key_length: usize,
        output: &mut Vec<u8>,
    ) {
        match self {
            Hash::Sha => pbkdf2_blocks::<Sha1>(64, password, salt, iterations, key_length, output),
            Hash::Sha224 => {
                pbkdf2_blocks::<Sha224>(64, password, salt, iterations, key_length, output)
            }
            Hash::Sha256 => {
                pbkdf2_blocks::<Sha256>(64, password, salt, iterations, key_length, output)
            }
            Hash::Sha384 => {
                pbkdf2_blocks::<Sha384>(128, password, salt, iterations, key_length, output)
            }
            Hash::Sha512 => {
                pbkdf2_blocks::<Sha512>(128, password, salt, iterations, key_length, output)
            }
            Hash::Sha512_224 => {
                pbkdf2_blocks::<Sha512_224>(128, password, salt, iterations, key_length, output)
            }
            Hash::Sha512_256 => {
                pbkdf2_blocks::<Sha512_256>(128, password, salt, iterations, key_length, output)
            }
        }
    }
}

/// Append `key_length` bytes of the PBKDF2-HMAC blocks of the digest `D`,
/// which processes `block_size` bytes at a time, to `output`.
///
/// # Examples
///
/// ```rust,ignore
/// let mut output = Vec::new();
/// pbkdf2_blocks::<Sha256>(64, b"password", b"salt", 1, 32, &mut output);
/// assert_eq!(output.len(), 32);
/// ```
fn pbkdf2_blocks<D: Digest>(
    block_size: usize,
    password: &[u8],
    salt: &[u8],
    iterations: i32,
    key_length: usize,
    output: &mut Vec<u8>,
) {
    let mut index: u32 = 1;
    while output.len() < key_length {
        let mut message = salt.to_vec();
        message.extend_from_slice(&index.to_be_bytes());
        let mut chain = hmac::<D>(block_size, password, &message);
        let mut block = chain.clone();
        for _ in 1..iterations {
            chain = hmac::<D>(block_size, password, &chain);
            for (block_byte, chain_byte) in block.iter_mut().zip(chain.iter()) {
                *block_byte ^= chain_byte;
            }
        }
        output.extend_from_slice(&block);
        index += 1;
    }
    output.truncate(key_length);
}

/// Return the HMAC of `data` with the key `key`, as RFC 2104 gives it for
/// the digest `D`, which processes `block_size` bytes at a time: the digest
/// of the outer pad and the digest of the inner pad and `data`. OpenSSL
/// computes the same value for `PKCS5_PBKDF2_HMAC`.
///
/// # Examples
///
/// ```rust,ignore
/// assert_eq!(hmac::<Sha1>(64, b"key", b"data").len(), 20);
/// ```
fn hmac<D: Digest>(block_size: usize, key: &[u8], data: &[u8]) -> Vec<u8> {
    let mut key_block = if key.len() > block_size {
        D::digest(key).to_vec()
    } else {
        key.to_vec()
    };
    key_block.resize(block_size, 0);
    let inner: Vec<u8> = key_block.iter().map(|byte| byte ^ 0x36).collect();
    let outer: Vec<u8> = key_block.iter().map(|byte| byte ^ 0x5c).collect();
    let mut inner_digest = D::new();
    inner_digest.update(&inner);
    inner_digest.update(data);
    let inner_digest = inner_digest.finalize();
    let mut outer_digest = D::new();
    outer_digest.update(&outer);
    outer_digest.update(&inner_digest);
    outer_digest.finalize().to_vec()
}

/// Run `crypto:strong_rand_bytes_nif(Bytes)`, as the ERTS NIF
/// `strong_rand_bytes_nif` (`rand.c:26-50`) does. A `Bytes` that is not an
/// unsigned C `int` raises `error:badarg`, and the result is a binary of
/// `Bytes` random bytes. ERTS returns `false` when it cannot make the
/// bytes; PBeam returns `false` when it cannot open `/dev/urandom` or
/// read the bytes from it.
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
    let read = urandom().and_then(|mut file| file.read_exact(&mut bytes));
    let bytes = read.map(|()| Term::binary(bytes));
    BifResult::Value(bytes.unwrap_or(Term::static_atom("false")))
}

/// Run `crypto:strong_rand_range_nif(Range)`, as the ERTS NIF
/// `strong_rand_range_nif` (`rand.c:54-85`) does. `Range` is a big-endian
/// unsigned binary, and the result is a binary of a uniform value below
/// the range, with no leading zero bytes: `<<>>` is the value 0. A range
/// of 0 gives `false`, like the failed `BN_rand_range`, and so does a
/// failed read of `/dev/urandom`; `crypto:strong_rand_range/1` then raises
/// `error:badarg` (`crypto.erl:2332-2344`). A term that is not a binary
/// raises `error:badarg` (`enif_inspect_binary`, `rand.c:61-62`).
///
/// # Examples
///
/// ```rust,ignore
/// assert!(matches!(strong_rand_range_nif(&Term::binary(vec![10])), BifResult::Value(Term::Binary(_))));
/// assert!(matches!(strong_rand_range_nif(&Term::binary(vec![0])), BifResult::Value(Term::Atom(_))));
/// assert!(matches!(strong_rand_range_nif(&Term::Integer(10)), BifResult::Raise { .. }));
/// ```
fn strong_rand_range_nif(range: &Term) -> BifResult {
    let range = match range {
        Term::Binary(range) if range.bit_size() % 8 == 0 => range,
        _ => return BifResult::badarg(),
    };
    match random_below(range) {
        Some(value) => BifResult::Value(Term::binary(value)),
        None => BifResult::Value(Term::static_atom("false")),
    }
}

/// Return a uniform value below the big-endian unsigned `range`, as
/// `BN_rand_range` gives it (`bn.h`): draw as many bytes as the range
/// needs, clear the bits above its highest bit, and draw again while the
/// value is not below the range. The value has no leading zero byte, so
/// the value 0 is `<<>>`. Return `None` for a range of 0, which
/// `BN_rand_range` rejects, and for a failed read of `/dev/urandom`.
///
/// # Examples
///
/// ```rust,ignore
/// assert_eq!(random_below(&[0]), None);
/// assert_eq!(random_below(&[1]), Some(vec![]));
/// assert!(random_below(&[10]).is_some_and(|value| value.len() <= 1));
/// ```
fn random_below(range: &[u8]) -> Option<Vec<u8>> {
    let start = range.iter().position(|&byte| byte != 0)?;
    let range = &range[start..];
    let highest_bits = 8 - range[0].leading_zeros() as usize;
    let mask = 0xff >> (8 - highest_bits);
    let mut candidate = vec![0; range.len()];
    loop {
        let mut file = urandom().ok()?;
        file.read_exact(&mut candidate).ok()?;
        candidate[0] &= mask;
        if candidate.as_slice() < range {
            let start = candidate
                .iter()
                .position(|&byte| byte != 0)
                .unwrap_or(candidate.len());
            return Some(candidate[start..].to_vec());
        }
    }
}

/// Run `crypto:info_nif/0`, as the ERTS NIF `info_nif` (`info.c:110-159`)
/// does: the map of the compilation and linking of the pinned libcrypto.
/// `crypto:info/0` adds the key `otp_crypto_version` to this map
/// (`crypto.erl:956-957`).
///
/// # Examples
///
/// ```rust,ignore
/// assert!(matches!(info_nif(&mut scheduler)?, BifResult::Value(Term::Map(_))));
/// ```
fn info_nif(scheduler: &mut Scheduler) -> Result<BifResult, BeamError> {
    let pairs = vec![
        (
            Term::Atom(scheduler.atoms.put("compile_type")?),
            Term::Atom(scheduler.atoms.put("normal")?),
        ),
        (
            Term::Atom(scheduler.atoms.put("link_type")?),
            Term::Atom(scheduler.atoms.put("static")?),
        ),
        (
            Term::Atom(scheduler.atoms.put("cryptolib_version_compiled")?),
            string(OPENSSL_VERSION),
        ),
        (
            Term::Atom(scheduler.atoms.put("cryptolib_version_linked")?),
            string(OPENSSL_VERSION),
        ),
        (
            Term::Atom(scheduler.atoms.put("fips_provider_available")?),
            Term::static_atom("false"),
        ),
    ];
    let map = Map::from_pairs(pairs, &scheduler.atoms).expect("atom keys fit each map form");
    Ok(BifResult::Value(Term::Map(map)))
}

/// Run `crypto:info_lib/0`, as the ERTS NIF `info_lib` (`info.c:162-204`)
/// does: a list of one tuple of the name `OpenSSL`, the numeric version,
/// and the version text of the pinned libcrypto.
///
/// # Examples
///
/// ```rust,ignore
/// assert!(matches!(info_lib(), BifResult::Value(Term::List(_))));
/// ```
fn info_lib() -> BifResult {
    BifResult::Value(Term::list_from([Term::tuple(vec![
        Term::binary(b"OpenSSL".to_vec()),
        Term::Integer(OPENSSL_VERSION_NUMBER),
        Term::binary(OPENSSL_VERSION.as_bytes().to_vec()),
    ])]))
}

/// Return the `error` exception that a crypto NIF raises for the failed
/// check at `line` of `file`: `{Tag, #{c_file_name => File,
/// c_file_line_num => Line, c_function_arg_num => Argument}, Message}`
/// (`raise_exception`, `common.c:29-90`). `crypto:hash/2` and
/// `crypto:pbkdf2_hmac/5` give this as `error:{Tag, {File, Line}, Message}`
/// (`crypto.erl:761-775`).
///
/// # Examples
///
/// ```rust,ignore
/// assert!(matches!(c_error(&mut scheduler, "badarg", "hash.c", 111, 0, "Bad digest type")?, BifResult::Raise { .. }));
/// ```
fn c_error(
    scheduler: &mut Scheduler,
    tag: &str,
    file: &str,
    line: i64,
    argument: i64,
    message: &str,
) -> Result<BifResult, BeamError> {
    let pairs = vec![
        (
            Term::Atom(scheduler.atoms.put("c_file_name")?),
            string(file),
        ),
        (
            Term::Atom(scheduler.atoms.put("c_file_line_num")?),
            Term::Integer(line),
        ),
        (
            Term::Atom(scheduler.atoms.put("c_function_arg_num")?),
            Term::Integer(argument),
        ),
    ];
    let map = Map::from_pairs(pairs, &scheduler.atoms).expect("atom keys fit each map form");
    Ok(BifResult::Raise {
        class: ExceptionClass::Error,
        reason: Term::tuple(vec![
            Term::Atom(scheduler.atoms.put(tag)?),
            Term::Map(map),
            string(message),
        ]),
    })
}

/// Return `text` as the list of its bytes, as `enif_make_string` with
/// `ERL_NIF_LATIN1` gives it, which the crypto errors and information use
/// for text.
///
/// # Examples
///
/// ```rust,ignore
/// assert_eq!(string("ab"), Term::list_from([Term::Integer(97), Term::Integer(98)]));
/// ```
fn string(text: &str) -> Term {
    Term::list_from(text.bytes().map(|byte| Term::Integer(i64::from(byte))))
}

/// Return the unsigned 64-bit value of `term`, as `enif_get_uint64` gives
/// it: a non-negative integer in the `u64` range, else `None`.
///
/// # Examples
///
/// ```rust,ignore
/// assert_eq!(unsigned(&Term::Integer(1)), Some(1));
/// assert_eq!(unsigned(&Term::Integer(-1)), None);
/// ```
fn unsigned(term: &Term) -> Option<u64> {
    match term {
        Term::Integer(value) => u64::try_from(*value).ok(),
        Term::BigInt(value) => value.to_u64(),
        _ => None,
    }
}

/// Give `/dev/urandom`, which the first call opens and each later call
/// shares. A failed open is not kept, so the next call opens it again.
///
/// # Errors
///
/// The error of the open.
fn urandom() -> std::io::Result<&'static File> {
    static URANDOM: OnceLock<File> = OnceLock::new();
    if let Some(file) = URANDOM.get() {
        return Ok(file);
    }
    let file = File::open("/dev/urandom")?;
    Ok(URANDOM.get_or_init(|| file))
}

#[cfg(test)]
#[path = "../../../../tests/runtime/interpreter/native/crypto.rs"]
mod tests;

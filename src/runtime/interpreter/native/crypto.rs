//! Native bindings of the `crypto` NIFs (`lib/crypto/c_src/crypto.c`).
//!
//! PBeam links `strong_rand_bytes_nif/1`, which reads the entropy of
//! the host from `/dev/urandom`, which it opens once, and the NIFs that the
//! `plug_hello` release calls: `hash_nif/2`, `pbkdf2_hmac_nif/5`,
//! `strong_rand_range_nif/1`, `info_nif/0`, and `info_lib/0`. It links
//! `info_fips/0`, `enable_fips_mode_nif/1`, and the algorithm lists of
//! `crypto:supports/0,1`, which name only the algorithms that PBeam can
//! run, and the one-shot hash and MAC NIFs `hash_info/1`, `mac_nif/4`, and
//! `hash_equals_nif/2`. As ERTS, PBeam computes the digests and MACs with
//! a static OpenSSL, through the safe `openssl` crate (`vendored`, OpenSSL
//! 3.5.5).
//! Each other function of the library gives the runtime error
//! `crypto:F/A NIF is not supported` (row "NIF stubs" of
//! `docs/erts-differences.md`).

use super::bif::iodata_bytes;
use super::*;
use num_traits::ToPrimitive;
use openssl::{
    hash::{Hasher, MessageDigest},
    md::Md,
    pkey::{Id, PKey},
    sign::Signer,
};
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
        name: "cipher_algorithms",
        arity: 0,
        job: Job::Normal,
        run: |_, _| Ok(BifResult::Value(Term::Nil)),
    },
    Binding {
        name: "curve_algorithms",
        arity: 0,
        job: Job::Normal,
        run: |_, _| Ok(BifResult::Value(Term::Nil)),
    },
    Binding {
        name: "enable_fips_mode_nif",
        arity: 1,
        job: Job::Normal,
        run: |_, arguments| Ok(enable_fips_mode_nif(&arguments[0])),
    },
    Binding {
        name: "hash_algorithms",
        arity: 0,
        job: Job::Normal,
        run: |scheduler, _| atom_list(scheduler, &HASH_ALGORITHMS),
    },
    Binding {
        name: "hash_equals_nif",
        arity: 2,
        job: Job::Normal,
        run: |_, arguments| Ok(hash_equals_nif(&arguments[0], &arguments[1])),
    },
    Binding {
        name: "hash_info",
        arity: 1,
        job: Job::Normal,
        run: |scheduler, arguments| hash_info(scheduler, &arguments[0]),
    },
    Binding {
        name: "hash_nif",
        arity: 2,
        job: Job::Normal,
        run: |scheduler, arguments| hash_nif(scheduler, &arguments[0], &arguments[1]),
    },
    Binding {
        name: "info_fips",
        arity: 0,
        job: Job::Normal,
        run: |scheduler, _| {
            Ok(BifResult::Value(Term::Atom(
                scheduler.atoms.put("not_supported")?,
            )))
        },
    },
    Binding {
        name: "info_lib",
        arity: 0,
        job: Job::Normal,
        run: |_, _| Ok(info_lib()),
    },
    Binding {
        name: "info_nif",
        arity: 0,
        job: Job::Normal,
        run: |scheduler, _| info_nif(scheduler),
    },
    Binding {
        name: "kem_algorithms_nif",
        arity: 0,
        job: Job::Normal,
        run: |_, _| Ok(BifResult::Value(Term::Nil)),
    },
    Binding {
        name: "mac_algorithms",
        arity: 0,
        job: Job::Normal,
        run: |scheduler, _| atom_list(scheduler, &MAC_ALGORITHMS),
    },
    Binding {
        name: "mac_nif",
        arity: 4,
        job: Job::Normal,
        run: |scheduler, arguments| {
            mac_nif(
                scheduler,
                &arguments[0],
                &arguments[1],
                &arguments[2],
                &arguments[3],
            )
        },
    },
    Binding {
        name: "pbkdf2_hmac_nif",
        arity: 5,
        job: Job::Normal,
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
        name: "pubkey_algorithms",
        arity: 0,
        job: Job::Normal,
        run: |_, _| Ok(BifResult::Value(Term::Nil)),
    },
    Binding {
        name: "rsa_opts_algorithms",
        arity: 0,
        job: Job::Normal,
        run: |_, _| Ok(BifResult::Value(Term::Nil)),
    },
    Binding {
        name: "strong_rand_bytes_nif",
        arity: 1,
        job: Job::Normal,
        run: |_, arguments| Ok(strong_rand_bytes_nif(&arguments[0])),
    },
    Binding {
        name: "strong_rand_range_nif",
        arity: 1,
        job: Job::Normal,
        run: |_, arguments| Ok(strong_rand_range_nif(&arguments[0])),
    },
];

/// Return the version text of the linked OpenSSL, which `crypto:info/0`
/// and `crypto:info_lib/0` report (`info.c:110-204`). PBeam links OpenSSL
/// 3.5.5 statically; the pinned ERTS links 3.5.8 (row "NIF stubs" of
/// `docs/erts-differences.md`).
///
/// # Examples
///
/// ```rust,ignore
/// assert!(openssl_version().starts_with("OpenSSL 3.5."));
/// ```
fn openssl_version() -> &'static str {
    openssl::version::version()
}

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
/// assert_eq!(load(&info, &atoms), 0);
/// assert_eq!(load(&Term::Integer(302), &atoms), 205);
/// ```
pub(super) fn load(load_info: &Term, _atoms: &AtomTable) -> i32 {
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
    if !path.is_binary() {
        return 217;
    }
    match fips_mode {
        Term::Atom(atom) if atom.is_static("false") => 0,
        _ => 287,
    }
}

/// A digest of the pinned libcrypto that `hash_nif/2`, `hash_info/1`,
/// `mac_nif/4`, and `pbkdf2_hmac_nif/5` use. The names and their OpenSSL
/// digests are those of `digest.c:25-179`. MD4 is not a `Hash`: ERTS
/// fetches each digest from the default provider of OpenSSL 3.5, which has
/// no MD4, so its `md.p` is NULL (`digest.c:186-188`). The static OpenSSL
/// of PBeam also loads only the default provider.
///
/// # Examples
///
/// ```rust,ignore
/// assert!(Hash::from_name("md5").is_some());
/// assert!(Hash::from_name("md4").is_none());
/// ```
#[derive(Clone, Copy)]
enum Hash {
    Md5,
    Ripemd160,
    Sha,
    Sha224,
    Sha256,
    Sha384,
    Sha512,
    Sha512_224,
    Sha512_256,
    Sha3_224,
    Sha3_256,
    Sha3_384,
    Sha3_512,
    Shake128,
    Shake256,
    Sm3,
    Blake2b,
    Blake2s,
}

impl Hash {
    /// Return the digest of `name`, or `None` for `md4` and for a name that
    /// is not in `digest.c:25-179`.
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// assert!(Hash::from_name("sha3_256").is_some());
    /// assert!(Hash::from_name("nosuch").is_none());
    /// ```
    fn from_name(name: &str) -> Option<Self> {
        Some(match name {
            "md5" => Hash::Md5,
            "ripemd160" => Hash::Ripemd160,
            "sha" => Hash::Sha,
            "sha224" => Hash::Sha224,
            "sha256" => Hash::Sha256,
            "sha384" => Hash::Sha384,
            "sha512" => Hash::Sha512,
            "sha512_224" => Hash::Sha512_224,
            "sha512_256" => Hash::Sha512_256,
            "sha3_224" => Hash::Sha3_224,
            "sha3_256" => Hash::Sha3_256,
            "sha3_384" => Hash::Sha3_384,
            "sha3_512" => Hash::Sha3_512,
            "shake128" => Hash::Shake128,
            "shake256" => Hash::Shake256,
            "sm3" => Hash::Sm3,
            "blake2b" => Hash::Blake2b,
            "blake2s" => Hash::Blake2s,
            _ => return None,
        })
    }

    /// Return the OpenSSL digest of `self`, as `init_digest_types`
    /// (`digest.c:181-188`) fetches it.
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// assert_eq!(Hash::Sha.message_digest().size(), 20);
    /// ```
    fn message_digest(self) -> MessageDigest {
        match self {
            Hash::Md5 => MessageDigest::md5(),
            Hash::Ripemd160 => MessageDigest::ripemd160(),
            Hash::Sha => MessageDigest::sha1(),
            Hash::Sha224 => MessageDigest::sha224(),
            Hash::Sha256 => MessageDigest::sha256(),
            Hash::Sha384 => MessageDigest::sha384(),
            Hash::Sha512 => MessageDigest::sha512(),
            Hash::Sha512_224 => {
                MessageDigest::from_name("SHA512-224").expect("OpenSSL has SHA512-224")
            }
            Hash::Sha512_256 => {
                MessageDigest::from_name("SHA512-256").expect("OpenSSL has SHA512-256")
            }
            Hash::Sha3_224 => MessageDigest::sha3_224(),
            Hash::Sha3_256 => MessageDigest::sha3_256(),
            Hash::Sha3_384 => MessageDigest::sha3_384(),
            Hash::Sha3_512 => MessageDigest::sha3_512(),
            Hash::Shake128 => MessageDigest::shake_128(),
            Hash::Shake256 => MessageDigest::shake_256(),
            Hash::Sm3 => MessageDigest::sm3(),
            Hash::Blake2b => {
                MessageDigest::from_name("BLAKE2b512").expect("OpenSSL has BLAKE2b512")
            }
            Hash::Blake2s => {
                MessageDigest::from_name("BLAKE2s256").expect("OpenSSL has BLAKE2s256")
            }
        }
    }

    /// Return the OpenSSL 3 fetch name of `self` (`str_v3`,
    /// `digest.c:25-179`).
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// assert_eq!(Hash::Sha.fetch_name(), "SHA1");
    /// ```
    fn fetch_name(self) -> &'static str {
        match self {
            Hash::Md5 => "MD5",
            Hash::Ripemd160 => "RIPEMD160",
            Hash::Sha => "SHA1",
            Hash::Sha224 => "SHA2-224",
            Hash::Sha256 => "SHA2-256",
            Hash::Sha384 => "SHA2-384",
            Hash::Sha512 => "SHA2-512",
            Hash::Sha512_224 => "SHA2-512/224",
            Hash::Sha512_256 => "SHA2-512/256",
            Hash::Sha3_224 => "SHA3-224",
            Hash::Sha3_256 => "SHA3-256",
            Hash::Sha3_384 => "SHA3-384",
            Hash::Sha3_512 => "SHA3-512",
            Hash::Shake128 => "SHAKE-128",
            Hash::Shake256 => "SHAKE-256",
            Hash::Sm3 => "SM3",
            Hash::Blake2b => "BLAKE2b512",
            Hash::Blake2s => "BLAKE2s256",
        }
    }

    /// Return whether PBKDF2 takes `self` (`PBKDF2_ELIGIBLE_DIGEST`,
    /// `digest.c:51-101`): `sha` and the SHA-2 family.
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// assert!(Hash::Sha256.pbkdf2_eligible());
    /// assert!(!Hash::Md5.pbkdf2_eligible());
    /// ```
    fn pbkdf2_eligible(self) -> bool {
        matches!(
            self,
            Hash::Sha
                | Hash::Sha224
                | Hash::Sha256
                | Hash::Sha384
                | Hash::Sha512
                | Hash::Sha512_224
                | Hash::Sha512_256
        )
    }

    /// Return the OpenSSL digest bytes of `data`, as `hash_nif` gives them:
    /// `EVP_Digest`, or for SHAKE `EVP_DigestFinalXOF` with the default
    /// `xoflen` of 16 or 32 bytes (`digest.c:135-151`, `hash.c:120-158`).
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// assert_eq!(Hash::Sha.digest(b"abc").len(), 20);
    /// assert_eq!(Hash::Shake128.digest(b"abc").len(), 16);
    /// ```
    fn digest(self, data: &[u8]) -> Vec<u8> {
        let length = match self {
            Hash::Shake128 => 16,
            Hash::Shake256 => 32,
            _ => {
                return openssl::hash::hash(self.message_digest(), data)
                    .expect("OpenSSL computes each digest of `Hash`")
                    .to_vec();
            }
        };
        let mut output = vec![0; length];
        let mut hasher = Hasher::new(self.message_digest()).expect("OpenSSL has SHAKE");
        hasher.update(data).expect("OpenSSL updates SHAKE");
        hasher.finish_xof(&mut output).expect("OpenSSL ends SHAKE");
        output
    }
}

/// Return the name of `term` when it is an atom, else `""`, which names
/// no algorithm.
///
/// # Examples
///
/// ```rust,ignore
/// assert_eq!(atom_name(&scheduler, &Term::Integer(1)), "");
/// ```
fn atom_name<'a>(scheduler: &'a Scheduler, term: &Term) -> &'a str {
    match term {
        Term::Atom(atom) => scheduler.atoms.name_of(atom),
        _ => "",
    }
}

/// Run `crypto:hash_info(Type)` as the ERTS NIF `hash_info_nif`
/// (`hash.c:74-99`): the map of the OpenSSL type, size, and block size of
/// the digest. A name that is not a digest raises `error:badarg`, and
/// `md4`, whose `md.p` is NULL, raises `error:notsup`. As ERTS, PBeam reads
/// the values from the fetched digest (`digest.c:186-188`): a fetched SHAKE
/// md gives 16/32 on OpenSSL 3.0-3.3 and 0 from 3.4 (`evp_lib.c:1041` in
/// 3.5.5).
///
/// # Examples
///
/// ```rust,ignore
/// assert!(matches!(hash_info(&mut scheduler, &Term::static_atom("sha"))?, BifResult::Value(Term::Map(_))));
/// ```
fn hash_info(scheduler: &mut Scheduler, digest_type: &Term) -> Result<BifResult, BeamError> {
    let name = atom_name(scheduler, digest_type);
    let Some(hash) = Hash::from_name(name) else {
        return Ok(if name == "md4" {
            BifResult::Raise {
                class: ExceptionClass::Error,
                reason: Term::Atom(scheduler.atoms.put("notsup")?),
            }
        } else {
            BifResult::badarg()
        });
    };
    let digest = Md::fetch(None, hash.fetch_name(), None).expect("OpenSSL fetches each `Hash`");
    let pairs = vec![
        (
            Term::Atom(scheduler.atoms.put("type")?),
            Term::Integer(i64::from(digest.type_().as_raw())),
        ),
        (
            Term::Atom(scheduler.atoms.put("size")?),
            Term::Integer(digest.size() as i64),
        ),
        (
            Term::Atom(scheduler.atoms.put("block_size")?),
            Term::Integer(digest.block_size() as i64),
        ),
    ];
    let map = Map::from_pairs(pairs, &scheduler.atoms).expect("atom keys fit each map form");
    Ok(BifResult::Value(Term::Map(map)))
}

/// Run `crypto:hash_nif(Type, Data)` as the ERTS NIF `hash_nif`
/// (`hash.c:101-164`) does. `Data` is a binary: the only caller,
/// `crypto:hash/2`, converts the iodata first (`crypto.erl:1092-1095`).
/// `crypto:hash/2` sends an input of more than 20000 bytes through
/// `hash_init_nif/1` (`crypto.erl:4535-4540`), which PBeam does not link, so
/// that input stops the run with a runtime error. A name that is not a
/// digest type raises the ERTS error `{badarg, #{c_file_name => "hash.c",
/// c_file_line_num => 111, c_function_arg_num => 0}, "Bad digest type"}`,
/// which `crypto:hash/2` gives as `error:{badarg, {"hash.c", 111}, "Bad
/// digest type"}` (`crypto.erl:761-775`). `md4`, whose `md.p` is NULL,
/// raises the `notsup` error of `hash.c:115`.
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
    let name = atom_name(scheduler, digest_type);
    let Some(hash) = Hash::from_name(name) else {
        if name == "md4" {
            return c_error(
                scheduler,
                "notsup",
                "hash.c",
                115,
                0,
                "Digest type not supported in this cryptolib",
            );
        }
        return c_error(scheduler, "badarg", "hash.c", 111, 0, "Bad digest type");
    };
    let data = match data.binary_bytes() {
        Some(data) => data,
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
    let name = atom_name(scheduler, digest_type);
    let hash = match Hash::from_name(name) {
        Some(hash) if hash.pbkdf2_eligible() => Ok(hash),
        Some(_) => Err((40, "Not eligible digest type")),
        None if name == "md4" => Err((38, "md.p is not NULL")),
        None => Err((36, "Bad digest type")),
    };
    let hash = match hash {
        Ok(hash) => hash,
        Err((line, message)) => {
            return c_error(scheduler, "badarg", "pbkdf2_hmac.c", line, 0, message);
        }
    };
    let password = match password.binary_bytes() {
        Some(password) => password,
        _ => return c_error(scheduler, "badarg", "pbkdf2_hmac.c", 43, 1, "Not binary"),
    };
    let salt = match salt.binary_bytes() {
        Some(salt) => salt,
        _ => return c_error(scheduler, "badarg", "pbkdf2_hmac.c", 46, 2, "Not binary"),
    };
    // The C code allocates the key before it calls `PKCS5_PBKDF2_HMAC`
    // (pbkdf2_hmac.c:55-61), and takes the key length as a `size_t`, so a
    // 32-bit host gives the allocation failure for a larger value.
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
    // `PKCS5_PBKDF2_HMAC` takes the iterations as a C `int`
    // (pbkdf2_hmac.c:58-61), so the high 32 bits do not count.
    let iterations = (iterations as u32) as i32;
    if iterations <= 0 {
        return low_level_call_failed(scheduler);
    }
    // `PKCS5_PBKDF2_HMAC` also takes the lengths as C `int`s
    // (pbkdf2_hmac.c:58-61). A key length whose `int` is negative fails
    // the call (OpenSSL `pbkdf2.c:397`); another length above `INT_MAX`,
    // with an `int` of 0 for which ERTS gives uninitialized bytes, stops
    // the run (row "NIF stubs" of `docs/erts-differences.md`).
    let int_max = i32::MAX as usize;
    if key_length > int_max && ((key_length as u32) as i32) < 0 {
        return low_level_call_failed(scheduler);
    }
    if key_length > int_max || password.len() > int_max || salt.len() > int_max {
        return Err(error(
            "crypto:pbkdf2_hmac_nif/5 does not support a key, password, or salt longer than INT_MAX bytes"
                .to_owned(),
        ));
    }
    let derived = hash.pbkdf2(password, salt, iterations, key_length, &mut output);
    let derived = derived.map(|()| BifResult::Value(Term::binary(output)));
    derived.or_else(|_| low_level_call_failed(scheduler))
}

/// Return the `error` tuple of a failed `PKCS5_PBKDF2_HMAC` call
/// (`pbkdf2_hmac.c:63`).
///
/// # Examples
///
/// ```rust,ignore
/// assert!(matches!(low_level_call_failed(&mut scheduler)?, BifResult::Raise { .. }));
/// ```
fn low_level_call_failed(scheduler: &mut Scheduler) -> Result<BifResult, BeamError> {
    c_error(
        scheduler,
        "error",
        "pbkdf2_hmac.c",
        63,
        -1,
        "Low-level call failed",
    )
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
    ) -> Result<(), openssl::error::ErrorStack> {
        let start = output.len();
        output.resize(start + key_length, 0);
        openssl::pkcs5::pbkdf2_hmac(
            password,
            salt,
            iterations as usize,
            self.message_digest(),
            &mut output[start..],
        )
    }
}

/// The ciphers of the pinned libcrypto that `get_cipher_type` finds
/// (`cipher.c:30-173`): the name, the OpenSSL fetch name, and the key
/// length, where 0 takes each length. Each name is in the table once, so a
/// linear search gives the result of the `bsearch` of `cipher.c:242-263`.
const CIPHERS: [(&str, &str, usize); 44] = [
    ("rc2_cbc", "rc2-cbc", 0),
    ("rc4", "rc4", 0),
    ("des_cbc", "des-cbc", 0),
    ("des_cfb", "des-cfb", 0),
    ("des_ecb", "des-ecb", 0),
    ("des_ede3_cbc", "des-ede3-cbc", 0),
    ("des_ede3_cfb", "des-ede3-cfb", 0),
    ("blowfish_cbc", "BF-CBC", 0),
    ("blowfish_cfb64", "BF-CFB", 0),
    ("blowfish_ofb64", "BF-OFB", 0),
    ("blowfish_ecb", "BF-ECB", 0),
    ("sm4_cbc", "sm4-cbc", 16),
    ("sm4_ecb", "sm4-ecb", 16),
    ("sm4_cfb", "sm4-cfb", 16),
    ("sm4_ofb", "sm4-ofb", 16),
    ("sm4_ctr", "sm4-ctr", 16),
    ("aes_128_cbc", "aes-128-cbc", 16),
    ("aes_192_cbc", "aes-192-cbc", 24),
    ("aes_256_cbc", "aes-256-cbc", 32),
    ("aes_128_ofb", "aes-128-ofb", 16),
    ("aes_192_ofb", "aes-192-ofb", 24),
    ("aes_256_ofb", "aes-256-ofb", 32),
    ("aes_128_cfb8", "aes-128-cfb8", 16),
    ("aes_192_cfb8", "aes-192-cfb8", 24),
    ("aes_256_cfb8", "aes-256-cfb8", 32),
    ("aes_128_cfb128", "aes-128-cfb", 16),
    ("aes_192_cfb128", "aes-192-cfb", 24),
    ("aes_256_cfb128", "aes-256-cfb", 32),
    ("aes_128_ecb", "aes-128-ecb", 16),
    ("aes_192_ecb", "aes-192-ecb", 24),
    ("aes_256_ecb", "aes-256-ecb", 32),
    ("aes_128_ctr", "aes-128-ctr", 16),
    ("aes_192_ctr", "aes-192-ctr", 24),
    ("aes_256_ctr", "aes-256-ctr", 32),
    ("chacha20", "chacha20", 32),
    ("chacha20_poly1305", "chacha20-poly1305", 32),
    ("sm4_gcm", "sm4-gcm", 16),
    ("sm4_ccm", "sm4-ccm", 16),
    ("aes_128_gcm", "aes-128-gcm", 16),
    ("aes_192_gcm", "aes-192-gcm", 24),
    ("aes_256_gcm", "aes-256-gcm", 32),
    ("aes_128_ccm", "aes-128-ccm", 16),
    ("aes_192_ccm", "aes-192-ccm", 24),
    ("aes_256_ccm", "aes-256-ccm", 32),
];

/// Run `crypto:mac_nif(Type, SubType, Key, Text)` as the ERTS NIF `mac_nif`
/// with the OpenSSL 3 code of `mac_one_time` (`mac.c:188-505`). ERTS checks
/// `Text`, then `Key`, then the MAC type and its key length, then the
/// subtype, and it computes the MAC with `EVP_Q_mac`. PBeam computes it with
/// an OpenSSL `EVP_PKEY` of the same algorithm through the safe `openssl`
/// crate. Each failure of the computation, such as a digest that HMAC does
/// not take or a cipher that is not CBC for CMAC, raises the `error` of
/// `mac.c:405`.
///
/// # Examples
///
/// ```rust,ignore
/// let (hmac, sha) = (Term::static_atom("hmac"), Term::static_atom("sha"));
/// let key = Term::binary(b"k".to_vec());
/// assert!(matches!(mac_nif(&mut scheduler, &hmac, &sha, &key, &key)?, BifResult::Value(Term::Binary(_))));
/// ```
fn mac_nif(
    scheduler: &mut Scheduler,
    mac_type: &Term,
    sub_type: &Term,
    key: &Term,
    text: &Term,
) -> Result<BifResult, BeamError> {
    // The checks of `Text` and `Key` in the ERTS order (mac.c:192-196,
    // 236-240). The only caller, `crypto:mac/4`, passes `Key` as a binary
    // (crypto.erl:1300-1302).
    let (text, key) = (iodata_bytes(text), iodata_bytes(key));
    let failed = [
        (text.is_none(), 193, 3, "Bad text"),
        (
            text.as_ref()
                .is_some_and(|text| text.len() > i32::MAX as usize),
            196,
            3,
            "Too long text",
        ),
        (key.is_none(), 238, 2, "Bad key"),
    ];
    if let Some(&(_, line, argument, message)) = failed.iter().find(|check| check.0) {
        return c_error(scheduler, "badarg", "mac.c", line, argument, message);
    }
    let (text, key) = (text.unwrap_or_default(), key.unwrap_or_default());
    let mac = match atom_name(scheduler, mac_type) {
        "hmac" => hmac(scheduler, sub_type, &key, &text),
        "cmac" => cmac(scheduler, sub_type, &key, &text),
        "poly1305" if key.len() == 32 => Ok(poly1305(&key, &text)),
        "poly1305" => return c_error(scheduler, "badarg", "mac.c", 253, 2, "Bad key length"),
        _ => {
            return c_error(
                scheduler,
                "badarg",
                "mac.c",
                251,
                0,
                "Unknown mac algorithm",
            );
        }
    };
    match mac {
        Ok(Ok(mac)) => Ok(BifResult::Value(Term::binary(mac))),
        Ok(Err(_)) => c_error(scheduler, "error", "mac.c", 405, -1, "Couldn't get mac"),
        Err(raise) => raise,
    }
}

/// The result of a MAC computation: the MAC, or the OpenSSL error that
/// `mac_nif` gives as `"Couldn't get mac"`, inside `Ok`; or the raise of a
/// failed check of the subtype in `Err`.
type MacResult = Result<Result<Vec<u8>, openssl::error::ErrorStack>, Result<BifResult, BeamError>>;

/// Compute the HMAC of `text` with the digest `sub_type` (`mac.c:275-318`).
/// A name that is not a digest raises the `badarg` of `mac.c:281`. ERTS
/// gives the OpenSSL 3 name of the digest to `EVP_Q_mac`, which cannot
/// fetch MD4, so `md4` fails as the computation.
///
/// # Examples
///
/// ```rust,ignore
/// assert!(matches!(hmac(&mut scheduler, &Term::static_atom("sha"), b"k", b"t"), Ok(Ok(_))));
/// ```
fn hmac(scheduler: &mut Scheduler, sub_type: &Term, key: &[u8], text: &[u8]) -> MacResult {
    let name = atom_name(scheduler, sub_type);
    let Some(hash) = Hash::from_name(name) else {
        if name == "md4" {
            return Ok(Err(openssl::error::ErrorStack::get()));
        }
        let message = "Bad digest algorithm for HMAC";
        return Err(c_error(scheduler, "badarg", "mac.c", 281, 1, message));
    };
    // HMAC pads the key with zero bytes to the block size (RFC 2104), so
    // the empty key, which `EVP_PKEY_new_mac_key` rejects and `EVP_Q_mac`
    // takes, gives the MAC of the key <<0>>.
    let key = if key.is_empty() { &[0][..] } else { key };
    let key = PKey::hmac(key).expect("OpenSSL makes each HMAC key");
    Ok(Signer::new(hash.message_digest(), &key)
        .and_then(|mut signer| signer.sign_oneshot_to_vec(text)))
}

/// Compute the CMAC of `text` with the cipher `sub_type` (`mac.c:324-365`).
/// A name that is not a cipher raises the `badarg` of `mac.c:331`, a key
/// length that the cipher does not take the `badarg` of `mac.c:334`, and a
/// cipher that the default provider does not have, such as `des_cbc`, the
/// `notsup` of `mac.c:346`.
///
/// # Examples
///
/// ```rust,ignore
/// let key = [0; 16];
/// assert!(matches!(cmac(&mut scheduler, &Term::static_atom("aes_128_cbc"), &key, b"t"), Ok(Ok(_))));
/// ```
fn cmac(scheduler: &mut Scheduler, sub_type: &Term, key: &[u8], text: &[u8]) -> MacResult {
    let name = atom_name(scheduler, sub_type);
    let Some(&(_, fetch_name, length)) = CIPHERS.iter().find(|cipher| cipher.0 == name) else {
        return Err(c_error(
            scheduler,
            "badarg",
            "mac.c",
            331,
            1,
            "Unknown cipher",
        ));
    };
    if length != 0 && length != key.len() {
        return Err(c_error(
            scheduler,
            "badarg",
            "mac.c",
            334,
            2,
            "Bad key size",
        ));
    }
    let Ok(cipher) = openssl::cipher::Cipher::fetch(None, fetch_name, None) else {
        let message = "Unsupported cipher algorithm";
        return Err(c_error(scheduler, "notsup", "mac.c", 346, 1, message));
    };
    let cipher = openssl::symm::Cipher::from_nid(cipher.nid());
    let cipher = cipher.ok_or_else(openssl::error::ErrorStack::get);
    Ok(cipher
        .and_then(|cipher| PKey::cmac(&cipher, key))
        .and_then(|key| Signer::new_without_digest(&key)?.sign_oneshot_to_vec(text)))
}

/// Compute the Poly1305 MAC of `text` with the 32-byte `key`
/// (`mac.c:372-383`).
///
/// # Examples
///
/// ```rust,ignore
/// assert_eq!(poly1305(&[0; 32], b"t").unwrap(), vec![0; 16]);
/// ```
fn poly1305(key: &[u8], text: &[u8]) -> Result<Vec<u8>, openssl::error::ErrorStack> {
    let key = PKey::private_key_from_raw_bytes(key, Id::POLY1305)?;
    Signer::new_without_digest(&key)?.sign_oneshot_to_vec(text)
}

/// Run `crypto:hash_equals_nif(A, B)` as the ERTS NIF `hash_equals_nif`
/// (`hash_equals.c:27-53`): `true` or `false` from `CRYPTO_memcmp` for two
/// binaries of the same size, and `error:badarg` for a term that is not a
/// binary or for two sizes that are not equal.
///
/// # Examples
///
/// ```rust,ignore
/// let a = Term::binary(b"a".to_vec());
/// assert!(matches!(hash_equals_nif(&a, &a), BifResult::Value(_)));
/// ```
fn hash_equals_nif(first: &Term, second: &Term) -> BifResult {
    match (first.binary_bytes(), second.binary_bytes()) {
        (Some(first), Some(second)) if first.len() == second.len() => {
            let equal = openssl::memcmp::eq(first, second);
            BifResult::Value(Term::static_atom(if equal { "true" } else { "false" }))
        }
        _ => BifResult::badarg(),
    }
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
    let range = match range.binary_bytes() {
        Some(range) => range,
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

/// The digests that `hash_algorithms/0` gives: the digests that PBeam
/// computes, in the order of the pinned ERTS. ERTS builds the list from
/// the digest table in reverse (`digest.c:226-242`, `algorithms.c:86-89`),
/// and lists each digest that the linked OpenSSL has: each digest of `Hash`.
/// It does not list `md4`, whose `md.p` is NULL.
const HASH_ALGORITHMS: [&str; 18] = [
    "blake2s",
    "blake2b",
    "sm3",
    "shake256",
    "shake128",
    "sha3_512",
    "sha3_384",
    "sha3_256",
    "sha3_224",
    "sha512_256",
    "sha512_224",
    "sha512",
    "sha384",
    "sha256",
    "sha224",
    "sha",
    "ripemd160",
    "md5",
];

/// The MACs that `mac_algorithms/0` gives: the MAC table in reverse
/// (`mac.c:60-102`, `mac.c:139-158`).
const MAC_ALGORITHMS: [&str; 3] = ["cmac", "hmac", "poly1305"];

/// Return the list of the atoms of `names`, which `hash_algorithms/0` and
/// `mac_algorithms/0` give.
///
/// # Examples
///
/// ```rust,ignore
/// assert!(matches!(atom_list(&mut scheduler, &MAC_ALGORITHMS)?, BifResult::Value(Term::List(_))));
/// ```
fn atom_list(scheduler: &mut Scheduler, names: &[&str]) -> Result<BifResult, BeamError> {
    let names = names
        .iter()
        .map(|name| Ok(Term::Atom(scheduler.atoms.put(name)?)))
        .collect::<Result<Vec<_>, BeamError>>()?;
    Ok(BifResult::Value(Term::list_from(names)))
}

/// Run `crypto:enable_fips_mode_nif/1` as the ERTS NIF of a build without
/// FIPS support (`fips.c:56-62`): `false` for `true`, `true` for `false`,
/// and `badarg` for each other term.
///
/// # Examples
///
/// ```rust,ignore
/// assert!(matches!(enable_fips_mode_nif(&Term::static_atom("true")), BifResult::Value(_)));
/// ```
fn enable_fips_mode_nif(mode: &Term) -> BifResult {
    match mode {
        Term::Atom(atom) if atom.is_static("true") => BifResult::Value(Term::static_atom("false")),
        Term::Atom(atom) if atom.is_static("false") => BifResult::Value(Term::static_atom("true")),
        _ => BifResult::badarg(),
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
            string(openssl_version()),
        ),
        (
            Term::Atom(scheduler.atoms.put("cryptolib_version_linked")?),
            string(openssl_version()),
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
        Term::Integer(openssl::version::number()),
        Term::binary(openssl_version().as_bytes().to_vec()),
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

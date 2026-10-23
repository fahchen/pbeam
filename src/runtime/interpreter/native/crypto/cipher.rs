//! The one-shot cipher NIFs of `crypto`: `cipher_algorithms/0`,
//! `cipher_info_nif/1`, `ng_crypto_one_time_nif/5`, `aead_cipher_nif/7`, and
//! `do_exor/2` (`cipher.c`, `api_ng.c`, `aead.c`, and `math.c` of OTP
//! 29.1.1). The safe `openssl` crate makes each OpenSSL call.

use super::*;
use openssl::cipher::{Cipher, CipherRef};
use openssl::cipher_ctx::{CipherCtx, CipherCtxRef};

/// The index in `CIPHERS` of the first AEAD cipher. ERTS gives the flag
/// `AEAD_CIPHER` to `chacha20_poly1305` and to each GCM and CCM cipher
/// (`cipher.c:128-168`), which are the last entries of the table.
const FIRST_AEAD: usize = 35;

/// The `EVP_CIPHER_type` of each cipher that PBeam runs and that the
/// pinned ERTS gives as an integer in `cipher_info_nif/1`
/// (`cipher.c:285-288`). Each other cipher that PBeam runs, such as
/// `chacha20`, `sm4_gcm`, or `aes_128_ctr`, has no OID, so its type is
/// `undefined`. A CFB8 cipher has the type of the CFB128 cipher of its key
/// size.
const CIPHER_TYPES: [(&str, i64); 28] = [
    ("des_ede3_cbc", 44),
    ("des_ede3_cfb", 61),
    ("sm4_ecb", 1133),
    ("sm4_cbc", 1134),
    ("sm4_ofb", 1135),
    ("sm4_cfb", 1137),
    ("sm4_ctr", 1139),
    ("aes_128_ecb", 418),
    ("aes_128_cbc", 419),
    ("aes_128_ofb", 420),
    ("aes_128_cfb128", 421),
    ("aes_128_cfb8", 421),
    ("aes_192_ecb", 422),
    ("aes_192_cbc", 423),
    ("aes_192_ofb", 424),
    ("aes_192_cfb128", 425),
    ("aes_192_cfb8", 425),
    ("aes_256_ecb", 426),
    ("aes_256_cbc", 427),
    ("aes_256_ofb", 428),
    ("aes_256_cfb128", 429),
    ("aes_256_cfb8", 429),
    ("aes_128_gcm", 895),
    ("aes_128_ccm", 896),
    ("aes_192_gcm", 898),
    ("aes_192_ccm", 899),
    ("aes_256_gcm", 901),
    ("aes_256_ccm", 902),
];

/// Return the fetched cipher of the entry `index` of `CIPHERS`, or `None`
/// when the default provider of the static OpenSSL does not have it. ERTS
/// fetches each cipher in the same way (`cipher.c:216-218`), and a cipher
/// that the provider does not have, such as `des_cbc`, has a NULL
/// `cipher.p`.
///
/// # Examples
///
/// ```rust,ignore
/// assert!(fetch_cipher(16).is_some());
/// assert!(fetch_cipher(2).is_none());
/// ```
fn fetch_cipher(index: usize) -> Option<Cipher> {
    Cipher::fetch(None, CIPHERS[index].1, None).ok()
}

/// Return the index in `CIPHERS` of the atom `term`, as
/// `get_cipher_type_no_key` finds it (`cipher.c:373-392`).
///
/// # Examples
///
/// ```rust,ignore
/// assert_eq!(cipher_index(&scheduler, &Term::static_atom("rc4")), Some(1));
/// ```
fn cipher_index(scheduler: &Scheduler, term: &Term) -> Option<usize> {
    let name = atom_name(scheduler, term);
    CIPHERS.iter().position(|cipher| cipher.0 == name)
}

/// Create the atom of each cipher name in the order of `CIPHERS`, as
/// `init_cipher_types` does when ERTS loads the library
/// (`cipher.c:208-215`). The order of `cipher_algorithms/0` follows the
/// indexes of these atoms.
///
/// # Examples
///
/// ```rust,ignore
/// put_cipher_atoms(&atoms);
/// assert!(atoms.get("aes_128_cbc").is_some());
/// ```
pub(super) fn put_cipher_atoms(atoms: &AtomTable) {
    for (name, _, _) in CIPHERS {
        // A full atom table fails the first call of `cipher_algorithms/0`.
        let _ = atoms.put(name);
    }
}

/// Return the list of the ciphers that `cipher_algorithms/0` gives
/// (`cipher_types_as_list`, `cipher.c:395-416`): each cipher of `CIPHERS`
/// that the static OpenSSL has. ERTS sorts the table by the atom of each
/// name (`cipher.c:239`) and builds the list in reverse, so the order is
/// that of the atom indexes, from the highest to the lowest.
///
/// # Examples
///
/// ```rust,ignore
/// assert!(matches!(cipher_algorithms(&mut scheduler)?, BifResult::Value(Term::List(_))));
/// ```
pub(super) fn cipher_algorithms(scheduler: &mut Scheduler) -> Result<BifResult, BeamError> {
    let mut atoms = (0..CIPHERS.len())
        .filter(|&index| fetch_cipher(index).is_some())
        .map(|index| scheduler.atoms.put(CIPHERS[index].0))
        .collect::<Result<Vec<_>, BeamError>>()?;
    atoms.sort_by_key(|atom| std::cmp::Reverse(atom.index()));
    Ok(BifResult::Value(Term::list_from(
        atoms.into_iter().map(Term::Atom),
    )))
}

/// Run `crypto:cipher_info_nif(Type)` as the ERTS NIF `cipher_info_nif`
/// (`cipher.c:266-371`): the map of the type, key length, IV length, block
/// size, AEAD property, and mode of the cipher. A term that is not a cipher
/// name raises `error:badarg`, and a cipher that the static OpenSSL does
/// not have raises `error:notsup`.
///
/// # Examples
///
/// ```rust,ignore
/// let info = cipher_info_nif(&mut scheduler, &Term::static_atom("aes_128_cbc"))?;
/// assert!(matches!(info, BifResult::Value(Term::Map(_))));
/// ```
pub(super) fn cipher_info_nif(
    scheduler: &mut Scheduler,
    cipher_type: &Term,
) -> Result<BifResult, BeamError> {
    let Some(index) = cipher_index(scheduler, cipher_type) else {
        return Ok(BifResult::badarg());
    };
    let Some(cipher) = fetch_cipher(index) else {
        return Ok(BifResult::Raise {
            class: ExceptionClass::Error,
            reason: Term::Atom(scheduler.atoms.put("notsup")?),
        });
    };
    let name = CIPHERS[index].0;
    let kind = match CIPHER_TYPES.iter().find(|entry| entry.0 == name) {
        Some(&(_, kind)) => Term::Integer(kind),
        None => Term::static_atom("undefined"),
    };
    let size = |value: usize| Term::Integer(value as i64);
    let aead = Term::static_atom(if index >= FIRST_AEAD { "true" } else { "false" });
    let pairs = [
        ("type", kind),
        ("key_length", size(cipher.key_length())),
        ("iv_length", size(cipher.iv_length())),
        ("block_size", size(cipher.block_size())),
        ("prop_aead", aead),
        ("mode", Term::Atom(scheduler.atoms.put(cipher_mode(name))?)),
    ];
    let pairs = pairs
        .into_iter()
        .map(|(key, value)| Ok((Term::Atom(scheduler.atoms.put(key)?), value)))
        .collect::<Result<Vec<_>, BeamError>>()?;
    let map = Map::from_pairs(pairs, &scheduler.atoms).expect("atom keys fit each map form");
    Ok(BifResult::Value(Term::Map(map)))
}

/// Return the mode atom of the cipher `name` that `cipher_info_nif/1`
/// gives from `EVP_CIPHER_mode` (`cipher.c:302-363`).
///
/// # Examples
///
/// ```rust,ignore
/// assert_eq!(cipher_mode("aes_128_cfb8"), "cfb_mode");
/// assert_eq!(cipher_mode("chacha20"), "stream_cipher");
/// ```
fn cipher_mode(name: &str) -> &'static str {
    let suffix = name.rsplit('_').next().unwrap_or_default();
    match suffix.get(..3).unwrap_or(suffix) {
        "cbc" => "cbc_mode",
        "ecb" => "ecb_mode",
        "cfb" => "cfb_mode",
        "ofb" => "ofb_mode",
        "ctr" => "ctr_mode",
        "gcm" => "gcm_mode",
        "ccm" => "ccm_mode",
        _ => "stream_cipher",
    }
}

/// A failed check or OpenSSL call of a cipher NIF: the exception tag, the C
/// line, the argument number, and the message of `raise_exception`
/// (`common.c:29-90`).
type CipherFailure = (&'static str, i64, i64, &'static str);

/// The `padding` option of `ng_crypto_one_time_nif/5` (`api_ng.c:156-171`).
#[derive(Clone, Copy, PartialEq)]
enum Padding {
    Undefined,
    None,
    Zero,
    Random,
    Pkcs,
}

/// Read the options of `ng_crypto_one_time_nif/5` as `get_opts` does
/// (`api_ng.c:91-191`): the encrypt flag (1, 0, or -1 for `undefined`) and
/// the padding. A bad option gives the line and the message of its
/// `badarg`.
///
/// # Examples
///
/// ```rust,ignore
/// assert!(matches!(cipher_options(&scheduler, &Term::static_atom("true")), Ok((1, Padding::Undefined))));
/// ```
fn cipher_options(scheduler: &Scheduler, options: &Term) -> Result<(i8, Padding), (i64, Vec<u8>)> {
    match atom_name(scheduler, options) {
        "true" => return Ok((1, Padding::Undefined)),
        "false" => return Ok((0, Padding::Undefined)),
        "undefined" => return Ok((-1, Padding::Undefined)),
        _ => {}
    }
    let fail = |line: i64, message: &str| Err((line, message.as_bytes().to_vec()));
    let Some(items) = options.proper_list() else {
        return fail(122, "Options are not a boolean or a proper list");
    };
    let (mut encrypt, mut padding) = (None, None);
    for item in items {
        let Term::Tuple(pair) = item else {
            return fail(141, "Options must be a property list!");
        };
        let [tag, value] = &pair[..] else {
            return fail(141, "Options must be a property list!");
        };
        let value = atom_name(scheduler, value);
        match atom_name(scheduler, tag) {
            "encrypt" if encrypt.is_some() => {
                return fail(146, "'encrypt' option is present more than once!");
            }
            "encrypt" => {
                encrypt = Some(match value {
                    "true" => 1,
                    "false" => 0,
                    "undefined" => -1,
                    _ => return fail(154, "The 'encrypt' option must be a boolean!"),
                });
            }
            "padding" if padding.is_some() => {
                return fail(159, "The 'padding' option is present more than once!");
            }
            "padding" => {
                padding = Some(match value {
                    "undefined" => Padding::Undefined,
                    "none" => Padding::None,
                    "zero" => Padding::Zero,
                    "random" => Padding::Random,
                    "pkcs_padding" => Padding::Pkcs,
                    _ => return fail(170, "Bad 'padding' option value"),
                });
            }
            _ => {
                // `enif_snprintf` into a buffer of 64 bytes (api_ng.c:174-176).
                let mut message = b"Bad tag in option: ".to_vec();
                message.extend(print_term(tag, &scheduler.atoms));
                message.truncate(63);
                return Err((176, message));
            }
        }
    }
    Ok((encrypt.unwrap_or(1), padding.unwrap_or(Padding::Undefined)))
}

/// Initialize `context` with the cipher, the key, or the IV, in the
/// direction `encrypt`. ERTS calls `EVP_CipherInit_ex` with its encrypt
/// flag first, then with -1, which keeps the direction; the flag -1 on a
/// new context gives decryption.
///
/// # Examples
///
/// ```rust,ignore
/// cipher_init(&mut context, true, Some(&cipher), None, None)?;
/// ```
fn cipher_init(
    context: &mut CipherCtxRef,
    encrypt: bool,
    cipher: Option<&CipherRef>,
    key: Option<&[u8]>,
    iv: Option<&[u8]>,
) -> Result<(), openssl::error::ErrorStack> {
    if encrypt {
        context.encrypt_init(cipher, key, iv)
    } else {
        context.decrypt_init(cipher, key, iv)
    }
}

/// Run `crypto:ng_crypto_one_time_nif(Cipher, Key, IV, Data, Options)` as
/// the ERTS NIF `ng_crypto_one_time_nif` (`api_ng.c:782-864`): the checks
/// of `Data` (`api_ng.c:850-854`), of the options (`get_opts`,
/// `api_ng.c:91-191`), and of `get_init_args` (`api_ng.c:194-406`) in the
/// ERTS order, then one `EVP_CipherUpdate` (`api_ng.c:411-497`) and the
/// padding and `EVP_CipherFinal_ex` of `get_final_args`
/// (`api_ng.c:503-664`).
///
/// # Examples
///
/// ```rust,ignore
/// let result = ng_crypto_one_time_nif(&mut scheduler, &arguments)?;
/// assert!(matches!(result, BifResult::Value(Term::Binary(_))));
/// ```
pub(super) fn ng_crypto_one_time_nif(
    scheduler: &mut Scheduler,
    arguments: &[Term],
) -> Result<BifResult, BeamError> {
    let data = iodata_bytes(&arguments[3]);
    let failed = [
        (data.is_none(), 851, "expected binary as data"),
        (
            data.as_ref()
                .is_some_and(|data| data.len() > i32::MAX as usize),
            854,
            "too long data",
        ),
    ];
    if let Some(&(_, line, message)) = failed.iter().find(|check| check.0) {
        return c_error(scheduler, "badarg", "api_ng.c", line, 3, message);
    }
    let (encrypt, padding) = match cipher_options(scheduler, &arguments[4]) {
        Ok(options) => options,
        Err((line, message)) => {
            return c_error_text(scheduler, "badarg", "api_ng.c", line, 4, &message);
        }
    };
    let data = data.unwrap_or_default();
    match one_time(scheduler, arguments, &data, encrypt, padding) {
        Ok(output) => Ok(BifResult::Value(Term::binary(output))),
        Err((tag, line, argument, message)) => {
            c_error(scheduler, tag, "api_ng.c", line, argument, message)
        }
    }
}

/// Encrypt or decrypt `data` for `ng_crypto_one_time_nif/5` after the
/// checks of `Data` and the options (`api_ng.c:227-664`).
///
/// # Errors
///
/// The failure of the first check or OpenSSL call that fails.
fn one_time(
    scheduler: &Scheduler,
    arguments: &[Term],
    data: &[u8],
    encrypt: i8,
    padding: Padding,
) -> Result<Vec<u8>, CipherFailure> {
    let key = iodata_bytes(&arguments[1]).ok_or(("badarg", 230, 1, "Bad key"))?;
    if !matches!(arguments[0], Term::Atom(_)) {
        return Err(("badarg", 237, 0, "Cipher id is not an atom"));
    }
    let index = cipher_index(scheduler, &arguments[0]);
    let index = index.ok_or(("badarg", 244, 0, "Unknown cipher"))?;
    let length = CIPHERS[index].2;
    if length != 0 && length != key.len() {
        return Err(("badarg", 246, 1, "Bad key size"));
    }
    if index >= FIRST_AEAD {
        return Err(("badarg", 252, 0, "Missing arguments for this cipher"));
    }
    let message = "Cipher not supported in this libcrypto version";
    let cipher = fetch_cipher(index).ok_or(("notsup", 286, 0, message))?;
    if atom_name(scheduler, &arguments[2]) == "undefined" {
        let message = "Dynamic IV is not supported since OTP 27.0";
        return Err(("notsup", 295, -1, message));
    }
    let iv_length = cipher.iv_length();
    let iv = match iv_length {
        0 => None,
        _ => Some(iodata_bytes(&arguments[2]).ok_or(("badarg", 303, 2, "Bad iv type"))?),
    };
    if iv.as_ref().is_some_and(|iv| iv.len() != iv_length) {
        return Err(("badarg", 309, 2, "Bad iv size"));
    }
    let mut context = CipherCtx::new().expect("OpenSSL allocates each cipher context");
    let direction = encrypt == 1;
    cipher_init(&mut context, direction, Some(&cipher), None, None)
        .map_err(|_| ("error", 356, -1, "Can't initialize context, step 1"))?;
    context
        .set_key_length(key.len())
        .map_err(|_| ("error", 362, 1, "Can't initialize context, key_length"))?;
    let initialized = cipher_init(&mut context, direction, None, Some(&key), iv.as_deref());
    let failure = match iv {
        None => ("badarg", 382, 1, "Can't initialize key"),
        Some(_) => ("error", 389, -1, "Can't initialize key or iv"),
    };
    initialized.map_err(|_| failure)?;
    if padding != Padding::Pkcs {
        context.set_padding(false);
    }
    let mut output = Vec::new();
    context
        .cipher_update_vec(data, &mut output)
        .map_err(|_| ("error", 475, -1, "Can't update"))?;
    one_time_final(&mut context, encrypt != 0, padding, data.len(), &mut output)?;
    Ok(output)
}

/// Append the output of `get_final_args` (`api_ng.c:503-664`) to `output`:
/// nothing for the padding `undefined`; else the zero or random padding of
/// an encryption, then `EVP_CipherFinal_ex`. The encrypt flag -1 takes the
/// encryption branch (`api_ng.c:541`).
///
/// # Errors
///
/// The failure of the padding or of `EVP_CipherFinal_ex`.
fn one_time_final(
    context: &mut CipherCtxRef,
    encrypt: bool,
    padding: Padding,
    size: usize,
    output: &mut Vec<u8>,
) -> Result<(), CipherFailure> {
    if padding == Padding::Undefined {
        return Ok(());
    }
    let failure = match (encrypt, padding) {
        (false, _) => ("error", 636, -1, "Can't finalize"),
        (true, Padding::None) => ("error", 608, -1, "Padding 'none' but unfilled last block"),
        (true, Padding::Pkcs) => ("error", 610, -1, "Can't finalize"),
        _ => ("error", 612, -1, "Padding failed"),
    };
    let block_size = context.block_size();
    let pad_size = (block_size - size % block_size) % block_size;
    if encrypt && matches!(padding, Padding::Zero | Padding::Random) && pad_size != 0 {
        let mut pad = vec![0; pad_size];
        if padding == Padding::Random {
            openssl::rand::rand_bytes(&mut pad).expect("OpenSSL makes each random byte");
        }
        context
            .cipher_update_vec(&pad, output)
            .map_err(|_| ("error", 576, -1, "Can't pad"))?;
    }
    context.cipher_final_vec(output).map_err(|_| failure)?;
    Ok(())
}

/// Run `crypto:aead_cipher_nif(Type, Key, IV, In, AAD, TagOrTagLength,
/// EncFlag)` as the ERTS NIF `aead_cipher_nif` with 7 arguments
/// (`aead.c:150-377`): `{Out, Tag}` for an encryption, the plain text for
/// a decryption, and the atom `error` when the tag does not match.
///
/// # Examples
///
/// ```rust,ignore
/// let result = aead_cipher_nif(&mut scheduler, &arguments)?;
/// assert!(matches!(result, BifResult::Value(Term::Tuple(_))));
/// ```
pub(super) fn aead_cipher_nif(
    scheduler: &mut Scheduler,
    arguments: &[Term],
) -> Result<BifResult, BeamError> {
    match aead(scheduler, arguments) {
        Ok(Some(output)) => Ok(BifResult::Value(output)),
        Ok(None) => Ok(BifResult::Value(Term::static_atom("error"))),
        Err((tag, line, argument, message)) => {
            c_error(scheduler, tag, "aead.c", line, argument, message)
        }
    }
}

/// Check the arguments of `aead_cipher_nif/7` in the ERTS order
/// (`aead.c:163-239`), then encrypt or decrypt (`aead.c:280-361`). A
/// failed decryption gives `Ok(None)`.
///
/// # Errors
///
/// The failure of the first check or OpenSSL call that fails.
fn aead(scheduler: &Scheduler, arguments: &[Term]) -> Result<Option<Term>, CipherFailure> {
    let encrypt: i8 = match atom_name(scheduler, &arguments[6]) {
        "true" => 1,
        "false" => 0,
        "undefined" => -1,
        _ => return Err(("badarg", 180, 6, "Bad enc flag")),
    };
    if !matches!(arguments[0], Term::Atom(_)) {
        return Err(("badarg", 187, 0, "non-atom cipher type"));
    }
    let key = iodata_bytes(&arguments[1]).ok_or(("badarg", 189, 1, "non-binary key"))?;
    let iv = iodata_bytes(&arguments[2]).ok_or(("badarg", 191, 2, "non-binary iv"))?;
    let text = iodata_bytes(&arguments[3]).ok_or(("badarg", 193, 3, "non-binary text"))?;
    let aad = iodata_bytes(&arguments[4]).ok_or(("badarg", 197, 4, "non-binary AAD"))?;
    // `Ok` holds the tag length of an encryption, `Err` the tag of a
    // decryption (aead.c:199-208).
    let tag = match (encrypt, &arguments[5]) {
        (0, tag) => Err(iodata_bytes(tag).ok_or(("badarg", 205, 5, "non-binary Tag"))?),
        (_, Term::Integer(length)) if u32::try_from(*length).is_ok() => Ok(*length as usize),
        _ => return Err(("badarg", 201, 5, "Bad Tag length")),
    };
    let tag_length = tag.as_ref().map_or_else(Vec::len, |length| *length);
    let sizes = [tag_length, key.len(), iv.len(), text.len(), aad.len()];
    let index = cipher_index(scheduler, &arguments[0])
        .filter(|&index| CIPHERS[index].2 == 0 || CIPHERS[index].2 == key.len());
    let failed = [
        (
            sizes.iter().any(|&size| size > i32::MAX as usize),
            ("badarg", 215, 5, "binary too long"),
        ),
        (
            index.is_none(),
            ("badarg", 218, 0, "Unknown cipher or invalid key size"),
        ),
    ];
    if let Some(&(_, failure)) = failed.iter().find(|check| check.0) {
        return Err(failure);
    }
    let index = index.unwrap_or_default();
    if index < FIRST_AEAD {
        return Err(("badarg", 222, 0, "Not aead cipher"));
    }
    let message = "The cipher is not supported in this libcrypto version";
    let cipher = fetch_cipher(index).ok_or(("notsup", 232, 0, message))?;
    let ccm = CIPHERS[index].0.ends_with("_ccm");
    let mut context = CipherCtx::new().expect("OpenSSL allocates each cipher context");
    let direction = encrypt == 1;
    cipher_init(&mut context, direction, Some(&cipher), None, None)
        .map_err(|_| ("error", 237, -1, "CipherInit failed"))?;
    context
        .set_key_length(key.len())
        .map_err(|_| ("badarg", 239, 1, "Bad Key length"))?;
    context
        .set_iv_length(iv.len())
        .map_err(|_| ("badarg", 281, 2, "Bad IV length"))?;
    if ccm {
        let set = match &tag {
            Err(tag) => context.set_tag(tag),
            Ok(length) => context.set_tag_length(*length),
        };
        set.map_err(|_| ("badarg", 286, 5, "Can't set tag"))?;
    }
    let (line, message) = match ccm {
        true => (288, "Can't set key or iv"),
        false => (296, "Can't set key and iv"),
    };
    cipher_init(&mut context, direction, None, Some(&key), Some(&iv))
        .map_err(|_| ("error", line, -1, message))?;
    if ccm {
        context
            .set_data_len(text.len())
            .map_err(|_| ("error", 290, -1, "Can't set text size"))?;
    }
    context
        .cipher_update(&aad, None)
        .map_err(|_| ("badarg", 301, 4, "Can't set AAD"))?;
    let mut output = Vec::new();
    let updated = context.cipher_update_vec(&text, &mut output);
    // A failed decryption gives the atom `error` (aead.c:312-320).
    let failure = match &tag {
        Ok(_) => Some(("badarg", 315, 3, "Can't set in-text")),
        Err(_) => None,
    };
    if updated.is_err() {
        return failure.map_or(Ok(None), Err);
    }
    match tag {
        Ok(length) => {
            context
                .cipher_final_vec(&mut output)
                .map_err(|_| ("error", 326, -1, "Encrypt error"))?;
            let mut tag = vec![0; length];
            context
                .tag(&mut tag)
                .map_err(|_| ("error", 332, -1, "Can't get Tag"))?;
            Ok(Some(Term::tuple(vec![
                Term::binary(output),
                Term::binary(tag),
            ])))
        }
        Err(_) if ccm => Ok(Some(Term::binary(output))),
        Err(tag) => {
            let verified = context
                .set_tag(&tag)
                .and_then(|()| context.cipher_final_vec(&mut output));
            Ok(verified.ok().map(|_| Term::binary(output)))
        }
    }
}

/// Run `crypto:do_exor(Data1, Data2)` as the ERTS NIF `do_exor`
/// (`math.c:25-54`): the XOR of two iodata of the same size, and
/// `error:badarg` for a term that is not iodata or for two sizes that are
/// not equal.
///
/// # Examples
///
/// ```rust,ignore
/// let a = Term::binary(vec![1, 2]);
/// assert!(matches!(do_exor(&a, &a), BifResult::Value(_)));
/// ```
pub(super) fn do_exor(first: &Term, second: &Term) -> BifResult {
    match (iodata_bytes(first), iodata_bytes(second)) {
        (Some(first), Some(second)) if first.len() == second.len() => {
            let xor = first.iter().zip(&second).map(|(a, b)| a ^ b).collect();
            BifResult::Value(Term::binary(xor))
        }
        _ => BifResult::badarg(),
    }
}

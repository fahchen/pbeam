//! The cipher context NIFs of `crypto`: `ng_crypto_init_nif/4`,
//! `ng_crypto_update_nif/2`, `ng_crypto_final_nif/1`,
//! `ng_crypto_get_data_nif/1` (`api_ng.c:411-892` of OTP 29.1.1),
//! `aead_cipher_init_nif/4`, and `aead_cipher_nif/4` (`aead.c:69-377`). The
//! safe `openssl` crate makes each OpenSSL call.

use super::cipher::{
    CipherDirection, CipherFailure, FIRST_AEAD, Padding, cipher_index, cipher_init, cipher_options,
    fetch_cipher, final_output, init_context,
};
use super::{
    BeamError, BifResult, CIPHERS, HashMap, Map, Reference, Scheduler, Term, atom_name, c_error,
    c_error_text, iodata_bytes, lock,
};
use openssl::cipher_ctx::CipherCtx;

/// The cipher contexts of the node, as the `evp_cipher_ctx` resources of
/// `cipher.h:62-74`. ERTS frees a context when no term refers to it. PBeam
/// has no garbage collection of resources, so it keeps each context until
/// the end of the run. A context changes in place: each update and final
/// moves the state of the OpenSSL context.
#[derive(Default)]
struct CipherContexts {
    /// The state of each context resource.
    by_reference: HashMap<Reference, CipherContext>,
}

/// The state of one `evp_cipher_ctx`.
struct CipherContext {
    /// The OpenSSL context.
    context: CipherCtx,
    /// The padding of `get_final_args`.
    padding: Padding,
    /// The C `int` `encflag`: 1, 0, or -1 for the option `undefined`.
    encrypt: i8,
    /// The C `unsigned int` sum of the sizes of the updates.
    size: u32,
    /// The size of the padding of the last final, -1 before it.
    padded_size: i32,
}

/// The AEAD contexts of the node, as the `aead_cipher_ctx` resources of
/// `aead.c:31-41`, which PBeam also keeps until the end of the run.
#[derive(Default)]
struct AeadContexts {
    /// The state of each context resource.
    by_reference: HashMap<Reference, AeadContext>,
}

/// The state of one `aead_cipher_ctx`.
struct AeadContext {
    /// True for a CCM cipher.
    ccm: bool,
    /// The OpenSSL context, which each call starts again with its IV.
    context: CipherCtx,
    /// The key of each call.
    key: Vec<u8>,
    /// True for an encryption.
    encrypt: bool,
    /// The size of the tag.
    tag_length: usize,
}

/// Give the name of the padding atom of `padding`.
fn padding_name(padding: Padding) -> &'static str {
    match padding {
        Padding::Undefined => "undefined",
        Padding::None => "none",
        Padding::Zero => "zero",
        Padding::Random => "random",
        Padding::Pkcs => "pkcs_padding",
    }
}

/// Give the `Bad State` error of argument 0 at line `line` of `file`.
fn bad_state(scheduler: &mut Scheduler, file: &str, line: i64) -> Result<BifResult, BeamError> {
    c_error(scheduler, "badarg", file, line, 0, "Bad State")
}

/// Give the binary `output`, or the error of `failure` in `api_ng.c`.
fn cipher_result(
    scheduler: &mut Scheduler,
    output: Result<Vec<u8>, CipherFailure>,
) -> Result<BifResult, BeamError> {
    match output {
        Ok(output) => Ok(BifResult::Value(Term::binary(output))),
        Err((tag, line, argument, message)) => {
            c_error(scheduler, tag, "api_ng.c", line, argument, message)
        }
    }
}

/// Run `crypto:ng_crypto_init_nif(Cipher, Key, IVec, Options)` as the ERTS
/// NIF `ng_crypto_init_nif` (`api_ng.c:671-713`). For a cipher atom, it
/// checks the options (argument 3), then runs `get_init_args`
/// (`api_ng.c:194-406`) and gives a new context. For a context, it sets the
/// direction `true` or `false` of argument 3 with `EVP_CipherInit_ex` and
/// gives back the context.
///
/// # Errors
///
/// A full reference counter gives the error of `Scheduler::make_ref`.
pub(super) fn ng_crypto_init_nif(
    scheduler: &mut Scheduler,
    arguments: &[Term],
) -> Result<BifResult, BeamError> {
    if matches!(arguments[0], Term::Atom(_)) {
        let (encrypt, padding) = match cipher_options(scheduler, &arguments[3]) {
            Ok(options) => options,
            Err((line, message)) => {
                return c_error_text(scheduler, "badarg", "api_ng.c", line, 3, &message);
            }
        };
        let context = match init_context(scheduler, arguments, encrypt, padding) {
            Ok(context) => context,
            Err(failure) => return cipher_result(scheduler, Err(failure)),
        };
        let reference = scheduler.make_magic_ref()?;
        let state = CipherContext {
            context,
            padding,
            encrypt,
            size: 0,
            padded_size: -1,
        };
        lock(&scheduler.node().resources.kind::<CipherContexts>())
            .by_reference
            .insert(reference, state);
        return Ok(BifResult::Value(Term::Reference(reference)));
    }
    let encrypt = match atom_name(scheduler, &arguments[3]) {
        "true" => Some(true),
        "false" => Some(false),
        _ => None,
    };
    let outcome = with_cipher(scheduler, &arguments[0], |state| {
        let encrypt = encrypt.ok_or(("badarg", 695, 3, "Expected true or false"))?;
        state.encrypt = i8::from(encrypt);
        cipher_init(
            &mut state.context,
            CipherDirection::from(encrypt),
            None,
            None,
            None,
        )
        .map_err(|_| ("error", 701, -1, "Can't initialize encflag"))?;
        Ok(())
    });
    match outcome {
        None => {
            let message = "Expected cipher name atom";
            c_error(scheduler, "badarg", "api_ng.c", 707, 0, message)
        }
        Some(Ok(())) => Ok(BifResult::Value(arguments[0].clone())),
        Some(Err(failure)) => cipher_result(scheduler, Err(failure)),
    }
}

/// Run `operation` on the cipher context of `term`, or give `None` when
/// `term` is not a cipher context resource.
fn with_cipher<T>(
    scheduler: &Scheduler,
    term: &Term,
    operation: impl FnOnce(&mut CipherContext) -> T,
) -> Option<T> {
    let Term::Reference(reference) = term else {
        return None;
    };
    let store = scheduler.node().resources.kind::<CipherContexts>();
    let mut store = lock(&store);
    store.by_reference.get_mut(reference).map(operation)
}

/// Run `crypto:ng_crypto_update_nif(Context, Data)` as the ERTS NIF
/// `ng_crypto_update_nif` (`api_ng.c:740-758`): the checks of `Data`, then
/// of the context (`api_ng.c:728-729`), then `EVP_CipherUpdate` of
/// `get_update_args` (`api_ng.c:411-497`), which adds the size of `Data` to
/// the C `unsigned int` size of the context.
pub(super) fn ng_crypto_update_nif(
    scheduler: &mut Scheduler,
    context: &Term,
    data: &Term,
) -> Result<BifResult, BeamError> {
    let data = iodata_bytes(data);
    let failed = [
        (data.is_none(), 745, "expected binary"),
        (
            data.as_ref()
                .is_some_and(|data| data.len() > i32::MAX as usize),
            748,
            "too long data",
        ),
    ];
    if let Some(&(_, line, message)) = failed.iter().find(|check| check.0) {
        return c_error(scheduler, "badarg", "api_ng.c", line, 1, message);
    }
    let data = data.unwrap_or_default();
    let output = with_cipher(scheduler, context, |state| {
        state.size = state.size.wrapping_add(data.len() as u32);
        let mut output = Vec::new();
        state
            .context
            .cipher_update_vec(&data, &mut output)
            .map_err(|_| ("error", 475, -1, "Can't update"))?;
        Ok(output)
    });
    match output {
        Some(output) => cipher_result(scheduler, output),
        None => bad_state(scheduler, "api_ng.c", 729),
    }
}

/// Run `crypto:ng_crypto_final_nif(Context)` as the ERTS NIF
/// `ng_crypto_final_nif` (`api_ng.c:764-776`): the padding and
/// `EVP_CipherFinal_ex` of `get_final_args` (`api_ng.c:503-664`), which
/// sets the padded size of an encryption.
pub(super) fn ng_crypto_final_nif(
    scheduler: &mut Scheduler,
    context: &Term,
) -> Result<BifResult, BeamError> {
    let output = with_cipher(scheduler, context, |state| {
        let mut output = Vec::new();
        let (encrypt, padding, size) = (
            CipherDirection::from(state.encrypt != 0),
            state.padding,
            state.size,
        );
        let padded_size = &mut state.padded_size;
        final_output(
            &mut state.context,
            encrypt,
            padding,
            size,
            padded_size,
            &mut output,
        )?;
        Ok(output)
    });
    match output {
        Some(output) => cipher_result(scheduler, output),
        None => bad_state(scheduler, "api_ng.c", 771),
    }
}

/// Run `crypto:ng_crypto_get_data_nif(Context)` as the ERTS NIF
/// `ng_crypto_get_data_nif` (`api_ng.c:871-892`): the map of the size, the
/// padding size, the padding, and the direction of the context. ERTS gives
/// the `unsigned int` size with `enif_make_int`, so a size above `INT_MAX`
/// wraps.
pub(super) fn ng_crypto_get_data_nif(
    scheduler: &mut Scheduler,
    context: &Term,
) -> Result<BifResult, BeamError> {
    let data = with_cipher(scheduler, context, |state| {
        (
            state.size as i32,
            state.padded_size,
            padding_name(state.padding),
            state.encrypt != 0,
        )
    });
    let Some((size, padded_size, padding, encrypt)) = data else {
        return bad_state(scheduler, "api_ng.c", 882);
    };
    let pairs = [
        ("size", Term::Integer(size.into())),
        ("padding_size", Term::Integer(padded_size.into())),
        ("padding_type", Term::Atom(scheduler.atoms.put(padding)?)),
        (
            "encrypt",
            Term::static_atom(if encrypt { "true" } else { "false" }),
        ),
    ];
    let pairs = pairs
        .into_iter()
        .map(|(key, value)| Ok((Term::Atom(scheduler.atoms.put(key)?), value)))
        .collect::<Result<Vec<_>, BeamError>>()?;
    let map = Map::from_pairs(pairs, &scheduler.atoms).expect("atom keys fit each map form");
    Ok(BifResult::Value(Term::Map(map)))
}

/// Run `crypto:aead_cipher_init_nif(Type, Key, TagLength, EncFlag)` as the
/// ERTS NIF `aead_cipher_init_nif` (`aead.c:69-148`): the checks in the
/// ERTS order, then `EVP_CipherInit_ex` and the key length of a new AEAD
/// context.
///
/// # Errors
///
/// A full reference counter gives the error of `Scheduler::make_ref`.
pub(super) fn aead_cipher_init_nif(
    scheduler: &mut Scheduler,
    arguments: &[Term],
) -> Result<BifResult, BeamError> {
    match aead_init(scheduler, arguments) {
        Ok(state) => {
            let reference = scheduler.make_magic_ref()?;
            lock(&scheduler.node().resources.kind::<AeadContexts>())
                .by_reference
                .insert(reference, state);
            Ok(BifResult::Value(Term::Reference(reference)))
        }
        Err((tag, line, argument, message)) => {
            c_error(scheduler, tag, "aead.c", line, argument, message)
        }
    }
}

/// Check the arguments of `aead_cipher_init_nif/4` (`aead.c:88-138`) and
/// start the context.
///
/// # Errors
///
/// The failure of the first check or OpenSSL call that fails.
fn aead_init(scheduler: &Scheduler, arguments: &[Term]) -> Result<AeadContext, CipherFailure> {
    let encrypt = match atom_name(scheduler, &arguments[3]) {
        "true" => true,
        "false" => false,
        _ => return Err(("badarg", 95, 3, "Bad enc flag")),
    };
    if !matches!(arguments[0], Term::Atom(_)) {
        return Err(("badarg", 102, 0, "non-atom cipher type"));
    }
    let key = arguments[1]
        .binary_bytes()
        .ok_or(("badarg", 105, 1, "non-binary key"))?
        .to_vec();
    let tag_length = match &arguments[2] {
        Term::Integer(length) => u32::try_from(*length).ok(),
        _ => None,
    };
    let tag_length = tag_length.ok_or(("badarg", 109, 2, "Bad Tag length"))? as usize;
    if tag_length > i32::MAX as usize || key.len() > i32::MAX as usize {
        return Err(("badarg", 113, 1, "key or tag too long"));
    }
    let index = cipher_index(scheduler, &arguments[0])
        .filter(|&index| CIPHERS[index].2 == 0 || CIPHERS[index].2 == key.len())
        .ok_or(("badarg", 116, 0, "Unknown cipher or invalid key size"))?;
    if index < FIRST_AEAD {
        return Err(("badarg", 120, 0, "Not aead cipher"));
    }
    let message = "The cipher is not supported in this libcrypto version";
    let cipher = fetch_cipher(index).ok_or(("notsup", 131, 0, message))?;
    let mut context = CipherCtx::new().expect("OpenSSL allocates each cipher context");
    cipher_init(
        &mut context,
        CipherDirection::from(encrypt),
        Some(&cipher),
        None,
        None,
    )
    .map_err(|_| ("error", 136, -1, "CipherInit failed"))?;
    context
        .set_key_length(key.len())
        .map_err(|_| ("badarg", 138, 1, "Bad Key length"))?;
    Ok(AeadContext {
        ccm: CIPHERS[index].0.ends_with("_ccm"),
        context,
        key,
        encrypt,
        tag_length,
    })
}

/// Run `crypto:aead_cipher_nif(Context, IV, InText, AAD)` as the ERTS NIF
/// `aead_cipher_nif` with 4 arguments (`aead.c:241-377`): the cipher text
/// with the tag at its end for an encryption; for a decryption, the plain
/// text of `InText` without its last `TagLength` bytes, which are the tag,
/// or the atom `error` when the tag does not match. As ERTS, the errors of
/// the shared steps give the argument numbers of `aead_cipher_nif/7`.
pub(super) fn aead_cipher_context_nif(
    scheduler: &mut Scheduler,
    arguments: &[Term],
) -> Result<BifResult, BeamError> {
    let Term::Reference(reference) = &arguments[0] else {
        return bad_state(scheduler, "aead.c", 244);
    };
    let store = scheduler.node().resources.kind::<AeadContexts>();
    let outcome = lock(&store)
        .by_reference
        .get_mut(reference)
        .map(|state| aead_run(state, arguments));
    match outcome {
        None => bad_state(scheduler, "aead.c", 244),
        Some(Ok(Some(output))) => Ok(BifResult::Value(Term::binary(output))),
        Some(Ok(None)) => Ok(BifResult::Value(Term::static_atom("error"))),
        Some(Err((tag, line, argument, message))) => {
            c_error(scheduler, tag, "aead.c", line, argument, message)
        }
    }
}

/// Encrypt or decrypt with the AEAD context `state` (`aead.c:248-361`). A
/// failed decryption gives `Ok(None)`.
///
/// # Errors
///
/// The failure of the first check or OpenSSL call that fails.
fn aead_run(state: &mut AeadContext, arguments: &[Term]) -> Result<Option<Vec<u8>>, CipherFailure> {
    let iv = iodata_bytes(&arguments[1]).ok_or(("badarg", 249, 1, "non-binary iv"))?;
    let text = iodata_bytes(&arguments[2]).ok_or(("badarg", 251, 2, "non-binary text"))?;
    let aad = iodata_bytes(&arguments[3]).ok_or(("badarg", 256, 3, "non-binary AAD"))?;
    let tag_length = state.tag_length;
    let (text, tag) = if state.encrypt {
        (&text[..], None)
    } else {
        let length = text.len().checked_sub(tag_length);
        let length = length.ok_or(("error", 270, -1, "Bad in data"))?;
        (&text[..length], Some(&text[length..]))
    };
    let context = &mut state.context;
    context
        .set_iv_length(iv.len())
        .map_err(|_| ("badarg", 281, 2, "Bad IV length"))?;
    if state.ccm {
        let set = match tag {
            Some(tag) => context.set_tag(tag),
            None => context.set_tag_length(tag_length),
        };
        set.map_err(|_| ("badarg", 286, 5, "Can't set tag"))?;
    }
    let (line, message) = if state.ccm {
        (288, "Can't set key or iv")
    } else {
        (296, "Can't set key and iv")
    };
    cipher_init(
        context,
        CipherDirection::from(state.encrypt),
        None,
        Some(&state.key),
        Some(&iv),
    )
    .map_err(|_| ("error", line, -1, message))?;
    if state.ccm {
        context
            .set_data_len(text.len())
            .map_err(|_| ("error", 290, -1, "Can't set text size"))?;
    }
    context
        .cipher_update(&aad, None)
        .map_err(|_| ("badarg", 301, 4, "Can't set AAD"))?;
    let mut output = Vec::new();
    let updated = context.cipher_update_vec(text, &mut output);
    // A failed decryption gives the atom `error` (aead.c:312-320).
    let failure = match tag {
        None => Some(("badarg", 315, 3, "Can't set in-text")),
        Some(_) => None,
    };
    if updated.is_err() {
        return failure.map_or(Ok(None), Err);
    }
    match tag {
        None => {
            context
                .cipher_final_vec(&mut output)
                .map_err(|_| ("error", 338, -1, "Encrypt error"))?;
            let mut tag = vec![0; tag_length];
            context
                .tag(&mut tag)
                .map_err(|_| ("error", 342, -1, "Can't get Tag"))?;
            output.extend(tag);
            Ok(Some(output))
        }
        Some(_) if state.ccm => Ok(Some(output)),
        Some(tag) => {
            let verified = context
                .set_tag(tag)
                .and_then(|()| context.cipher_final_vec(&mut output));
            Ok(verified.ok().map(|_| output))
        }
    }
}

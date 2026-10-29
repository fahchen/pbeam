//! The MAC context NIFs of `crypto`: `mac_init_nif/3`, `mac_update_nif/2`,
//! and `mac_final_nif/1` (`mac.c:514-887` of OTP 29.1.1, with the OpenSSL 3
//! API). The safe `openssl` crate makes each OpenSSL call.

use super::{
    BeamError, BifResult, Hash, HashMap, Id, Md, PKey, Reference, Scheduler, Signer, Term,
    atom_name, c_error, cmac_cipher, error, iodata_bytes, lock,
};
use openssl::cipher_ctx::CipherCtx;
use openssl::error::ErrorStack;
use openssl::md_ctx::MdCtx;
use openssl::pkey::Private;

/// The MAC contexts of the node, as the `mac_context` resources of
/// `mac.c:514-556`. ERTS frees a context when no term refers to it. PBeam
/// has no garbage collection of resources, so it keeps each context until
/// the end of the run. A context changes in place: `mac_update` gives back
/// its argument (`mac.c:823-848`).
#[derive(Default)]
struct MacContexts {
    /// The state of each context resource.
    by_reference: HashMap<Reference, MacContext>,
}

/// The state of one MAC context.
enum MacContext {
    /// An HMAC. `ended` is true after a final: `HMAC_Final` ends the digest
    /// of the context (OpenSSL 3.5.5 `hmac.c`), so each later update and final
    /// fails.
    Hmac { context: MdCtx, ended: bool },
    /// A CMAC, as the OpenSSL 3.5.5 `CMAC_CTX` (`cmac.c:179-276`).
    Cmac(Cmac),
    /// A Poly1305 MAC. After a final, `ended` gives the count of bytes that
    /// later updates keep in the block buffer of the context, which
    /// `Poly1305_Final` clears.
    Poly1305 {
        context: MdCtx,
        ended: Option<usize>,
    },
}

/// The state of `CMAC_CTX`: the CBC context of the cipher with its chain,
/// the subkeys, and the last block with its count of bytes.
struct Cmac {
    cipher: CipherCtx,
    k1: Vec<u8>,
    k2: Vec<u8>,
    last_block: Vec<u8>,
    last_length: usize,
}

impl Cmac {
    /// Start a CMAC with the CBC `cipher` and `key`, as `ossl_cmac_init`
    /// (OpenSSL 3.5.5 `cmac.c:111-171`): the subkeys come from the block of
    /// zeros, and the chain starts again at zero.
    fn new(cipher: &openssl::cipher::CipherRef, key: &[u8]) -> Result<Self, ErrorStack> {
        let mut context = CipherCtx::new()?;
        let zero = vec![0; cipher.iv_length()];
        context.encrypt_init(Some(cipher), Some(key), Some(&zero))?;
        context.set_padding(false);
        let mut cmac = Cmac {
            cipher: context,
            k1: Vec::new(),
            k2: Vec::new(),
            last_block: zero.clone(),
            last_length: 0,
        };
        let l = cmac.encrypt(&zero);
        cmac.k1 = subkey(&l);
        cmac.k2 = subkey(&cmac.k1);
        cmac.cipher.encrypt_init(None, None, Some(&zero))?;
        Ok(cmac)
    }

    /// Encrypt whole blocks with the CBC chain of the context, as
    /// `EVP_Cipher`.
    fn encrypt(&mut self, blocks: &[u8]) -> Vec<u8> {
        let mut output = vec![0; blocks.len() + self.last_block.len()];
        let length = self
            .cipher
            .cipher_update(blocks, Some(&mut output))
            .expect("OpenSSL encrypts whole CBC blocks");
        output.truncate(length);
        output
    }

    /// Add `data`, as `CMAC_Update` (OpenSSL 3.5.5 `cmac.c:179-245`). The
    /// last block, complete or not, stays for the final.
    fn update(&mut self, mut data: &[u8]) {
        if data.is_empty() {
            return;
        }
        let size = self.last_block.len();
        if self.last_length > 0 {
            let count = (size - self.last_length).min(data.len());
            self.last_block[self.last_length..self.last_length + count]
                .copy_from_slice(&data[..count]);
            self.last_length += count;
            data = &data[count..];
            if data.is_empty() {
                return;
            }
            let block = self.last_block.clone();
            self.encrypt(&block);
        }
        let whole = (data.len() - 1) / size * size;
        self.encrypt(&data[..whole]);
        data = &data[whole..];
        self.last_block[..data.len()].copy_from_slice(data);
        self.last_length = data.len();
    }

    /// Give the CMAC, as `CMAC_Final` (OpenSSL 3.5.5 `cmac.c:247-276`). As
    /// OpenSSL, the final pads the last block in place and moves the CBC
    /// chain, so a second final gives another value.
    fn finish(&mut self) -> Vec<u8> {
        let size = self.last_block.len();
        let key = if self.last_length == size {
            &self.k1
        } else {
            self.last_block[self.last_length] = 0x80;
            self.last_block[self.last_length + 1..].fill(0);
            &self.k2
        };
        let block: Vec<u8> = self
            .last_block
            .iter()
            .zip(key)
            .map(|(a, b)| a ^ b)
            .collect();
        self.encrypt(&block)
    }
}

/// Give the next CMAC subkey of `block`, as `make_kn` (OpenSSL 3.5.5
/// `cmac.c:41-52`): `block` shifted left by one bit, with the constant of
/// the block size when the high bit was set.
///
/// # Examples
///
/// ```rust,ignore
/// assert_eq!(subkey(&[0x80, 0, 0, 0, 0, 0, 0, 0]), vec![0, 0, 0, 0, 0, 0, 0, 0x1b]);
/// ```
fn subkey(block: &[u8]) -> Vec<u8> {
    let mut output: Vec<u8> = block
        .iter()
        .zip(block.iter().skip(1).chain([&0]))
        .map(|(byte, next)| (byte << 1) | (next >> 7))
        .collect();
    if block[0] & 0x80 != 0 {
        let last = output.len() - 1;
        output[last] ^= if block.len() == 16 { 0x87 } else { 0x1b };
    }
    output
}

/// Run `crypto:mac_init_nif(Type, SubType, Key)` as the ERTS NIF
/// `mac_init_nif` with the OpenSSL 3 API (`mac.c:564-798`). ERTS checks
/// `Key` (`mac.c:594-598`), then the MAC type and its key length
/// (`mac.c:600-607`), then the subtype, and a failed `EVP_MAC_init` raises
/// the `error` of `mac.c:749-750`, such as for an HMAC of `md4` or SHAKE, or
/// a CMAC of a cipher that is not CBC.
///
/// # Errors
///
/// A full reference counter gives the error of `Scheduler::make_ref`.
pub(super) fn mac_init_nif(
    scheduler: &mut Scheduler,
    mac_type: &Term,
    sub_type: &Term,
    key: &Term,
) -> Result<BifResult, BeamError> {
    let Some(key) = iodata_bytes(key) else {
        return c_error(scheduler, "badarg", "mac.c", 596, 2, "Bad key");
    };
    let context = match atom_name(scheduler, mac_type) {
        "hmac" => hmac_context(scheduler, sub_type, &key),
        "cmac" => cmac_context(scheduler, sub_type, &key),
        "poly1305" if key.len() == 32 => Ok(Some(poly1305_context(&key))),
        "poly1305" => return c_error(scheduler, "badarg", "mac.c", 605, 2, "Bad key length"),
        _ => {
            let message = "Unknown mac algorithm";
            return c_error(scheduler, "badarg", "mac.c", 603, 0, message);
        }
    };
    match context {
        Ok(Some(context)) => {
            let reference = scheduler.make_magic_ref()?;
            lock(&scheduler.node().resources.kind::<MacContexts>())
                .by_reference
                .insert(reference, context);
            Ok(BifResult::Value(Term::Reference(reference)))
        }
        Ok(None) => c_error(
            scheduler,
            "error",
            "mac.c",
            750,
            -1,
            "Can't initialize EVP_MAC_CTX",
        ),
        Err(raise) => raise,
    }
}

/// The result of the start of a context: the context, or `None` for a
/// failed `EVP_MAC_init`, inside `Ok`; or the raise of a failed check of
/// the subtype in `Err`.
type ContextResult = Result<Option<MacContext>, Result<BifResult, BeamError>>;

/// Start an HMAC context of the digest `sub_type` (`mac.c:627-660`). A name
/// that is not a digest raises the `badarg` of `mac.c:633`. ERTS gives the
/// OpenSSL 3 name of the digest to `EVP_MAC_init`, which cannot fetch MD4 or
/// make an HMAC of SHAKE.
fn hmac_context(scheduler: &mut Scheduler, sub_type: &Term, key: &[u8]) -> ContextResult {
    let name = atom_name(scheduler, sub_type);
    let Some(hash) = Hash::from_name(name) else {
        if name == "md4" {
            return Ok(None);
        }
        let message = "Bad digest algorithm for HMAC";
        return Err(c_error(scheduler, "badarg", "mac.c", 633, 1, message));
    };
    // HMAC pads the key with zero bytes to the block size (RFC 2104), so
    // the empty key, which `EVP_PKEY_new_mac_key` rejects and `EVP_MAC_init`
    // takes, gives the MAC of the key <<0>>.
    let key = if key.is_empty() { &[0][..] } else { key };
    let key = PKey::hmac(key).expect("OpenSSL makes each HMAC key");
    let digest = Md::fetch(None, hash.fetch_name(), None).expect("OpenSSL fetches each `Hash`");
    let mut context = MdCtx::new().expect("OpenSSL makes a digest context");
    let started = context.digest_sign_init(Some(&digest), &key).is_ok();
    Ok(started.then_some(MacContext::Hmac {
        context,
        ended: false,
    }))
}

/// Start a CMAC context of the cipher `sub_type` (`mac.c:667-699`), with
/// the checks of `mac.c:670-690`. A cipher that `EVP_MAC_init` does not take
/// for CMAC, such as a cipher that is not CBC, gives `None`. PBeam asks
/// OpenSSL with the CMAC key of the one-shot `mac_nif/4`.
fn cmac_context(scheduler: &mut Scheduler, sub_type: &Term, key: &[u8]) -> ContextResult {
    let cipher = cmac_cipher(scheduler, sub_type, key.len(), [673, 676, 688])?;
    let usable = openssl::symm::Cipher::from_nid(cipher.nid())
        .ok_or_else(ErrorStack::get)
        .and_then(|symm| PKey::cmac(&symm, key))
        .and_then(|key| Signer::new_without_digest(&key).map(|_| ()));
    Ok(usable
        .ok()
        .map(|()| Cmac::new(&cipher, key).expect("OpenSSL starts each CBC cipher of a CMAC key"))
        .map(MacContext::Cmac))
}

/// Start a Poly1305 context with the 32-byte `key` (`mac.c:707-713`).
fn poly1305_context(key: &[u8]) -> MacContext {
    let key = PKey::<Private>::private_key_from_raw_bytes(key, Id::POLY1305)
        .expect("OpenSSL makes each 32-byte Poly1305 key");
    let mut context = MdCtx::new().expect("OpenSSL makes a digest context");
    context
        .digest_sign_init::<Private>(None, &key)
        .expect("OpenSSL starts each Poly1305 key");
    MacContext::Poly1305 {
        context,
        ended: None,
    }
}

/// The result of an operation on a context, which the NIF gives after it
/// frees the lock of the store.
enum Outcome {
    /// The context is not a MAC context resource.
    BadRef,
    /// The operation gives this binary, or the argument for `None`.
    Value(Option<Vec<u8>>),
    /// The HMAC context has ended.
    Ended,
    /// The pinned ERTS stops: `Poly1305_Final` cleared the context.
    Stops,
}

/// Run `operation` on the context of `term`.
fn with_context(
    scheduler: &Scheduler,
    term: &Term,
    operation: impl FnOnce(&mut MacContext) -> Outcome,
) -> Outcome {
    let Term::Reference(reference) = term else {
        return Outcome::BadRef;
    };
    let store = scheduler.node().resources.kind::<MacContexts>();
    let mut store = lock(&store);
    store
        .by_reference
        .get_mut(reference)
        .map_or(Outcome::BadRef, operation)
}

/// Run `crypto:mac_update_nif(Context, Data)` as the ERTS NIF
/// `mac_update_nif` (`mac.c:802-848`): add `Data` to the context and give
/// back `Context`. ERTS checks `Data` first (`mac.c:806-810`), then the
/// context (`mac.c:829-830`). An update of an ended HMAC context raises the
/// `error` of `mac.c:836-840`.
///
/// # Errors
///
/// An update of a Poly1305 context after its final that fills a block of
/// 16 bytes stops the run: it stops the pinned ERTS (row "NIF stubs" of
/// `docs/erts-differences.md`).
pub(super) fn mac_update_nif(
    scheduler: &mut Scheduler,
    context: &Term,
    data: &Term,
) -> Result<BifResult, BeamError> {
    let data = iodata_bytes(data);
    let failed = [
        (data.is_none(), 807, "Bad text"),
        (
            data.as_ref()
                .is_some_and(|data| data.len() > i32::MAX as usize),
            810,
            "Too long text",
        ),
    ];
    if let Some(&(_, line, message)) = failed.iter().find(|check| check.0) {
        return c_error(scheduler, "badarg", "mac.c", line, 1, message);
    }
    let data = data.unwrap_or_default();
    let outcome = with_context(scheduler, context, |state| match state {
        // `EVP_DigestUpdate` returns 1 for an empty update before its
        // FINALISED check (OpenSSL 3.x `crypto/evp/digest.c`).
        MacContext::Hmac { ended: true, .. } if !data.is_empty() => Outcome::Ended,
        MacContext::Hmac { ended: true, .. } => Outcome::Value(None),
        MacContext::Hmac { context, .. }
        | MacContext::Poly1305 {
            context,
            ended: None,
        } => {
            context
                .digest_sign_update(&data)
                .expect("OpenSSL updates each MAC");
            Outcome::Value(None)
        }
        MacContext::Poly1305 {
            ended: Some(buffered),
            ..
        } => {
            *buffered += data.len();
            if *buffered >= 16 {
                Outcome::Stops
            } else {
                Outcome::Value(None)
            }
        }
        MacContext::Cmac(cmac) => {
            cmac.update(&data);
            Outcome::Value(None)
        }
    });
    result(
        scheduler,
        outcome,
        context,
        (830, 840, "mac update"),
        "mac_update_nif/2",
    )
}

/// Run `crypto:mac_final_nif(Context)` as the ERTS NIF `mac_final_nif`
/// (`mac.c:852-887`): the MAC of the context, which `EVP_MAC_final`
/// changes. A second final of an HMAC context raises the `error` of
/// `mac.c:873-879`, and a second final of a CMAC context gives the CMAC of
/// the moved chain (OpenSSL 3.5.5 `cmac.c:247-276`).
///
/// # Errors
///
/// A second final of a Poly1305 context stops the run: it stops the pinned
/// ERTS (row "NIF stubs" of `docs/erts-differences.md`).
pub(super) fn mac_final_nif(
    scheduler: &mut Scheduler,
    context: &Term,
) -> Result<BifResult, BeamError> {
    let outcome = with_context(scheduler, context, |state| match state {
        MacContext::Hmac { ended: true, .. } => Outcome::Ended,
        MacContext::Poly1305 { ended: Some(_), .. } => Outcome::Stops,
        MacContext::Hmac { context, ended } => {
            *ended = true;
            Outcome::Value(Some(sign(context)))
        }
        MacContext::Poly1305 { context, ended } => {
            *ended = Some(0);
            Outcome::Value(Some(sign(context)))
        }
        MacContext::Cmac(cmac) => Outcome::Value(Some(cmac.finish())),
    });
    result(
        scheduler,
        outcome,
        context,
        (860, 879, "Signing"),
        "mac_final_nif/1",
    )
}

/// Give the MAC of `context` with `EVP_DigestSignFinal`.
fn sign(context: &mut MdCtx) -> Vec<u8> {
    let mut output = Vec::new();
    context
        .digest_sign_final_to_vec(&mut output)
        .expect("OpenSSL ends each MAC");
    output
}

/// Give the result of `outcome` of the NIF `name`: the binary or `context`,
/// the `Bad ref` error of line `lines.0`, or the `error` of an ended HMAC
/// at line `lines.1`.
fn result(
    scheduler: &mut Scheduler,
    outcome: Outcome,
    context: &Term,
    lines: (i64, i64, &str),
    name: &str,
) -> Result<BifResult, BeamError> {
    match outcome {
        Outcome::BadRef => c_error(scheduler, "badarg", "mac.c", lines.0, 0, "Bad ref"),
        Outcome::Ended => c_error(scheduler, "error", "mac.c", lines.1, -1, lines.2),
        Outcome::Value(Some(mac)) => Ok(BifResult::Value(Term::binary(mac))),
        Outcome::Value(None) => Ok(BifResult::Value(context.clone())),
        Outcome::Stops => Err(error(format!(
            "crypto:{name} on a Poly1305 context after its final is not supported: the pinned ERTS stops"
        ))),
    }
}

//! The engine NIFs of `crypto` (`engine.c:353-1045`). PBeam has no engine
//! API: OpenSSL 3 deprecates engines, and the safe `openssl` crate does
//! not wrap them. PBeam gives the ERTS result of each call that needs no
//! loaded engine, and a runtime error for each call that would load one.

use super::*;

/// The engine method atoms of `engine_get_all_methods_nif/0`
/// (`engine.c:868-913`): the `ENGINE_METHOD_*` macros that OpenSSL 3
/// defines, in the ERTS order.
const METHODS: [&str; 9] = [
    "engine_method_rsa",
    "engine_method_dsa",
    "engine_method_dh",
    "engine_method_rand",
    "engine_method_ciphers",
    "engine_method_digests",
    "engine_method_pkey_meths",
    "engine_method_pkey_asn1_meths",
    "engine_method_ec",
];

/// Return the runtime error of an engine call that PBeam cannot make.
fn unsupported(name: &str, arity: u32, part: &str) -> BeamError {
    error(format!(
        "crypto:{name}/{arity} NIF is not supported for {part}: the safe openssl crate has no engine API"
    ))
}

/// Run `engine_by_id_nif/1` (`engine.c:353-406`). An argument that is not
/// a binary gives `badarg` (`engine.c:366-367`). `ENGINE_by_id`
/// (`engine.c:375`) adds the built-in engine `dynamic` to the OpenSSL
/// list and can load an engine library, so each binary gives a runtime
/// error.
pub(super) fn engine_by_id_nif(
    _: &mut Scheduler,
    arguments: &[Term],
) -> Result<BifResult, BeamError> {
    if arguments[0].binary_bytes().is_none() {
        return Ok(BifResult::badarg());
    }
    Err(unsupported("engine_by_id_nif", 1, "an engine lookup"))
}

/// Run `engine_load_dynamic_nif/0` (`engine.c:469-483`). The call
/// `OPENSSL_init_crypto(OPENSSL_INIT_ENGINE_DYNAMIC, NULL)` adds the engine
/// `dynamic` to the OpenSSL list, so it gives a runtime error.
pub(super) fn engine_load_dynamic_nif(
    _: &mut Scheduler,
    _: &[Term],
) -> Result<BifResult, BeamError> {
    Err(unsupported(
        "engine_load_dynamic_nif",
        0,
        "the engine dynamic",
    ))
}

/// Run an engine NIF whose first argument is an engine resource
/// (`engine.c:417`, `441`, `500`, `576`, `605`, `634`, `678`, `748`,
/// `801`, `840`). PBeam makes no engine resource, so
/// `enif_get_resource` fails for each term and the NIF gives `badarg`.
pub(super) fn engine_resource_nif(_: &mut Scheduler, _: &[Term]) -> Result<BifResult, BeamError> {
    Ok(BifResult::badarg())
}

/// Run `engine_get_first_nif/0` (`engine.c:696-733`). PBeam adds no engine
/// to the OpenSSL list (each call that adds one gives a runtime error), so `ENGINE_get_first` gives NULL and the NIF gives
/// `{ok, <<>>}` (`engine.c:706-710`).
pub(super) fn engine_get_first_nif(_: &mut Scheduler, _: &[Term]) -> Result<BifResult, BeamError> {
    Ok(BifResult::Value(Term::tuple(vec![
        Term::static_atom("ok"),
        Term::binary(Vec::new()),
    ])))
}

/// Run `engine_get_all_methods_nif/0` (`engine.c:868-913`).
pub(super) fn engine_get_all_methods_nif(
    scheduler: &mut Scheduler,
    _: &[Term],
) -> Result<BifResult, BeamError> {
    let mut methods = Vec::with_capacity(METHODS.len());
    for name in METHODS {
        methods.push(Term::Atom(scheduler.atoms.put(name)?));
    }
    Ok(BifResult::Value(Term::list_from(methods)))
}

/// Run `ensure_engine_loaded_nif/2` (`engine.c:919-1045`). An argument
/// that is not a binary gives `badarg` (`engine.c:936-947`). The load of
/// the engine gives a runtime error.
pub(super) fn ensure_engine_loaded_nif(
    _: &mut Scheduler,
    arguments: &[Term],
) -> Result<BifResult, BeamError> {
    if arguments
        .iter()
        .any(|argument| argument.binary_bytes().is_none())
    {
        return Ok(BifResult::badarg());
    }
    Err(unsupported("ensure_engine_loaded_nif", 2, "an engine load"))
}

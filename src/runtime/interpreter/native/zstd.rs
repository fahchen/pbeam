//! Native bindings of the `zstd` NIFs
//! (`erts/emulator/nifs/common/zstd_nif.c`), which `zstd:on_load/0` loads
//! (`zstd.erl:251-252`).
//!
//! The pinned `zstd` module is `lib/stdlib/src/zstd.erl` in OTP 29.1.1.
//! PBeam binds the NIFs of a compression or decompression context: the
//! context, its parameters, the stream calls, the reset, and the close. Each
//! context is a reference resource in [`ZstdContexts`], as each `zlib`
//! stream is. The codec is libzstd 1.5.7 of the `zstd-sys` crate, the
//! version that pinned ERTS vendors (`erts/emulator/zstd/vendor.info`), and
//! PBeam calls it through the safe API of `zstd-safe`. The compressed bytes
//! are the bytes of pinned ERTS for the same input and parameters.
//!
//! The dictionary functions and the parameter reads have no binding:
//! `load_compress_dictionary_nif/2`, `ref_compress_dictionary_nif/2`,
//! `get_compress_parameter_nif/2`, `load_decompress_dictionary_nif/2`,
//! `ref_decompress_dictionary_nif/2`, `get_decompress_parameter_nif/2`,
//! `create_cdict_nif/2`, `create_ddict_nif/1`, the four `getDictId_*_nif/1`,
//! and `get_frame_header_nif/1`. A call of one stops the run with the
//! runtime error of the static library (`Library::unbound_fails`). The load
//! callback of the library returns 0 for each load info
//! (`zstd_nif.c:1008-1070`).

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use ::zstd::zstd_safe::{
    CCtx, CParameter, DCtx, DParameter, InBuffer, OutBuffer, ResetDirective, SafeResult, Strategy,
    find_decompressed_size, get_error_name, zstd_sys::ZSTD_EndDirective,
};

use super::{BeamError, BifResult, Binding, ExceptionClass, Job, Scheduler, Term, error};
use crate::runtime::resources::lock;
use crate::term::{ProcessId, Reference};

/// The bits of one byte, for the bit range of a binary.
const BITS_PER_BYTE: usize = 8;

/// The error code of `ZSTD_error_parameter_outOfBound`, as libzstd returns
/// it: the negative error number as a `size_t` (`zstd_errors.h:75`,
/// `error_private.h` `ERROR`).
const PARAMETER_OUT_OF_BOUND: usize = 0usize.wrapping_sub(42);

/// The functions of the ERTS `zstd` NIF library, as `(name, arity)` in the
/// order of `nif_funcs` (`zstd_nif.c:1082-1116`). `erlang:load_nif/2`
/// checks each of them in this order. The flags of
/// `load_compress_dictionary_nif/2`, `load_decompress_dictionary_nif/2`,
/// `create_cdict_nif/2`, and `create_ddict_nif/1` are
/// `ERL_NIF_DIRTY_JOB_CPU_BOUND` (`zstd_nif.c:1086-1087`, `1093-1094`,
/// `1108-1109`), and the flags of each other function are 0.
///
/// # Examples
///
/// ```rust,ignore
/// assert_eq!(FUNCTIONS[0], ("init_compress_nif", 0));
/// assert_eq!(FUNCTIONS[23], ("get_frame_header_nif", 1));
/// ```
pub(super) const FUNCTIONS: [(&str, u32); 24] = [
    ("init_compress_nif", 0),
    ("init_decompress_nif", 0),
    ("load_compress_dictionary_nif", 2),
    ("ref_compress_dictionary_nif", 2),
    ("set_compress_parameter_nif", 3),
    ("get_compress_parameter_nif", 2),
    ("set_pledged_src_size_nif", 2),
    ("load_decompress_dictionary_nif", 2),
    ("ref_decompress_dictionary_nif", 2),
    ("set_decompress_parameter_nif", 3),
    ("get_decompress_parameter_nif", 2),
    ("compress_stream_nif", 3),
    ("decompress_stream_nif", 3),
    ("compress_reset_nif", 1),
    ("decompress_reset_nif", 1),
    ("compress_close_nif", 1),
    ("decompress_close_nif", 1),
    ("create_cdict_nif", 2),
    ("create_ddict_nif", 1),
    ("getDictId_fromCDict_nif", 1),
    ("getDictId_fromDDict_nif", 1),
    ("getDictId_fromDict_nif", 1),
    ("getDictId_fromFrame_nif", 1),
    ("get_frame_header_nif", 1),
];

/// The registered bindings of `zstd`, sorted by name, then by arity. Each
/// bound function has the flags 0 in ERTS, so each binding is `Normal`.
///
/// # Examples
///
/// ```rust,ignore
/// assert_eq!((BINDINGS[0].name, BINDINGS[0].arity), ("compress_close_nif", 1));
/// ```
pub(super) const BINDINGS: &[Binding] = &[
    Binding {
        name: "compress_close_nif",
        arity: 1,
        job: Job::Normal,
        run: |scheduler, arguments| compress_close_nif(scheduler, &arguments[0]),
    },
    Binding {
        name: "compress_reset_nif",
        arity: 1,
        job: Job::Normal,
        run: |scheduler, arguments| compress_reset_nif(scheduler, &arguments[0]),
    },
    Binding {
        name: "compress_stream_nif",
        arity: 3,
        job: Job::Normal,
        run: |scheduler, arguments| {
            compress_stream_nif(scheduler, &arguments[0], &arguments[1], &arguments[2])
        },
    },
    Binding {
        name: "decompress_close_nif",
        arity: 1,
        job: Job::Normal,
        run: |scheduler, arguments| decompress_close_nif(scheduler, &arguments[0]),
    },
    Binding {
        name: "decompress_reset_nif",
        arity: 1,
        job: Job::Normal,
        run: |scheduler, arguments| decompress_reset_nif(scheduler, &arguments[0]),
    },
    Binding {
        name: "decompress_stream_nif",
        arity: 3,
        job: Job::Normal,
        run: |scheduler, arguments| {
            decompress_stream_nif(scheduler, &arguments[0], &arguments[1], &arguments[2])
        },
    },
    Binding {
        name: "init_compress_nif",
        arity: 0,
        job: Job::Normal,
        run: |scheduler, _| init_compress_nif(scheduler),
    },
    Binding {
        name: "init_decompress_nif",
        arity: 0,
        job: Job::Normal,
        run: |scheduler, _| init_decompress_nif(scheduler),
    },
    Binding {
        name: "set_compress_parameter_nif",
        arity: 3,
        job: Job::Normal,
        run: |scheduler, arguments| {
            set_compress_parameter_nif(scheduler, &arguments[0], &arguments[1], &arguments[2])
        },
    },
    Binding {
        name: "set_decompress_parameter_nif",
        arity: 3,
        job: Job::Normal,
        run: |scheduler, arguments| {
            set_decompress_parameter_nif(scheduler, &arguments[0], &arguments[1], &arguments[2])
        },
    },
    Binding {
        name: "set_pledged_src_size_nif",
        arity: 2,
        job: Job::Normal,
        run: |scheduler, arguments| {
            set_pledged_src_size_nif(scheduler, &arguments[0], &arguments[1])
        },
    },
];

/// The `zstd` contexts of the node, as the `compress_type` and
/// `decompress_type` resources of ERTS (`zstd_nif.c:47-48`). A reference of
/// one map is not a resource of the other type. ERTS frees a context when no
/// term refers to it. PBeam has no garbage collection of resources, so it
/// keeps each context until `close/1` or the end of the run.
#[derive(Default)]
pub(super) struct ZstdContexts {
    /// Each open compression context by its reference.
    compress: HashMap<Reference, Context<Compressor>>,
    /// Each open decompression context by its reference.
    decompress: HashMap<Reference, Context<Decompressor>>,
}

/// One open context, as a `ZstdCtx` with a handle that is not `NULL`
/// (`zstd_nif.c:66-76`). The close of a context removes it from its map,
/// so a later call finds no context and raises `badarg`, as a `NULL` handle
/// does (`zstd_nif.c:191-194`).
struct Context<H> {
    /// The libzstd state of the context.
    handle: H,
    /// The process that made the context (`zstd_nif.c:156`).
    owner: ProcessId,
}

/// The state of a compression context.
struct Compressor {
    /// The libzstd compression context.
    cctx: CCtx<'static>,
    /// The `ZSTD_c_strategy` value of the requested parameters: 0 for the
    /// default strategy. The safe API takes only the strategies 1 to 9, so
    /// PBeam keeps the value to know when the `default` strategy is the
    /// current value.
    strategy: i32,
}

/// The state of a decompression context.
struct Decompressor {
    /// The libzstd decompression context.
    dctx: DCtx<'static>,
    /// True while a frame is in decompression, as `in_use` of `ZstdCtx`
    /// (`zstd_nif.c:75`). The first stream call of a frame sets it, and a
    /// reset or an exception clears it.
    in_use: bool,
}

/// The result of a `zstd` NIF before PBeam makes its atoms and terms.
enum Native {
    /// The atom `ok`.
    Ok,
    /// An `error` exception with the atom of the name as the reason.
    Raise(&'static str),
    /// The exception `{badarg, {invalid_key, Key}}` (`zstd_nif.c:371-377`).
    InvalidKey(Term),
    /// The exception `{badarg, {invalid_value, Value}}`
    /// (`zstd_nif.c:402-406`).
    InvalidValue(Term),
    /// The exception `{zstd_error, Name}` of a libzstd error code, as
    /// `make_zstd_error` makes it (`zstd_nif.c:100-110`).
    Zstd(usize),
    /// The result of a stream call: `{Tag, Output}`, or
    /// `{continue, Remainder, Output}` when the input is not consumed
    /// (`zstd_nif.c:646-671`).
    Codec {
        /// The tag atom: `continue`, `flush`, or `done`.
        tag: &'static str,
        /// The sub-binary of the input that the codec did not consume.
        remainder: Option<Term>,
        /// The output bytes.
        output: Vec<u8>,
    },
}

impl Native {
    /// Return whether the result is an exception.
    fn is_exception(&self) -> bool {
        !matches!(self, Native::Ok | Native::Codec { .. })
    }
}

/// Return the result term or exception of `native`.
///
/// # Examples
///
/// ```rust,ignore
/// let result = result(&mut scheduler, Native::Ok)?;
/// assert!(matches!(result, BifResult::Value(Term::Atom(_))));
/// ```
///
/// # Errors
///
/// A new result atom in a full atom table gives the error of
/// `AtomTable::put`.
fn result(scheduler: &mut Scheduler, native: Native) -> Result<BifResult, BeamError> {
    let atom =
        |name: &str| -> Result<Term, BeamError> { Ok(Term::Atom(scheduler.atoms.put(name)?)) };
    let raise = |reason| BifResult::Raise {
        class: ExceptionClass::Error,
        reason,
    };
    Ok(match native {
        Native::Ok => BifResult::Value(atom("ok")?),
        Native::Raise(name) => raise(atom(name)?),
        Native::InvalidKey(key) => raise(Term::tuple(vec![
            atom("badarg")?,
            Term::tuple(vec![atom("invalid_key")?, key]),
        ])),
        Native::InvalidValue(value) => raise(Term::tuple(vec![
            atom("badarg")?,
            Term::tuple(vec![atom("invalid_value")?, value]),
        ])),
        Native::Zstd(code) => raise(Term::tuple(vec![
            atom("zstd_error")?,
            Term::binary(get_error_name(code).as_bytes().to_vec()),
        ])),
        Native::Codec {
            tag,
            remainder,
            output,
        } => {
            let mut elements = vec![atom(tag)?];
            elements.extend(remainder);
            elements.push(Term::binary(output));
            BifResult::Value(Term::tuple(elements))
        }
    })
}

/// Return the open context of `term` in `contexts`, as `get_zstd_context`
/// does (`zstd_nif.c:184-204`): the reason `badarg` for a term that is not
/// an open context of the map, and `not_on_controlling_process` for a call
/// of another process than the owner.
///
/// # Examples
///
/// ```rust,ignore
/// assert_eq!(open(&mut contexts.compress, &Term::Nil, current).err(), Some("badarg"));
/// ```
fn open<'a, H>(
    contexts: &'a mut HashMap<Reference, Context<H>>,
    term: &Term,
    current: ProcessId,
) -> Result<&'a mut H, &'static str> {
    let context = match term {
        Term::Reference(reference) => contexts.get_mut(reference),
        _ => None,
    };
    let Some(context) = context else {
        return Err("badarg");
    };
    if context.owner != current {
        return Err("not_on_controlling_process");
    }
    Ok(&mut context.handle)
}

/// Return the contexts of the node and the current process.
fn contexts(scheduler: &mut Scheduler) -> (Arc<Mutex<ZstdContexts>>, ProcessId) {
    let current = scheduler.current_process().id;
    (scheduler.node().resources.kind::<ZstdContexts>(), current)
}

/// Return the C `int` of a term, as `enif_get_int` accepts it.
///
/// # Examples
///
/// ```rust,ignore
/// assert_eq!(integer(&Term::Integer(2)), Some(2));
/// assert_eq!(integer(&Term::Integer(i64::from(i32::MAX) + 1)), None);
/// ```
fn integer(term: &Term) -> Option<i32> {
    match term {
        Term::Integer(value) => i32::try_from(*value).ok(),
        _ => None,
    }
}

/// Run one stream call of a context, as `codec_nif` does
/// (`zstd_nif.c:586-672`): check the flush and the input, run `step` into an
/// output buffer of `chunk_size` bytes until the input is consumed or the
/// buffer is full, and tag the result. `step` takes the flush value and
/// returns the result of the libzstd stream function.
///
/// # Examples
///
/// ```rust,ignore
/// let native = codec(&data, &Term::Integer(2), CCtx::out_size(), |out, input, _| {
///     cctx.compress_stream2(out, input, ZSTD_EndDirective::ZSTD_e_end)
/// });
/// ```
fn codec(
    data: &Term,
    flush: &Term,
    chunk_size: usize,
    mut step: impl FnMut(&mut OutBuffer<'_, [u8]>, &mut InBuffer<'_>, i32) -> SafeResult,
) -> Native {
    let Some(flush) = integer(flush) else {
        return Native::Raise("badarg");
    };
    // `enif_inspect_binary` accepts only a whole number of bytes.
    let (Term::Binary(binary), Some(bytes)) = (data, data.binary_bytes()) else {
        return Native::Raise("badarg");
    };
    let mut output = vec![0; chunk_size];
    let mut out = OutBuffer::around(&mut output[..]);
    let mut input = InBuffer::around(bytes);
    let remaining = loop {
        let remaining = match step(&mut out, &mut input, flush) {
            Ok(remaining) => remaining,
            Err(code) => return Native::Zstd(code),
        };
        if input.pos() >= bytes.len() || out.pos() >= chunk_size {
            break remaining;
        }
    };
    let produced = out.pos();
    let consumed = input.pos();
    output.truncate(produced);
    if flush != 0 && consumed == bytes.len() {
        // The output buffer is full and libzstd has more data: the caller
        // calls again (`zstd_nif.c:646-655`).
        let tag = if produced == chunk_size && remaining > 0 {
            "flush"
        } else {
            "done"
        };
        return Native::Codec {
            tag,
            remainder: None,
            output,
        };
    }
    let remainder = (consumed < bytes.len())
        .then(|| Term::Binary(binary.bits(consumed * BITS_PER_BYTE, bytes.len() * BITS_PER_BYTE)));
    Native::Codec {
        tag: "continue",
        remainder,
        output,
    }
}

/// Run `init_compress_nif/0` (`zstd_nif.c:146-163`): a new compression
/// context with the default parameters, owned by the caller.
///
/// # Examples
///
/// ```rust,ignore
/// assert!(matches!(init_compress_nif(&mut scheduler)?, BifResult::Value(Term::Reference(_))));
/// ```
///
/// # Errors
///
/// A full reference counter gives the error of `Scheduler::make_ref`.
fn init_compress_nif(scheduler: &mut Scheduler) -> Result<BifResult, BeamError> {
    let reference = scheduler.make_magic_ref()?;
    let (kind, owner) = contexts(scheduler);
    let mut contexts = lock(&kind);
    let handle = Compressor {
        cctx: CCtx::create(),
        strategy: 0,
    };
    contexts
        .compress
        .insert(reference, Context { handle, owner });
    Ok(BifResult::Value(Term::Reference(reference)))
}

/// Run `init_decompress_nif/0` (`zstd_nif.c:165-182`): a new decompression
/// context with the default parameters, owned by the caller.
///
/// # Examples
///
/// ```rust,ignore
/// assert!(matches!(init_decompress_nif(&mut scheduler)?, BifResult::Value(Term::Reference(_))));
/// ```
///
/// # Errors
///
/// A full reference counter gives the error of `Scheduler::make_ref`.
fn init_decompress_nif(scheduler: &mut Scheduler) -> Result<BifResult, BeamError> {
    let reference = scheduler.make_magic_ref()?;
    let (kind, owner) = contexts(scheduler);
    let mut contexts = lock(&kind);
    let handle = Decompressor {
        dctx: DCtx::create(),
        in_use: false,
    };
    contexts
        .decompress
        .insert(reference, Context { handle, owner });
    Ok(BifResult::Value(Term::Reference(reference)))
}

/// Return the compression parameter of a key and a value, as `get_cparam`
/// and the value checks of `set_compress_parameter_nif` give it
/// (`zstd_nif.c:318-433`). `Ok(None)` is the `default` strategy, which the
/// safe API cannot set.
fn compress_parameter(
    scheduler: &Scheduler,
    key: &Term,
    value: &Term,
) -> Result<Option<CParameter>, Native> {
    let name = match key {
        Term::Atom(atom) => scheduler.atoms.name_of(atom),
        _ => "",
    };
    let flag = |parameter: fn(bool) -> CParameter| match value {
        Term::Atom(atom) if scheduler.atoms.name_of(atom) == "true" => Ok(Some(parameter(true))),
        Term::Atom(atom) if scheduler.atoms.name_of(atom) == "false" => Ok(Some(parameter(false))),
        _ => Err(Native::InvalidValue(value.clone())),
    };
    // libzstd reads each `int` parameter as an `int`, and the safe API
    // converts the `u32` back to the same `int` (`zstd-safe` `set_parameter`).
    let int = |parameter: fn(u32) -> CParameter| match integer(value) {
        Some(int) => Ok(Some(parameter(int as u32))),
        None => Err(Native::InvalidValue(value.clone())),
    };
    match name {
        "compressionLevel" => match integer(value) {
            Some(level) => Ok(Some(CParameter::CompressionLevel(level))),
            None => Err(Native::InvalidValue(value.clone())),
        },
        "windowLog" => int(CParameter::WindowLog),
        "hashLog" => int(CParameter::HashLog),
        "chainLog" => int(CParameter::ChainLog),
        "searchLog" => int(CParameter::SearchLog),
        "minMatch" => int(CParameter::MinMatch),
        "targetLength" => int(CParameter::TargetLength),
        "strategy" => strategy(scheduler, value),
        "targetCBlockSize" => int(CParameter::TargetCBlockSize),
        "enableLongDistanceMatching" => flag(CParameter::EnableLongDistanceMatching),
        "ldmHashLog" => int(CParameter::LdmHashLog),
        "ldmMinMatch" => int(CParameter::LdmMinMatch),
        "ldmBucketSizeLog" => int(CParameter::LdmBucketSizeLog),
        "ldmHashRateLog" => int(CParameter::LdmHashRateLog),
        "contentSizeFlag" => flag(CParameter::ContentSizeFlag),
        "checksumFlag" => flag(CParameter::ChecksumFlag),
        "dictIDFlag" => flag(CParameter::DictIdFlag),
        _ => Err(Native::InvalidKey(key.clone())),
    }
}

/// Return the strategy parameter of a `strategy` value
/// (`zstd_nif.c:380-408`): `Ok(None)` for `default`.
fn strategy(scheduler: &Scheduler, value: &Term) -> Result<Option<CParameter>, Native> {
    let name = match value {
        Term::Atom(atom) => scheduler.atoms.name_of(atom),
        _ => "",
    };
    let strategy = match name {
        "default" => return Ok(None),
        "fast" => Strategy::ZSTD_fast,
        "dfast" => Strategy::ZSTD_dfast,
        "greedy" => Strategy::ZSTD_greedy,
        "lazy" => Strategy::ZSTD_lazy,
        "lazy2" => Strategy::ZSTD_lazy2,
        "btlazy2" => Strategy::ZSTD_btlazy2,
        "btopt" => Strategy::ZSTD_btopt,
        "btultra" => Strategy::ZSTD_btultra,
        "btultra2" => Strategy::ZSTD_btultra2,
        _ => return Err(Native::InvalidValue(value.clone())),
    };
    Ok(Some(CParameter::Strategy(strategy)))
}

/// Run `set_compress_parameter_nif/3` (`zstd_nif.c:358-442`): set one
/// parameter of a compression context and give `ok`. An unknown key raises
/// `{badarg, {invalid_key, Key}}`, a value of the wrong type raises
/// `{badarg, {invalid_value, Value}}`, and a value that libzstd rejects
/// raises `{zstd_error, Name}`.
///
/// # Examples
///
/// ```rust,ignore
/// let context = init_compress_nif(&mut scheduler)?;
/// set_compress_parameter_nif(&mut scheduler, &context, &level, &Term::Integer(5))?;
/// ```
///
/// # Errors
///
/// The `default` strategy after another strategy gives a runtime error:
/// the safe API of libzstd cannot set the strategy value 0 again. A new
/// result atom in a full atom table gives the error of `AtomTable::put`.
fn set_compress_parameter_nif(
    scheduler: &mut Scheduler,
    context: &Term,
    key: &Term,
    value: &Term,
) -> Result<BifResult, BeamError> {
    let parameter = compress_parameter(scheduler, key, value);
    let (kind, current) = contexts(scheduler);
    let mut contexts = lock(&kind);
    let native = match (open(&mut contexts.compress, context, current), parameter) {
        (Err(reason), _) => Native::Raise(reason),
        (Ok(_), Err(native)) => native,
        (Ok(compressor), Ok(None)) if compressor.strategy == 0 => Native::Ok,
        (Ok(_), Ok(None)) => {
            return Err(error(
                "zstd:set_compress_parameter_nif/3 cannot set the default strategy \
                 after another strategy",
            ));
        }
        (Ok(compressor), Ok(Some(parameter))) => match compressor.cctx.set_parameter(parameter) {
            Ok(_) => {
                if let CParameter::Strategy(strategy) = parameter {
                    compressor.strategy = strategy as i32;
                }
                Native::Ok
            }
            Err(code) => Native::Zstd(code),
        },
    };
    result(scheduler, native)
}

/// Run `set_pledged_src_size_nif/2` (`zstd_nif.c:499-516`): set the pledged
/// size of the next frame and give `ok`. A size that is not an unsigned
/// 64-bit integer raises `badarg`, and a frame in progress raises
/// `{zstd_error, Name}`.
///
/// # Examples
///
/// ```rust,ignore
/// let context = init_compress_nif(&mut scheduler)?;
/// set_pledged_src_size_nif(&mut scheduler, &context, &Term::Integer(3))?;
/// ```
///
/// # Errors
///
/// A new result atom in a full atom table gives the error of
/// `AtomTable::put`.
fn set_pledged_src_size_nif(
    scheduler: &mut Scheduler,
    context: &Term,
    size: &Term,
) -> Result<BifResult, BeamError> {
    // `enif_get_uint64` takes each integer from 0 to 2^64 - 1.
    let size = match size {
        Term::Integer(value) => u64::try_from(*value).ok(),
        Term::BigInt(value) => u64::try_from(value.as_ref()).ok(),
        _ => None,
    };
    let (kind, current) = contexts(scheduler);
    let mut contexts = lock(&kind);
    let native = match (open(&mut contexts.compress, context, current), size) {
        (Err(reason), _) => Native::Raise(reason),
        (Ok(_), None) => Native::Raise("badarg"),
        (Ok(compressor), Some(size)) => match compressor.cctx.set_pledged_src_size(Some(size)) {
            Ok(_) => Native::Ok,
            Err(code) => Native::Zstd(code),
        },
    };
    result(scheduler, native)
}

/// Run `set_decompress_parameter_nif/3` (`zstd_nif.c:526-554`): set one
/// parameter of a decompression context and give `ok`. A value that is not
/// a C `int` raises `badarg` before the key check, an unknown key raises
/// `{badarg, {invalid_key, Key}}`, and a value that libzstd rejects raises
/// `{zstd_error, Name}`.
///
/// # Examples
///
/// ```rust,ignore
/// let context = init_decompress_nif(&mut scheduler)?;
/// set_decompress_parameter_nif(&mut scheduler, &context, &window_log_max, &Term::Integer(20))?;
/// ```
///
/// # Errors
///
/// A new result atom in a full atom table gives the error of
/// `AtomTable::put`.
fn set_decompress_parameter_nif(
    scheduler: &mut Scheduler,
    context: &Term,
    key: &Term,
    value: &Term,
) -> Result<BifResult, BeamError> {
    let window_log_max =
        matches!(key, Term::Atom(atom) if scheduler.atoms.name_of(atom) == "windowLogMax");
    let (kind, current) = contexts(scheduler);
    let mut contexts = lock(&kind);
    let native = match (
        open(&mut contexts.decompress, context, current),
        integer(value),
    ) {
        (Err(reason), _) => Native::Raise(reason),
        (Ok(_), None) => Native::Raise("badarg"),
        (Ok(_), Some(_)) if !window_log_max => Native::InvalidKey(key.clone()),
        (Ok(decompressor), Some(value)) => {
            match decompressor
                .dctx
                .set_parameter(DParameter::WindowLogMax(value as u32))
            {
                Ok(_) => Native::Ok,
                Err(code) => Native::Zstd(code),
            }
        }
    };
    result(scheduler, native)
}

/// Run `compress_stream_nif/3` (`zstd_nif.c:685-710`): compress the binary
/// `data` with the end directive `flush` (0 continue, 1 flush, 2 end). An
/// exception resets the session of the context.
///
/// # Examples
///
/// ```rust,ignore
/// let context = init_compress_nif(&mut scheduler)?;
/// compress_stream_nif(&mut scheduler, &context, &Term::binary(b"a".to_vec()), &Term::Integer(2))?;
/// ```
///
/// # Errors
///
/// A new result atom in a full atom table gives the error of
/// `AtomTable::put`.
fn compress_stream_nif(
    scheduler: &mut Scheduler,
    context: &Term,
    data: &Term,
    flush: &Term,
) -> Result<BifResult, BeamError> {
    let (kind, current) = contexts(scheduler);
    let mut contexts = lock(&kind);
    let native = match open(&mut contexts.compress, context, current) {
        Err(reason) => Native::Raise(reason),
        Ok(compressor) => {
            let cctx = &mut compressor.cctx;
            let native = codec(data, flush, CCtx::out_size(), |out, input, flush| {
                // `ZSTD_compressStream2` rejects an end directive above
                // `ZSTD_e_end` (`zstd_compress.c:6456`).
                let directive = match flush {
                    0 => ZSTD_EndDirective::ZSTD_e_continue,
                    1 => ZSTD_EndDirective::ZSTD_e_flush,
                    2 => ZSTD_EndDirective::ZSTD_e_end,
                    _ => return Err(PARAMETER_OUT_OF_BOUND),
                };
                cctx.compress_stream2(out, input, directive)
            });
            if native.is_exception() {
                let _ = cctx.reset(ResetDirective::SessionOnly);
            }
            native
        }
    };
    result(scheduler, native)
}

/// Run `decompress_stream_nif/3` (`zstd_nif.c:723-783`): decompress the
/// binary `data`. The first call of a frame checks the binary, and uses the
/// decompressed size of the input as the output size when the size is
/// known and smaller than `ZSTD_DStreamOutSize()`. An exception resets the
/// session of the context.
///
/// # Examples
///
/// ```rust,ignore
/// let context = init_decompress_nif(&mut scheduler)?;
/// decompress_stream_nif(&mut scheduler, &context, &frame, &Term::Integer(2))?;
/// ```
///
/// # Errors
///
/// A new result atom in a full atom table gives the error of
/// `AtomTable::put`.
fn decompress_stream_nif(
    scheduler: &mut Scheduler,
    context: &Term,
    data: &Term,
    flush: &Term,
) -> Result<BifResult, BeamError> {
    let flush_true = matches!(flush, Term::Atom(atom) if scheduler.atoms.name_of(atom) == "true");
    let (kind, current) = contexts(scheduler);
    let mut contexts = lock(&kind);
    let native = match open(&mut contexts.decompress, context, current) {
        Err(reason) => Native::Raise(reason),
        Ok(decompressor) => decompress(decompressor, data, flush, flush_true),
    };
    result(scheduler, native)
}

/// Run the stream call of `decompress_stream_nif/3` on an open context.
/// `flush_true` is true when the flush is the atom `true`.
fn decompress(
    decompressor: &mut Decompressor,
    data: &Term,
    flush: &Term,
    flush_true: bool,
) -> Native {
    let mut chunk_size = DCtx::out_size();
    if !decompressor.in_use {
        let Some(binary) = data.binary_bytes() else {
            return Native::Raise("badarg");
        };
        // An empty binary with the flush atom `true` gives `done` and does
        // not start the context (`zstd_nif.c:747-753`). `zstd.erl` passes
        // only an integer flush, so no `zstd` call reaches this result.
        if binary.is_empty() && flush_true {
            return Native::Codec {
                tag: "done",
                remainder: None,
                output: Vec::new(),
            };
        }
        if let Ok(Some(size)) = find_decompressed_size(binary)
            && size < chunk_size as u64
        {
            chunk_size = size as usize;
        }
        decompressor.in_use = true;
    }
    let dctx = &mut decompressor.dctx;
    let native = codec(data, flush, chunk_size, |out, input, _| {
        dctx.decompress_stream(out, input)
    });
    if native.is_exception() {
        let _ = dctx.reset(ResetDirective::SessionOnly);
        decompressor.in_use = false;
    }
    native
}

/// Run `compress_reset_nif/1` (`zstd_nif.c:785-794`): drop the frame in
/// progress, keep the parameters, and give `ok`.
///
/// # Examples
///
/// ```rust,ignore
/// let context = init_compress_nif(&mut scheduler)?;
/// compress_reset_nif(&mut scheduler, &context)?;
/// ```
///
/// # Errors
///
/// A new result atom in a full atom table gives the error of
/// `AtomTable::put`.
fn compress_reset_nif(scheduler: &mut Scheduler, context: &Term) -> Result<BifResult, BeamError> {
    let (kind, current) = contexts(scheduler);
    let mut contexts = lock(&kind);
    let native = match open(&mut contexts.compress, context, current) {
        Err(reason) => Native::Raise(reason),
        Ok(compressor) => {
            let _ = compressor.cctx.reset(ResetDirective::SessionOnly);
            Native::Ok
        }
    };
    result(scheduler, native)
}

/// Run `decompress_reset_nif/1` (`zstd_nif.c:796-805`): drop the frame in
/// progress, keep the parameters, and give `ok`.
///
/// # Examples
///
/// ```rust,ignore
/// let context = init_decompress_nif(&mut scheduler)?;
/// decompress_reset_nif(&mut scheduler, &context)?;
/// ```
///
/// # Errors
///
/// A new result atom in a full atom table gives the error of
/// `AtomTable::put`.
fn decompress_reset_nif(scheduler: &mut Scheduler, context: &Term) -> Result<BifResult, BeamError> {
    let (kind, current) = contexts(scheduler);
    let mut contexts = lock(&kind);
    let native = match open(&mut contexts.decompress, context, current) {
        Err(reason) => Native::Raise(reason),
        Ok(decompressor) => {
            let _ = decompressor.dctx.reset(ResetDirective::SessionOnly);
            decompressor.in_use = false;
            Native::Ok
        }
    };
    result(scheduler, native)
}

/// Run `compress_close_nif/1` (`zstd_nif.c:807-816`): free the context and
/// give `ok`. Each later call with the context raises `badarg`.
///
/// # Examples
///
/// ```rust,ignore
/// let context = init_compress_nif(&mut scheduler)?;
/// compress_close_nif(&mut scheduler, &context)?;
/// ```
///
/// # Errors
///
/// A new result atom in a full atom table gives the error of
/// `AtomTable::put`.
fn compress_close_nif(scheduler: &mut Scheduler, context: &Term) -> Result<BifResult, BeamError> {
    let (kind, current) = contexts(scheduler);
    let mut contexts = lock(&kind);
    let native = close(&mut contexts.compress, context, current);
    result(scheduler, native)
}

/// Run `decompress_close_nif/1` (`zstd_nif.c:818-827`): free the context
/// and give `ok`. Each later call with the context raises `badarg`.
///
/// # Examples
///
/// ```rust,ignore
/// let context = init_decompress_nif(&mut scheduler)?;
/// decompress_close_nif(&mut scheduler, &context)?;
/// ```
///
/// # Errors
///
/// A new result atom in a full atom table gives the error of
/// `AtomTable::put`.
fn decompress_close_nif(scheduler: &mut Scheduler, context: &Term) -> Result<BifResult, BeamError> {
    let (kind, current) = contexts(scheduler);
    let mut contexts = lock(&kind);
    let native = close(&mut contexts.decompress, context, current);
    result(scheduler, native)
}

/// Remove the open context `term` of `contexts` and give `ok`, or the
/// exception of `get_zstd_context`.
fn close<H>(
    contexts: &mut HashMap<Reference, Context<H>>,
    term: &Term,
    current: ProcessId,
) -> Native {
    if let Err(reason) = open(contexts, term, current) {
        return Native::Raise(reason);
    }
    if let Term::Reference(reference) = term {
        contexts.remove(reference);
    }
    Native::Ok
}

#[cfg(test)]
#[path = "../../../../tests/runtime/interpreter/native/zstd.rs"]
mod tests;

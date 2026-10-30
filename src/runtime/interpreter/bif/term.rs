//! Bodies of `erlang:term_to_binary/1,2`, `erlang:binary_to_term/1,2`, and
//! `erlang:external_size/1`: the external term format of ERTS OTP 29.1.1
//! (`external.c`).
//!
//! The encoder writes the tags of `external.h` for each term that PBeam can
//! make. A pid, port, or reference of another node fails with a
//! runtime error. Row "External term format" of
//! `docs/erts-differences.md` gives these differences.
//!
//! # Examples
//!
//! ```rust,ignore
//! let bytes = encode(&Term::Integer(1), &atoms, &Options::default())?;
//! assert_eq!(bytes, [97, 1]);
//! ```

use std::{cmp::Ordering, io::Write, ops::Range, sync::Arc};

use flate2::{Compression, Decompress, FlushDecompress, Status, write::ZlibEncoder};
use num_bigint::{BigInt, Sign};
use num_traits::ToPrimitive;

use super::BifResult::Value;
use super::{
    BeamError, BifContext, BifResult, Binary, Term, error, integer, records, to_i64, to_u64,
    to_usize,
};
use crate::runtime::interpreter::code_table::CodeTable;
use crate::runtime::unicode::CharEncoding;
use crate::term::{
    AtomTable, ExternalId, Fun, Lambda, MAX_ATOM_CHARACTERS, MAX_TUPLE_SIZE, ProcessId, Record,
    RecordDef, Reference, Target,
    external::{self},
    external_id::ExternalData,
    float_text,
};

/// `VERSION_MAGIC`, the first byte of the external term format
/// (`external.h:84`).
const VERSION_MAGIC: u8 = 131;
/// `COMPRESSED`, the tag of a zlib-compressed term (`external.h:75`).
const COMPRESSED: u8 = 80;
/// `LOCAL_EXT`, the tag of a term that only the local node decodes
/// (`external.h:78`, `external.c:3305-3310`).
const LOCAL_EXT: u8 = 121;

/// The size in bytes of the text form of a float under `minor_version` 0
/// (`FLOAT_EXT`, `external.c:3834-3848`).
const FLOAT_TEXT_SIZE: usize = 31;

/// The number of bits of the pid number (`_PID_NUM_SIZE`, `erl_term.h:639`).
const PID_NUMBER_BITS: u32 = 28;
/// The largest pid number of the local node
/// (`ERTS_MAX_INTERNAL_PID_NUMBER`, `erl_node_container_utils.h:182`).
const MAX_PID_NUMBER: u32 = (1 << PID_NUMBER_BITS) - 1;
/// The first reference number holds at most 18 bits (`MAX_REFERENCE`,
/// `erl_term.h:180`).
const MAX_REFERENCE: u32 = 1 << 18;
/// The mask of the counter bits in the second reference number
/// (`ERTS_REF1_NUM_MASK__`).
const REF_NUMBER_1_MASK: u32 = 0xFFFC_0000;
/// The scheduler number of the references of PBeam (`REF_THREAD`).
const REF_THREAD: u32 = 1;
/// The magic reference marker bit of the second reference number
/// (`ERTS_REF1_MAGIC_MARKER_BIT__`, `erl_bif_unique.h:102-105`).
const MAGIC_REFERENCE_BIT: u32 = 1 << 17;
/// The bit of the second reference number that marks a pid reference
/// (`ERTS_REF1_PID_MARKER_BIT__`, `erl_bif_unique.h:106-109`).
const PID_REFERENCE_BIT: u32 = 1 << 30;
/// The largest free-variable count of a fun (`MAX_ARG`, `erl_vm.h:54`).
const MAX_FUN_ARG: usize = 255;
/// The largest count of reference numbers (`ERTS_MAX_REF_NUMBERS`).
const MAX_REFERENCE_NUMBERS: usize = 5;
/// The largest creation of a legacy `PID_EXT`, `PORT_EXT`, or
/// `REFERENCE_EXT` (`ERTS_MAX_TINY_CREATION`, `external.c:71`).
const MAX_TINY_CREATION: u32 = 3;
/// The largest port number of the local node
/// (`ERTS_MAX_INTERNAL_PORT_NUMBER`, `erl_node_container_utils.h:250`):
/// 2^60 - 1 on a 64-bit host (`_PORT_DATA_SIZE`, `erl_term.h:696`).
const MAX_PORT_NUMBER: u64 = (1 << 60) - 1;

/// The largest bitstring of a heap binary, in bits
/// (`ERL_ONHEAP_BITS_LIMIT`, `erl_bits.h:166-168`).
const ONHEAP_BITS_LIMIT: usize = 64 * 8;

/// The options of `erlang:term_to_binary/2`, as `parse_t2b_opts`
/// (`external.c:1483-1557`) reads them.
#[derive(Debug)]
struct Options {
    /// `minor_version` 0, 1, or 2: 2 (`Default`) writes UTF-8 atom tags and
    /// `NEW_FLOAT_EXT`; 1 writes Latin-1 atom tags; 0 also writes
    /// `FLOAT_EXT`.
    minor: u8,
    /// The zlib level: 0 writes no compression, and the atom `compressed`
    /// gives the default level 6.
    level: u32,
    /// `local` wraps the term in `LOCAL_EXT`.
    local: bool,
    /// `deterministic` sorts the keys of each map in term order.
    deterministic: bool,
}

impl Default for Options {
    /// Return the default options: `minor_version` 2, no compression, and
    /// neither `local` nor `deterministic` (`TERM_TO_BINARY_DFLAGS`,
    /// `dist.h:138-140`).
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// assert_eq!(Options::default().minor, 2);
    /// ```
    fn default() -> Self {
        Options {
            minor: 2,
            level: 0,
            local: false,
            deterministic: false,
        }
    }
}

/// Parse the option list of `erlang:term_to_binary/2` as `parse_t2b_opts`
/// (`external.c:1483-1557`) does. Return `None` for each other list.
///
/// # Examples
///
/// ```rust,ignore
/// assert!(parse_options(&Term::nil()).is_some());
/// assert!(parse_options(&Term::static_atom("compressed")).is_none());
/// ```
fn parse_options(options: &Term) -> Option<Options> {
    let mut parsed = Options::default();
    for option in options.proper_list()? {
        match option {
            Term::Atom(atom) if atom.is_static("compressed") => parsed.level = 6,
            Term::Atom(atom) if atom.is_static("local") => parsed.local = true,
            Term::Atom(atom) if atom.is_static("deterministic") => parsed.deterministic = true,
            Term::Tuple(fields) => {
                let [Term::Atom(name), Term::Integer(value)] = fields.as_ref() else {
                    return None;
                };
                if name.is_static("minor_version") {
                    parsed.minor = match value {
                        0..=2 => {
                            u8::try_from(*value).expect("the pattern bounds the version to 0..=2")
                        }
                        _ => return None,
                    };
                } else if name.is_static("compressed") {
                    parsed.level = match value {
                        0..=9 => {
                            u32::try_from(*value).expect("the pattern bounds the level to 0..=9")
                        }
                        _ => return None,
                    };
                } else {
                    return None;
                }
            }
            _ => return None,
        }
    }
    if parsed.deterministic && parsed.local {
        return None;
    }
    Some(parsed)
}

/// Run `erlang:term_to_binary(Term)`, as `term_to_binary_1`
/// (`external.c:1442-1463`) does with the default options.
///
/// # Examples
///
/// ```rust,ignore
/// let result = term_to_binary(&mut context, &Term::Integer(1))?;
/// assert!(matches!(result, BifResult::Value(Term::Binary(_))));
/// ```
///
/// # Errors
///
/// The errors of [`term_to_binary_2`].
pub(super) fn term_to_binary(
    context: &mut BifContext<'_>,
    term: &Term,
) -> Result<BifResult, BeamError> {
    term_to_binary_2(context, term, &Term::nil())
}

/// Run `erlang:term_to_binary(Term, Options)`, as `term_to_binary_2`
/// (`external.c:1565-1591`) does: a bad option list raises `badarg`, and
/// the result is `VERSION_MAGIC` then the encoded term, or `COMPRESSED` with
/// the length of the encoded term and its zlib stream when the compressed
/// form fits the space of the plain form (`erts_term_to_binary_simple`,
/// `external.c:2263-2317`). The `local` option puts `LOCAL_EXT` and the
/// block hash of the term before it (`external.c:3305-3310`, `4046-4056`).
///
/// # Examples
///
/// ```rust,ignore
/// let options = Term::list_from(vec![Term::static_atom("compressed")]);
/// let result = term_to_binary_2(&mut context, &Term::Integer(1), &options)?;
/// assert!(matches!(result, BifResult::Value(Term::Binary(_))));
/// ```
///
/// # Errors
///
/// A term that PBeam does not encode yet gives a runtime error, and a full
/// atom table gives its error.
pub(super) fn term_to_binary_2(
    context: &mut BifContext<'_>,
    term: &Term,
    options: &Term,
) -> Result<BifResult, BeamError> {
    external_term(context, term, options, &mut Vec::new())
}

/// Run `erlang:term_to_iovec(Term)`, as `term_to_iovec_1`
/// (`external.c:1461-1480`) does with the default options.
///
/// # Examples
///
/// ```rust,ignore
/// let result = term_to_iovec(&mut context, &Term::Integer(1))?;
/// assert!(matches!(result, BifResult::Value(Term::List(_))));
/// ```
///
/// # Errors
///
/// The errors of [`term_to_binary_2`].
pub(super) fn term_to_iovec(
    context: &mut BifContext<'_>,
    term: &Term,
) -> Result<BifResult, BeamError> {
    term_to_iovec_2(context, term, &Term::nil())
}

/// Run `erlang:term_to_iovec(Term, Options)`, as `term_to_iovec_2`
/// (`external.c:1592-1616`) does. Without compression the result is a list
/// of binaries whose bytes are the bytes of `term_to_binary/2`: the payload
/// of each bitstring of more than `ERL_ONHEAP_BITS_LIMIT` bits that starts
/// on a byte boundary is its own element, and the other bytes between them
/// are one element each (`external.c:3864-3867`, `3953-3960`,
/// `store_in_vec`, `4210-4245`). A level other than 0 clears the iovec
/// flag (`external.c:2495-2502`), so the result is the binary of
/// `term_to_binary/2` without a list (`external.c:2541-2550`, `2806-2813`,
/// `2844-2850`).
///
/// # Examples
///
/// ```rust,ignore
/// let result = term_to_iovec_2(&mut context, &Term::Integer(1), &Term::nil())?;
/// assert!(matches!(result, BifResult::Value(Term::List(_))));
/// ```
///
/// # Errors
///
/// The errors of [`term_to_binary_2`].
pub(super) fn term_to_iovec_2(
    context: &mut BifContext<'_>,
    term: &Term,
    options: &Term,
) -> Result<BifResult, BeamError> {
    let mut refs = Vec::new();
    let result = external_term(context, term, options, &mut refs)?;
    let Value(Term::Binary(binary)) = &result else {
        return Ok(result);
    };
    if parse_options(options).is_some_and(|parsed| parsed.level != 0) {
        return Ok(result);
    }
    let bytes = binary.bytes().unwrap_or_default();
    // Each element: its byte range in `bytes`, and the source bitstring of
    // a payload, which the element refers to as ERTS does
    // (`external.c:3958`). The other elements refer to one binary of the
    // bytes between the payloads, as the result binary of ERTS holds them
    // (`external.c:2585-2587`).
    let mut pieces: Vec<(Range<usize>, Option<Binary>)> = Vec::new();
    let mut cursor = 0;
    // `LOCAL_EXT` and its hash end one element, and the hash covers the
    // elements after it (`external.c:3305-3310`, `4051-4056`).
    let local = bytes.get(1) == Some(&LOCAL_EXT);
    if local {
        pieces.push((0..6, None));
        cursor = 6;
    }
    for (range, source) in refs
        .into_iter()
        .map(|(range, source)| (range, Some(source)))
        .chain(std::iter::once((bytes.len()..bytes.len(), None)))
    {
        if cursor < range.start {
            pieces.push((cursor..range.start, None));
        }
        if range.start < range.end {
            pieces.push((range.clone(), source));
        }
        cursor = range.end;
    }
    let mut common = Vec::new();
    for (range, source) in &pieces {
        if source.is_none() {
            common.extend_from_slice(&bytes[range.clone()]);
        }
    }
    if local {
        let last = pieces.last().map_or(0, |(range, _)| range.len());
        let seed = context.scheduler.local_node_hash();
        let hash = external::iov_block_hash(&bytes[6..], last, seed);
        common[2..6].copy_from_slice(&hash.to_be_bytes());
    }
    let common = Binary::from(common);
    let mut offset = 0;
    let elements: Vec<Term> = pieces
        .into_iter()
        .map(|(range, source)| {
            if let Some(source) = source {
                Term::Binary(source.off_heap())
            } else {
                offset += range.len();
                Term::Binary(
                    common
                        .bits((offset - range.len()) * 8, offset * 8)
                        .off_heap(),
                )
            }
        })
        .collect();
    Ok(Value(Term::list_from(elements)))
}

/// Return the result of [`term_to_binary_2`], and put in `refs` the byte
/// range and the source bitstring of each payload of the result that
/// `term_to_iovec/2` gives as its own element.
///
/// # Errors
///
/// The errors of [`term_to_binary_2`].
fn external_term(
    context: &mut BifContext<'_>,
    term: &Term,
    options: &Term,
    refs: &mut Vec<(Range<usize>, Binary)>,
) -> Result<BifResult, BeamError> {
    let Some(parsed) = parse_options(options) else {
        return Ok(BifResult::badarg());
    };
    // The one-go path of `erts_term_to_binary_int` charges 1 reduction
    // (`external.c:2520-2530`). See row "External term format" of
    // `docs/erts-differences.md` for the charge of a larger term.
    context.scheduler.current_process_mut().bump_reductions(1);
    // The existing `[local]` reference path of `erlang.rs` gives the bytes
    // that `prim_inet:send/3` needs; keep it for that one case.
    let local = Term::list_from(vec![Term::static_atom("local")]);
    if matches!(term, Term::Reference(reference) if !reference.is_pid_reference())
        && *options == local
    {
        return super::erlang::term_to_binary_2(context, term, options);
    }
    let options = parsed;
    let scheduler = &mut *context.scheduler;
    let seed = scheduler.local_node_hash();
    let mut body = encode_parts(term, &scheduler.atoms, &options, refs)?;
    let unprefixed = body.len();
    if options.local {
        let hash = external::block_hash(&body, seed);
        body = [&[LOCAL_EXT][..], &hash.to_be_bytes(), &body].concat();
    }
    let bytes = compress_term(&body, options.level);
    let shift = bytes.len().saturating_sub(unprefixed);
    for (range, _) in refs.iter_mut() {
        *range = range.start + shift..range.end + shift;
    }
    Ok(Value(Term::binary(bytes)))
}

/// Return the external term bytes and the `VERSION_MAGIC` and compression
/// header of [`term_to_binary_2`].
///
/// # Examples
///
/// ```rust,ignore
/// assert_eq!(compress_term(&[97, 1], 0), [131, 97, 1]);
/// assert_eq!(compress_term(&[97, 1], 6), [131, 97, 1]);
/// ```
fn compress_term(body: &[u8], level: u32) -> Vec<u8> {
    if level != 0
        && body.len() >= 5
        && let Some(compressed) = deflate(body, level)
        && compressed.len() <= body.len() - 5
    {
        return [
            &[VERSION_MAGIC, COMPRESSED][..],
            &be32(body.len()),
            &compressed,
        ]
        .concat();
    }
    [&[VERSION_MAGIC][..], body].concat()
}

/// Return the zlib stream of `bytes` at `level`, as `erl_zlib_compress2`
/// gives it.
///
/// # Examples
///
/// ```rust,ignore
/// assert!(deflate(&[1, 2, 3], 6).is_some());
/// ```
fn deflate(bytes: &[u8], level: u32) -> Option<Vec<u8>> {
    let mut encoder = ZlibEncoder::new(Vec::new(), Compression::new(level));
    encoder.write_all(bytes).ok()?;
    encoder.finish().ok()
}

/// Run `erlang:external_size(Term)`, as `external_size_1`
/// (`external.c:2177-2214`) does: return the size in bytes of
/// `term_to_binary(Term)`.
///
/// # Examples
///
/// ```rust,ignore
/// let atoms = AtomTable::new();
/// let size = external_size(&atoms, &Term::Integer(1));
/// assert!(matches!(size, BifResult::Value(Term::Integer(3))));
/// ```
///
/// # Errors
///
/// The errors of [`encode`].
pub(super) fn external_size(atoms: &AtomTable, term: &Term) -> Result<BifResult, BeamError> {
    let body = encode(term, atoms, &Options::default())?;
    Ok(Value(Term::Integer(1 + to_i64(body.len()))))
}

/// Run `erlang:external_size(Term, Options)`, as `external_size_2`
/// (`external.c:2220-2246`) does: a bad option list raises `badarg`, and
/// the result is the size of `term_to_binary(Term, Options)` without
/// compression (`erts_encode_ext_size_2`, `external.c:735-743`), with the
/// `LOCAL_EXT` tag and its 4-byte hash for `local`.
///
/// # Examples
///
/// ```rust,ignore
/// let atoms = AtomTable::new();
/// let size = external_size_2(&atoms, &Term::Integer(1), &Term::nil());
/// assert!(matches!(size, BifResult::Value(Term::Integer(3))));
/// ```
///
/// # Errors
///
/// The errors of [`encode`].
pub(super) fn external_size_2(
    atoms: &AtomTable,
    term: &Term,
    options: &Term,
) -> Result<BifResult, BeamError> {
    let Some(options) = parse_options(options) else {
        return Ok(BifResult::badarg());
    };
    let body = encode(term, atoms, &options)?;
    let local = if options.local { 5 } else { 0 };
    Ok(Value(Term::Integer(1 + local + to_i64(body.len()))))
}

/// Return the external term bytes of `term` without `VERSION_MAGIC`, as
/// `enc_term` (`external.c:3252-4060`) writes them. The work list holds the
/// terms that the walk still writes, so a deep term uses no Rust stack.
///
/// # Examples
///
/// ```rust,ignore
/// let atoms = AtomTable::new();
/// assert_eq!(encode(&Term::Integer(1), &atoms, &Options::default())?, [97, 1]);
/// ```
///
/// # Errors
///
/// A match context and a pid reference give a runtime error, and a
/// full atom table gives its error.
fn encode(term: &Term, atoms: &AtomTable, options: &Options) -> Result<Vec<u8>, BeamError> {
    encode_parts(term, atoms, options, &mut Vec::new())
}

/// Return the bytes of [`encode`], and put in `refs` the byte range and the
/// whole bytes of the payload of each bitstring of more than `ERL_ONHEAP_BITS_LIMIT` bits
/// (`erl_bits.h:166-168`) that starts on a byte boundary: the payload that
/// `enc_term_int` refers to in an iovec, without its trailing bits
/// (`external.c:3864-3867`, `3953-3960`).
///
/// # Errors
///
/// The errors of [`encode`].
#[expect(
    clippy::too_many_lines,
    reason = "mirrors the encoder `enc_term` of `external.c`: one arm for each term type with the patch of the fun size; the arms share the output and the work stack"
)]
fn encode_parts(
    term: &Term,
    atoms: &AtomTable,
    options: &Options,
    refs: &mut Vec<(Range<usize>, Binary)>,
) -> Result<Vec<u8>, BeamError> {
    let mut out = Vec::new();
    let mut pending = vec![term];
    // Each open `NEW_FUN_EXT`: the length of `pending` below its free
    // variables and the position of its size field, which the walk patches
    // when it has written the free variables (`ENC_PATCH_FUN_SIZE`,
    // `external.c:3350-3370`).
    let mut fun_sizes: Vec<(usize, usize)> = Vec::new();
    loop {
        while let Some(&(_, start)) = fun_sizes
            .last()
            .filter(|(depth, _)| *depth == pending.len())
        {
            fun_sizes.pop();
            let size = low_u32(to_u64(out.len() - start));
            out[start..start + 4].copy_from_slice(&size.to_be_bytes());
        }
        let Some(term) = pending.pop() else {
            break;
        };
        match term {
            Term::Nil => out.push(106),
            Term::Atom(atom) => encode_atom(atoms.name_of(atom), options, &mut out),
            Term::Integer(value) => encode_integer(*value, &mut out),
            Term::BigInt(value) => encode_big(value, &mut out),
            Term::Float(value) if options.minor == 0 => {
                out.push(99);
                let text = float_text::scientific(*value, 20);
                out.extend_from_slice(&text);
                out.resize(out.len() + FLOAT_TEXT_SIZE - text.len(), 0);
            }
            Term::Float(value) => {
                out.push(70);
                out.extend_from_slice(&value.to_bits().to_be_bytes());
            }
            Term::Cons(_) => {
                if let Some(bytes) = list_string(term) {
                    out.push(107);
                    out.extend_from_slice(&be16(bytes.len()));
                    out.extend_from_slice(&bytes);
                } else {
                    let mut heads = Vec::new();
                    let mut walk = term;
                    while let Some((head, tail)) = walk.cons_cell() {
                        heads.push(head);
                        walk = tail;
                    }
                    out.push(108);
                    out.extend_from_slice(&be32(heads.len()));
                    pending.push(walk);
                    pending.extend(heads.into_iter().rev());
                }
            }
            Term::Tuple(elements) => {
                if elements.len() <= 0xff {
                    out.extend_from_slice(&[
                        104,
                        u8::try_from(elements.len()).expect("the branch bounds the arity to 255"),
                    ]);
                } else {
                    out.push(105);
                    out.extend_from_slice(&be32(elements.len()));
                }
                pending.extend(elements.iter().rev());
            }
            Term::Map(map) => {
                out.push(116);
                out.extend_from_slice(&be32(map.len()));
                let mut pairs: Vec<(&Term, &Term)> = map.iter().collect();
                if options.deterministic {
                    pairs.sort_by(|(left, _), (right, _)| exact_map_key_order(left, right, atoms));
                } else if map.is_hashmap() {
                    // ERTS writes a flatmap in its key order
                    // (`ENC_MAP_PAIR`, `external.c:3391-3403`, 3766-3768)
                    // and a hashmap in the reverse of the HAMT traversal
                    // order (`enc_term`, `external.c:3819-3825`).
                    pairs.reverse();
                }
                for (key, value) in pairs.into_iter().rev() {
                    pending.push(value);
                    pending.push(key);
                }
            }
            Term::Binary(binary) => {
                if binary.bit_size() > ONHEAP_BITS_LIMIT && binary.starts_on_byte() {
                    // The tag, the 4-byte size, and the tail-bit byte of
                    // `BIT_BINARY_EXT` come before the payload.
                    let start = out.len() + if binary.bit_size() % 8 == 0 { 5 } else { 6 };
                    let whole = binary.bit_size() / 8;
                    refs.push((start..start + whole, binary.bits(0, whole * 8)));
                }
                if let Some(bytes) = binary.bytes() {
                    out.push(109);
                    out.extend_from_slice(&be32(bytes.len()));
                    out.extend_from_slice(bytes);
                } else {
                    // `BIT_BINARY_EXT` holds the byte count, the count of
                    // the bits of the last byte, and the bytes with the
                    // unused low bits of the last byte cleared
                    // (`external.c:3861`, `3916-3935`).
                    let tail = binary.bit_size() % 8;
                    let bytes = &binary.bit_bytes()[..binary.bit_size().div_ceil(8)];
                    out.push(77);
                    out.extend_from_slice(&be32(bytes.len()));
                    out.push(u8::try_from(tail).expect("a tail has fewer than 8 bits"));
                    out.extend_from_slice(bytes);
                    if let Some(last) = out.last_mut() {
                        *last &= 0xFF << (8 - tail);
                    }
                }
            }
            Term::Pid(ProcessId(data)) => {
                out.push(88);
                encode_sysname(atoms, options, &mut out);
                out.extend_from_slice(&low_u32(data & u64::from(MAX_PID_NUMBER)).to_be_bytes());
                out.extend_from_slice(&low_u32(data >> PID_NUMBER_BITS).to_be_bytes());
                out.extend_from_slice(&0u32.to_be_bytes());
            }
            Term::Port(port) => {
                out.push(120);
                encode_sysname(atoms, options, &mut out);
                out.extend_from_slice(&port.0.to_be_bytes());
                out.extend_from_slice(&0u32.to_be_bytes());
            }
            // An identifier of another node gives its node name and its
            // creation (`enc_pid`, `external.c:3025-3060`, `enc_term_int`,
            // `external.c:3602-3700`).
            Term::External(id) => {
                let node = atoms.name_of(&id.node);
                match &id.data {
                    ExternalData::Pid { number, serial } => {
                        out.push(88);
                        encode_atom(node, options, &mut out);
                        out.extend_from_slice(&number.to_be_bytes());
                        out.extend_from_slice(&serial.to_be_bytes());
                    }
                    ExternalData::Port(number) => {
                        out.push(120);
                        encode_atom(node, options, &mut out);
                        out.extend_from_slice(&number.to_be_bytes());
                    }
                    ExternalData::Reference(numbers) => {
                        out.push(90);
                        let count = u16::try_from(numbers.len())
                            .expect("a reference has at most 5 numbers");
                        out.extend_from_slice(&count.to_be_bytes());
                        encode_atom(node, options, &mut out);
                    }
                }
                out.extend_from_slice(&id.creation.to_be_bytes());
                if let ExternalData::Reference(numbers) = &id.data {
                    for number in numbers {
                        out.extend_from_slice(&number.to_be_bytes());
                    }
                }
            }
            // A resource term is its magic reference.
            Term::Reference(_) | Term::Resource(_) => {
                let reference = term.reference().unwrap_or(Reference(0));
                if reference.is_pid_reference() {
                    return Err(error(
                        "unsupported erlang:term_to_binary/1,2 of a pid reference: ERTS writes the monitored pid in the alias reference: see \"External term format\" in docs/erts-differences.md",
                    ));
                }
                out.push(90);
                out.extend_from_slice(&3u16.to_be_bytes());
                encode_sysname(atoms, options, &mut out);
                out.extend_from_slice(&0u32.to_be_bytes());
                for number in reference.numbers() {
                    out.extend_from_slice(&number.to_be_bytes());
                }
            }
            Term::Fun(fun) => match fun.target() {
                Target::External {
                    module,
                    function,
                    arity,
                } => {
                    out.push(113);
                    encode_atom(atoms.name_of(module), options, &mut out);
                    encode_atom(atoms.name_of(function), options, &mut out);
                    encode_integer(i64::from(*arity), &mut out);
                }
                // The size, the arity, the module MD5, the index, the free
                // count, the module, the old index, the old unique value,
                // the creator pid `<0.0.0>` (`erts_init_process_id`), and
                // the free variables (`enc_term`, `external.c:3970-3993`).
                Target::Local(lambda) => {
                    out.push(112);
                    fun_sizes.push((pending.len(), out.len()));
                    out.extend_from_slice(&[0; 4]);
                    out.push(u8::try_from(lambda.arity()).expect("a fun arity is at most 255"));
                    out.extend_from_slice(lambda.uniq());
                    out.extend_from_slice(&lambda.index().to_be_bytes());
                    out.extend_from_slice(&be32(fun.free().len()));
                    encode_atom(atoms.name_of(lambda.module()), options, &mut out);
                    encode_integer(i64::from(lambda.old_index()), &mut out);
                    encode_integer(i64::from(lambda.old_unique()), &mut out);
                    out.push(88);
                    encode_sysname(atoms, options, &mut out);
                    out.extend_from_slice(&[0; 12]);
                    pending.extend(fun.free().iter().rev());
                }
            },
            // The field count, the export flag, the module, the name, the
            // field names, and the values, both in definition order
            // (`enc_term`, `external.c:4006-4036`).
            Term::Record(record) => {
                let def = record.def();
                out.push(67);
                out.extend_from_slice(&be32(def.order.len()));
                out.push(u8::from(def.is_exported));
                encode_atom(atoms.name_of(&def.module), options, &mut out);
                encode_atom(atoms.name_of(&def.name), options, &mut out);
                for &index in &def.order {
                    encode_atom(atoms.name_of(&def.keys[index]), options, &mut out);
                }
                pending.extend(def.order.iter().rev().map(|&index| &record.values()[index]));
            }
            Term::MatchContext(_) => {
                return Err(error(
                    "unsupported erlang:term_to_binary/1,2 of a match context: see \"External term format\" in docs/erts-differences.md",
                ));
            }
        }
    }
    Ok(out)
}

/// Compare two map keys in the exact term order of the `deterministic`
/// option (`map_key_compare`, `external.c:3217-3227`). The exact order has
/// an error only for two different references, and a PBeam map key is not a
/// reference (see `Map`), so the comparison always gives an order.
///
/// # Examples
///
/// ```rust,ignore
/// let atoms = AtomTable::new();
/// assert_eq!(exact_map_key_order(&Term::Integer(2), &Term::Float(1.0), &atoms), Ordering::Less);
/// ```
fn exact_map_key_order(left: &Term, right: &Term, atoms: &AtomTable) -> Ordering {
    left.compare_exact(right, atoms).unwrap_or(Ordering::Equal)
}

/// Write the node atom of a pid, port, or reference: `NIL_EXT` with `local`,
/// else the local node atom (`enc_internal_pid`, `external.c:2968-2980`).
fn encode_sysname(atoms: &AtomTable, options: &Options, out: &mut Vec<u8>) {
    if options.local {
        out.push(106);
    } else {
        let atom =
            crate::term::Atom::from_static("nonode@nohost").expect("the local node atom is static");
        encode_atom(atoms.name_of(&atom), options, out);
    }
}

/// Return the term of the pid, port, or reference `data` of the node `node`
/// with the creation `creation`.
///
/// # Examples
///
/// ```rust,ignore
/// let port = remote_id(node, 5, ExternalData::Port(7));
/// assert!(matches!(port, Term::External(_)));
/// ```
fn remote_id(node: crate::term::Atom, creation: u32, data: ExternalData) -> Term {
    Term::External(std::sync::Arc::new(ExternalId {
        node,
        creation,
        data,
    }))
}

/// Write an atom as `enc_atom` (`external.c:2877-2957`) does: with
/// `minor_version` 2 the UTF-8 tags, and else the Latin-1 tags for an atom
/// that Latin-1 holds and the UTF-8 tags for each other atom.
fn encode_atom(name: &str, options: &Options, out: &mut Vec<u8>) {
    let latin1 = if options.minor >= 2 {
        None
    } else {
        latin1(name)
    };
    let bytes = latin1.as_deref().unwrap_or(name.as_bytes());
    // `TERM_TO_BINARY_DFLAGS` has no `DFLAG_SMALL_ATOM_TAGS`, so a Latin-1
    // atom takes `ATOM_EXT` and never `SMALL_ATOM_EXT` (`external.c:2927-2949`).
    match (latin1.is_some(), bytes.len() <= 0xff) {
        (true, _) => {
            out.push(100);
            out.extend_from_slice(&be16(bytes.len()));
        }
        (false, true) => out.extend_from_slice(&[
            119,
            u8::try_from(bytes.len()).expect("the match bounds the name to 255 bytes"),
        ]),
        (false, false) => {
            out.push(118);
            out.extend_from_slice(&be16(bytes.len()));
        }
    }
    out.extend_from_slice(bytes);
}

/// Return the Latin-1 bytes of an atom name, or `None` when a character is
/// above U+00FF (`a->latin1_chars < 0`, `external.c:2898`).
fn latin1(name: &str) -> Option<Vec<u8>> {
    name.chars()
        .map(|character| u8::try_from(u32::from(character)).ok())
        .collect()
}

/// Return the low 32 bits of a number, as `put_int32` of `external.c` writes
/// an unsigned number of more bits.
fn low_u32(value: u64) -> u32 {
    u32::try_from(value & u64::from(u32::MAX)).expect("the mask keeps 32 bits")
}

/// Return a count as the 4 big-endian bytes of `put_int32`, which keep the
/// low 32 bits.
fn be32(count: usize) -> [u8; 4] {
    low_u32(to_u64(count)).to_be_bytes()
}

/// Return a length as 2 big-endian bytes. The callers bound the length to
/// 65535: an atom name has at most 255 characters, and a string list has at
/// most 65535 bytes.
fn be16(length: usize) -> [u8; 2] {
    u16::try_from(length)
        .expect("the format bounds the length to 65535")
        .to_be_bytes()
}

/// Write an integer as the `SMALL_DEF` branch of `enc_term`
/// (`external.c:3552-3577`): `SMALL_INTEGER_EXT` below 256, `INTEGER_EXT`
/// in the 32-bit range, and else `SMALL_BIG_EXT`.
fn encode_integer(value: i64, out: &mut Vec<u8>) {
    if (0..256).contains(&value) {
        out.extend_from_slice(&[
            97,
            u8::try_from(value).expect("the range check bounds the value to 0..256"),
        ]);
    } else if let Ok(value) = i32::try_from(value) {
        out.push(98);
        out.extend_from_slice(&value.to_be_bytes());
    } else {
        out.push(110);
        let bytes = value.unsigned_abs().to_le_bytes();
        let first = bytes
            .iter()
            .rposition(|&byte| byte != 0)
            .map_or(0, |index| index + 1);
        out.push(u8::try_from(first).expect("a 64-bit magnitude has at most 8 bytes"));
        out.push(u8::from(value < 0));
        out.extend_from_slice(&bytes[..first]);
    }
}

/// Write a bignum as the `BIG_DEF` branch of `enc_term`
/// (`external.c:3580-3600`) does: `SMALL_BIG_EXT` below 256 magnitude bytes
/// and else `LARGE_BIG_EXT`.
fn encode_big(value: &BigInt, out: &mut Vec<u8>) {
    let (sign, bytes) = value.to_bytes_le();
    if bytes.len() <= 0xff {
        out.push(110);
        out.push(u8::try_from(bytes.len()).expect("the branch bounds the length to 255"));
    } else {
        out.push(111);
        out.extend_from_slice(&be32(bytes.len()));
    }
    out.push(u8::from(sign == Sign::Minus));
    out.extend_from_slice(&bytes);
}

/// Return the bytes of a proper list of at most 65535 bytes, or `None` for
/// each other list (`is_external_string`, `external.c:4239-4264`).
fn list_string(term: &Term) -> Option<Vec<u8>> {
    let mut bytes = Vec::new();
    let mut walk = term;
    while let Some((head, tail)) = walk.cons_cell() {
        let Term::Integer(value) = head else {
            return None;
        };
        if !(0..256).contains(value) || bytes.len() == 0xffff {
            return None;
        }
        bytes.push(u8::try_from(*value).expect("the range check bounds the value to 0..256"));
        walk = tail;
    }
    walk.is_nil().then_some(bytes)
}

/// Run `erlang:binary_to_term(Binary, Options)`, as `binary_to_term_2`
/// (`external.c:2159-2198`) does: a bad option list raises `badarg` with the
/// `error_info` cause `badopt`, and each byte sequence that ERTS rejects
/// raises `badarg`. The `used` option returns `{Term, Used}`, where `Used`
/// counts the bytes of the input that the term and its header use.
///
/// # Examples
///
/// ```rust,ignore
/// let result = binary_to_term(&mut context, &Term::binary(vec![131, 97, 1]), &Term::nil())?;
/// assert!(matches!(result, BifResult::Value(Term::Integer(1))));
/// ```
///
/// # Errors
///
/// A byte sequence that ERTS decodes to a term that PBeam cannot hold gives
/// a runtime error. See row "External term format" of
/// `docs/erts-differences.md`.
pub(super) fn binary_to_term(
    context: &mut BifContext<'_>,
    binary: &Term,
    options: &Term,
) -> Result<BifResult, BeamError> {
    let Some((safe, used)) = binary_options(options) else {
        return Ok(BifResult::badopt());
    };
    let Some(binary) = binary.binary_bytes() else {
        return Ok(BifResult::badarg());
    };
    let seed = context.scheduler.local_node_hash();
    let mut decoder = Decoder {
        input: binary,
        pos: 0,
        overrun: 0,
        safe,
        local: false,
        atoms: &context.scheduler.atoms,
        code: &context.scheduler.code,
        ets: &context.scheduler.ets,
    };
    match decoder.decode_input(seed) {
        Ok((term, used_bytes)) if used => Ok(Value(Term::tuple(vec![
            term,
            Term::Integer(to_i64(used_bytes)),
        ]))),
        Ok((term, _)) => Ok(Value(term)),
        Err(DecodeError::Badarg) => Ok(BifResult::badarg()),
        Err(DecodeError::Unsupported(what)) => Err(error(format!(
            "unsupported erlang:binary_to_term/1,2 of {what}: see \"External term format\" in docs/erts-differences.md"
        ))),
        Err(DecodeError::Error(cause)) => Err(cause),
    }
}

/// Parse the option list of `erlang:binary_to_term/2` as `binary_to_term_2`
/// does (`external.c:2165-2194`): return `(safe, used)`, or `None` for each
/// other list.
///
/// # Examples
///
/// ```rust,ignore
/// assert_eq!(binary_options(&Term::nil()), Some((false, false)));
/// assert_eq!(binary_options(&Term::static_atom("safe")), None);
/// ```
fn binary_options(options: &Term) -> Option<(bool, bool)> {
    let mut safe = false;
    let mut used = false;
    for option in options.proper_list()? {
        let Term::Atom(atom) = option else {
            return None;
        };
        if atom.is_static("safe") {
            safe = true;
        } else if atom.is_static("used") {
            used = true;
        } else {
            return None;
        }
    }
    Some((safe, used))
}

/// A byte sequence that ERTS rejects, or a term that PBeam does not hold.
#[derive(Debug)]
enum DecodeError {
    /// ERTS `badarg`.
    Badarg,
    /// A term of the external term format that PBeam cannot represent.
    Unsupported(&'static str),
    /// A runtime error of the byte sequence, such as a new atom in a full
    /// atom table.
    Error(BeamError),
}

/// The decoder of one external term: the input bytes, the cursor, and the
/// atom table of the run.
struct Decoder<'a> {
    /// The input binary.
    input: &'a [u8],
    /// The position of the next byte.
    pos: usize,
    /// The bytes after the end of `input` that the decode has read: `dec_term`
    /// reads the first number of a reference with no numbers also after the
    /// end, and `used` counts them (see [`Decoder::first_reference_number`]).
    overrun: usize,
    /// The `safe` option: each atom must exist already.
    safe: bool,
    /// True inside `LOCAL_EXT`: `NIL_EXT` stands for the local node.
    local: bool,
    /// The atom table of the run.
    atoms: &'a AtomTable,
    /// The code table of the run, which holds the fun entries of the loaded
    /// code.
    code: &'a CodeTable,
    /// The ETS tables of the node, which turn a magic reference back into
    /// the identifier of its live table.
    ets: &'a crate::runtime::ets::NodeTables,
}

/// Return the record of `def` with `items`, the values in definition order.
fn record_term(def: Arc<RecordDef>, items: Vec<Term>) -> Term {
    let mut values = vec![Term::Nil; items.len()];
    for (position, value) in items.into_iter().enumerate() {
        values[def.order[position]] = value;
    }
    Term::Record(Record::new(def, values))
}

/// The container that the decoder walk still fills.
enum FrameKind {
    /// A tuple of the remaining elements.
    Tuple,
    /// A list of the remaining heads, then its tail.
    List,
    /// A map of the remaining 2 * size keys and values.
    Map,
    /// An `EXPORT_EXT` of the module, the function, and the arity.
    Export,
    /// A `NEW_FUN_EXT` with its arity, module MD5, and index, of the module,
    /// the old index, the old unique value, the creator pid, and the free
    /// variables.
    Fun {
        arity: u8,
        uniq: [u8; 16],
        index: i32,
    },
    /// A `RECORD_EXT` with its export flag, of the module, the name, the
    /// field names, and then the values of the definition that the header
    /// gives.
    Record {
        exported: bool,
        header: Vec<crate::term::Atom>,
        def: Option<Arc<RecordDef>>,
    },
    /// A `LOCAL_EXT` with the local flag of the enclosing term. The
    /// validation walk checks its hash, as ERTS checks it only in
    /// `decoded_size` (`external.c:5250-5255`).
    Local { prior: bool },
}

/// One open container of the decoder walk: its kind, the terms that it still
/// expects, and the terms that it has.
struct Frame {
    /// The kind of the container.
    kind: FrameKind,
    /// The number of terms that the container still expects.
    remaining: usize,
    /// The terms that the container has, in order.
    items: Vec<Term>,
}

/// The container that [`validate`] still fills.
enum CheckKind {
    /// A tuple of the remaining elements.
    Tuple,
    /// A list of the remaining heads, then its tail.
    List,
    /// A map of the remaining 2 * size keys and values.
    Map,
    /// An `EXPORT_EXT` of the module, the function, and the arity.
    Export,
    /// A `LOCAL_EXT` with its hash, the start of the wrapped term, and the
    /// local flag of the enclosing term.
    Local {
        hash: u32,
        start: usize,
        prior: NodeScope,
    },
}

/// One open container of the validation walk.
struct CheckFrame {
    /// The kind of the container.
    kind: CheckKind,
    /// The number of terms that the container still expects.
    remaining: usize,
}

/// Take `count` bytes at `index`, as the decoder does.
fn check_skip(input: &[u8], index: &mut usize, count: usize) -> Result<(), DecodeError> {
    let end = index.checked_add(count).ok_or(DecodeError::Badarg)?;
    if end > input.len() {
        return Err(DecodeError::Badarg);
    }
    *index = end;
    Ok(())
}

/// Take a big-endian number of `count` bytes at `index` as a length.
fn check_length(input: &[u8], index: &mut usize, count: usize) -> Result<usize, DecodeError> {
    Ok(usize::try_from(check_number(input, index, count)?).expect("a u32 fits in usize"))
}

/// Take a big-endian number of `count` bytes at `index`.
fn check_number(input: &[u8], index: &mut usize, count: usize) -> Result<u32, DecodeError> {
    let start = *index;
    check_skip(input, index, count)?;
    Ok(input[start..*index]
        .iter()
        .fold(0u32, |value, &byte| (value << 8) | u32::from(byte)))
}

/// Take an atom name of `length` bytes at `index`. The validation pass
/// reads only the structure, so the checks of `dec_atom`
/// (`external.c:4490-4550`) run in `Decoder::atom` after the atoms of the
/// earlier terms exist, as `decoded_size` and `dec_term` do.
fn check_atom_name(input: &[u8], index: &mut usize, length: usize) -> Result<(), DecodeError> {
    check_skip(input, index, length)
}

/// Take and check the creation of a legacy `PID_EXT`, `PORT_EXT`, or
/// `REFERENCE_EXT`: it is at most 3 (`is_tiny_creation`,
/// `external.c:71-72`).
fn check_tiny_creation(input: &[u8], index: &mut usize) -> Result<(), DecodeError> {
    let creation = *input.get(*index).ok_or(DecodeError::Badarg)?;
    *index += 1;
    (u32::from(creation) <= MAX_TINY_CREATION)
        .then_some(())
        .ok_or(DecodeError::Badarg)
}

/// Whether the term that a size check reads is inside a `LOCAL_EXT` term.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum NodeScope {
    /// The term is not in a `LOCAL_EXT` term: `LOCAL_EXT` cannot name its node.
    Remote,
    /// The term is in a `LOCAL_EXT` term, which only the local node decodes.
    Local,
}

/// Take the node atom of a pid, port, or reference at `index`.
fn check_sysname(input: &[u8], index: &mut usize, scope: NodeScope) -> Result<(), DecodeError> {
    match *input.get(*index).ok_or(DecodeError::Badarg)? {
        106 => {
            if scope == NodeScope::Remote {
                return Err(DecodeError::Badarg);
            }
            *index += 1;
            Ok(())
        }
        100 | 118 => {
            *index += 1;
            let length = check_length(input, index, 2)?;
            check_atom_name(input, index, length)
        }
        115 | 119 => {
            *index += 1;
            let length = usize::from(*input.get(*index).ok_or(DecodeError::Badarg)?);
            *index += 1;
            check_atom_name(input, index, length)
        }
        _ => Err(DecodeError::Badarg),
    }
}

/// Validate the structure of one external term as `decoded_size`
/// (`external.c:6150-6500`) does before the decoder adds an atom: the tags,
/// the lengths, the atom encodings, the reference counts, and the local
/// block hashes. A term that PBeam decodes to an unsupported form passes the
/// check, because its structure is valid.
#[expect(
    clippy::too_many_lines,
    reason = "mirrors the size walk `decoded_size` of `external.c`: one arm for each external term tag with the check of the node scope; each arm is short"
)]
fn validate(input: &[u8], seed: u32) -> Result<(), DecodeError> {
    let mut index = 0usize;
    let mut scope = NodeScope::Remote;
    let mut frames: Vec<CheckFrame> = Vec::new();
    'parse: loop {
        let tag = *input.get(index).ok_or(DecodeError::Badarg)?;
        index += 1;
        let mut complete = true;
        #[expect(
            clippy::match_same_arms,
            reason = "tag 117 is the old local fun tag, which the decoder names as its own case, so the size check keeps an arm for it apart from the wildcard"
        )]
        match tag {
            97 => check_skip(input, &mut index, 1)?,
            98 => check_skip(input, &mut index, 4)?,
            99 => check_skip(input, &mut index, FLOAT_TEXT_SIZE)?,
            70 => check_skip(input, &mut index, 8)?,
            100 | 118 => {
                let length = check_length(input, &mut index, 2)?;
                check_atom_name(input, &mut index, length)?;
            }
            115 | 119 => {
                let length = usize::from(*input.get(index).ok_or(DecodeError::Badarg)?);
                index += 1;
                check_atom_name(input, &mut index, length)?;
            }
            106 => {}
            107 => {
                let length = check_length(input, &mut index, 2)?;
                check_skip(input, &mut index, length)?;
            }
            108 => {
                let count = check_length(input, &mut index, 4)?;
                frames.push(CheckFrame {
                    kind: CheckKind::List,
                    remaining: count,
                });
                complete = false;
            }
            104 => {
                let count = usize::from(*input.get(index).ok_or(DecodeError::Badarg)?);
                index += 1;
                if count != 0 {
                    frames.push(CheckFrame {
                        kind: CheckKind::Tuple,
                        remaining: count,
                    });
                    complete = false;
                }
            }
            105 => {
                let count = check_length(input, &mut index, 4)?;
                if count > MAX_TUPLE_SIZE {
                    return Err(DecodeError::Badarg);
                }
                if count != 0 {
                    frames.push(CheckFrame {
                        kind: CheckKind::Tuple,
                        remaining: count,
                    });
                    complete = false;
                }
            }
            116 => {
                let count = check_length(input, &mut index, 4)?;
                let terms = count.checked_mul(2).ok_or(DecodeError::Badarg)?;
                if terms != 0 {
                    frames.push(CheckFrame {
                        kind: CheckKind::Map,
                        remaining: terms,
                    });
                    complete = false;
                }
            }
            109 => {
                let length = check_length(input, &mut index, 4)?;
                check_skip(input, &mut index, length)?;
            }
            77 => {
                let length = check_length(input, &mut index, 4)?;
                let bits = *input.get(index).ok_or(DecodeError::Badarg)?;
                index += 1;
                if length == 0 || !(1..=8).contains(&bits) {
                    return Err(DecodeError::Badarg);
                }
                check_skip(input, &mut index, length)?;
            }
            110 => {
                let length = usize::from(*input.get(index).ok_or(DecodeError::Badarg)?);
                index += 1;
                check_skip(input, &mut index, 1 + length)?;
            }
            111 => {
                let length = check_length(input, &mut index, 4)?;
                check_skip(input, &mut index, 1 + length)?;
            }
            88 | 120 => {
                check_sysname(input, &mut index, scope)?;
                check_skip(input, &mut index, 12)?;
            }
            103 => {
                check_sysname(input, &mut index, scope)?;
                check_skip(input, &mut index, 8)?;
                check_tiny_creation(input, &mut index)?;
            }
            89 => {
                check_sysname(input, &mut index, scope)?;
                check_skip(input, &mut index, 8)?;
            }
            102 | 101 => {
                check_sysname(input, &mut index, scope)?;
                check_skip(input, &mut index, 4)?;
                check_tiny_creation(input, &mut index)?;
            }
            90 => {
                let count = check_length(input, &mut index, 2)?;
                if count > MAX_REFERENCE_NUMBERS {
                    return Err(DecodeError::Badarg);
                }
                check_sysname(input, &mut index, scope)?;
                check_skip(input, &mut index, 4 + 4 * count)?;
            }
            114 => {
                let count = check_length(input, &mut index, 2)?;
                if count > MAX_REFERENCE_NUMBERS {
                    return Err(DecodeError::Badarg);
                }
                check_sysname(input, &mut index, scope)?;
                check_tiny_creation(input, &mut index)?;
                check_skip(input, &mut index, 4 * count)?;
            }
            113 => {
                frames.push(CheckFrame {
                    kind: CheckKind::Export,
                    remaining: 3,
                });
                complete = false;
            }
            112 => {
                // The size, the arity, the unique value, the index, and the
                // free count, then the module, the old index, the old
                // unique value, the creator pid, and the free variables
                // (`enc_term`, `external.c:3976-4005`).
                check_skip(input, &mut index, 4 + 1 + 16 + 4)?;
                let free = check_length(input, &mut index, 4)?;
                if free > MAX_FUN_ARG {
                    return Err(DecodeError::Badarg);
                }
                frames.push(CheckFrame {
                    kind: CheckKind::Tuple,
                    remaining: 4 + free,
                });
                complete = false;
            }
            117 => return Err(DecodeError::Badarg),
            67 => {
                // The field count, the exported flag, the module and the
                // name, the field names, and the values (`dec_term`,
                // `external.c:5264-5330`).
                let count = check_length(input, &mut index, 4)?;
                let exported = *input.get(index).ok_or(DecodeError::Badarg)?;
                index += 1;
                if exported > 1 {
                    return Err(DecodeError::Badarg);
                }
                frames.push(CheckFrame {
                    kind: CheckKind::Tuple,
                    remaining: 2 + 2 * count,
                });
                complete = false;
            }
            121 => {
                let hash = check_number(input, &mut index, 4)?;
                let prior = scope;
                scope = NodeScope::Local;
                frames.push(CheckFrame {
                    kind: CheckKind::Local {
                        hash,
                        start: index,
                        prior,
                    },
                    remaining: 1,
                });
                complete = false;
            }
            _ => return Err(DecodeError::Badarg),
        }
        if !complete {
            continue 'parse;
        }
        loop {
            let Some(frame) = frames.last_mut() else {
                return Ok(());
            };
            match frame.kind {
                CheckKind::Tuple | CheckKind::Map | CheckKind::Export => {
                    frame.remaining -= 1;
                    if frame.remaining == 0 {
                        frames.pop();
                        continue;
                    }
                    break;
                }
                CheckKind::List => {
                    if frame.remaining > 0 {
                        frame.remaining -= 1;
                        if frame.remaining == 0 {
                            continue 'parse;
                        }
                        break;
                    }
                    frames.pop();
                }
                CheckKind::Local { hash, start, prior } => {
                    frames.pop();
                    scope = prior;
                    if external::block_hash(&input[start..index], seed) != hash {
                        return Err(DecodeError::Badarg);
                    }
                }
            }
        }
    }
}

impl Decoder<'_> {
    /// Decode `VERSION_MAGIC`, the optional `COMPRESSED` header, and one
    /// term, as `binary2term_prepare` and `dec_term`
    /// (`external.c:1765-1845`, `4270-4300`) do. Return the term and the
    /// number of input bytes that the term and its header use.
    fn decode_input(&mut self, seed: u32) -> Result<(Term, usize), DecodeError> {
        if self.peek() != Some(VERSION_MAGIC) {
            return Err(DecodeError::Badarg);
        }
        self.pos = 1;
        let (plain, start, header) = if self.peek() == Some(COMPRESSED) {
            self.pos += 1;
            let declared = self.u32()?;
            let (plain, consumed) = self.inflate(declared)?;
            (Some(plain), 0, 6 + consumed)
        } else {
            (None, 1, 1)
        };
        let body: &[u8] = match &plain {
            Some(plain) => plain,
            None => &self.input[start..],
        };
        validate(body, seed)?;
        let mut decoder = Decoder {
            input: body,
            pos: 0,
            overrun: 0,
            safe: self.safe,
            local: false,
            atoms: self.atoms,
            code: self.code,
            ets: self.ets,
        };
        let term = decoder.term()?;
        // The compressed header counts the bytes of the stream, and the
        // decompressed body adds no byte of the input (`binary2term_prepare`,
        // `external.c:1826-1839`); the plain body adds its cells to the
        // magic byte.
        let used = if plain.is_some() {
            header
        } else {
            header + decoder.pos + decoder.overrun
        };
        Ok((term, used))
    }

    /// Return the tag byte at the cursor without taking it.
    fn peek(&self) -> Option<u8> {
        self.input.get(self.pos).copied()
    }

    /// Take `count` bytes.
    fn take(&mut self, count: usize) -> Result<&[u8], DecodeError> {
        let end = self.pos.checked_add(count).ok_or(DecodeError::Badarg)?;
        let bytes = self.input.get(self.pos..end).ok_or(DecodeError::Badarg)?;
        self.pos = end;
        Ok(bytes)
    }

    /// Take one byte.
    fn u8(&mut self) -> Result<u8, DecodeError> {
        Ok(self.take(1)?[0])
    }

    /// Take a big-endian 16-bit number.
    fn u16(&mut self) -> Result<u16, DecodeError> {
        Ok(u16::from_be_bytes(
            self.take(2)?.try_into().expect("two bytes"),
        ))
    }

    /// Take a big-endian 32-bit number as a length.
    fn length(&mut self) -> Result<usize, DecodeError> {
        Ok(usize::try_from(self.u32()?).expect("a u32 fits in usize"))
    }

    /// Take the first number of a reference with no numbers as `dec_term`
    /// does (`external.c:4780-4781`, `4795-4796`): it reads 4 bytes, which
    /// the size check of `decoded_size` does not count
    /// (`external.c:6217-6241`), so at the end of the input it reads after
    /// the end. Each byte after the end counts as 0 and adds 1 to
    /// [`Decoder::overrun`]; ERTS reads the memory after the binary there.
    fn first_reference_number(&mut self) -> u32 {
        let mut bytes = [0u8; 4];
        let rest = self.input.get(self.pos..).unwrap_or_default();
        let taken = rest.len().min(4);
        bytes[..taken].copy_from_slice(&rest[..taken]);
        self.pos += taken;
        self.overrun += 4 - taken;
        u32::from_be_bytes(bytes)
    }

    /// Take a big-endian 32-bit number.
    fn u32(&mut self) -> Result<u32, DecodeError> {
        Ok(u32::from_be_bytes(
            self.take(4)?.try_into().expect("four bytes"),
        ))
    }

    /// Take a big-endian 64-bit number.
    fn u64(&mut self) -> Result<u64, DecodeError> {
        Ok(u64::from_be_bytes(
            self.take(8)?.try_into().expect("eight bytes"),
        ))
    }

    /// Return the number of bytes that the cursor does not use.
    fn remaining(&self) -> usize {
        self.input.len().saturating_sub(self.pos)
    }

    /// Inflate the compressed term bytes as `binary2term_prepare`
    /// (`external.c:1783-1843`) does: the result must hold exactly
    /// `declared` bytes, as `erl_zlib_uncompress` requires. Return the bytes
    /// and the number of compressed bytes that the stream uses.
    fn inflate(&mut self, declared: u32) -> Result<(Vec<u8>, usize), DecodeError> {
        let declared = usize::try_from(declared).expect("a u32 fits in usize");
        let compressed = &self.input[self.pos..];
        let mut decompress = Decompress::new(true);
        let mut out = Vec::new();
        let mut chunk = vec![0u8; 64 * 1024];
        loop {
            let (before_in, before_out) = (decompress.total_in(), decompress.total_out());
            let input = &compressed[to_usize(before_in)..];
            let status = decompress
                .decompress(input, &mut chunk, FlushDecompress::None)
                .map_err(|_| DecodeError::Badarg)?;
            out.extend_from_slice(&chunk[..to_usize(decompress.total_out() - before_out)]);
            if out.len() > declared {
                return Err(DecodeError::Badarg);
            }
            let stalled =
                decompress.total_in() == before_in && decompress.total_out() == before_out;
            match status {
                Status::StreamEnd => break,
                Status::Ok | Status::BufError => {
                    if stalled {
                        return Err(DecodeError::Badarg);
                    }
                }
            }
        }
        if out.len() != declared {
            return Err(DecodeError::Badarg);
        }
        Ok((out, to_usize(decompress.total_in())))
    }

    /// Decode one term, as `dec_term` (`external.c:4270-5340`) does. The
    /// frames hold the containers that the walk still fills, so a deep term
    /// uses no Rust stack.
    #[expect(
        clippy::too_many_lines,
        reason = "mirrors the decoder `dec_term` of `external.c`: one arm for each external term tag, and the work stack that fills the containers"
    )]
    fn term(&mut self) -> Result<Term, DecodeError> {
        let mut frames: Vec<Frame> = Vec::new();
        'parse: loop {
            let tag = self.u8()?;
            // The module, the name, and the field names of a `RECORD_EXT`
            // take only an atom tag (`dec_atom`, `external.c:5300-5318`).
            // The header ends when the values remain.
            let mut term = if let Some(frame) = frames.last_mut()
                && let FrameKind::Record {
                    exported,
                    header,
                    def: def @ None,
                } = &mut frame.kind
            {
                header.push(self.header_atom(tag)?);
                frame.remaining -= 1;
                if header.len() < frame.remaining + 2 {
                    continue 'parse;
                }
                let record = self.record_def(*exported, std::mem::take(header))?;
                if frame.remaining > 0 {
                    *def = Some(record);
                    continue 'parse;
                }
                frames.pop();
                record_term(record, Vec::new())
            } else {
                match tag {
                    97 => Term::Integer(i64::from(self.u8()?)),
                    98 => Term::Integer(i64::from(i32::from_be_bytes(
                        self.take(4)?.try_into().expect("four bytes"),
                    ))),
                    99 => self.float_text()?,
                    70 => {
                        let bits =
                            u64::from_be_bytes(self.take(8)?.try_into().expect("eight bytes"));
                        let value = f64::from_bits(bits);
                        value
                            .is_finite()
                            .then_some(Term::Float(value))
                            .ok_or(DecodeError::Badarg)?
                    }
                    100 => {
                        let length = usize::from(self.u16()?);
                        Term::Atom(self.atom(length, CharEncoding::Latin1)?)
                    }
                    115 => {
                        let length = usize::from(self.u8()?);
                        Term::Atom(self.atom(length, CharEncoding::Latin1)?)
                    }
                    118 => {
                        let length = usize::from(self.u16()?);
                        Term::Atom(self.atom(length, CharEncoding::Unicode)?)
                    }
                    119 => {
                        let length = usize::from(self.u8()?);
                        Term::Atom(self.atom(length, CharEncoding::Unicode)?)
                    }
                    106 => Term::Nil,
                    107 => {
                        let count = usize::from(self.u16()?);
                        let bytes = self.take(count)?;
                        Term::list_from(bytes.iter().map(|&byte| Term::Integer(i64::from(byte))))
                    }
                    108 => {
                        let count = self.length()?;
                        if count > self.remaining() {
                            return Err(DecodeError::Badarg);
                        }
                        frames.push(Frame {
                            kind: FrameKind::List,
                            remaining: count,
                            items: Vec::with_capacity(count),
                        });
                        continue 'parse;
                    }
                    104 => {
                        let count = usize::from(self.u8()?);
                        if count > self.remaining() {
                            return Err(DecodeError::Badarg);
                        }
                        if count == 0 {
                            Term::tuple(vec![])
                        } else {
                            frames.push(Frame {
                                kind: FrameKind::Tuple,
                                remaining: count,
                                items: Vec::with_capacity(count),
                            });
                            continue 'parse;
                        }
                    }
                    105 => {
                        let count = self.length()?;
                        if count > MAX_TUPLE_SIZE || count > self.remaining() {
                            return Err(DecodeError::Badarg);
                        }
                        if count == 0 {
                            Term::tuple(vec![])
                        } else {
                            frames.push(Frame {
                                kind: FrameKind::Tuple,
                                remaining: count,
                                items: Vec::with_capacity(count),
                            });
                            continue 'parse;
                        }
                    }
                    116 => {
                        let count = self.length()?;
                        if count
                            .checked_mul(2)
                            .is_none_or(|terms| terms > self.remaining())
                        {
                            return Err(DecodeError::Badarg);
                        }
                        if count == 0 {
                            self.build_map(Vec::new())?
                        } else {
                            frames.push(Frame {
                                kind: FrameKind::Map,
                                remaining: count * 2,
                                items: Vec::with_capacity(count * 2),
                            });
                            continue 'parse;
                        }
                    }
                    109 => {
                        let count = self.length()?;
                        Term::binary(self.take(count)?.to_vec())
                    }
                    77 => {
                        let count = self.length()?;
                        let bits = self.u8()?;
                        if count == 0 || !(1..=8).contains(&bits) {
                            return Err(DecodeError::Badarg);
                        }
                        // `dec_term` keeps the high `bits` bits of the last
                        // byte (`external.c:4895-4923`).
                        let bytes = Binary::from(self.take(count)?.to_vec());
                        Term::Binary(bytes.bits(0, count * 8 - (8 - usize::from(bits))))
                    }
                    110 => {
                        let count = usize::from(self.u8()?);
                        self.big(count)?
                    }
                    111 => {
                        let count = self.length()?;
                        self.big(count)?
                    }
                    88 => self.pid(4)?,
                    103 => self.pid(1)?,
                    89 => self.port(4)?,
                    102 => self.port(1)?,
                    120 => {
                        let sysname = self.sysname()?;
                        let number = self.u64()?;
                        let creation = self.u32()?;
                        self.local_port(sysname, number, creation)?
                    }
                    90 => self.reference(4)?,
                    114 => self.reference(1)?,
                    101 => self.old_reference()?,
                    113 => {
                        frames.push(Frame {
                            kind: FrameKind::Export,
                            remaining: 3,
                            items: Vec::with_capacity(3),
                        });
                        continue 'parse;
                    }
                    112 => {
                        // The size, which the decoder skips, the arity, the
                        // module MD5, the index, and the free count; then the
                        // module, the old index, the old unique value, the
                        // creator pid, and the free variables (`dec_term`,
                        // `external.c:5102-5176`).
                        self.take(4)?;
                        let arity = self.take(1)?[0];
                        let uniq: [u8; 16] =
                            self.take(16)?.try_into().expect("16 bytes of the MD5");
                        let index = self.u32()?.cast_signed();
                        let free = self.length()?;
                        frames.push(Frame {
                            kind: FrameKind::Fun { arity, uniq, index },
                            remaining: 4 + free,
                            items: Vec::with_capacity(4 + free),
                        });
                        continue 'parse;
                    }
                    117 => return Err(DecodeError::Unsupported("an old local fun")),
                    67 => {
                        // The field count, the export flag that `validate`
                        // checks, then the module, the name, the field names,
                        // and the values (`dec_term`, `external.c:5257-5380`).
                        let count = self.length()?;
                        let exported = self.u8()? == 1;
                        frames.push(Frame {
                            kind: FrameKind::Record {
                                exported,
                                header: Vec::with_capacity(2 + count),
                                def: None,
                            },
                            remaining: 2 + 2 * count,
                            items: Vec::with_capacity(count),
                        });
                        continue 'parse;
                    }
                    121 => {
                        self.take(4)?;
                        let prior = self.local;
                        self.local = true;
                        frames.push(Frame {
                            kind: FrameKind::Local { prior },
                            remaining: 1,
                            items: Vec::new(),
                        });
                        continue 'parse;
                    }
                    _ => return Err(DecodeError::Badarg),
                }
            };
            loop {
                let Some(frame) = frames.last_mut() else {
                    return Ok(term);
                };
                match frame.kind {
                    FrameKind::Tuple => {
                        frame.items.push(term);
                        frame.remaining -= 1;
                        if frame.remaining == 0 {
                            let frame = frames.pop().expect("a tuple frame");
                            term = Term::tuple(frame.items);
                            continue;
                        }
                        break;
                    }
                    FrameKind::List => {
                        if frame.remaining > 0 {
                            frame.items.push(term);
                            frame.remaining -= 1;
                            if frame.remaining == 0 {
                                continue 'parse;
                            }
                            break;
                        }
                        let frame = frames.pop().expect("a list frame");
                        let mut list = term;
                        for head in frame.items.into_iter().rev() {
                            list = Term::cons(head, list);
                        }
                        term = list;
                    }
                    FrameKind::Map => {
                        frame.items.push(term);
                        frame.remaining -= 1;
                        if frame.remaining == 0 {
                            let frame = frames.pop().expect("a map frame");
                            term = self.build_map(frame.items)?;
                            continue;
                        }
                        break;
                    }
                    FrameKind::Export => {
                        frame.items.push(term);
                        frame.remaining -= 1;
                        if frame.remaining == 0 {
                            let frame = frames.pop().expect("an export frame");
                            term = self.complete_export(&frame.items)?;
                            continue;
                        }
                        break;
                    }
                    FrameKind::Fun { arity, uniq, index } => {
                        frame.items.push(term);
                        frame.remaining -= 1;
                        if frame.remaining == 0 {
                            let frame = frames.pop().expect("a fun frame");
                            term = self.complete_fun(arity, uniq, index, frame.items)?;
                            continue;
                        }
                        break;
                    }
                    FrameKind::Record { ref mut def, .. } => {
                        frame.items.push(term);
                        frame.remaining -= 1;
                        if frame.remaining == 0 {
                            let def = def.take().expect("the header precedes the values");
                            let items = std::mem::take(&mut frame.items);
                            frames.pop();
                            term = record_term(def, items);
                            continue;
                        }
                        break;
                    }
                    FrameKind::Local { prior } => {
                        frames.pop();
                        self.local = prior;
                    }
                }
            }
        }
    }

    /// Return the definition of a `RECORD_EXT` header: the module, the
    /// name, and the field names in definition order. A field name that
    /// occurs twice is `badarg` before any value is decoded. The definition
    /// is the loaded one when it has the same export flag and fields
    /// (`dec_term`, `external.c:5300-5367`).
    #[expect(
        clippy::fn_params_excessive_bools,
        reason = "`exported` is the `is_exported` boolean of the `RECORD_EXT` header (`external.c:5300-5367`), passed to `RecordDef::new` as data"
    )]
    fn record_def(
        &self,
        exported: bool,
        header: Vec<crate::term::Atom>,
    ) -> Result<Arc<RecordDef>, DecodeError> {
        let mut atoms = header.into_iter();
        let module = atoms.next().expect("a module");
        let name = atoms.next().expect("a name");
        let fields = atoms.map(|key| (key, None)).collect();
        let def = RecordDef::new(module, name, exported, fields, self.atoms);
        if def.keys.windows(2).any(|pair| pair[0] == pair[1]) {
            return Err(DecodeError::Badarg);
        }
        Ok(records::canonical(self.code, self.atoms, def))
    }

    /// Decode `FLOAT_EXT`: 31 bytes of C text, as `sys_chars_to_double`
    /// reads them (`external.c:4636-4651`).
    fn float_text(&mut self) -> Result<Term, DecodeError> {
        let bytes = self.take(FLOAT_TEXT_SIZE)?;
        let end = bytes
            .iter()
            .position(|&byte| byte == 0)
            .unwrap_or(bytes.len());
        let text = std::str::from_utf8(&bytes[..end]).map_err(|_| DecodeError::Badarg)?;
        // `sys_chars_to_double` takes an optional sign, digits, one '.' or
        // ',', digits, and an optional exponent with no other bytes
        // (`sys/unix/sys_float.c:74-135`). `strtod` reads either radix
        // character there, so the parse here reads '.' for both.
        let syntax = text.as_bytes();
        let mut index = usize::from(matches!(syntax.first(), Some(b'+' | b'-')));
        let digits = |index: &mut usize| {
            let start = *index;
            while syntax.get(*index).is_some_and(u8::is_ascii_digit) {
                *index += 1;
            }
            *index > start
        };
        if !digits(&mut index) || !matches!(syntax.get(index), Some(b'.' | b',')) || {
            index += 1;
            !digits(&mut index)
        } {
            return Err(DecodeError::Badarg);
        }
        if matches!(syntax.get(index), Some(b'e' | b'E')) {
            index += 1;
            if matches!(syntax.get(index), Some(b'+' | b'-')) {
                index += 1;
            }
            if !digits(&mut index) {
                return Err(DecodeError::Badarg);
            }
        }
        if index != syntax.len() {
            return Err(DecodeError::Badarg);
        }
        let value: f64 = text
            .replace(',', ".")
            .parse()
            .map_err(|_| DecodeError::Badarg)?;
        value
            .is_finite()
            .then_some(Term::Float(value))
            .ok_or(DecodeError::Badarg)
    }

    /// Decode an atom of a `RECORD_EXT` header. Each other tag is `badarg`
    /// (`dec_atom`, `external.c:5300-5318`).
    fn header_atom(&mut self, tag: u8) -> Result<crate::term::Atom, DecodeError> {
        match tag {
            100 | 118 => {
                let length = usize::from(self.u16()?);
                self.atom(length, CharEncoding::from(tag == 100))
            }
            115 | 119 => {
                let length = usize::from(self.u8()?);
                self.atom(length, CharEncoding::from(tag == 115))
            }
            _ => Err(DecodeError::Badarg),
        }
    }

    /// Decode an atom of `length` bytes in Latin-1 or UTF-8, as
    /// `dec_term_atom_common` (`external.c:4525-4548`) does. The `safe`
    /// option takes only an atom that the run has.
    fn atom(
        &mut self,
        length: usize,
        charset: CharEncoding,
    ) -> Result<crate::term::Atom, DecodeError> {
        let bytes = self.take(length)?;
        let name = if charset == CharEncoding::Latin1 {
            bytes.iter().map(|&byte| char::from(byte)).collect()
        } else {
            std::str::from_utf8(bytes)
                .map_err(|_| DecodeError::Badarg)?
                .to_string()
        };
        if name.chars().count() > MAX_ATOM_CHARACTERS {
            return Err(DecodeError::Badarg);
        }
        if self.safe {
            self.atoms.get(&name).ok_or(DecodeError::Badarg)
        } else {
            self.atoms.put(&name).map_err(DecodeError::Error)
        }
    }

    /// Build a map of the decoded pair terms as the `MAP_EXT` case of
    /// `dec_term` (`external.c:5069-5150`) does. A duplicate key raises
    /// badarg, and a key that PBeam cannot hold gives a runtime error.
    fn build_map(&mut self, items: Vec<Term>) -> Result<Term, DecodeError> {
        let count = items.len() / 2;
        let mut pairs = Vec::with_capacity(count);
        let mut items = items.into_iter();
        while let (Some(key), Some(value)) = (items.next(), items.next()) {
            pairs.push((key, value));
        }
        let map = crate::term::Map::from_pairs(pairs, self.atoms)
            .map_err(|_| DecodeError::Unsupported("a map with a key that PBeam cannot hold"))?;
        if map.len() != count {
            return Err(DecodeError::Badarg);
        }
        Ok(Term::Map(map))
    }

    /// Decode a bignum of `count` magnitude bytes and a sign byte, as the
    /// `BIG_DEF` cases of `dec_term` (`external.c:4453-4500`) do: strip the
    /// leading zero bytes, make 0 of a zero magnitude, and make an
    /// `INTEGER_EXT`-range value a small integer.
    fn big(&mut self, count: usize) -> Result<Term, DecodeError> {
        let negative = self.u8()? != 0;
        let bytes = self.take(count)?;
        let first = bytes
            .iter()
            .rposition(|&byte| byte != 0)
            .map_or(0, |index| index + 1);
        let magnitude = &bytes[..first];
        if magnitude.is_empty() {
            return Ok(Term::Integer(0));
        }
        let mut be = magnitude.iter().rev().copied().collect::<Vec<u8>>();
        let sign = if negative { Sign::Minus } else { Sign::Plus };
        let value = BigInt::from_bytes_be(sign, &be);
        be.clear();
        match value.to_i64() {
            Some(value) => Ok(Term::Integer(value)),
            None => Ok(Term::BigInt(Arc::new(value))),
        }
    }

    /// Decode the node atom of a pid, port, or reference: `NIL_EXT` for the
    /// local node inside `LOCAL_EXT`, else an atom tag (`dec_sysname` and
    /// `dec_atom`, `external.c:3110-3122`, `4490-4550`). Each other tag
    /// raises badarg.
    fn sysname(&mut self) -> Result<Option<crate::term::Atom>, DecodeError> {
        if self.peek() == Some(106) {
            if !self.local {
                return Err(DecodeError::Badarg);
            }
            self.pos += 1;
            return Ok(None);
        }
        let atom = match self.u8()? {
            100 => {
                let length = usize::from(self.u16()?);
                self.atom(length, CharEncoding::Latin1)?
            }
            115 => {
                let length = usize::from(self.u8()?);
                self.atom(length, CharEncoding::Latin1)?
            }
            118 => {
                let length = usize::from(self.u16()?);
                self.atom(length, CharEncoding::Unicode)?
            }
            119 => {
                let length = usize::from(self.u8()?);
                self.atom(length, CharEncoding::Unicode)?
            }
            _ => return Err(DecodeError::Badarg),
        };
        Ok(Some(atom))
    }

    /// Return true when `sysname` and `creation` name the local node, as
    /// `dec_is_this_node` (`external.c:3124-3131`) does.
    fn is_local(&self, sysname: Option<crate::term::Atom>, creation: u32) -> bool {
        if creation != 0 {
            return false;
        }
        let atom =
            crate::term::Atom::from_static("nonode@nohost").expect("the local node atom is static");
        match sysname {
            None => true,
            Some(sysname) => self.atoms.name_of(&sysname) == self.atoms.name_of(&atom),
        }
    }

    /// Return the node name of `sysname` and `creation` when they do not
    /// name the local node, as `dec_get_node` (`external.c:3133-3139`)
    /// gives another node entry.
    fn remote(
        &self,
        sysname: Option<crate::term::Atom>,
        creation: u32,
    ) -> Option<crate::term::Atom> {
        if self.is_local(sysname.clone(), creation) {
            None
        } else {
            sysname
        }
    }

    /// Decode a pid as `dec_pid` (`external.c:3133-3200`) does. `tag` is 4
    /// for `NEW_PID_EXT` and 1 for `PID_EXT`.
    fn pid(&mut self, tag: u8) -> Result<Term, DecodeError> {
        let sysname = self.sysname()?;
        let number = self.u32()?;
        let serial = self.u32()?;
        let creation = if tag == 4 {
            self.u32()?
        } else {
            let creation = u32::from(self.u8()?);
            if creation > MAX_TINY_CREATION {
                return Err(DecodeError::Badarg);
            }
            creation
        };
        if let Some(node) = self.remote(sysname, creation) {
            return Ok(remote_id(
                node,
                creation,
                ExternalData::Pid { number, serial },
            ));
        }
        if number > MAX_PID_NUMBER {
            return Err(DecodeError::Badarg);
        }
        Ok(Term::Pid(ProcessId(
            (u64::from(serial) << PID_NUMBER_BITS) | u64::from(number),
        )))
    }

    /// Decode a port of `tag` 4 (`NEW_PORT_EXT`) or 1 (`PORT_EXT`).
    fn port(&mut self, tag: u8) -> Result<Term, DecodeError> {
        let sysname = self.sysname()?;
        let number = u64::from(self.u32()?);
        let creation = if tag == 4 {
            self.u32()?
        } else {
            let creation = u32::from(self.u8()?);
            if creation > MAX_TINY_CREATION {
                return Err(DecodeError::Badarg);
            }
            creation
        };
        self.local_port(sysname, number, creation)
    }

    /// Return the local port of `number`, or a runtime error for a port of
    /// another node.
    fn local_port(
        &self,
        sysname: Option<crate::term::Atom>,
        number: u64,
        creation: u32,
    ) -> Result<Term, DecodeError> {
        if let Some(node) = self.remote(sysname, creation) {
            return Ok(remote_id(node, creation, ExternalData::Port(number)));
        }
        if number > MAX_PORT_NUMBER {
            return Err(DecodeError::Badarg);
        }
        Ok(Term::Port(crate::term::PortId(number)))
    }

    /// Decode a reference as the `NEWER_REFERENCE_EXT` and
    /// `NEW_REFERENCE_EXT` cases of `dec_term` (`external.c:4786-4855`) do.
    /// `tag` is 4 for the 32-bit creation and 1 for the 8-bit creation.
    fn reference(&mut self, tag: u8) -> Result<Term, DecodeError> {
        let count = usize::from(self.u16()?);
        if count > MAX_REFERENCE_NUMBERS {
            return Err(DecodeError::Badarg);
        }
        let sysname = self.sysname()?;
        let creation = if tag == 4 {
            self.u32()?
        } else {
            let creation = u32::from(self.u8()?);
            if creation > MAX_TINY_CREATION {
                return Err(DecodeError::Badarg);
            }
            creation
        };
        // ERTS reads the first reference number even when the count is
        // zero (`dec_term`, `external.c:4780-4781`, `4795-4796`).
        let first = if count == 0 {
            self.first_reference_number()
        } else {
            self.u32()?
        };
        // `NEW_REFERENCE_EXT` rejects a first number of 18 bits or more
        // also for another node (`external.c:4780-4783`).
        if tag == 1 && first >= MAX_REFERENCE {
            return Err(DecodeError::Badarg);
        }
        let mut read = vec![first];
        for _ in 1..count {
            read.push(self.u32()?);
        }
        // A reference of another node keeps its `count` numbers; ERTS sets
        // the first number word of a count of 0 to 0
        // (`external.c:4859-4890`).
        if let Some(node) = self.remote(sysname, creation) {
            read.truncate(count);
            return Ok(remote_id(
                node,
                creation,
                ExternalData::Reference(read.into_boxed_slice()),
            ));
        }
        let mut numbers = [0u32; 3];
        for (number, value) in numbers.iter_mut().zip(&read) {
            *number = *value;
        }
        if numbers[0] >= MAX_REFERENCE {
            return Err(DecodeError::Badarg);
        }
        // ERTS marks a pid reference in the second reference number
        // (`ERTS_REF1_PID_MARKER_BIT__`, `erl_bif_unique.h:106-109`,
        // `140-156`). The alias reference holds the monitored pid, which
        // PBeam does not keep.
        if numbers[1] & PID_REFERENCE_BIT != 0 {
            return Err(DecodeError::Unsupported("a pid reference"));
        }
        // A PBeam reference belongs to the first scheduler, whose number is
        // in the low bits of the second reference number (`REF_THREAD`,
        // `erl_bif_unique.h:117-131`).
        // ERTS marks a magic reference in bit 17 of the second reference
        // number (`ERTS_REF1_MAGIC_MARKER_BIT__`, `erl_bif_unique.h:102-105`).
        // `dec_term` gives the live magic reference, or an ordinary
        // reference with the same numbers when the magic binary is gone
        // (`external.c:4845-4851`). The two compare equal, and PBeam keys its
        // tables by the magic value, so the magic value finds the live table.
        // A live resource object gives its resource term.
        let magic = numbers[1] & MAGIC_REFERENCE_BIT != 0;
        if numbers[1] & !(REF_NUMBER_1_MASK | MAGIC_REFERENCE_BIT) != REF_THREAD {
            return Err(DecodeError::Unsupported("a reference of another scheduler"));
        }
        let value = (u64::from(numbers[2]) << 32)
            | (u64::from(numbers[1]) & 0xFFFC_0000)
            | u64::from(numbers[0]);
        // The high bit of the third reference number is not a marker in
        // ERTS, and PBeam keeps its pid marker in bit 63 of the value, so
        // PBeam cannot hold such an ordinary reference.
        Reference::ordinary(value)
            .map(|reference| {
                if !magic {
                    return Term::Reference(reference);
                }
                let reference = reference.magic_reference();
                crate::runtime::nif::abi::find_resource(self.atoms, reference)
                    .unwrap_or_else(|| Term::Reference(self.live_magic(reference)))
            })
            .ok_or(DecodeError::Unsupported(
                "a reference with the high bit of its third number",
            ))
    }

    /// Return the identifier of the live or deleted table of the magic
    /// reference `reference`, else `reference`. ERTS gives the magic
    /// reference itself (`external.c:4845-4851`), which stays a table
    /// identifier after the table is deleted (`tid2tab`, `erl_db.c:280-306`).
    /// ERTS gives an ordinary reference, and so the cause `type`, only after
    /// the garbage collector frees each holder of the identifier. PBeam has
    /// no such collector, so the deleted set of the tables keeps the common
    /// case.
    fn live_magic(&self, reference: Reference) -> Reference {
        self.ets.shared().decoded(self.atoms, reference)
    }

    /// Decode an old `REFERENCE_EXT`, as `dec_term`
    /// (`external.c:4743-4766`) does.
    fn old_reference(&mut self) -> Result<Term, DecodeError> {
        let sysname = self.sysname()?;
        let number = self.u32()?;
        let creation = u32::from(self.u8()?);
        if creation > MAX_TINY_CREATION {
            return Err(DecodeError::Badarg);
        }
        if number >= MAX_REFERENCE {
            return Err(DecodeError::Badarg);
        }
        // A reference of another node keeps its one number
        // (`external.c:4753-4766`, `4797-4890`).
        if let Some(node) = self.remote(sysname, creation) {
            return Ok(remote_id(
                node,
                creation,
                ExternalData::Reference(Box::new([number])),
            ));
        }
        // An old reference has no scheduler number, so its second reference
        // number is 0 (`dec_term`, `external.c:4786-4855`). PBeam holds the
        // references of its first scheduler only.
        Err(DecodeError::Unsupported("a reference of another scheduler"))
    }

    /// Build the local fun of a decoded `NEW_FUN_EXT` as its case in
    /// `dec_term` (`external.c:5102-5176`) does. The module must be an atom,
    /// the old index and the old unique value small integers, and the
    /// creator a pid, else the decode gives badarg. ERTS keeps the old index
    /// and the old unique value in a C `int` (`erl_fun.c:299-304`), so they
    /// wrap to 32 bits. The fun gets the entry of the loaded code with the
    /// key of `fun_cmp` (`erl_fun.c:79-85`), or else a new stub entry with
    /// the decoded values (`erts_fun_entry_get_or_make_stub`,
    /// `erl_fun.c:299-309`).
    #[expect(
        clippy::as_conversions,
        clippy::cast_possible_truncation,
        reason = "ERTS keeps the old index and the old unique value in a C `int` (`erl_fun.c:299-304`), so they wrap to 32 bits"
    )]
    fn complete_fun(
        &mut self,
        arity: u8,
        uniq: [u8; 16],
        index: i32,
        mut items: Vec<Term>,
    ) -> Result<Term, DecodeError> {
        let free = items.split_off(4);
        let [Term::Atom(module), old_index, old_unique, Term::Pid(_)] = &items[..] else {
            return Err(DecodeError::Badarg);
        };
        let small = |term: &Term| match term {
            Term::Integer(value) if (integer::MIN_SMALL..=integer::MAX_SMALL).contains(value) => {
                Ok(*value as i32)
            }
            _ => Err(DecodeError::Badarg),
        };
        let (old_index, old_unique) = (small(old_index)?, small(old_unique)?);
        let arity = u32::from(arity);
        let lambda = self
            .code
            .fun_entry(self.atoms.name_of(module), &uniq, index, arity)
            .unwrap_or_else(|| {
                Arc::new(
                    Lambda::new(module.clone(), index, old_index, old_unique, 0, arity)
                        .with_uniq(uniq),
                )
            });
        Ok(Term::Fun(Fun::new(lambda, free)))
    }

    /// Build the external fun of a decoded `EXPORT_EXT` as its case in
    /// `dec_term` (`external.c:5004-5047`) does. The `safe` option gives
    /// badarg, because PBeam does not check that the export is active.
    fn complete_export(&mut self, items: &[Term]) -> Result<Term, DecodeError> {
        let Term::Atom(module) = &items[0] else {
            return Err(DecodeError::Badarg);
        };
        let Term::Atom(function) = &items[1] else {
            return Err(DecodeError::Badarg);
        };
        let Term::Integer(arity) = items[2] else {
            return Err(DecodeError::Badarg);
        };
        if arity < 0 {
            return Err(DecodeError::Badarg);
        }
        if self.safe {
            return Err(DecodeError::Unsupported(
                "an external fun under the safe option",
            ));
        }
        if arity > i64::from(u32::MAX) {
            return Err(DecodeError::Unsupported(
                "an external fun with an arity above 2^32 - 1",
            ));
        }
        Ok(Term::Fun(Fun::external(
            module.clone(),
            function.clone(),
            u32::try_from(arity).expect("the checks above bound the arity to a u32"),
        )))
    }
}

#[cfg(test)]
#[path = "../../../../tests/runtime/interpreter/bif/term.rs"]
mod tests;

//! Bodies of `erlang:term_to_binary/1,2`, `erlang:binary_to_term/1,2`, and
//! `erlang:external_size/1`: the external term format of ERTS OTP 29.1.1
//! (`external.c`).
//!
//! The encoder writes the tags of `external.h` for each term that PBeam can
//! make. A local fun has no bytes that PBeam can compute: the `NEW_FUN_EXT`
//! form holds the 16-byte unique value of the `FunT` entry, and PBeam does
//! not load that value. A local fun, a record, a bitstring of a size that is
//! not a whole number of bytes, and a pid, port, or reference of another
//! node fail with a runtime error. Row "External term format" of
//! `docs/erts-differences.md` gives these differences.
//!
//! # Examples
//!
//! ```rust,ignore
//! let bytes = encode(&Term::Integer(1), &atoms, &Options::default())?;
//! assert_eq!(bytes, [97, 1]);
//! ```

use std::{cmp::Ordering, io::Write, sync::Arc};

use flate2::{Compression, Decompress, FlushDecompress, Status, write::ZlibEncoder};
use num_bigint::{BigInt, Sign};
use num_traits::ToPrimitive;

use super::BifResult::Value;
use super::*;
use crate::term::{
    AtomTable, Fun, MAX_ATOM_CHARACTERS, MAX_TUPLE_SIZE, ProcessId, Reference, Target, external,
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
                        0..=2 => *value as u8,
                        _ => return None,
                    };
                } else if name.is_static("compressed") {
                    parsed.level = match value {
                        0..=9 => *value as u32,
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
    // The existing `[local]` reference path of `erlang.rs` gives the bytes
    // that `prim_inet:send/3` needs; keep it for that one case.
    let local = Term::list_from(vec![Term::static_atom("local")]);
    if matches!(term, Term::Reference(reference) if !reference.is_pid_reference())
        && *options == local
    {
        return super::erlang::term_to_binary_2(context, term, options);
    }
    let Some(options) = parse_options(options) else {
        return Ok(BifResult::badarg());
    };
    let scheduler = &mut *context.scheduler;
    let seed = scheduler.local_node_hash();
    let mut body = encode(term, &scheduler.atoms, &options)?;
    if options.local {
        let hash = external::block_hash(&body, seed);
        body = [&[LOCAL_EXT][..], &hash.to_be_bytes(), &body].concat();
    }
    let bytes = compress_term(&body, options.level);
    // The one-go path of `erts_term_to_binary_int` charges 1 reduction
    // (`external.c:2520-2530`). See row "External term format" of
    // `docs/erts-differences.md` for the charge of a larger term.
    scheduler.current_process_mut().bump_reductions(1);
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
            &(body.len() as u32).to_be_bytes(),
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
    Ok(Value(Term::Integer(1 + body.len() as i64)))
}

/// Return the external term bytes of `term` without `VERSION_MAGIC`, as
/// `enc_term` (`external.c:3252-4060`) writes them.
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
/// A local fun, a record, a bitstring, or a match context gives a runtime
/// error, and a full atom table gives its error.
fn encode(term: &Term, atoms: &AtomTable, options: &Options) -> Result<Vec<u8>, BeamError> {
    let mut bytes = Vec::new();
    encode_term(term, atoms, options, &mut bytes)?;
    Ok(bytes)
}

/// Write the external term bytes of `term` to `out`.
///
/// # Errors
///
/// The errors of [`encode`].
fn encode_term(
    term: &Term,
    atoms: &AtomTable,
    options: &Options,
    out: &mut Vec<u8>,
) -> Result<(), BeamError> {
    match term {
        Term::Nil => out.push(106),
        Term::Atom(atom) => encode_atom(atoms.name_of(atom), options, out),
        Term::Integer(value) => encode_integer(*value, out),
        Term::BigInt(value) => encode_big(value, out),
        Term::Float(value) if options.minor == 0 => {
            out.push(99);
            let text = float_text::scientific(*value, 17);
            out.extend_from_slice(&text);
            out.resize(out.len() + FLOAT_TEXT_SIZE - text.len(), 0);
        }
        Term::Float(value) => {
            out.push(70);
            out.extend_from_slice(&value.to_bits().to_be_bytes());
        }
        Term::Cons(_) => encode_list(term, atoms, options, out)?,
        Term::Tuple(elements) => {
            if elements.len() <= 0xff {
                out.extend_from_slice(&[104, elements.len() as u8]);
            } else {
                out.push(105);
                out.extend_from_slice(&(elements.len() as u32).to_be_bytes());
            }
            for element in elements.iter() {
                encode_term(element, atoms, options, out)?;
            }
        }
        Term::Map(map) => {
            out.push(116);
            out.extend_from_slice(&(map.len() as u32).to_be_bytes());
            let mut pairs: Vec<(&Term, &Term)> = map.iter().collect();
            if options.deterministic {
                pairs.sort_by(|(left, _), (right, _)| exact_map_key_order(left, right, atoms));
            } else if map.len() <= 32 {
                // A flatmap of ERTS keeps its pairs in descending key order
                // (`erl_map.c`), while `Map::iter` gives the ascending order.
                pairs.reverse();
            }
            for (key, value) in pairs {
                encode_term(key, atoms, options, out)?;
                encode_term(value, atoms, options, out)?;
            }
        }
        Term::Binary(binary) => {
            out.push(109);
            out.extend_from_slice(&(binary.len() as u32).to_be_bytes());
            out.extend_from_slice(binary);
        }
        Term::Pid(ProcessId(data)) => {
            out.push(88);
            encode_sysname(atoms, options, out);
            out.extend_from_slice(&((data & u64::from(MAX_PID_NUMBER)) as u32).to_be_bytes());
            out.extend_from_slice(&((data >> PID_NUMBER_BITS) as u32).to_be_bytes());
            out.extend_from_slice(&0u32.to_be_bytes());
        }
        Term::Port(port) => {
            out.push(120);
            encode_sysname(atoms, options, out);
            out.extend_from_slice(&port.0.to_be_bytes());
            out.extend_from_slice(&0u32.to_be_bytes());
        }
        Term::Reference(reference) => {
            out.push(90);
            out.extend_from_slice(&3u16.to_be_bytes());
            encode_sysname(atoms, options, out);
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
                encode_atom(atoms.name_of(module), options, out);
                encode_atom(atoms.name_of(function), options, out);
                encode_integer(i64::from(*arity), out);
            }
            Target::Local(_) => {
                return Err(error(
                    "unsupported erlang:term_to_binary/1,2 of a local fun: see \"External term format\" in docs/erts-differences.md",
                ));
            }
        },
        Term::MatchContext(_) => {
            return Err(error(
                "unsupported erlang:term_to_binary/1,2 of a match context: see \"External term format\" in docs/erts-differences.md",
            ));
        }
    }
    Ok(())
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
            out.extend_from_slice(&(bytes.len() as u16).to_be_bytes());
        }
        (false, true) => out.extend_from_slice(&[119, bytes.len() as u8]),
        (false, false) => {
            out.push(118);
            out.extend_from_slice(&(bytes.len() as u16).to_be_bytes());
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

/// Write an integer as the `SMALL_DEF` branch of `enc_term`
/// (`external.c:3552-3577`): `SMALL_INTEGER_EXT` below 256, `INTEGER_EXT`
/// in the 32-bit range, and else `SMALL_BIG_EXT`.
fn encode_integer(value: i64, out: &mut Vec<u8>) {
    if (0..256).contains(&value) {
        out.extend_from_slice(&[97, value as u8]);
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
        out.push(first as u8);
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
        out.push(bytes.len() as u8);
    } else {
        out.push(111);
        out.extend_from_slice(&(bytes.len() as u32).to_be_bytes());
    }
    out.push(u8::from(sign == Sign::Minus));
    out.extend_from_slice(&bytes);
}

/// Write a list as the `LIST_DEF` branch of `enc_term`
/// (`external.c:3695-3725`) does: `STRING_EXT` for a proper list of bytes
/// of at most 65535 elements, else `LIST_EXT` with the number of cons cells,
/// the cells, and the tail.
fn encode_list(
    term: &Term,
    atoms: &AtomTable,
    options: &Options,
    out: &mut Vec<u8>,
) -> Result<(), BeamError> {
    if let Some(bytes) = list_string(term) {
        out.push(107);
        out.extend_from_slice(&(bytes.len() as u16).to_be_bytes());
        out.extend_from_slice(&bytes);
        return Ok(());
    }
    let mut count = 0u32;
    let mut walk = term;
    while let Some((_, tail)) = walk.cons_cell() {
        count += 1;
        walk = tail;
    }
    out.push(108);
    out.extend_from_slice(&count.to_be_bytes());
    let mut walk = term;
    while let Some((head, tail)) = walk.cons_cell() {
        encode_term(head, atoms, options, out)?;
        walk = tail;
    }
    encode_term(walk, atoms, options, out)
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
        bytes.push(*value as u8);
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
    let Term::Binary(binary) = binary else {
        return Ok(BifResult::badarg());
    };
    if binary.bit_size() % 8 != 0 {
        return Ok(BifResult::badarg());
    }
    let seed = context.scheduler.local_node_hash();
    let mut decoder = Decoder {
        input: binary,
        pos: 0,
        safe,
        local: false,
        atoms: &mut context.scheduler.atoms,
        seed,
    };
    match decoder.decode_input(seed) {
        Ok((term, used_bytes)) if used => Ok(Value(Term::tuple(vec![
            term,
            Term::Integer(used_bytes as i64),
        ]))),
        Ok((term, _)) => Ok(Value(term)),
        Err(DecodeError::Badarg) => Ok(BifResult::badarg()),
        Err(DecodeError::Unsupported(what)) => Err(error(format!(
            "unsupported erlang:binary_to_term/1,2 of {what}: see \"External term format\" in docs/erts-differences.md"
        ))),
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
}

/// The decoder of one external term: the input bytes, the cursor, and the
/// atom table of the run.
struct Decoder<'a> {
    /// The input binary.
    input: &'a [u8],
    /// The position of the next byte.
    pos: usize,
    /// The `safe` option: each atom must exist already.
    safe: bool,
    /// True inside `LOCAL_EXT`: `NIL_EXT` stands for the local node.
    local: bool,
    /// The atom table of the run.
    atoms: &'a mut AtomTable,
    /// The seed of the block hash of `LOCAL_EXT`.
    seed: u32,
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
        let mut decoder = Decoder {
            input: body,
            pos: 0,
            safe: self.safe,
            local: false,
            atoms: &mut *self.atoms,
            seed,
        };
        let term = decoder.term()?;
        // The compressed header counts the bytes of the stream, and the
        // decompressed body adds no byte of the input (`binary2term_prepare`,
        // `external.c:1826-1839`); the plain body adds its cells to the
        // magic byte.
        let used = if plain.is_some() {
            header
        } else {
            header + decoder.pos
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
        let compressed = &self.input[self.pos..];
        let mut decompress = Decompress::new(true);
        let mut out = Vec::new();
        let mut chunk = [0u8; 64 * 1024];
        loop {
            let (before_in, before_out) = (decompress.total_in(), decompress.total_out());
            let input = &compressed[before_in as usize..];
            let status = decompress
                .decompress(input, &mut chunk, FlushDecompress::None)
                .map_err(|_| DecodeError::Badarg)?;
            out.extend_from_slice(&chunk[..(decompress.total_out() - before_out) as usize]);
            if out.len() > declared as usize {
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
        if out.len() != declared as usize {
            return Err(DecodeError::Badarg);
        }
        Ok((out, decompress.total_in() as usize))
    }

    /// Decode one term, as `dec_term` (`external.c:4270-5340`) does.
    fn term(&mut self) -> Result<Term, DecodeError> {
        match self.u8()? {
            97 => Ok(Term::Integer(i64::from(self.u8()?))),
            98 => Ok(Term::Integer(i64::from(i32::from_be_bytes(
                self.take(4)?.try_into().expect("four bytes"),
            )))),
            99 => self.float_text(),
            70 => {
                let bits = u64::from_be_bytes(self.take(8)?.try_into().expect("eight bytes"));
                let value = f64::from_bits(bits);
                value
                    .is_finite()
                    .then_some(Term::Float(value))
                    .ok_or(DecodeError::Badarg)
            }
            100 => {
                let length = usize::from(self.u16()?);
                self.atom(length, true)
            }
            115 => {
                let length = usize::from(self.u8()?);
                self.atom(length, true)
            }
            118 => {
                let length = usize::from(self.u16()?);
                self.atom(length, false)
            }
            119 => {
                let length = usize::from(self.u8()?);
                self.atom(length, false)
            }
            106 => Ok(Term::Nil),
            107 => {
                let count = usize::from(self.u16()?);
                let bytes = self.take(count)?;
                Ok(Term::list_from(
                    bytes.iter().map(|&byte| Term::Integer(i64::from(byte))),
                ))
            }
            108 => {
                let count = self.u32()? as usize;
                if count > self.remaining() {
                    return Err(DecodeError::Badarg);
                }
                let mut heads = Vec::with_capacity(count);
                for _ in 0..count {
                    heads.push(self.term()?);
                }
                let tail = self.term()?;
                let mut list = tail;
                for head in heads.into_iter().rev() {
                    list = Term::cons(head, list);
                }
                Ok(list)
            }
            104 => {
                let count = usize::from(self.u8()?);
                self.tuple(count)
            }
            105 => {
                let count = self.u32()? as usize;
                self.tuple(count)
            }
            116 => self.map(),
            109 => {
                let count = self.u32()? as usize;
                Ok(Term::binary(self.take(count)?.to_vec()))
            }
            77 => {
                let count = self.u32()? as usize;
                let bits = self.u8()?;
                let bytes = self.take(count)?;
                if bits != 8 {
                    return Err(DecodeError::Unsupported(
                        "a bitstring that is not a whole number of bytes",
                    ));
                }
                Ok(Term::binary(bytes.to_vec()))
            }
            110 => {
                let count = usize::from(self.u8()?);
                self.big(count)
            }
            111 => {
                let count = self.u32()? as usize;
                self.big(count)
            }
            88 => self.pid(4),
            103 => self.pid(1),
            89 => self.port(4),
            102 => self.port(1),
            120 => {
                let sysname = self.sysname()?;
                let number = self.u64()?;
                let creation = self.u32()?;
                self.local_port(sysname, number, creation)
            }
            90 => self.reference(4),
            114 => self.reference(1),
            101 => self.old_reference(),
            113 => self.export(),
            112 => Err(DecodeError::Unsupported("a local fun")),
            117 => Err(DecodeError::Unsupported("an old local fun")),
            67 => Err(DecodeError::Unsupported("a record")),
            121 => {
                let hash = u32::from_be_bytes(
                    self.take(4)?
                        .try_into()
                        .expect("four bytes of the local hash"),
                );
                let start = self.pos;
                self.local = true;
                let term = self.term()?;
                if external::block_hash(&self.input[start..self.pos], self.seed) != hash {
                    return Err(DecodeError::Badarg);
                }
                Ok(term)
            }
            _ => Err(DecodeError::Badarg),
        }
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
        let value: f64 = text.parse().map_err(|_| DecodeError::Badarg)?;
        value
            .is_finite()
            .then_some(Term::Float(value))
            .ok_or(DecodeError::Badarg)
    }

    /// Decode an atom of `length` bytes in Latin-1 or UTF-8, as
    /// `dec_term_atom_common` (`external.c:4525-4548`) does. The `safe`
    /// option takes only an atom that the run has.
    fn atom(&mut self, length: usize, latin1: bool) -> Result<Term, DecodeError> {
        let bytes = self.take(length)?;
        let name = if latin1 {
            bytes.iter().map(|&byte| char::from(byte)).collect()
        } else {
            std::str::from_utf8(bytes)
                .map_err(|_| DecodeError::Badarg)?
                .to_string()
        };
        if name.chars().count() > MAX_ATOM_CHARACTERS {
            return Err(DecodeError::Badarg);
        }
        let atom = if self.safe {
            self.atoms.get(&name).ok_or(DecodeError::Badarg)?
        } else {
            self.atoms.put(&name).map_err(|_| DecodeError::Badarg)?
        };
        Ok(Term::Atom(atom))
    }

    /// Decode a tuple of `count` elements.
    fn tuple(&mut self, count: usize) -> Result<Term, DecodeError> {
        if count > MAX_TUPLE_SIZE || count > self.remaining() {
            return Err(DecodeError::Badarg);
        }
        let mut elements = Vec::with_capacity(count);
        for _ in 0..count {
            elements.push(self.term()?);
        }
        Ok(Term::tuple(elements))
    }

    /// Decode a map as the `MAP_EXT` case of `dec_term`
    /// (`external.c:5069-5150`) does. A key that PBeam cannot hold gives a
    /// runtime error.
    fn map(&mut self) -> Result<Term, DecodeError> {
        let count = self.u32()? as usize;
        if count
            .checked_mul(2)
            .is_none_or(|terms| terms > self.remaining())
        {
            return Err(DecodeError::Badarg);
        }
        let mut pairs = Vec::with_capacity(count);
        for _ in 0..count {
            let key = self.term()?;
            let value = self.term()?;
            pairs.push((key, value));
        }
        let map = crate::term::Map::from_pairs(pairs, self.atoms)
            .map_err(|_| DecodeError::Unsupported("a map with a key that PBeam cannot hold"))?;
        // ERTS rejects a duplicate key (`dec_term`, `external.c:5069-5150`).
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
    /// local node inside `LOCAL_EXT`, else an atom (`dec_sysname`,
    /// `external.c:3110-3122`).
    fn sysname(&mut self) -> Result<Option<crate::term::Atom>, DecodeError> {
        if self.peek() == Some(106) {
            if !self.local {
                return Err(DecodeError::Badarg);
            }
            self.pos += 1;
            return Ok(None);
        }
        let term = self.term()?;
        let Term::Atom(atom) = &term else {
            return Err(DecodeError::Badarg);
        };
        Ok(Some(atom.clone()))
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

    /// Decode a pid as `dec_pid` (`external.c:3133-3200`) does. `tag` is 4
    /// for `NEW_PID_EXT` and 1 for `PID_EXT`.
    fn pid(&mut self, tag: u8) -> Result<Term, DecodeError> {
        let sysname = self.sysname()?;
        let number = self.u32()?;
        let serial = self.u32()?;
        let creation = if tag == 4 {
            self.u32()?
        } else {
            u32::from(self.u8()?)
        };
        if !self.is_local(sysname, creation) {
            return Err(DecodeError::Unsupported("a pid of another node"));
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
            u32::from(self.u8()?)
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
        if !self.is_local(sysname, creation) {
            return Err(DecodeError::Unsupported("a port of another node"));
        }
        Ok(Term::Port(crate::term::PortId(number)))
    }

    /// Decode a reference as the `NEWER_REFERENCE_EXT` and
    /// `NEW_REFERENCE_EXT` cases of `dec_term` (`external.c:4786-4855`) do.
    /// `tag` is 4 for the 32-bit creation and 1 for the 8-bit creation.
    fn reference(&mut self, tag: u8) -> Result<Term, DecodeError> {
        let count = usize::from(self.u16()?);
        let sysname = self.sysname()?;
        let creation = if tag == 4 {
            self.u32()?
        } else {
            u32::from(self.u8()?)
        };
        let mut numbers = [0u32; 3];
        for number in numbers.iter_mut().take(count.min(3)) {
            *number = self.u32()?;
        }
        for _ in 3..count {
            self.u32()?;
        }
        if !self.is_local(sysname, creation) {
            return Err(DecodeError::Unsupported("a reference of another node"));
        }
        if numbers[0] >= MAX_REFERENCE {
            return Err(DecodeError::Badarg);
        }
        // A PBeam reference belongs to the first scheduler, whose number is
        // in the low bits of the second reference number (`REF_THREAD`,
        // `erl_bif_unique.h:117-131`).
        if numbers[1] & !REF_NUMBER_1_MASK != REF_THREAD {
            return Err(DecodeError::Unsupported("a reference of another scheduler"));
        }
        let value = (u64::from(numbers[2]) << 32)
            | (u64::from(numbers[1]) & 0xFFFC_0000)
            | u64::from(numbers[0]);
        Reference::ordinary(value)
            .map(Term::Reference)
            .ok_or(DecodeError::Unsupported("a pid reference"))
    }

    /// Decode an old `REFERENCE_EXT`, as `dec_term`
    /// (`external.c:4743-4766`) does.
    fn old_reference(&mut self) -> Result<Term, DecodeError> {
        let sysname = self.sysname()?;
        let number = self.u32()?;
        let creation = u32::from(self.u8()?);
        if !self.is_local(sysname, creation) {
            return Err(DecodeError::Unsupported("a reference of another node"));
        }
        if number >= MAX_REFERENCE {
            return Err(DecodeError::Badarg);
        }
        Ok(Term::Reference(
            Reference::ordinary(u64::from(number))
                .expect("a number below MAX_REFERENCE is ordinary"),
        ))
    }

    /// Decode an `EXPORT_EXT` as the `EXPORT_EXT` case of `dec_term`
    /// (`external.c:5004-5047`) does. An external fun holds its module,
    /// function, and arity, as `erlang:make_fun/3` makes it. The `safe`
    /// option gives `badarg`, because PBeam does not check that the export
    /// is active.
    fn export(&mut self) -> Result<Term, DecodeError> {
        let module = self.term()?;
        let Term::Atom(module) = &module else {
            return Err(DecodeError::Badarg);
        };
        let function = self.term()?;
        let Term::Atom(function) = &function else {
            return Err(DecodeError::Badarg);
        };
        let count = self.term()?;
        let Term::Integer(arity) = count else {
            return Err(DecodeError::Badarg);
        };
        if !(0..=i64::from(u32::MAX)).contains(&arity) || self.safe {
            return Err(DecodeError::Badarg);
        }
        Ok(Term::Fun(Fun::external(
            module.clone(),
            function.clone(),
            arity as u32,
        )))
    }
}

#[cfg(test)]
#[path = "../../../../tests/runtime/interpreter/bif/term.rs"]
mod tests;

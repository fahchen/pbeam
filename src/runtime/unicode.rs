//! Convert chardata as the ERTS BIFs `unicode:characters_to_binary/2` and
//! `unicode:characters_to_list/2` do, check and convert file names to their
//! native bytes, as ERTS does before it uses a name, and convert native
//! bytes back to a name (`erl_unicode.c`).
//!
//! The chardata functions are ports of the ERTS walks, which a BIF runs
//! with a budget of loop steps (`left`) and continues in a trap. Each walk
//! returns the steps that it used, so the BIF can charge the ERTS
//! reductions.

use super::host::FilenameEncoding;
use crate::{
    error::{BeamError, error},
    term::{Binary, Term},
};

/// The encoding of the characters of a chardata, which the `Encoding`
/// argument of `unicode:characters_to_binary/2` and
/// `unicode:characters_to_list/2` names, and which the atom tags of the
/// external term format name.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum CharEncoding {
    /// The `unicode` or `utf8` encoding.
    Unicode,
    /// The `latin1` encoding.
    Latin1,
}

/// The UTF-8 output of a chardata walk, as the target buffer of
/// `do_build_utf8` keeps it (`erl_unicode.c:508-749`).
#[derive(Debug, Default)]
pub(super) struct Utf8Output {
    /// The UTF-8 bytes of the characters so far.
    pub(super) bytes: Vec<u8>,
    /// The number of characters so far.
    pub(super) characters: usize,
    /// The first bytes of a UTF-8 sequence that a binary did not complete
    /// (`leftover` and `num_leftovers`). The next binary completes it.
    pub(super) leftover: Vec<u8>,
}

/// The state of an `analyze_utf8` walk over a binary
/// (`erl_unicode.c:1177-1275`).
#[derive(Debug, PartialEq)]
pub(super) enum Utf8State {
    /// Each byte is part of a valid sequence (`ERTS_UTF8_OK`).
    Ok,
    /// A sequence at the end is shorter than its first byte needs
    /// (`ERTS_UTF8_INCOMPLETE`).
    Incomplete,
    /// A sequence is not valid (`ERTS_UTF8_ERROR`).
    Error,
    /// The budget ended before the end of the binary
    /// (`ERTS_UTF8_ANALYZE_MORE`).
    AnalyzeMore,
}

/// Return the loop steps of `utf8_need` for chardata, or `None` when the
/// term is not chardata, for which ERTS raises `badarg`
/// (`erl_unicode.c:342-505`). A small integer costs 1. A binary costs 1,
/// or with `latin1` the bytes that its UTF-8 form needs. A walk stops with
/// its steps at an integer outside the small range, without a check of the
/// rest, because the conversion stops there (`is_big`). Each element must
/// be a small integer, a binary, or a list of the same, and each tail must
/// be `[]` or a binary. The walk uses an explicit stack, as ERTS does with
/// an `ESTACK`.
///
/// # Examples
///
/// ```rust,ignore
/// let list = Term::list_from(vec![Term::Integer(97), Term::binary(vec![195, 169])]);
/// assert_eq!(utf8_need(&list, CharEncoding::Unicode), Some(2));
/// assert_eq!(utf8_need(&list, CharEncoding::Latin1), Some(5));
/// assert_eq!(utf8_need(&Term::static_atom("foo"), CharEncoding::Unicode), None);
/// // [1 bsl 60 | x]: the walk stops at the big integer.
/// assert_eq!(utf8_need(&Term::list_with_tail(vec![Term::Integer(1 << 60)], Term::static_atom("x")), CharEncoding::Unicode), Some(0));
/// ```
pub(super) fn utf8_need(chardata: &Term, charset: CharEncoding) -> Option<usize> {
    let latin1 = charset == CharEncoding::Latin1;
    let binary_cost = |bytes: &[u8]| {
        if latin1 {
            bytes.len() + bytes.iter().filter(|&&byte| byte >= 0x80).count()
        } else {
            1
        }
    };
    // Each frame is the rest of a list: its next cons cell, or its tail.
    let mut frames = match chardata {
        Term::Binary(bytes) => return Some(binary_cost(bytes.bytes()?)),
        _ if chardata.is_list() => vec![chardata],
        _ => return None,
    };
    let mut cost = 0;
    while let Some(frame) = frames.last_mut() {
        let Some((element, rest)) = frame.cons_cell() else {
            match frame {
                Term::Binary(bytes) => cost += binary_cost(bytes.bytes()?),
                tail if tail.is_nil() => {}
                _ => return None,
            }
            frames.pop();
            continue;
        };
        *frame = rest;
        match element {
            Term::Integer(_) | Term::BigInt(_) if element.small_integer().is_none() => {
                return Some(cost);
            }
            Term::Integer(_) => cost += 1,
            Term::Binary(bytes) => cost += binary_cost(bytes.bytes()?),
            _ if element.is_list() => frames.push(element),
            _ => return None,
        }
    }
    Some(cost)
}

/// Return the bytes of a binary of chardata that [`utf8_need`] accepted. A
/// bitstring is not chardata, so `utf8_need` rejects it before a build.
///
/// # Examples
///
/// ```rust,ignore
/// assert_eq!(checked_bytes(&Binary::from(vec![97])), &[97]);
/// ```
fn checked_bytes(binary: &Binary) -> &[u8] {
    binary
        .bytes()
        .expect("utf8_need rejects a bitstring before a build")
}

/// Convert chardata to UTF-8 with a budget of `left` loop steps, as
/// `do_build_utf8` does (`erl_unicode.c:508-749`). Return the rest of the
/// chardata, which is `[]` when the walk ended, and whether the walk stopped
/// at an element that is not valid.
///
/// A small integer uses one step, and a binary one step for each byte. A
/// walk stops when the steps end, and returns the rest as ERTS builds it:
/// the rest of the current list, and for each list that the walk entered,
/// a list cell of that rest and the rest of the enclosing list. A binary
/// that is longer than the steps that remain splits after them, at the
/// next character boundary with UTF-8, and the steps end. A code point
/// that is not valid, an integer after the start of a UTF-8 sequence in a
/// binary, and a binary that is not valid UTF-8 stop the walk with an
/// error and the rest from that element. The bytes of such a binary use no
/// steps.
///
/// # Examples
///
/// ```rust,ignore
/// let mut output = Utf8Output::default();
/// let mut left = 2;
/// let list = Term::list_from(vec![Term::Integer(97), Term::Integer(98), Term::Integer(99)]);
/// let (rest, error) = build_utf8(&list, &mut left, CharEncoding::Unicode, &mut output);
/// assert_eq!((rest, error, left), (Term::list_from(vec![Term::Integer(99)]), false, 0));
/// assert_eq!(output.bytes, b"ab");
/// ```
pub(super) fn build_utf8(
    chardata: &Term,
    left: &mut i64,
    charset: CharEncoding,
    output: &mut Utf8Output,
) -> (Term, bool) {
    let nil = || Term::nil();
    if *left <= 0 || *chardata == nil() {
        return (chardata.clone(), false);
    }
    // Each frame is the rest of a list: its next cons cell, or its tail.
    let mut frames = match chardata {
        Term::Binary(bytes) => return build_binary(checked_bytes(bytes), left, charset, output),
        _ if chardata.is_list() => vec![chardata],
        _ => return (chardata.clone(), true),
    };
    let (rest, error) = loop {
        let top = frames.len() - 1;
        let frame = frames[top];
        let Some((element, after)) = frame.cons_cell() else {
            match frame {
                Term::Binary(bytes) => {
                    let (rest, error) = build_binary(checked_bytes(bytes), left, charset, output);
                    if error || *left == 0 {
                        break (rest, error);
                    }
                }
                tail if tail.is_nil() => {}
                tail => break (tail.clone(), true),
            }
            frames.pop();
            if frames.is_empty() {
                return (nil(), false);
            }
            continue;
        };
        match element {
            Term::Binary(bytes) => {
                frames[top] = after;
                let (rest, error) = build_binary(checked_bytes(bytes), left, charset, output);
                if error || rest != nil() {
                    break (Term::cons(rest, after.clone()), error);
                }
            }
            Term::Integer(_) | Term::BigInt(_) => {
                let character = element
                    .small_integer()
                    .filter(|&value| {
                        output.leftover.is_empty()
                            && (charset == CharEncoding::Unicode || value <= 255)
                    })
                    .and_then(|value| u32::try_from(value).ok())
                    .and_then(char::from_u32);
                let Some(character) = character else {
                    break (frame.clone(), true);
                };
                frames[top] = after;
                output
                    .bytes
                    .extend_from_slice(character.encode_utf8(&mut [0; 4]).as_bytes());
                output.characters += 1;
                *left -= 1;
            }
            _ if element.is_list() => {
                frames[top] = after;
                frames.push(element);
                continue;
            }
            _ => break (frame.clone(), true),
        }
        if *left == 0 {
            break (after.clone(), false);
        }
    };
    frames.pop();
    let rest = frames
        .into_iter()
        .rev()
        .fold(rest, |rest, frame| Term::cons(rest, frame.clone()));
    (rest, error)
}

/// Convert a binary with a budget of `left` loop steps, as the binary part
/// of `do_build_utf8` does (`erl_unicode.c:521-596`). Return the part that
/// the steps did not reach, or `[]`, and whether the binary is not valid
/// UTF-8. With `latin1`, each byte is one character. Else the split moves to
/// the next character boundary, and [`copy_utf8_bin`] converts the bytes. A
/// binary that is not valid returns the rest from the first sequence that
/// is not valid, and uses no steps.
///
/// # Examples
///
/// ```rust,ignore
/// let mut output = Utf8Output::default();
/// let mut left = 1;
/// let (rest, error) = build_binary(&[195, 169, 97], &mut left, CharEncoding::Unicode, &mut output);
/// // The split moves past the continuation byte 169.
/// assert_eq!((rest, error, left), (Term::binary(vec![97]), false, 0));
/// ```
fn build_binary(
    bytes: &[u8],
    left: &mut i64,
    charset: CharEncoding,
    output: &mut Utf8Output,
) -> (Term, bool) {
    let latin1 = charset == CharEncoding::Latin1;
    let mut size = bytes.len();
    if !latin1 {
        while size as i64 > *left && bytes[*left as usize] & 0xC0 == 0x80 {
            *left += 1;
        }
    }
    let mut rest = Term::nil();
    if size as i64 > *left {
        rest = Term::binary(bytes[*left as usize..].to_vec());
        size = *left as usize;
    }
    if latin1 {
        for &byte in &bytes[..size] {
            let character = char::from(byte);
            output
                .bytes
                .extend_from_slice(character.encode_utf8(&mut [0; 4]).as_bytes());
            output.characters += 1;
        }
    } else if let Err(position) = copy_utf8_bin(&bytes[..size], output) {
        return (Term::binary(bytes[position..].to_vec()), true);
    }
    *left -= size as i64;
    (rest, false)
}

/// Copy the UTF-8 sequences of a binary to the output, as `copy_utf8_bin`
/// with a leftover buffer does (`erl_unicode.c:237-338`). The bytes in
/// `leftover` start a sequence of an earlier binary: the first bytes of
/// `source` complete it, or all of `source` goes to `leftover` when they are
/// too few. A sequence at the end that is shorter than its first byte needs
/// goes to `leftover`. Return the position of the first sequence that is
/// not valid. A completed leftover sequence that is not valid gives
/// position 0, and `leftover` keeps its first bytes, as in ERTS.
///
/// # Examples
///
/// ```rust,ignore
/// let mut output = Utf8Output::default();
/// assert_eq!(copy_utf8_bin(&[97, 195], &mut output), Ok(()));
/// assert_eq!((output.bytes.as_slice(), output.leftover.as_slice()), (&b"a"[..], &[195][..]));
/// assert_eq!(copy_utf8_bin(&[169, 255], &mut output), Err(1));
/// assert_eq!(output.characters, 2);
/// ```
fn copy_utf8_bin(source: &[u8], output: &mut Utf8Output) -> Result<(), usize> {
    let mut position = 0;
    if let Some(&first) = output.leftover.first() {
        // A sequence has at most 4 bytes, so at most 3 complete it.
        let needed =
            sequence_length(first).expect("a leftover starts a sequence") - output.leftover.len();
        if source.len() < needed {
            output.leftover.extend_from_slice(source);
            return Ok(());
        }
        let mut sequence = output.leftover.clone();
        sequence.extend_from_slice(&source[..needed]);
        if !matches!(utf8_sequence(&sequence), Utf8Sequence::Character(..)) {
            return Err(0);
        }
        output.bytes.extend_from_slice(&sequence);
        output.characters += 1;
        output.leftover.clear();
        position = needed;
    }
    while position < source.len() {
        match utf8_sequence(&source[position..]) {
            Utf8Sequence::Character(_, length) => {
                output
                    .bytes
                    .extend_from_slice(&source[position..position + length]);
                output.characters += 1;
                position += length;
            }
            Utf8Sequence::Short => {
                output.leftover.extend_from_slice(&source[position..]);
                break;
            }
            Utf8Sequence::Invalid => return Err(position),
        }
    }
    Ok(())
}

/// Return true when the bytes that a binary did not complete can start a
/// valid sequence, as `check_leftovers` returns 0 for them
/// (`erl_unicode.c:751-770`): each byte after the first is a continuation
/// byte.
///
/// # Examples
///
/// ```rust,ignore
/// assert!(is_incomplete(&[226, 130]));
/// assert!(!is_incomplete(&[226, 97]));
/// ```
pub(super) fn is_incomplete(leftover: &[u8]) -> bool {
    leftover[1..].iter().all(|&byte| byte & 0xC0 == 0x80)
}

/// Read the UTF-8 sequences of a binary from `start` with a budget of `left`
/// characters, as `analyze_utf8` does (`erl_unicode.c:1177-1275`). Return
/// the state, the number of characters, and the position after the last
/// valid sequence. Each character uses one step, and the walk stops with
/// [`Utf8State::AnalyzeMore`] when the steps end before the end of the
/// binary. The last character can make `left` 0 with [`Utf8State::Ok`].
///
/// # Examples
///
/// ```rust,ignore
/// let mut left = 2;
/// assert_eq!(analyze_utf8(b"abc", 0, &mut left), (Utf8State::AnalyzeMore, 2, 2));
/// let mut left = 10;
/// assert_eq!(analyze_utf8(&[97, 195], 0, &mut left), (Utf8State::Incomplete, 1, 1));
/// ```
pub(super) fn analyze_utf8(
    bytes: &[u8],
    start: usize,
    left: &mut i64,
) -> (Utf8State, usize, usize) {
    let (mut position, mut characters) = (start, 0);
    while position < bytes.len() {
        match utf8_sequence(&bytes[position..]) {
            Utf8Sequence::Character(_, length) => position += length,
            Utf8Sequence::Short => return (Utf8State::Incomplete, characters, position),
            Utf8Sequence::Invalid => return (Utf8State::Error, characters, position),
        }
        characters += 1;
        *left -= 1;
        if *left <= 0 && position < bytes.len() {
            return (Utf8State::AnalyzeMore, characters, position);
        }
    }
    (Utf8State::Ok, characters, position)
}

/// Return the code points of valid UTF-8 bytes as a list, as
/// `erts_make_list_from_utf8_buf` builds it (`erl_unicode.c:1291-1337`).
///
/// # Examples
///
/// ```rust,ignore
/// assert_eq!(code_points("aé".as_bytes()), Term::list_from(vec![Term::Integer(97), Term::Integer(233)]));
/// ```
pub(super) fn code_points(bytes: &[u8]) -> Term {
    let text = std::str::from_utf8(bytes).expect("the walks write valid UTF-8");
    Term::list_from(
        text.chars()
            .map(|character| Term::Integer(u32::from(character).into())),
    )
}

/// Return the length of the UTF-8 sequence that a first byte starts, as
/// `utf8_len` does (`erl_unicode.c:223-235`), or `None` for a byte that
/// starts no sequence.
///
/// # Examples
///
/// ```rust,ignore
/// assert_eq!(sequence_length(0xE2), Some(3));
/// assert_eq!(sequence_length(0x80), None);
/// ```
fn sequence_length(first: u8) -> Option<usize> {
    match first {
        0x00..=0x7F => Some(1),
        0xC0..=0xDF => Some(2),
        0xE0..=0xEF => Some(3),
        0xF0..=0xF7 => Some(4),
        _ => None,
    }
}

/// The first UTF-8 sequence of a byte slice, as ERTS reads it.
#[derive(Debug, PartialEq)]
enum Utf8Sequence {
    /// A valid sequence: its code point and its length in bytes.
    Character(u32, usize),
    /// The first byte starts a sequence, but the slice has fewer bytes than
    /// the sequence needs. ERTS does not check the other bytes here.
    Short,
    /// The sequence is not valid.
    Invalid,
}

/// Read the first UTF-8 sequence of a slice that is not empty, as
/// `copy_utf8_bin` and `analyze_utf8` do (`erl_unicode.c:237-338,
/// 1177-1275`). The first byte gives the length: `0xxxxxxx` 1, `110xxxxx` 2,
/// `1110xxxx` 3, and `11110xxx` 4. Each other first byte is not valid. ERTS
/// compares the length with the size of the slice before it checks the
/// bytes. A complete sequence is valid when it is standard UTF-8: Rust and
/// ERTS both reject overlong forms, surrogates, and code points above
/// `0x10FFFF`.
///
/// # Examples
///
/// ```rust,ignore
/// assert_eq!(utf8_sequence(&[195, 169, 97]), Utf8Sequence::Character(233, 2));
/// assert_eq!(utf8_sequence(&[224, 128]), Utf8Sequence::Short);
/// assert_eq!(utf8_sequence(&[224, 128, 128]), Utf8Sequence::Invalid);
/// assert_eq!(utf8_sequence(&[128]), Utf8Sequence::Invalid);
/// ```
fn utf8_sequence(bytes: &[u8]) -> Utf8Sequence {
    let Some(length) = sequence_length(bytes[0]) else {
        return Utf8Sequence::Invalid;
    };
    let Some(sequence) = bytes.get(..length) else {
        return Utf8Sequence::Short;
    };
    std::str::from_utf8(sequence)
        .ok()
        .and_then(|text| text.chars().next())
        .map_or(Utf8Sequence::Invalid, |character| {
            Utf8Sequence::Character(character.into(), length)
        })
}

/// Return `true` when ERTS accepts `name` as a file name in `encoding`, as
/// `erts_convert_filename_to_encoding` with `allow_empty` and without
/// `allow_atom` does (`erl_unicode.c:2057-2119`). A binary is valid with
/// each content, because ERTS copies its bytes without a check. A bitstring
/// whose size is not a whole number of bytes is not valid
/// (`erl_unicode.c:2093-2096`). `[]` and a
/// list are valid when [`native_filename`] accepts them. Each other term is
/// not valid, for example an atom, an integer, and a tuple.
///
/// # Examples
///
/// ```rust,ignore
/// let name = Term::list_from(vec![Term::list_from(vec![Term::Integer(97)]), Term::nil(), Term::Integer(256)]);
/// assert!(is_native_filename(&name, FilenameEncoding::Utf8));
/// assert!(!is_native_filename(&name, FilenameEncoding::Latin1));
/// assert!(is_native_filename(&Term::binary(vec![97, 0]), FilenameEncoding::Latin1));
/// assert!(!is_native_filename(&Term::list_from(vec![Term::Integer(0)]), FilenameEncoding::Utf8));
/// ```
pub(super) fn is_native_filename(name: &Term, encoding: FilenameEncoding) -> bool {
    name.is_binary() || native_filename(name, encoding).is_some()
}

/// Return the native bytes of the file name `name` in `encoding`, as
/// `erts_native_filename_put` writes them (`erl_unicode.c:2400-2555`), or
/// `None` when `erts_native_filename_need` rejects the name
/// (`erl_unicode.c:2222-2398`). `[]` and a proper list are valid when each
/// element is a character, `[]`, or a proper list of the same. A character
/// is a small integer from 1 to 255 in Latin-1, which gives one byte, and
/// from 1 to `0x10FFFF`, except `0xD800` to `0xDFFF`, in UTF-8, which gives
/// its UTF-8 bytes. ERTS reads a negative small integer as a large unsigned
/// value, so it is not valid. Each other term is not valid: an atom, a
/// binary, an improper list, and a bignum, a binary, or another term in a
/// list. The callers that accept an atom or a binary check it first.
///
/// The walk uses an explicit work stack, as ERTS does with an `ESTACK`, so
/// nested lists of any depth do not use the Rust stack.
///
/// # Examples
///
/// ```rust,ignore
/// let name = Term::list_from(vec![Term::list_from(vec![Term::Integer(97)]), Term::nil(), Term::Integer(233)]);
/// assert_eq!(native_filename(&name, FilenameEncoding::Latin1), Some(vec![97, 233]));
/// assert_eq!(native_filename(&name, FilenameEncoding::Utf8), Some(vec![97, 195, 169]));
/// assert_eq!(native_filename(&Term::list_from(vec![Term::Integer(256)]), FilenameEncoding::Latin1), None);
/// assert_eq!(native_filename(&Term::binary(vec![97]), FilenameEncoding::Utf8), None);
/// ```
pub(super) fn native_filename(name: &Term, encoding: FilenameEncoding) -> Option<Vec<u8>> {
    let elements = name.proper_list()?;
    let mut bytes = Vec::new();
    let mut pending = vec![elements];
    while let Some(elements) = pending.last_mut() {
        let Some(element) = elements.next() else {
            pending.pop();
            continue;
        };
        if let Some(nested) = element.proper_list() {
            pending.push(nested);
            continue;
        }
        let character = u32::try_from(element.small_integer()?)
            .ok()
            .filter(|&character| character != 0)?;
        match encoding {
            FilenameEncoding::Latin1 => bytes.push(u8::try_from(character).ok()?),
            FilenameEncoding::Utf8 | FilenameEncoding::Utf8Mac => {
                let character = char::from_u32(character)?;
                bytes.extend_from_slice(character.encode_utf8(&mut [0; 4]).as_bytes());
            }
        }
    }
    Some(bytes)
}

/// The code points that start a composition in ERTS on macOS: the
/// candidates of `is_candidate` (`erl_unicode.c:1418-1428`), in order. They
/// are the bits of `comp_candidate_map` (`erl_unicode_normalize.h:1587-1690`)
/// and the code points 12441 and 12442, which are the first-level keys of
/// `compose_tab` (`erl_unicode_normalize.h:1523-1585`).
const COMPOSITION_CANDIDATES: [u32; 61] = [
    768, 769, 770, 771, 772, 774, 775, 776, 777, 778, 779, 780, 781, 783, 785, 787, 788, 795, 803,
    804, 805, 807, 808, 813, 814, 816, 817, 834, 837, 1463, 1464, 1465, 1468, 1471, 1473, 1474,
    2364, 2492, 2494, 2519, 2620, 2876, 2878, 2902, 2903, 3006, 3031, 3158, 3266, 3285, 3286, 3390,
    3415, 3634, 3762, 3953, 3968, 4021, 4023, 12441, 12442,
];

/// Convert the native bytes of a name to a list of characters, as
/// `erts_convert_native_to_filename` does (`erl_unicode.c:2164-2220`). In
/// Latin-1, and in UTF-8 when the bytes are not valid UTF-8, each byte is
/// one character. Else the result is the one of [`name_from_native`].
///
/// # Examples
///
/// ```rust,ignore
/// // ERTS on macOS: a\xc3\xa9 gives [97,233], and a\xff gives [97,255].
/// let list = |characters: &[i64]| Term::list_from(characters.iter().copied().map(Term::Integer));
/// assert_eq!(filename_from_native(b"a\xc3\xa9", FilenameEncoding::Utf8Mac)?, list(&[97, 233]));
/// assert_eq!(filename_from_native(b"a\xc3\xa9", FilenameEncoding::Latin1)?, list(&[97, 195, 169]));
/// assert_eq!(filename_from_native(b"a\xff", FilenameEncoding::Utf8)?, list(&[97, 255]));
/// assert!(filename_from_native(b"ae\xcc\x81", FilenameEncoding::Utf8Mac).is_err());
/// ```
///
/// # Errors
///
/// The runtime error of [`name_from_native`].
pub(super) fn filename_from_native(
    bytes: &[u8],
    encoding: FilenameEncoding,
) -> Result<Term, BeamError> {
    Ok(name_from_native(bytes, encoding)?.unwrap_or_else(|| byte_list(bytes)))
}

/// Convert the native bytes of a file name to a list of characters, as
/// `prim_file_internal_native2name_1` does (`erl_unicode.c:2735-2824`), or
/// return `None` when the bytes are not valid UTF-8 in a UTF-8 encoding. In
/// Latin-1, each byte is one character. In UTF-8, valid UTF-8 gives its
/// code points. The UTF-8 check of ERTS (`erts_analyze_utf8` in
/// `erl_unicode.c:1180-1270`) rejects overlong forms, surrogates, code
/// points above `0x10FFFF`, and an incomplete sequence, as
/// [`std::str::from_utf8`] does.
///
/// In `Utf8Mac`, ERTS composes the code points (`do_utf8_to_list_normalize`
/// in `erl_unicode.c:1569-1630`). PBeam has no composition table, so valid
/// UTF-8 with a code point of [`COMPOSITION_CANDIDATES`] returns a runtime
/// error (see "OS environment" in `docs/erts-differences.md`). ERTS changes
/// no other value.
///
/// # Examples
///
/// ```rust,ignore
/// let list = |characters: &[i64]| Term::list_from(characters.iter().copied().map(Term::Integer));
/// assert_eq!(name_from_native(b"a\xc3\xa9", FilenameEncoding::Utf8)?, Some(list(&[97, 233])));
/// assert_eq!(name_from_native(b"a\xff", FilenameEncoding::Latin1)?, Some(list(&[97, 255])));
/// assert_eq!(name_from_native(b"a\xff", FilenameEncoding::Utf8)?, None);
/// assert!(name_from_native(b"ae\xcc\x81", FilenameEncoding::Utf8Mac).is_err());
/// ```
///
/// # Errors
///
/// In `Utf8Mac`, valid UTF-8 with a composition candidate.
pub(super) fn name_from_native(
    bytes: &[u8],
    encoding: FilenameEncoding,
) -> Result<Option<Term>, BeamError> {
    let text = match encoding {
        FilenameEncoding::Latin1 => return Ok(Some(byte_list(bytes))),
        FilenameEncoding::Utf8 | FilenameEncoding::Utf8Mac => {
            let Ok(text) = std::str::from_utf8(bytes) else {
                return Ok(None);
            };
            text
        }
    };
    if encoding == FilenameEncoding::Utf8Mac
        && let Some(candidate) = text.chars().find(|&character| {
            COMPOSITION_CANDIDATES
                .binary_search(&character.into())
                .is_ok()
        })
    {
        return Err(error(format!(
            "unsupported composition of the native name code point {} on macOS: see \"OS environment\" in docs/erts-differences.md",
            u32::from(candidate)
        )));
    }
    Ok(Some(Term::list_from(text.chars().map(|character| {
        Term::Integer(u32::from(character).into())
    }))))
}

/// Return the list of the bytes of `bytes`, each as one character.
fn byte_list(bytes: &[u8]) -> Term {
    Term::list_from(bytes.iter().map(|&byte| Term::Integer(byte.into())))
}

#[cfg(test)]
#[path = "../../tests/runtime/unicode.rs"]
mod tests;

//! Bodies of the `unicode` BIFs that do not fit on one table line.
//!
//! The BIFs run the chardata walks of `crate::runtime::unicode` with the
//! loop budget of ERTS, and charge and trap as `erl_unicode.c` does:
//! `allowed_iterations` gives 10 loop steps for each reduction left, at most
//! `CONTEXT_REDS * 10` (`erl_unicode.c:48`, `81`, `166-174`), and
//! `cost_to_proc` bumps one reduction for each 10 steps
//! (`erl_unicode.c:176-180`). A trap goes to an internal export, which
//! continues with no cost in the next turn.

use super::BifResult::{Trap, TrapBif, Value};
use super::{BeamError, BifResult, ProcessState, Term, error};
use crate::runtime::scheduler::CONTEXT_REDS;
use crate::runtime::unicode::{
    Utf8Output, Utf8State, analyze_utf8, build_utf8, code_points, is_incomplete, utf8_need,
};
use crate::term::{AtomTable, debug_term};

/// The loop steps for each reduction (`LOOP_FACTOR`, `erl_unicode.c:48`).
const LOOP_FACTOR: i64 = 10;

/// The steps of `utf8_need` for each loop step: `LOOP_FACTOR_SIMPLE /
/// LOOP_FACTOR` (`erl_unicode.c:49`, `182-186`).
const SIMPLE_FACTOR: usize = 5;

/// The smallest budget of the first walk of
/// `unicode:characters_to_binary/2`, so that a heap binary never traps
/// (`ERL_ONHEAP_BINARY_LIMIT`, `erl_bits.h:167`; `erl_unicode.c:979-981`).
const ONHEAP_BINARY_LIMIT: i64 = 64;

/// Return the loop steps that a call can use, as `allowed_iterations` does
/// (`erl_unicode.c:166-174`): 10 for each reduction left, at most
/// `CONTEXT_REDS * 10`. ERTS keeps the steps in an `int`. A negative budget
/// reads as 0: the walks stop when `--left <= 0` (`erl_unicode.c:1264`),
/// so they do with a negative count what they do with 0. The other callers
/// read the budget after `cost_to_proc`, whose `BUMP_REDS` sets a negative
/// budget to 0 (`erl_unicode.c:973-975`, `1152-1154`, `bif.h:71-78`).
///
/// # Examples
///
/// ```rust,ignore
/// process.fcalls = 3;
/// assert_eq!(allowed_iterations(&process), 30);
/// ```
fn allowed_iterations(process: &ProcessState) -> i64 {
    let left = process.bif_reductions_or_zero() as i64 * LOOP_FACTOR;
    left.min(i64::from(CONTEXT_REDS) * LOOP_FACTOR)
}

/// Bump one reduction for each 10 loop steps, as `cost_to_proc` does
/// (`erl_unicode.c:176-180`).
///
/// # Examples
///
/// ```rust,ignore
/// process.fcalls = 10;
/// cost_to_proc(&mut process, 39);
/// assert_eq!(process.fcalls, 7);
/// ```
fn cost_to_proc(process: &mut ProcessState, steps: i64) {
    process.bump_reductions((steps / LOOP_FACTOR) as i32);
}

/// Run `unicode:characters_to_binary/2`: convert `chardata` to one UTF-8
/// binary, as `unicode_characters_to_binary_2` does
/// (`erl_unicode.c:931-1022`). The `latin1` encoding reads each byte of a
/// binary and each integer as one code point from 0 to 255, and `unicode`
/// and `utf8` read binaries as UTF-8 and integers as code points. A list of
/// one binary reads as the binary. A term that is not chardata raises
/// `badarg`.
///
/// The `utf8_need` walk costs one reduction for each 50 steps
/// ([`utf8_need`]). A valid UTF-8 binary with `unicode` or `utf8` then
/// returns itself (`erl_unicode.c:966-971`). ERTS checks the same for
/// `latin1`, but it compares the size of the binary in bits with the bytes
/// of its UTF-8 form (`aligned_binary_size`, `erl_unicode.c:188-197`), so
/// only an empty binary passes. The conversion of an empty binary gives an
/// equal binary with no cost, so PBeam converts each `latin1` binary. Else
/// the conversion starts with the steps left, but at least 64
/// ([`utf8_turn`]).
///
/// # Examples
///
/// ```rust,ignore
/// let atoms = AtomTable::new();
/// let chardata = Term::list_from(vec![Term::Integer(104), Term::Integer(105)]);
/// let result = characters_to_binary(&mut process, &chardata, &Term::static_atom("unicode"), &atoms)?;
/// assert!(matches!(result, BifResult::Value(Term::Binary(bytes)) if bytes == b"hi"));
/// let latin1 = Term::binary(vec![200]);
/// let result = characters_to_binary(&mut process, &latin1, &Term::static_atom("latin1"), &atoms)?;
/// assert!(matches!(result, BifResult::Value(Term::Binary(bytes)) if bytes == [195, 136]));
/// let foo = Term::list_from(vec![Term::static_atom("foo")]);
/// assert!(matches!(characters_to_binary(&mut process, &foo, &Term::static_atom("unicode"), &atoms)?, BifResult::Raise { .. }));
/// assert!(characters_to_binary(&mut process, &chardata, &Term::static_atom("utf16"), &atoms).is_err());
/// ```
///
/// # Errors
///
/// Return a runtime error for an encoding other than `latin1`, `unicode`,
/// and `utf8`, for chardata that ERTS returns as an `error` or `incomplete`
/// tuple (see [`utf8_turn`]).
pub(super) fn characters_to_binary(
    process: &mut ProcessState,
    chardata: &Term,
    encoding: &Term,
    atoms: &AtomTable,
) -> Result<BifResult, BeamError> {
    let latin1 = match encoding {
        Term::Atom(encoding) if encoding.is_static("latin1") => true,
        Term::Atom(encoding) if encoding.is_one_of(&["unicode", "utf8"]) => false,
        // ponytail: ERTS traps each other encoding to `unicode:characters_to_binary_int/2`; see "Chardata conversion" in docs/erts-differences.md.
        _ => {
            return Err(error(format!(
                "unsupported unicode:characters_to_binary/2 encoding {}",
                debug_term(encoding, atoms)
            )));
        }
    };
    let subject = match chardata.cons_cell() {
        Some((binary @ Term::Binary(_), tail)) if tail.is_nil() => binary,
        _ => chardata,
    };
    let Some(cost) = utf8_need(subject, latin1) else {
        return Ok(BifResult::badarg());
    };
    let simple = (cost / SIMPLE_FACTOR) as i64;
    cost_to_proc(process, simple);
    if let Some(bytes) = subject.binary_bytes()
        && !latin1
        && std::str::from_utf8(bytes).is_ok()
    {
        return Ok(Value(subject.clone()));
    }
    let left = (allowed_iterations(process) - simple).max(ONHEAP_BINARY_LIMIT);
    utf8_turn(process, Utf8Output::default(), subject, left, encoding)
}

/// Convert `chardata` after the bytes of `output` with `left` loop steps,
/// as `do_build_utf8` and `build_utf8_return` do
/// (`erl_unicode.c:508-749`, `787-849`), and bump one reduction for each 10
/// steps. With the encoding `latin1`, each byte of a binary and each
/// integer is one code point from 0 to 255. When the steps end before the
/// chardata, the BIF uses all reductions and traps to
/// `erlang:characters_to_utf8_trap/3` with the bytes, the rest, and the
/// encoding, which continues with the
/// steps of its budget (`erl_unicode.c:851-886`). The bytes of an
/// unfinished UTF-8 sequence go before the rest as a binary.
///
/// # Examples
///
/// ```rust,ignore
/// process.fcalls = 3;
/// let unicode = Term::static_atom("unicode");
/// let trap = utf8_turn(&mut process, Utf8Output::default(), &Term::binary(vec![97; 100]), 64, &unicode)?;
/// assert!(matches!(trap, BifResult::TrapBif { function: "characters_to_utf8_trap", .. }));
/// assert_eq!(process.fcalls, 0);
/// ```
///
/// # Errors
///
/// Return a runtime error for chardata that ERTS returns as an `error` or
/// `incomplete` tuple: PBeam does not build these tuples yet (row
/// "Chardata conversion" in `docs/erts-differences.md`).
fn utf8_turn(
    process: &mut ProcessState,
    mut output: Utf8Output,
    chardata: &Term,
    mut left: i64,
    encoding: &Term,
) -> Result<BifResult, BeamError> {
    let latin1 = matches!(encoding, Term::Atom(name) if name.is_static("latin1"));
    let start = left;
    let (rest, failed) = build_utf8(chardata, &mut left, latin1, &mut output);
    cost_to_proc(process, start - left);
    let done = rest == Term::nil();
    if failed || (done && !output.leftover.is_empty()) {
        return Err(error(
            "unsupported unicode:characters_to_binary/2 result: ERTS returns an error or \
             incomplete tuple (row \"Chardata conversion\" in docs/erts-differences.md)",
        ));
    }
    if done {
        return Ok(Value(Term::binary(output.bytes)));
    }
    process.bump_all_reductions();
    Ok(TrapBif {
        module: "erlang",
        function: "characters_to_utf8_trap",
        arguments: vec![
            Term::binary(output.bytes),
            leftover_before(output.leftover, rest),
            encoding.clone(),
        ],
    })
}

/// Continue `unicode:characters_to_binary/2` in the export
/// `erlang:characters_to_utf8_trap/3` with the steps of the budget and the
/// encoding of the call, as `characters_to_utf8_trap` does
/// (`erl_unicode.c:851-886`).
///
/// # Examples
///
/// ```rust,ignore
/// let utf8 = Term::static_atom("utf8");
/// let result = utf8_trap(&mut process, b"ab".to_vec(), &Term::binary(b"c".to_vec()), &utf8)?;
/// assert!(matches!(result, BifResult::Value(Term::Binary(bytes)) if bytes == b"abc"));
/// ```
///
/// # Errors
///
/// Return the runtime errors of [`utf8_turn`].
pub(super) fn utf8_trap(
    process: &mut ProcessState,
    bytes: Vec<u8>,
    rest: &Term,
    encoding: &Term,
) -> Result<BifResult, BeamError> {
    let left = allowed_iterations(process);
    let output = Utf8Output {
        bytes,
        ..Utf8Output::default()
    };
    utf8_turn(process, output, rest, left, encoding)
}

/// Return `rest` with the bytes of an unfinished UTF-8 sequence before it
/// as a binary, or `rest` when there are no such bytes (`erl_unicode.c:800-807`,
/// `1063-1069`).
///
/// # Examples
///
/// ```rust,ignore
/// let rest = leftover_before(vec![195], Term::list_from(vec![Term::Integer(97)]));
/// assert_eq!(rest, Term::list_from(vec![Term::binary(vec![195]), Term::Integer(97)]));
/// ```
fn leftover_before(leftover: Vec<u8>, rest: Term) -> Term {
    if leftover.is_empty() {
        return rest;
    }
    Term::list_with_tail(vec![Term::binary(leftover)], rest)
}

/// Run `unicode:characters_to_list/2`: convert `chardata` to a list of code
/// points, as `unicode_characters_to_list_2` does
/// (`erl_unicode.c:1122-1169`). The `latin1` encoding reads each byte of a
/// binary as one code point, and `unicode` and `utf8` read binaries as
/// UTF-8. Invalid input gives `{error, Converted, Rest}`, an incomplete
/// UTF-8 sequence at the end gives `{incomplete, Converted, Rest}`, and a
/// term that is not chardata raises `badarg`. Each other encoding traps to
/// `unicode:characters_to_list_int/2`, as `BIF_TRAP2` does in ERTS
/// (`erl_unicode.c:1135-1143`).
///
/// A binary with `unicode` or `utf8` runs the `analyze_utf8` path
/// ([`analyze_turn`]). Each other term costs one reduction for each 50
/// steps of `utf8_need`, and then converts with the steps left, but at
/// least 1 ([`list_turn`]).
///
/// # Examples
///
/// ```rust,ignore
/// let chardata = Term::binary(b"hi".to_vec());
/// let result = characters_to_list(&mut process, &chardata, &Term::static_atom("utf8"))?;
/// assert!(matches!(result, BifResult::Value(list) if list == Term::list_from([Term::Integer(104), Term::Integer(105)])));
/// let foo = Term::static_atom("foo");
/// assert!(matches!(characters_to_list(&mut process, &foo, &Term::static_atom("latin1"))?, BifResult::Raise { .. }));
/// let result = characters_to_list(&mut process, &chardata, &Term::static_atom("utf16"))?;
/// assert!(matches!(result, BifResult::Trap { function: "characters_to_list_int", .. }));
/// ```
pub(super) fn characters_to_list(
    process: &mut ProcessState,
    chardata: &Term,
    encoding: &Term,
) -> Result<BifResult, BeamError> {
    let latin1 = match encoding {
        Term::Atom(encoding) if encoding.is_static("latin1") => true,
        Term::Atom(encoding) if encoding.is_one_of(&["unicode", "utf8"]) => false,
        _ => {
            return Ok(Trap {
                module: "unicode",
                function: "characters_to_list_int",
                arguments: vec![chardata.clone(), encoding.clone()],
            });
        }
    };
    if let Some(bytes) = chardata.binary_bytes()
        && !latin1
    {
        return analyze_turn(process, bytes, 0, 0);
    }
    let Some(cost) = utf8_need(chardata, latin1) else {
        return Ok(BifResult::badarg());
    };
    let simple = (cost / SIMPLE_FACTOR) as i64;
    cost_to_proc(process, simple);
    let left = (allowed_iterations(process) - simple).max(1);
    Ok(list_turn(
        process,
        Utf8Output::default(),
        chardata,
        left,
        latin1,
    ))
}

/// Convert `chardata` after the characters of `output` with `left` loop
/// steps, as `do_build_utf8` and `build_list_return` do
/// (`erl_unicode.c:508-749`, `1024-1084`), and bump one reduction for each
/// 10 steps. When the steps end before the chardata, the BIF uses all
/// reductions and traps to `erlang:characters_to_list_trap_1/4` with the
/// UTF-8 bytes, the number of characters, the rest, and the encoding
/// (`erl_unicode.c:1086-1120`). Else it builds the result
/// ([`finish_list`]) with the steps left, but at least 1.
///
/// # Examples
///
/// ```rust,ignore
/// let chardata = Term::list_from(vec![Term::Integer(97)]);
/// let result = list_turn(&mut process, Utf8Output::default(), &chardata, 100, false);
/// assert!(matches!(result, BifResult::Value(list) if list.is_list()));
/// ```
fn list_turn(
    process: &mut ProcessState,
    mut output: Utf8Output,
    chardata: &Term,
    mut left: i64,
    latin1: bool,
) -> BifResult {
    let start = left;
    let (rest, failed) = build_utf8(chardata, &mut left, latin1, &mut output);
    cost_to_proc(process, start - left);
    let leftover = std::mem::take(&mut output.leftover);
    let nil = Term::nil();
    let (reason, rest) = if failed {
        let rest = if leftover.is_empty() {
            rest
        } else {
            Term::list_from(vec![Term::binary(leftover), rest])
        };
        (Some("error"), rest)
    } else if rest == nil && !leftover.is_empty() {
        let reason = if is_incomplete(&leftover) {
            "incomplete"
        } else {
            "error"
        };
        (Some(reason), Term::binary(leftover))
    } else if rest != nil {
        process.bump_all_reductions();
        return TrapBif {
            module: "erlang",
            function: "characters_to_list_trap_1",
            arguments: vec![
                Term::binary(output.bytes),
                Term::Integer(output.characters as i64),
                leftover_before(leftover, rest),
                Term::static_atom(if latin1 { "latin1" } else { "unicode" }),
            ],
        };
    } else {
        (None, nil)
    };
    let converted = code_points(&output.bytes);
    let result = match reason {
        Some(reason) => Term::tuple(vec![Term::static_atom(reason), converted, rest]),
        None => converted,
    };
    let characters = if output.bytes.is_empty() {
        0
    } else {
        output.characters
    };
    finish_list(
        process,
        result,
        characters,
        left.max(1),
        "characters_to_list_trap_2",
    )
}

/// Continue `unicode:characters_to_list/2` in the export
/// `erlang:characters_to_list_trap_1/4` with the steps of the budget, as
/// `characters_to_list_trap_1` does (`erl_unicode.c:1086-1120`).
///
/// # Examples
///
/// ```rust,ignore
/// let result = list_trap(&mut process, b"a".to_vec(), 1, &Term::list_from(vec![Term::Integer(98)]), false)?;
/// assert!(matches!(result, BifResult::Value(list) if list.proper_list().is_some_and(|list| list.len() == 2)));
/// ```
#[expect(
    clippy::unnecessary_wraps,
    reason = "trap export arm of `bif_trap`; every trap arm returns `Result<BifResult, BeamError>`"
)]
pub(super) fn list_trap(
    process: &mut ProcessState,
    bytes: Vec<u8>,
    characters: usize,
    rest: &Term,
    latin1: bool,
) -> Result<BifResult, BeamError> {
    let left = allowed_iterations(process);
    let output = Utf8Output {
        bytes,
        characters,
        ..Utf8Output::default()
    };
    Ok(list_turn(process, output, rest, left, latin1))
}

/// Run the `analyze_utf8` path of `unicode:characters_to_list/2` for a
/// binary from byte `start`, after `characters` characters, as
/// `do_bif_utf8_to_list` does (`erl_unicode.c:1728-1843`). The walk counts
/// the characters with the steps of the budget, and bumps one reduction for
/// each 10. When the steps end before the binary, the BIF uses all
/// reductions and traps to `erlang:characters_to_list_trap_3/3` with the
/// binary, the position, and the characters so far. Else it builds the
/// result with the steps that the walk left ([`finish_list`]). An
/// incomplete sequence at the end gives `incomplete` when its bytes can
/// start a valid sequence, else `error`, and an invalid sequence gives
/// `error`, each with the rest of the binary.
///
/// # Examples
///
/// ```rust,ignore
/// let result = analyze_turn(&mut process, b"hi", 0, 0)?;
/// assert!(matches!(result, BifResult::Value(list) if list.proper_list().is_some_and(|list| list.len() == 2)));
/// ```
#[expect(
    clippy::unnecessary_wraps,
    reason = "trap export arm of `bif_trap`; every trap arm returns `Result<BifResult, BeamError>`"
)]
pub(super) fn analyze_turn(
    process: &mut ProcessState,
    bytes: &[u8],
    start: usize,
    characters: usize,
) -> Result<BifResult, BeamError> {
    let mut left = allowed_iterations(process);
    let (state, counted, end) = analyze_utf8(bytes, start, &mut left);
    cost_to_proc(process, counted as i64);
    let characters = characters + counted;
    if state == Utf8State::AnalyzeMore {
        process.bump_all_reductions();
        return Ok(TrapBif {
            module: "erlang",
            function: "characters_to_list_trap_3",
            arguments: vec![
                Term::binary(bytes.to_vec()),
                Term::Integer(end as i64),
                Term::Integer(characters as i64),
            ],
        });
    }
    let converted = code_points(&bytes[..end]);
    let rest = &bytes[end..];
    let reason = match state {
        Utf8State::Incomplete if is_incomplete(rest) => Some("incomplete"),
        Utf8State::Ok => None,
        _ => Some("error"),
    };
    let result = match reason {
        Some(reason) => Term::tuple(vec![
            Term::static_atom(reason),
            converted,
            Term::binary(rest.to_vec()),
        ]),
        None => converted,
    };
    let characters = if end == 0 { 0 } else { characters };
    Ok(finish_list(
        process,
        result,
        characters,
        left,
        "characters_to_list_trap_4",
    ))
}

/// Use the reductions of the list build of `unicode:characters_to_list/2`,
/// of which `remaining` characters remain, and return `result`, as
/// `finalize_list_to_list` and the build part of `do_bif_utf8_to_list` do
/// (`erl_unicode.c:1637-1691`, `1781-1808`). A call builds at most `left`
/// characters, but at least 1 (`do_utf8_to_list`,
/// `erl_unicode.c:1342-1361`), and bumps one reduction for each 10. When
/// characters remain, the BIF uses all reductions and traps to the export
/// `trap`, `erlang:characters_to_list_trap_2/2` or
/// `erlang:characters_to_list_trap_4/2`, which continues with the steps of
/// its budget. ERTS builds the list in each call. PBeam builds the result
/// in the first call, so the trap keeps the result and the characters that
/// remain.
///
/// # Examples
///
/// ```rust,ignore
/// process.fcalls = 3;
/// let trap = finish_list(&mut process, Term::nil(), 40, 30, "characters_to_list_trap_2");
/// assert!(matches!(trap, BifResult::TrapBif { arguments, .. } if arguments[1] == Term::Integer(10)));
/// ```
pub(super) fn finish_list(
    process: &mut ProcessState,
    result: Term,
    remaining: usize,
    left: i64,
    trap: &'static str,
) -> BifResult {
    if remaining == 0 {
        return Value(result);
    }
    let built = if left < remaining as i64 {
        left.max(1) as usize
    } else {
        remaining
    };
    cost_to_proc(process, built as i64);
    if built == remaining {
        return Value(result);
    }
    process.bump_all_reductions();
    TrapBif {
        module: "erlang",
        function: trap,
        arguments: vec![result, Term::Integer((remaining - built) as i64)],
    }
}

/// Continue the list build of `unicode:characters_to_list/2` in the export
/// `erlang:characters_to_list_trap_2/2` or `_4/2` with the steps of the
/// budget, as `characters_to_list_trap_2` and `characters_to_list_trap_4`
/// do (`erl_unicode.c:1693-1705`, `1880-1912`).
///
/// # Examples
///
/// ```rust,ignore
/// let result = finish_trap(&mut process, Term::nil(), 5, "characters_to_list_trap_2")?;
/// assert!(matches!(result, BifResult::Value(_)));
/// ```
#[expect(
    clippy::unnecessary_wraps,
    reason = "trap export arm of `bif_trap`; every trap arm returns `Result<BifResult, BeamError>`"
)]
pub(super) fn finish_trap(
    process: &mut ProcessState,
    result: Term,
    remaining: usize,
    trap: &'static str,
) -> Result<BifResult, BeamError> {
    let left = allowed_iterations(process);
    Ok(finish_list(process, result, remaining, left, trap))
}

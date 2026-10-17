//! Bodies of the `re` BIFs that do not fit on one table line.

use std::ffi::c_int;

use super::BifResult::{Trap, Value};
use super::*;
use crate::runtime::scheduler::CONTEXT_REDS;
use crate::runtime::{pcre2, regex_code::RegexCodes};

/// The atom options of `re:compile/2` that set one `PCRE2_*` compile bit
/// (`parse_options`, `erl_bif_re.c:442-537`).
const COMPILE_OPTIONS: [(&str, u32); 14] = [
    ("anchored", pcre2::ANCHORED),
    ("caseless", pcre2::CASELESS),
    ("dollar_endonly", pcre2::DOLLAR_ENDONLY),
    ("dotall", pcre2::DOTALL),
    ("dupnames", pcre2::DUPNAMES),
    ("extended", pcre2::EXTENDED),
    ("firstline", pcre2::FIRSTLINE),
    ("multiline", pcre2::MULTILINE),
    ("never_utf", pcre2::NEVER_UTF),
    ("no_auto_capture", pcre2::NO_AUTO_CAPTURE),
    ("no_start_optimize", pcre2::NO_START_OPTIMIZE),
    ("ucp", pcre2::UCP),
    ("ungreedy", pcre2::UNGREEDY),
    ("unicode", pcre2::UTF),
];

/// The values of the `{newline, X}` option (`erl_bif_re.c:410-433`).
const NEWLINES: [(&str, u32); 6] = [
    ("cr", pcre2::NEWLINE_CR),
    ("crlf", pcre2::NEWLINE_CRLF),
    ("lf", pcre2::NEWLINE_LF),
    ("nul", pcre2::NEWLINE_NUL),
    ("anycrlf", pcre2::NEWLINE_ANYCRLF),
    ("any", pcre2::NEWLINE_ANY),
];

/// The flags of an option list, as `PARSE_FLAG_*` (`erl_bif_re.c:321-330`)
/// names them.
const FLAG_UNIQUE_COMPILE_OPT: i32 = 1;
const FLAG_UNIQUE_EXEC_OPT: i32 = 2;
const FLAG_UNICODE: i32 = 4;
const FLAG_STARTOFFSET: i32 = 8;
const FLAG_CAPTURE_OPT: i32 = 16;
const FLAG_GLOBAL: i32 = 32;
const FLAG_REPORT_ERRORS: i32 = 64;
const FLAG_MATCH_LIMIT: i32 = 128;
const FLAG_MATCH_LIMIT_RECURSION: i32 = 256;
const FLAG_EXPORT: i32 = 512;

/// The compile atoms of an option list that set one `PCRE2_*` compile bit
/// and the flag `PARSE_FLAG_UNIQUE_COMPILE_OPT`
/// (`erl_bif_re.c:466-513`). `unicode` and `export` set a flag of their own,
/// and `anchored` sets no flag, so each of them is separate.
const COMPILE_ONLY: [&str; 12] = [
    "caseless",
    "dollar_endonly",
    "dotall",
    "dupnames",
    "extended",
    "firstline",
    "multiline",
    "never_utf",
    "no_auto_capture",
    "no_start_optimize",
    "ucp",
    "ungreedy",
];

/// The match atoms of an option list that set one `PCRE2_*` match bit and
/// the flag `PARSE_FLAG_UNIQUE_EXEC_OPT` (`erl_bif_re.c:450-465`).
const MATCH_ONLY: [(&str, u32); 4] = [
    ("notbol", pcre2::NOTBOL),
    ("noteol", pcre2::NOTEOL),
    ("notempty", pcre2::NOTEMPTY),
    ("notempty_atstart", pcre2::NOTEMPTY_ATSTART),
];

/// The options of one `re:compile/2`, `re:run/2`, `re:run/3`, or
/// `re:internal_run/4` call, as the fields of `struct parsed_options`
/// (`erl_bif_re.c:338-348`) that they use.
#[derive(Debug, Default)]
struct Options {
    /// The `PCRE2_*` compile bits.
    compile: u32,
    /// The last `{newline, X}` value, if any.
    newline: Option<u32>,
    /// The last `bsr_anycrlf` or `bsr_unicode` value, if any.
    bsr: Option<u32>,
    /// The `PCRE2_*` match bits, the `match` field of the ERTS struct.
    match_options: u32,
    /// The `PARSE_FLAG_*` bits.
    flags: i32,
    /// The `{offset, N}` value, or 0 without the option.
    startoffset: i32,
    /// The values and the type of the `capture` option, or `None` without
    /// the option.
    capture: Option<(Term, Term)>,
    /// The `{match_limit, N}` value, or 0 without the option.
    match_limit: u32,
    /// The `{match_limit_recursion, N}` value, or 0 without the option.
    match_limit_recursion: u32,
}

impl Options {
    /// Return the two terms of the `capture` option, as `re_run` reads them
    /// (`erl_bif_re.c:1513-1514`, `1646-1647`), or `None` without the
    /// option.
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// let options = parse_options(&atoms, &Term::list_from(vec![Term::static_atom("global")])).unwrap();
    /// assert!(options.capture().is_none());
    /// ```
    fn capture(&self) -> Option<(&Term, &Term)> {
        self.capture.as_ref().map(|(values, kind)| (values, kind))
    }
}

/// Read the options of an option list, or return `None` when ERTS
/// `parse_options` (`erl_bif_re.c:350-543`) fails for them.
///
/// It fails for a term that is not a proper list, an unknown atom, a tuple
/// that is neither `{Atom, Value}` nor `{capture, Values, Type}`, an unknown
/// tuple key, an unknown `{newline, X}` value, a `{offset, N}`,
/// `{match_limit, N}`, or `{match_limit_recursion, N}` value that is not a
/// non-negative integer of the `int` range, and an improper list. It reads
/// every option, also the ones that the caller later rejects: `re_compile`
/// raises `badarg` for an option that sets `PARSE_FLAG_UNIQUE_EXEC_OPT`
/// (`erl_bif_re.c:914-916`) and `re:run` for an option that sets
/// `PARSE_FLAG_EXPORT` (`erl_bif_re.c:1501-1504`). A later `newline` or
/// `bsr` option replaces an earlier one.
///
/// # Examples
///
/// ```rust,ignore
/// let options = parse_options(&atoms, &Term::list_from(vec![Term::static_atom("unicode")])).unwrap();
/// assert_eq!(options.compile, pcre2::UTF);
/// assert_eq!(options.flags & FLAG_UNICODE, FLAG_UNICODE);
/// let options = parse_options(&atoms, &Term::list_from(vec![Term::static_atom("global")])).unwrap();
/// assert_eq!(options.flags & FLAG_UNIQUE_EXEC_OPT, FLAG_UNIQUE_EXEC_OPT);
/// assert!(parse_options(&atoms, &Term::static_atom("foo")).is_none());
/// ```
fn parse_options(atoms: &AtomTable, options: &Term) -> Option<Options> {
    let options = options.proper_list()?;
    let mut parsed = Options::default();
    for option in options {
        match option {
            Term::Atom(atom) => {
                let name = atoms.name_of(atom);
                if name == "anchored" {
                    parsed.compile |= pcre2::ANCHORED;
                    parsed.match_options |= pcre2::ANCHORED;
                } else if COMPILE_ONLY.contains(&name) {
                    parsed.compile |= lookup(&COMPILE_OPTIONS, name)?;
                    parsed.flags |= FLAG_UNIQUE_COMPILE_OPT;
                } else if let Some(&(_, bit)) = MATCH_ONLY.iter().find(|&&(name_, _)| name_ == name)
                {
                    parsed.match_options |= bit;
                    parsed.flags |= FLAG_UNIQUE_EXEC_OPT;
                } else if name == "unicode" {
                    parsed.compile |= pcre2::UTF;
                    parsed.flags |= FLAG_UNIQUE_COMPILE_OPT | FLAG_UNICODE;
                } else if name == "export" {
                    parsed.flags |= FLAG_EXPORT | FLAG_UNIQUE_COMPILE_OPT;
                } else if name == "report_errors" {
                    parsed.flags |= FLAG_UNIQUE_EXEC_OPT | FLAG_REPORT_ERRORS;
                } else if name == "global" {
                    parsed.flags |= FLAG_UNIQUE_EXEC_OPT | FLAG_GLOBAL;
                } else if name == "bsr_anycrlf" {
                    parsed.bsr = Some(pcre2::BSR_ANYCRLF);
                } else if name == "bsr_unicode" {
                    parsed.bsr = Some(pcre2::BSR_UNICODE);
                } else {
                    return None;
                }
            }
            Term::Tuple(elements) => match elements.as_ref() {
                [Term::Atom(key), Term::Atom(value)] if key.is_static("newline") => {
                    parsed.newline = Some(lookup(&NEWLINES, atoms.name_of(value))?);
                }
                [Term::Atom(key), values] if key.is_static("capture") => {
                    parsed.capture = Some((values.clone(), Term::static_atom("index")));
                    parsed.flags |= FLAG_CAPTURE_OPT | FLAG_UNIQUE_EXEC_OPT;
                }
                [Term::Atom(key), values, kind] if key.is_static("capture") => {
                    parsed.capture = Some((values.clone(), kind.clone()));
                    parsed.flags |= FLAG_CAPTURE_OPT | FLAG_UNIQUE_EXEC_OPT;
                }
                [Term::Atom(key), value] if key.is_static("offset") => {
                    parsed.startoffset = option_integer(value)?;
                    parsed.flags |= FLAG_UNIQUE_EXEC_OPT | FLAG_STARTOFFSET;
                }
                [Term::Atom(key), value] if key.is_static("match_limit") => {
                    parsed.match_limit = u32::try_from(option_integer(value)?).ok()?;
                    parsed.flags |= FLAG_UNIQUE_EXEC_OPT | FLAG_MATCH_LIMIT;
                }
                [Term::Atom(key), value] if key.is_static("match_limit_recursion") => {
                    parsed.match_limit_recursion = u32::try_from(option_integer(value)?).ok()?;
                    parsed.flags |= FLAG_UNIQUE_EXEC_OPT | FLAG_MATCH_LIMIT_RECURSION;
                }
                _ => return None,
            },
            _ => return None,
        }
    }
    Some(parsed)
}

/// Return the value of an `{Option, N}` integer or a capture index, as
/// `term_to_int` (`erl_bif_re.c:244-293`) reads it on a 64-bit host: a small
/// integer from 0 to `INT_MAX`. That function reads the small value as an
/// unsigned `Uint`, so a negative small is above `INT_MAX` and it rejects
/// it too. `erl_bif_re.c:388`, `398`, and `408` check the value again, but
/// no negative value passes the first check.
///
/// # Examples
///
/// ```rust,ignore
/// assert_eq!(option_integer(&Term::Integer(5)), Some(5));
/// assert_eq!(option_integer(&Term::Integer(-1)), None);
/// assert_eq!(option_integer(&Term::Float(1.0)), None);
/// assert_eq!(option_integer(&Term::Integer(i64::from(i32::MAX) + 1)), None);
/// ```
fn option_integer(value: &Term) -> Option<i32> {
    let Term::Integer(value) = value else {
        return None;
    };
    i32::try_from(*value).ok().filter(|value| *value >= 0)
}

/// Return the value of `name` in a table of `(name, value)` pairs.
///
/// # Examples
///
/// ```rust,ignore
/// assert_eq!(lookup(&NEWLINES, "crlf"), Some(pcre2::NEWLINE_CRLF));
/// assert_eq!(lookup(&NEWLINES, "bogus"), None);
/// ```
fn lookup(table: &[(&str, u32)], name: &str) -> Option<u32> {
    table
        .iter()
        .find_map(|&(entry, value)| (entry == name).then_some(value))
}

/// Run `re:compile(Pattern, Options)` as `re_compile`
/// (`erl_bif_re.c:891-955`) does, in its order:
///
/// 1. Options that `parse_options` rejects, and each execution option,
///    raise `badarg` (`erl_bif_re.c:908-916`).
/// 2. With `unicode`, a pattern that is not a binary traps to
///    `re:ucompile/2` with the same arguments (`erl_bif_re.c:921-923`).
///    `re:ucompile/2` converts the pattern to UTF-8 with
///    `unicode:characters_to_binary/2` and calls `re:compile/2` again.
/// 3. A pattern that is not iodata raises `badarg` (`erl_bif_re.c:927-929`).
/// 4. A pattern that PCRE2 does not compile gives
///    `{error, {Message, Offset}}`, where `Message` is a list of character
///    codes (`build_compile_error`, `erl_bif_re.c:802-836`).
/// 5. With `export`, the result is `{ok, {re_exported_pattern, Header,
///    Source, Options, Encoded}}` (`build_compile_export`,
///    `erl_bif_re.c:644-700`), and no reference is made. See `export`.
/// 6. Else the result is `{ok, {re_pattern, CaptureCount, Unicode, UseCrlf,
///    Ref}}` (`build_compile_result`, `erl_bif_re.c:581-615`). `Unicode` is
///    1 for the `unicode` option. `UseCrlf` is 1 when the newline
///    convention of the code is `any`, `crlf`, or `anycrlf`, also from an
///    item such as `(*CRLF)` in the pattern. `Ref` is a new reference, and
///    the compiled code is the resource of that reference.
///
/// # Examples
///
/// ```rust,ignore
/// let unicode = Term::list_from(vec![Term::static_atom("unicode")]);
/// let result = compile(&mut context, &Term::binary(b"(a)".to_vec()), &unicode)?;
/// // {ok, {re_pattern, 1, 1, 0, Ref}}
/// assert!(matches!(result, BifResult::Value(Term::Tuple(_))));
/// let result = compile(&mut context, &Term::list_from(vec![Term::Integer(256)]), &unicode)?;
/// assert!(matches!(result, BifResult::Trap { function: "ucompile", .. }));
/// let result = compile(&mut context, &Term::static_atom("foo"), &Term::nil())?;
/// assert!(matches!(result, BifResult::Raise { .. }));
/// ```
///
/// # Errors
///
/// Return a runtime error when the scheduler has no new reference.
pub(super) fn compile(
    context: &mut BifContext<'_>,
    pattern: &Term,
    options: &Term,
) -> Result<BifResult, BeamError> {
    let Some(parsed) = parse_options(&context.scheduler.atoms, options) else {
        return Ok(BifResult::badopt());
    };
    if parsed.flags & FLAG_UNIQUE_EXEC_OPT != 0 {
        return Ok(BifResult::badopt());
    }
    let unicode = parsed.flags & FLAG_UNICODE != 0;
    if unicode && !matches!(pattern, Term::Binary(_)) {
        return Ok(Trap {
            module: "re",
            function: "ucompile",
            arguments: vec![pattern.clone(), options.clone()],
        });
    }
    let Some(bytes) = iolist::iodata_bytes(pattern) else {
        return Ok(BifResult::badarg());
    };
    let code = match pcre2::compile(&bytes, parsed.compile, parsed.newline, parsed.bsr) {
        Ok(code) => code,
        Err(failure) => return Ok(Value(compile_error(&failure, None))),
    };
    if parsed.flags & FLAG_EXPORT != 0 {
        return Ok(Value(export(&code, unicode, bytes, options)));
    }
    let result = compile_result(context, code, unicode.into())?;
    Ok(Value(Term::tuple(vec![Term::static_atom("ok"), result])))
}

/// Run `re:import(Exported)` as `re_import_1` (`erl_bif_re.c:703-796`)
/// does, in its order:
///
/// 1. A term that is not a 5-tuple, or whose first element is not
///    `re_exported_pattern`, raises `badarg` (`erl_bif_re.c:722-728`).
/// 2. `Header` must be a binary of at least 14 bytes that starts with
///    `re-PCRE2` (`erl_bif_re.c:730-734`).
/// 3. With encode version 1, `Encoded` must be a binary whose zlib CRC-32
///    equals the checksum of the header (`erl_bif_re.c:749-759`). A failed
///    check raises `badarg`.
/// 4. With encode version 1 and a matching checksum, PCRE2 decodes
///    `Encoded`, and the unicode byte of the header becomes the unicode
///    element of the result (`erl_bif_re.c:759-781`).
/// 5. A decoded code gives `{re_pattern, CaptureCount, Unicode, UseCrlf,
///    Ref}`, with no `ok` (`build_compile_result`,
///    `erl_bif_re.c:581-615`).
/// 6. When the encode version, the magic number, the PCRE2 version, or the
///    PCRE2 configuration does not match, `Source` and `Options` are
///    compiled again as `re:compile/2` does, but a bad option, a source
///    that is not a binary, and a compile error raise `badarg`
///    (`erl_bif_re.c:765-789`, `927-949`). The unicode element then comes
///    from the options.
/// 7. Any other decode error raises `badarg` (`erl_bif_re.c:790-795`).
///
/// # Examples
///
/// ```rust,ignore
/// let options = Term::list_from(vec![Term::static_atom("export")]);
/// let compiled = compile(&mut context, &Term::binary(b"(a)".to_vec()), &options)?;
/// let BifResult::Value(Term::Tuple(ok)) = compiled else { panic!() };
/// let exported = &ok[1];
/// // {re_pattern, 1, 0, 0, Ref}
/// assert!(matches!(import(&mut context, exported)?, BifResult::Value(Term::Tuple(_))));
/// assert!(matches!(import(&mut context, &Term::static_atom("foo"))?, BifResult::Raise { .. }));
/// ```
pub(super) fn import(
    context: &mut BifContext<'_>,
    exported: &Term,
) -> Result<BifResult, BeamError> {
    let Term::Tuple(elements) = exported else {
        return Ok(BifResult::badarg());
    };
    let [tag, header, source, options, encoded] = elements.as_ref() else {
        return Ok(BifResult::badarg());
    };
    if !is_atom(tag, "re_exported_pattern") {
        return Ok(BifResult::badarg());
    }
    let Term::Binary(header) = header else {
        return Ok(BifResult::badarg());
    };
    let header: &[u8] = header;
    if header.len() < EXPORT_HEADER_SIZE || header[..EXPORT_TITLE.len()] != EXPORT_TITLE[..] {
        return Ok(BifResult::badarg());
    }
    if header[EXPORT_VERSION_OFFSET] == EXPORT_VERSION {
        let Term::Binary(encoded) = encoded else {
            return Ok(BifResult::badarg());
        };
        let checksum = u32::from_be_bytes(
            header[EXPORT_CHECKSUM_OFFSET..EXPORT_VERSION_OFFSET]
                .try_into()
                .expect("the header holds the checksum"),
        );
        if checksum != crc32(encoded) {
            return Ok(BifResult::badarg());
        }
        let unicode = header[EXPORT_UNICODE_OFFSET];
        match pcre2::deserialize(encoded) {
            pcre2::Deserialized::Code(code) => {
                return Ok(Value(compile_result(context, code, unicode)?));
            }
            pcre2::Deserialized::Incompatible => {}
            pcre2::Deserialized::Invalid => return Ok(BifResult::badarg()),
        }
    }
    import_fallback(context, source, options)
}

/// Compile the source and options of an export that PCRE2 did not decode,
/// as the fallback of `re_import_1` (`erl_bif_re.c:783-788`) and
/// `re_compile` with `is_import` (`erl_bif_re.c:891-955`) do. The source
/// must be a binary, the options must parse and hold no execution option,
/// and the source must compile; each other result is `badarg`.
///
/// # Examples
///
/// ```rust,ignore
/// let source = Term::binary(b"(a)".to_vec());
/// let options = Term::list_from(vec![Term::static_atom("unicode")]);
/// // {re_pattern, 1, 1, 0, Ref}
/// assert!(matches!(import_fallback(&mut context, &source, &options)?, BifResult::Value(Term::Tuple(_))));
/// assert!(matches!(import_fallback(&mut context, &Term::Integer(1), &Term::nil())?, BifResult::Raise { .. }));
/// ```
fn import_fallback(
    context: &mut BifContext<'_>,
    source: &Term,
    options: &Term,
) -> Result<BifResult, BeamError> {
    let Term::Binary(source) = source else {
        return Ok(BifResult::badarg());
    };
    let Some(parsed) = parse_options(&context.scheduler.atoms, options) else {
        return Ok(BifResult::badopt());
    };
    if parsed.flags & FLAG_UNIQUE_EXEC_OPT != 0 {
        return Ok(BifResult::badopt());
    }
    let unicode = parsed.flags & FLAG_UNICODE != 0;
    let code = match pcre2::compile(source, parsed.compile, parsed.newline, parsed.bsr) {
        Ok(code) => code,
        Err(_) => return Ok(BifResult::badarg()),
    };
    Ok(Value(compile_result(context, code, unicode.into())?))
}

/// Return `{error, {Message, Offset}}`, or `{error, {Tag, {Message,
/// Offset}}}` with a tag, as `build_compile_error`
/// (`erl_bif_re.c:802-836`) makes it. `Message` is the PCRE2 text as a list
/// of character codes. `re:compile/2` gives no tag, and `re:run/2,3` gives
/// the tag `compile` with `report_errors`.
///
/// # Examples
///
/// ```rust,ignore
/// let failure = pcre2::compile(b"(", 0, None, None).unwrap_err();
/// // {error, {[109, 105, ...], 1}}
/// let error = compile_error(&failure, None);
/// // {error, {compile, {[109, 105, ...], 1}}}
/// let tagged = compile_error(&failure, Some("compile"));
/// ```
fn compile_error(failure: &pcre2::CompileError, tag: Option<&str>) -> Term {
    let message = failure
        .message
        .iter()
        .map(|&byte| Term::Integer(byte.into()));
    let offset = i64::try_from(failure.offset).expect("a pattern offset fits in i64");
    let error = Term::tuple(vec![Term::list_from(message), Term::Integer(offset)]);
    let error = match tag {
        Some(tag) => Term::tuple(vec![Term::static_atom(tag), error]),
        None => error,
    };
    Term::tuple(vec![Term::static_atom("error"), error])
}

/// Return `{re_pattern, CaptureCount, Unicode, UseCrlf, Ref}`, as
/// `build_compile_result` (`erl_bif_re.c:581-615`) makes it with
/// `with_ok` false. `Unicode` is 1 for the `unicode` option. `UseCrlf` is 1
/// when the newline convention of the code is `any`, `crlf`, or `anycrlf`,
/// also from an item such as `(*CRLF)` in the pattern (`erl_bif_re.c:597-599`).
/// `Ref` is a new reference, and the compiled code is the resource of that
/// reference.
///
/// # Examples
///
/// ```rust,ignore
/// let code = pcre2::compile(b"(a)", pcre2::UTF, None, None).unwrap();
/// // {re_pattern, 1, 1, 0, Ref}
/// let result = compile_result(&mut context, code, 1)?;
/// assert!(matches!(result, Term::Tuple(_)));
/// ```
///
/// # Errors
///
/// Return a runtime error when the scheduler has no new reference.
fn compile_result(
    context: &mut BifContext<'_>,
    code: pcre2::Code,
    unicode: u8,
) -> Result<Term, BeamError> {
    let use_crlf = matches!(
        code.newline(),
        pcre2::NEWLINE_ANY | pcre2::NEWLINE_CRLF | pcre2::NEWLINE_ANYCRLF
    );
    let capture_count = code.capture_count();
    let reference = context.scheduler.make_ref()?;
    context
        .scheduler
        .resources
        .borrow_mut::<RegexCodes>()
        .0
        .insert(reference, code);
    Ok(Term::tuple(vec![
        Term::static_atom("re_pattern"),
        Term::Integer(capture_count.into()),
        Term::Integer(unicode.into()),
        Term::Integer(use_crlf.into()),
        Term::Reference(reference),
    ]))
}

/// What one element of the result of a match holds, as the capture type of
/// `ReturnInfo` (`erl_bif_re.c:1322-1334`).
#[derive(Clone, Copy)]
enum ReturnType {
    /// The type `index`: an offset pair.
    Index,
    /// The type `list`: a list of character codes.
    List,
    /// The type `binary`: a binary.
    Binary,
}

/// The groups that the result of a match holds, as `num_spec` and its
/// vector (`erl_bif_re.c:983-987`, `1336-1466`).
enum Groups {
    /// The capture `all`: each group of the match.
    All,
    /// The capture `all_but_first`: each group from group 1.
    AllButFirst,
    /// The capture `first` and a list of group numbers: the group of each
    /// element of the result.
    Indexes(Vec<i32>),
}

/// The capture specification of a match, as the `ReturnInfo` of
/// `build_capture` (`erl_bif_re.c:1309-1479`). `kind` is `None` for the
/// capture `none`, which makes the result the atom `match`.
struct Capture {
    /// What each element of the result holds, or `None` for no element.
    kind: Option<ReturnType>,
    /// The groups of the result.
    groups: Groups,
}

/// Return whether `term` is the static atom `name`, as the option and
/// capture checks of `erl_bif_re.c:378-536` and `1322-1466` compare each
/// value with one atom.
///
/// # Examples
///
/// ```rust,ignore
/// assert!(is_atom(&Term::static_atom("all"), "all"));
/// assert!(!is_atom(&Term::static_atom("none"), "all"));
/// assert!(!is_atom(&Term::Integer(1), "all"));
/// ```
fn is_atom(term: &Term, name: &str) -> bool {
    matches!(term, Term::Atom(atom) if atom.is_static(name))
}

/// Return whether `pattern` is a binary or a list, the pattern forms that
/// the textual path of `re_run` accepts (`erl_bif_re.c:1517`). `[]` is a
/// list.
///
/// # Examples
///
/// ```rust,ignore
/// assert!(is_text_pattern(&Term::binary(b"a".to_vec())));
/// assert!(is_text_pattern(&Term::nil()));
/// assert!(!is_text_pattern(&Term::static_atom("a")));
/// ```
fn is_text_pattern(pattern: &Term) -> bool {
    matches!(pattern, Term::Binary(_)) || pattern.is_list()
}

/// Return the capture specification of a match, or `None` when
/// `build_capture` (`erl_bif_re.c:1309-1479`) gives `badarg` for it.
///
/// It gives `badarg` for a capture type that is not `index`, `list`, or
/// `binary` (`erl_bif_re.c:1322-1334`), for values that are neither a
/// capture atom nor a list (`erl_bif_re.c:1463-1465`), for a list that is
/// not proper (`erl_bif_re.c:1460-1462`), and for an element of a list that
/// is neither an integer nor a name (`erl_bif_re.c:1456-1458`). PBeam does
/// not support a named group yet, so an element that is a name, and
/// `all_names` on a pattern with names, give the runtime error
/// [`unsupported_name`].
///
/// # Examples
///
/// ```rust,ignore
/// let code = pcre2::compile(b"(a)(b)", 0, None, None).unwrap();
/// let all = Term::static_atom("all");
/// let index = Term::static_atom("index");
/// // [{0, 0}]
/// let capture = build_capture(&code, Some((&all, &Term::static_atom("none"))))?;
/// assert!(capture.is_some_and(|capture| capture.kind.is_none()));
/// let bad = Term::list_from(vec![Term::Float(1.0)]);
/// assert!(build_capture(&code, Some((&bad, &index))).unwrap().is_none());
/// ```
///
/// # Errors
///
/// Return a runtime error when the capture names a group, which PBeam does
/// not support yet.
fn build_capture(
    code: &pcre2::Code,
    capture: Option<(&Term, &Term)>,
) -> Result<Option<Capture>, BeamError> {
    let default = Capture {
        kind: Some(ReturnType::Index),
        groups: Groups::All,
    };
    let Some((values, kind)) = capture else {
        return Ok(Some(default));
    };
    let kind = if is_atom(kind, "index") {
        Some(ReturnType::Index)
    } else if is_atom(kind, "list") {
        Some(ReturnType::List)
    } else if is_atom(kind, "binary") {
        Some(ReturnType::Binary)
    } else {
        return Ok(None);
    };
    if is_atom(values, "none") || matches!(values, Term::Nil) {
        // `build_capture` gives the capture `none` for the atom `none` and
        // for `[]` (`erl_bif_re.c:1340-1344`).
        return Ok(Some(Capture {
            kind: None,
            groups: Groups::All,
        }));
    }
    if is_atom(values, "all_names") {
        // `all_names` gives the capture `none` for a pattern with no name
        // (`erl_bif_re.c:1367-1373`).
        if code.name_count() > 0 {
            return Err(unsupported_name());
        }
        return Ok(Some(Capture {
            kind: None,
            groups: Groups::All,
        }));
    }
    let groups = if is_atom(values, "all") {
        Groups::All
    } else if is_atom(values, "all_but_first") {
        Groups::AllButFirst
    } else if is_atom(values, "first") {
        // `build_capture` writes the index 0 of the capture `first`
        // (`erl_bif_re.c:1348-1355`).
        Groups::Indexes(vec![0])
    } else if let Some(elements) = values.proper_list() {
        let mut indexes = Vec::new();
        for element in elements {
            if let Term::Atom(_) | Term::Binary(_) = element {
                return Err(unsupported_name());
            }
            if element.is_list() {
                return Err(unsupported_name());
            }
            let Some(index) = option_integer(element) else {
                return Ok(None);
            };
            indexes.push(index);
        }
        Groups::Indexes(indexes)
    } else {
        return Ok(None);
    };
    Ok(Some(Capture { kind, groups }))
}

/// Return the runtime error of a `re:run/2,3` call that names a capture
/// group. PBeam does not resolve a group name to its number yet (row
/// "Regular expressions" in `docs/erts-differences.md`), and a name that it
/// ignored would give a wrong capture, so the call stops.
///
/// # Examples
///
/// ```rust,ignore
/// let error = unsupported_name();
/// ```
fn unsupported_name() -> BeamError {
    crate::error::error(
        "unsupported capture group name in re:run/2,3 \
         (see \"Regular expressions\" in docs/erts-differences.md)",
    )
}

/// Return the value of a match, as `build_exec_return`
/// (`erl_bif_re.c:1047-1256`) makes it:
///
/// - A return code of 0 or less gives `nomatch`, or `{error,
///   match_limit}` and `{error, match_limit_recursion}` for a match that
///   `report_errors` stopped at that limit (`erl_bif_re.c:1051-1064`).
/// - The capture `none` gives `match` (`erl_bif_re.c:1081-1082`).
/// - Else the value is `{match, Captures}`, and each element is an offset
///   pair, a byte list, or a binary, of the group that the capture
///   specification names. A group that did not take part in the match gives
///   `{-1, 0}`, an empty list, or an empty binary
///   (`erl_bif_re.c:1090-1097`, `1155-1161`, `1233-1240`). An index that is
///   not a group of the match gives the same empty value
///   (`erl_bif_re.c:1126-1132`, `1205-1210`, `1233-1240`).
///
/// # Examples
///
/// ```rust,ignore
/// let capture = Capture { kind: Some(ReturnType::Index), groups: Groups::All };
/// // {match, [{2, 1}]}
/// let value = exec_return(&data, 1, &capture, 0, b"abc");
/// // nomatch
/// let value = exec_return(&data, ERROR_NOMATCH, &capture, 0, b"abc");
/// ```
fn exec_return(
    data: &pcre2::MatchData,
    rc: c_int,
    capture: &Capture,
    flags: i32,
    subject: &[u8],
) -> Term {
    if rc <= 0 {
        if flags & FLAG_REPORT_ERRORS != 0 {
            if rc == pcre2::ERROR_MATCHLIMIT {
                return error_reason("match_limit");
            }
            if rc == pcre2::ERROR_DEPTHLIMIT {
                return error_reason("match_limit_recursion");
            }
        }
        return Term::static_atom("nomatch");
    }
    let Some(kind) = capture.kind else {
        return Term::static_atom("match");
    };
    let rc = rc as usize;
    let ovector = data.ovector(rc);
    let element = |index: usize| capture_element(kind, ovector, index, subject);
    let elements: Vec<Term> = match &capture.groups {
        Groups::All => (0..rc).map(element).collect(),
        Groups::AllButFirst => (1..rc).map(element).collect(),
        Groups::Indexes(indexes) => indexes
            .iter()
            .map(|&index| match usize::try_from(index) {
                Ok(index) if index < rc => element(index),
                _ => empty_element(kind),
            })
            .collect(),
    };
    Term::tuple(vec![Term::static_atom("match"), Term::list_from(elements)])
}

/// Return the `error:internal_error` exception, which ERTS raises with
/// `EXC_INTERNAL_ERROR` for an unexpected match result
/// (`erl_bif_re.c:1788-1793`, `1903-1905`). ERTS marks the error
/// `EXF_PANIC` (`error.h:133`) and ends the runtime.
fn internal_error() -> BifResult {
    BifResult::Raise {
        class: ExceptionClass::Error,
        reason: Term::static_atom("internal_error"),
    }
}

/// Return the element of a capture that the result holds with no bytes: a
/// group that did not take part in the match, and an index that is not a
/// group of the match, give `{-1, 0}`, an empty list, or an empty binary
/// (`erl_bif_re.c:1092-1093`, `1130-1131`, `1233-1240`).
///
/// # Examples
///
/// ```rust,ignore
/// // {-1, 0}
/// let pair = empty_element(ReturnType::Index);
/// // <<>>
/// let binary = empty_element(ReturnType::Binary);
/// ```
fn empty_element(kind: ReturnType) -> Term {
    match kind {
        ReturnType::Index => Term::tuple(vec![Term::Integer(-1), Term::Integer(0)]),
        ReturnType::List => Term::nil(),
        ReturnType::Binary => Term::binary(Vec::new()),
    }
}

/// Return `{error, Reason}`, as `build_exec_return` makes it for a match
/// that a limit stopped (`erl_bif_re.c:1053-1058`).
///
/// # Examples
///
/// ```rust,ignore
/// // {error, match_limit}
/// let value = error_reason("match_limit");
/// ```
fn error_reason(reason: &str) -> Term {
    Term::tuple(vec![Term::static_atom("error"), Term::static_atom(reason)])
}

/// Return the element of one group of a match, as `build_exec_return` makes
/// it (`erl_bif_re.c:1090-1097`, `1155-1179`): an offset pair, a byte list,
/// or a binary. A group that did not take part in the match gives `{-1, 0}`,
/// an empty list, or an empty binary. `subject` is the whole subject when
/// the group took part in the match, and `[]` when it did not.
///
/// # Examples
///
/// ```rust,ignore
/// let ovector = [2, 3, UNSET, UNSET];
/// // {2, 1}
/// let pair = capture_element(ReturnType::Index, &ovector, 0, b"abc");
/// // {-1, 0}
/// let unset = capture_element(ReturnType::Index, &ovector, 1, b"abc");
/// ```
fn capture_element(kind: ReturnType, ovector: &[usize], index: usize, subject: &[u8]) -> Term {
    let (from, to) = (ovector[index * 2], ovector[index * 2 + 1]);
    if from == pcre2::UNSET {
        return empty_element(kind);
    }
    let bytes = &subject[from..to];
    match kind {
        ReturnType::Index => Term::tuple(vec![
            Term::Integer(from as i64),
            Term::Integer((to - from) as i64),
        ]),
        ReturnType::List => Term::list_from(bytes.iter().map(|&byte| Term::Integer(byte.into()))),
        ReturnType::Binary => Term::binary(bytes.to_vec()),
    }
}

/// The loops of one reduction (`LOOP_FACTOR`, `erl_bif_re.c:42`). A match
/// charges one reduction for each group of this many loops.
const LOOP_FACTOR: i32 = 16;

/// The largest loop limit of one turn, `CONTEXT_REDS * LOOP_FACTOR`
/// (`max_loop_limit`, `erl_bif_re.c:219`).
const MAX_LOOP_LIMIT: i32 = CONTEXT_REDS * LOOP_FACTOR;

/// Return the loop limit of one turn that starts with `reds` reductions, as
/// `re_run` reads it (`erl_bif_re.c:1639`, `1852`).
///
/// # Examples
///
/// ```rust,ignore
/// assert_eq!(loop_limit(1), 16);
/// assert_eq!(loop_limit(CONTEXT_REDS), 64000);
/// ```
fn loop_limit(reds: i32) -> i32 {
    (reds * LOOP_FACTOR).min(MAX_LOOP_LIMIT)
}

/// Return the number of offset pairs of the match data of a precompiled
/// pattern: its capture count plus one for the whole match, as `re_run`
/// reads `unsigned_val(tp[2]) + 1` into an `int` (`erl_bif_re.c:1607`). The
/// count is a small integer of the 64-bit range, and an `int` keeps the low
/// 32 bits of the sum, so a forged negative count gives 0 as it does in
/// ERTS. `pcre2_match_data_create` then keeps at least one pair and at
/// most `UINT16_MAX` (`pcre2_match_data.c:58-59`).
///
/// # Examples
///
/// ```rust,ignore
/// assert_eq!(ovec_pairs(0), 1);
/// assert_eq!(ovec_pairs(1), 2);
/// assert_eq!(ovec_pairs(-1), 0);
/// ```
fn ovec_pairs(count: i64) -> u32 {
    (count as u64).wrapping_add(1) as u32
}

/// Run `re:run(Subject, RE, Options)`, or
/// `re:internal_run(Subject, RE, Options, First)` with `first` false, as
/// `re_run` (`erl_bif_re.c:1485-1828`) does:
///
/// 1. Options that `parse_options` rejects, and an option that sets
///    `PARSE_FLAG_EXPORT`, raise `badarg` (`erl_bif_re.c:1501-1504`).
/// 2. With `first` false the match sets `PCRE2_NO_UTF_CHECK`, because
///    `re:grun/3` ran the subject through `re:run/3` already
///    (`erl_bif_re.c:1505-1512`).
/// 3. A pattern that is a 5-tuple is a precompiled pattern: a compile
///    option raises `badarg` (`erl_bif_re.c:1581-1583`), so does a term
///    that is not `{re_pattern, Count, Unicode, Crlf, Ref}` of the compiled
///    expressions of the node (`erl_bif_re.c:1586-1590`), and so does a
///    `{newline, X}` or `bsr` option that differs from the compiled code
///    (`erl_bif_re.c:1609-1632`). Its ovector pairs are `Count + 1`.
/// 4. Else the pattern is text: a term that is not a binary, a list, or
///    `[]` raises `badarg` (`erl_bif_re.c:1516-1518`), and a pattern that
///    PCRE2 does not compile raises `badarg`, or gives
///    `{error, {compile, {Message, Offset}}}` with `report_errors`
///    (`erl_bif_re.c:1546-1556`). Its ovector pairs are the capture count
///    of the code plus one (`erl_bif_re.c:1574-1575`).
/// 5. With `unicode`, a pattern or a subject that is not a binary, and a
///    list capture that is not `global`, trap to `re:urun/3` with the same
///    arguments (`erl_bif_re.c:1530-1534`, `1592-1597`). With `global`, the
///    call traps to `re:grun/3` with the pattern and the options
///    (`erl_bif_re.c:1557-1572`, `1599-1605`).
/// 6. A subject that is not iodata raises `badarg`
///    (`erl_bif_re.c:1691-1701`), and so does a match that PCRE2 stops for
///    a bad offset, a bad UTF-8 subject, or a bad code
///    (`erl_bif_re.c:1756-1786`).
/// 7. Else the value is `{match, Captures}`, `match`, `nomatch`,
///    `{error, match_limit}`, or `{error, match_limit_recursion}`
///    (`build_exec_return`, `erl_bif_re.c:1047-1256`), and `badarg` for a
///    capture specification that `build_capture` rejects
///    (`erl_bif_re.c:1646-1651`).
/// 8. The match charges one reduction for each `LOOP_FACTOR` loops, and
///    each turn of a long match has a fresh budget, as the trap to
///    `erlang:re_run_trap/3` gives it (`erl_bif_re.c:1639-1643`,
///    `1718-1748`, `1852-1866`). PBeam runs the turns of one call in one
///    step: it charges the same reductions, but the process switches out
///    only after the call.
///
/// # Examples
///
/// ```rust,ignore
/// // {match, [{1, 1}]}
/// let value = run(&mut context, &Term::binary(b"abc".to_vec()), &Term::binary(b"b".to_vec()), &Term::nil(), true)?;
/// // {match, [[{1, 1}], [{4, 1}]]}
/// let global = Term::list_from(vec![Term::static_atom("global")]);
/// let value = run(&mut context, &Term::binary(b"abcabc".to_vec()), &Term::binary(b"b".to_vec()), &global, true)?;
/// // badarg
/// let value = run(&mut context, &Term::binary(b"a".to_vec()), &Term::binary(b"a".to_vec()), &Term::list_from(vec![Term::static_atom("export")]), true)?;
/// ```
///
/// # Errors
///
/// Return the runtime error of [`unsupported_name`] for a capture that
/// names a group, the runtime error of the scheduler for a run without a
/// new reference, and a runtime error for a PCRE2 result that this code
/// does not expect.
pub(super) fn run(
    context: &mut BifContext<'_>,
    subject: &Term,
    pattern: &Term,
    options: &Term,
    first: bool,
) -> Result<BifResult, BeamError> {
    let Some(mut parsed) = parse_options(&context.scheduler.atoms, options) else {
        return Ok(BifResult::badarg());
    };
    if parsed.flags & FLAG_EXPORT != 0 {
        return Ok(BifResult::badarg());
    }
    if !first {
        parsed.match_options |= pcre2::NO_UTF_CHECK;
    }
    let unicode = parsed.flags & FLAG_UNICODE != 0;
    let global = parsed.flags & FLAG_GLOBAL != 0;
    let list_cap = parsed
        .capture()
        .is_some_and(|(_, kind)| is_atom(kind, "list"));
    let binary_subject = matches!(subject, Term::Binary(_));

    // The code of the match and the number of offset pairs of its ovector:
    // the pairs of a precompiled pattern, or the capture count of a code
    // that this call compiles, plus one (`erl_bif_re.c:1574-1575`, `1607`).
    // A five-element tuple gives the five fields of a precompiled pattern
    // (`erl_bif_re.c:1585-1590`); each other term is a text pattern.
    let pattern_elements: Option<&[Term; 5]> = match pattern {
        Term::Tuple(elements) if elements.len() == 5 => {
            <&[Term; 5]>::try_from(elements.as_ref()).ok()
        }
        _ => None,
    };
    let mut own_code = None;
    let mut stored_code = None;
    let pairs;
    if let Some([tag, count, pattern_unicode, crlf, magic]) = pattern_elements {
        if parsed.flags & FLAG_UNIQUE_COMPILE_OPT != 0 {
            return Ok(BifResult::badarg());
        }
        let Term::Reference(magic) = magic else {
            return Ok(BifResult::badarg());
        };
        let (Some(count), Some(pattern_unicode)) =
            (count.small_integer(), pattern_unicode.small_integer())
        else {
            return Ok(BifResult::badarg());
        };
        if !is_atom(tag, "re_pattern") || crlf.small_integer().is_none() {
            return Ok(BifResult::badarg());
        }
        let ovec_pairs = ovec_pairs(count);
        let codes = context.scheduler.resources.borrow_mut::<RegexCodes>();
        let Some(code) = codes.0.get_mut(magic) else {
            return Ok(BifResult::badarg());
        };
        pairs = ovec_pairs;
        if pattern_unicode != 0 && (!binary_subject || (list_cap && !global)) {
            return Ok(urun_trap(subject, pattern, options));
        }
        if global {
            return Ok(BifResult::Trap {
                module: "re",
                function: "grun",
                arguments: vec![
                    subject.clone(),
                    pattern.clone(),
                    Term::tuple(vec![options.clone(), Term::static_atom("false")]),
                ],
            });
        }
        // A `newline` or `bsr` option must agree with the compiled code
        // (`erl_bif_re.c:1609-1632`).
        if parsed
            .newline
            .is_some_and(|newline| code.newline() != newline)
            || parsed.bsr.is_some_and(|bsr| code.bsr() != bsr)
        {
            return Ok(BifResult::badarg());
        }
        stored_code = Some(*magic);
    } else {
        // The text pattern (`erl_bif_re.c:1516-1577`).
        if !is_text_pattern(pattern) {
            return Ok(BifResult::badarg());
        }
        if unicode
            && (!matches!(pattern, Term::Binary(_)) || !binary_subject || (list_cap && !global))
        {
            return Ok(urun_trap(subject, pattern, options));
        }
        let Some(bytes) = iolist::iodata_bytes(pattern) else {
            return Ok(BifResult::badarg());
        };
        match pcre2::compile(&bytes, parsed.compile, parsed.newline, parsed.bsr) {
            Ok(code) if global => {
                // The pattern becomes a compiled pattern, and the call
                // continues in `re:grun/3` (`erl_bif_re.c:1557-1572`).
                let precompiled = compile_result(context, code, unicode.into())?;
                let clean = match parsed.flags & FLAG_UNIQUE_COMPILE_OPT != 0 {
                    true => "true",
                    false => "false",
                };
                return Ok(BifResult::Trap {
                    module: "re",
                    function: "grun",
                    arguments: vec![
                        subject.clone(),
                        precompiled,
                        Term::tuple(vec![
                            options.clone(),
                            Term::static_atom(clean),
                            pattern.clone(),
                        ]),
                    ],
                });
            }
            Ok(code) => {
                pairs = code.capture_count() + 1;
                own_code = Some(code);
            }
            Err(failure) if parsed.flags & FLAG_REPORT_ERRORS != 0 => {
                return Ok(Value(compile_error(&failure, Some("compile"))));
            }
            Err(_) => return Ok(BifResult::badarg()),
        }
    }

    // The subject bytes (`erl_bif_re.c:1667-1706`).
    let Some(subject_bytes) = iolist::iodata_bytes(subject) else {
        return Ok(BifResult::badarg());
    };
    let mut data = pcre2::MatchData::new(pairs);
    data.clear_restart();
    let mut limit = loop_limit(
        i32::try_from(
            context
                .scheduler
                .current_process_mut()
                .bif_reductions_left()?,
        )
        .expect("a reduction budget fits in i32"),
    );
    data.set_loops_left(limit);
    let match_context =
        (parsed.flags & (FLAG_MATCH_LIMIT | FLAG_MATCH_LIMIT_RECURSION) != 0).then(|| {
            pcre2::MatchContext::new(
                (parsed.flags & FLAG_MATCH_LIMIT != 0).then_some(parsed.match_limit),
                (parsed.flags & FLAG_MATCH_LIMIT_RECURSION != 0)
                    .then_some(parsed.match_limit_recursion),
            )
        });
    let start = parsed.startoffset as usize;

    // The match and the capture specification, which reads the ovector of
    // the match. A precompiled pattern gives a borrow of the resources of
    // the node, which ends before the reductions below change the process.
    let (capture, mut rc) = {
        let borrow;
        let code = match &own_code {
            Some(code) => code,
            None => {
                borrow = context.scheduler.resources.borrow_mut::<RegexCodes>();
                borrow
                    .0
                    .get_mut(stored_code.as_ref().expect("a precompiled pattern"))
                    .expect("the reference holds a compiled expression")
            }
        };
        let Some(capture) = build_capture(code, parsed.capture())? else {
            return Ok(BifResult::badarg());
        };
        let rc = pcre2::execute(
            code,
            &data,
            match_context.as_ref(),
            &subject_bytes,
            start,
            parsed.match_options,
        );
        (capture, rc)
    };

    // A match that uses its loop limit continues with a fresh budget, as
    // the trap to `erlang:re_run_trap/3` does (`erl_bif_re.c:1852-1858`).
    let mut restarted = false;
    while rc == pcre2::ERROR_LOOP_LIMIT {
        let used = (limit - data.loops_left()) / LOOP_FACTOR;
        context
            .scheduler
            .current_process_mut()
            .bump_reductions(used);
        // ERTS switches out at the trap and adds the whole turn to the
        // process total (`erl_process.c:9678-9700`). PBeam runs the turns
        // in one call, so it adds each ended turn here and starts a fresh
        // budget.
        context.scheduler.account_turn();
        limit = loop_limit(CONTEXT_REDS);
        data.set_loops_left(limit);
        data.clear_restart();
        rc = data.resume();
        restarted = true;
    }
    // A bad UTF-8 subject uses the whole budget of the turn and raises
    // `badarg` (`erl_bif_re.c:1756-1786`); a bad offset, code, or mode
    // raises it with no charge at all. The trap handles these codes the
    // same way (`erl_bif_re.c:1872-1902`).
    if pcre2::is_utf8_error(rc) || rc == pcre2::ERROR_BADUTFOFFSET {
        context
            .scheduler
            .current_process_mut()
            .bump_all_reductions();
        return Ok(BifResult::badarg());
    }
    if matches!(
        rc,
        pcre2::ERROR_BADOFFSET | pcre2::ERROR_BADMAGIC | pcre2::ERROR_BADMODE
    ) {
        return Ok(BifResult::badarg());
    }
    // The first match lists `HEAPLIMIT` and `RECURSELOOP` also
    // (`erl_bif_re.c:1720-1753`); the switch of `re_match_trap` lists only
    // `NOMATCH`, `MATCHLIMIT`, and `RECURSIONLIMIT`
    // (`erl_bif_re.c:1860-1866`). Each other negative code raises
    // `internal_error`, and ERTS ends the runtime (`erl_bif_re.c:1788-1793`,
    // `1903-1905`). ERTS asserts that PCRE2 does not return one for a live
    // code and a subject that this call checked, but a build can.
    let expected = matches!(
        rc,
        pcre2::ERROR_NOMATCH | pcre2::ERROR_MATCHLIMIT | pcre2::ERROR_DEPTHLIMIT
    ) || (!restarted
        && matches!(rc, pcre2::ERROR_HEAPLIMIT | pcre2::ERROR_RECURSELOOP));
    if rc < 0 && !expected {
        return Ok(internal_error());
    }
    // A pattern that recursed without a match charges all reductions of
    // one turn and gives `nomatch` (`erl_bif_re.c:1751-1754`). A restart
    // cannot give this code, because the trap gives `internal_error`.
    let used = if rc == pcre2::ERROR_RECURSELOOP {
        CONTEXT_REDS
    } else {
        (limit - data.loops_left()) / LOOP_FACTOR
    };
    context
        .scheduler
        .current_process_mut()
        .bump_reductions(used);
    Ok(Value(exec_return(
        &data,
        rc,
        &capture,
        parsed.flags,
        &subject_bytes,
    )))
}

/// Run `re:internal_run(Subject, RE, Options, First)` as `re_internal_run_4`
/// (`erl_bif_re.c:1805-1816`) does: the fourth argument must be the atom
/// `true` or `false`, and each other term raises `badarg`. `re:grun/3` calls
/// this function with `false`, because it ran the subject through
/// `re:run/3` already.
///
/// # Examples
///
/// ```rust,ignore
/// // {match, [{1, 1}]}
/// let value = internal_run(&mut context, &Term::binary(b"abc".to_vec()), &Term::binary(b"b".to_vec()), &Term::nil(), &Term::static_atom("false"))?;
/// // badarg
/// let value = internal_run(&mut context, &Term::binary(b"a".to_vec()), &Term::binary(b"a".to_vec()), &Term::nil(), &Term::Integer(1))?;
/// ```
pub(super) fn internal_run(
    context: &mut BifContext<'_>,
    subject: &Term,
    pattern: &Term,
    options: &Term,
    first: &Term,
) -> Result<BifResult, BeamError> {
    if is_atom(first, "true") {
        run(context, subject, pattern, options, true)
    } else if is_atom(first, "false") {
        run(context, subject, pattern, options, false)
    } else {
        Ok(BifResult::badarg())
    }
}

/// Return the trap of `re:run/2,3` to `re:urun/3` with the same arguments,
/// as `re_run` traps for a `unicode` pattern or subject that needs the
/// UTF-8 conversion of the Erlang function (`erl_bif_re.c:1530-1534`,
/// `1592-1597`).
///
/// # Examples
///
/// ```rust,ignore
/// // re:urun(<<"a">>, <<"a" utf8 list>>, [unicode])
/// let trap = urun_trap(&Term::binary(b"a".to_vec()), &Term::binary(b"a".to_vec()), &unicode);
/// ```
fn urun_trap(subject: &Term, pattern: &Term, options: &Term) -> BifResult {
    Trap {
        module: "re",
        function: "urun",
        arguments: vec![subject.clone(), pattern.clone(), options.clone()],
    }
}

/// The title of the header of an exported pattern
/// (`erl_bif_re.c:687`).
const EXPORT_TITLE: &[u8; 8] = b"re-PCRE2";
/// The encode version of the header of an exported pattern,
/// `EXPORTED_ENCODE_VERSION` (`erl_bif_re.c:633`).
const EXPORT_VERSION: u8 = 1;
/// The size of the header of an exported pattern, `EXPORTED_HDR_SZ`
/// (`erl_bif_re.c:617-626`).
const EXPORT_HEADER_SIZE: usize = 14;
/// The offset of the checksum in that header, `EXPORTED_HDR_CHECKSUM_OFFS`
/// (`erl_bif_re.c:623`).
const EXPORT_CHECKSUM_OFFSET: usize = 8;
/// The offset of the encode version in that header,
/// `EXPORTED_HDR_ENCODE_VER_OFFS` (`erl_bif_re.c:624`).
const EXPORT_VERSION_OFFSET: usize = 12;
/// The offset of the unicode byte in that header,
/// `EXPORTED_HDR_UNICODE_OFFS` (`erl_bif_re.c:625`).
const EXPORT_UNICODE_OFFSET: usize = 13;

/// Return `{ok, {re_exported_pattern, Header, Source, Options, Encoded}}`,
/// as `build_compile_export` (`erl_bif_re.c:644-700`) makes it:
///
/// - `Header` is 14 bytes (`erl_bif_re.c:617-627`): the title `re-PCRE2`,
///   the zlib CRC-32 of `Encoded` as a big-endian 32-bit value, the encode
///   version 1, and the `unicode` byte.
/// - `Source` is the pattern bytes as a binary (`get_iolist_as_bytes`,
///   `erl_bif_re.c:850-889`).
/// - `Options` is the option term of the call, as it is.
/// - `Encoded` is the `pcre2_serialize_encode` form of the code.
///
/// # Examples
///
/// ```rust,ignore
/// let code = pcre2::compile(b"a", 0, None, None)?;
/// let options = Term::list_from(vec![Term::static_atom("export")]);
/// // {ok, {re_exported_pattern, <<"re-PCRE2",163,240,192,168,1,0>>,
/// //       <<"a">>, [export], Encoded}} on aarch64 macOS, as ERTS
/// let result = export(&code, false, b"a".to_vec(), &options);
/// ```
fn export(code: &pcre2::Code, unicode: bool, source: Vec<u8>, options: &Term) -> Term {
    let encoded = code.serialize();
    let mut header = EXPORT_TITLE.to_vec();
    header.extend_from_slice(&crc32(&encoded).to_be_bytes());
    header.push(EXPORT_VERSION);
    header.push(unicode.into());
    Term::tuple(vec![
        Term::static_atom("ok"),
        Term::tuple(vec![
            Term::static_atom("re_exported_pattern"),
            Term::binary(header),
            Term::binary(source),
            options.clone(),
            Term::binary(encoded),
        ]),
    ])
}

/// Return the zlib CRC-32 of `bytes`, as `calc_checksum`
/// (`erl_bif_re.c:635-639`) gets it with `crc32(0, bytes, size)`: the
/// reflected polynomial `0xEDB88320`, the initial value and the final XOR
/// `0xFFFFFFFF`.
///
/// # Examples
///
/// ```rust,ignore
/// assert_eq!(crc32(b"123456789"), 0xCBF4_3926);
/// assert_eq!(crc32(b""), 0);
/// ```
fn crc32(bytes: &[u8]) -> u32 {
    !bytes.iter().fold(u32::MAX, |crc, &byte| {
        (0..8).fold(crc ^ u32::from(byte), |crc, _| {
            (crc >> 1) ^ (0xEDB8_8320 & (crc & 1).wrapping_neg())
        })
    })
}

#[cfg(test)]
#[path = "../../../../tests/runtime/interpreter/bif/re.rs"]
mod tests;

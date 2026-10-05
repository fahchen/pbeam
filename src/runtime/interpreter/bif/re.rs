//! Bodies of the `re` BIFs that do not fit on one table line.

use super::BifResult::{Trap, Value};
use super::*;
use crate::runtime::pcre2;

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

/// The compile options of one `re:compile/2` call, as the fields of
/// `struct parsed_options` (`erl_bif_re.c:337-348`) that it uses.
#[derive(Debug, Default)]
struct CompileOptions {
    /// The `PCRE2_*` compile bits.
    compile: u32,
    /// The last `{newline, X}` value, if any.
    newline: Option<u32>,
    /// The last `bsr_anycrlf` or `bsr_unicode` value, if any.
    bsr: Option<u32>,
    /// Whether the `export` option is present.
    export: bool,
}

/// Read the options of `re:compile/2`, or return `None` when `re_compile`
/// raises `badarg` for them.
///
/// `parse_options` (`erl_bif_re.c:350-545`) fails for a term that is not a
/// proper list, an unknown atom, a tuple that is not `{Atom, Value}` or
/// `{capture, V, T}`, an unknown tuple key, and an unknown `{newline, X}`
/// value. It accepts the execution options `{capture, V}`,
/// `{capture, V, T}`, `{offset, N}`, `{match_limit, N}`,
/// `{match_limit_recursion, N}`, `notempty`, `notempty_atstart`, `notbol`,
/// `noteol`, `report_errors`, and `global`, but each one makes `re_compile`
/// raise `badarg` (`erl_bif_re.c:914-916`). So this function rejects them
/// at once, with the same result. A later `newline` or `bsr` option replaces
/// an earlier one.
///
/// # Examples
///
/// ```rust,ignore
/// let options = parse_options(&Term::List(vec![Term::static_atom("unicode")])).unwrap();
/// assert_eq!(options.compile, pcre2::UTF);
/// assert!(parse_options(&Term::List(vec![Term::static_atom("global")])).is_none());
/// assert!(parse_options(&Term::static_atom("foo")).is_none());
/// ```
fn parse_options(options: &Term) -> Option<CompileOptions> {
    let Term::List(options) = options else {
        return None;
    };
    let mut parsed = CompileOptions::default();
    for option in options {
        match option {
            Term::Atom(atom) => match atom.name() {
                "bsr_anycrlf" => parsed.bsr = Some(pcre2::BSR_ANYCRLF),
                "bsr_unicode" => parsed.bsr = Some(pcre2::BSR_UNICODE),
                "export" => parsed.export = true,
                name => parsed.compile |= lookup(&COMPILE_OPTIONS, name)?,
            },
            Term::Tuple(elements) => match elements.as_slice() {
                [Term::Atom(key), Term::Atom(value)] if key.name() == "newline" => {
                    parsed.newline = Some(lookup(&NEWLINES, value.name())?);
                }
                _ => return None,
            },
            _ => return None,
        }
    }
    Some(parsed)
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
/// 5. Else the result is `{ok, {re_pattern, CaptureCount, Unicode, UseCrlf,
///    Ref}}` (`build_compile_result`, `erl_bif_re.c:581-615`). `Unicode` is
///    1 for the `unicode` option. `UseCrlf` is 1 when the newline
///    convention of the code is `any`, `crlf`, or `anycrlf`, also from an
///    item such as `(*CRLF)` in the pattern. `Ref` is a new reference, and
///    the compiled code is the resource of that reference.
///
/// # Examples
///
/// ```rust,ignore
/// let unicode = Term::List(vec![Term::static_atom("unicode")]);
/// let result = compile(&mut context, &Term::Binary(b"(a)".to_vec()), &unicode)?;
/// // {ok, {re_pattern, 1, 1, 0, Ref}}
/// assert!(matches!(result, BifResult::Value(Term::Tuple(_))));
/// let result = compile(&mut context, &Term::List(vec![Term::Integer(256)]), &unicode)?;
/// assert!(matches!(result, BifResult::Trap { function: "ucompile", .. }));
/// let result = compile(&mut context, &Term::static_atom("foo"), &Term::List(vec![]))?;
/// assert!(matches!(result, BifResult::Raise { .. }));
/// ```
///
/// # Errors
///
/// Return a runtime error for the `export` option of a pattern that
/// compiles, because PBeam does not make exported patterns yet.
pub(super) fn compile(
    context: &mut BifContext<'_>,
    pattern: &Term,
    options: &Term,
) -> Result<BifResult, BeamError> {
    let Some(parsed) = parse_options(options) else {
        return Ok(BifResult::badarg());
    };
    let unicode = parsed.compile & pcre2::UTF != 0;
    if unicode && !matches!(pattern, Term::Binary(_)) {
        return Ok(Trap {
            module: "re",
            function: "ucompile",
            arguments: vec![pattern.clone(), options.clone()],
        });
    }
    let Some(bytes) = erlang::iodata_bytes(pattern) else {
        return Ok(BifResult::badarg());
    };
    let code = match pcre2::compile(&bytes, parsed.compile, parsed.newline, parsed.bsr) {
        Ok(code) => code,
        Err(failure) => {
            let message = failure
                .message
                .into_iter()
                .map(|byte| Term::Integer(byte.into()));
            let offset = i64::try_from(failure.offset).expect("a pattern offset fits in i64");
            return Ok(Value(Term::Tuple(vec![
                Term::static_atom("error"),
                Term::Tuple(vec![Term::List(message.collect()), Term::Integer(offset)]),
            ])));
        }
    };
    if parsed.export {
        // ponytail: `build_compile_export` (erl_bif_re.c:644-700) comes in the next commit of #113.
        return Err(error("unsupported re:compile/2 option export"));
    }
    let use_crlf = matches!(
        code.newline(),
        pcre2::NEWLINE_ANY | pcre2::NEWLINE_CRLF | pcre2::NEWLINE_ANYCRLF
    );
    let capture_count = code.capture_count();
    let reference = context.scheduler.make_ref()?;
    context
        .scheduler
        .resources
        .insert_regex_code(reference, code);
    Ok(Value(Term::Tuple(vec![
        Term::static_atom("ok"),
        Term::Tuple(vec![
            Term::static_atom("re_pattern"),
            Term::Integer(capture_count.into()),
            Term::Integer(unicode.into()),
            Term::Integer(use_crlf.into()),
            Term::Reference(reference),
        ]),
    ])))
}

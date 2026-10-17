//! Bodies of the float text BIFs `erlang:float_to_list/1,2`,
//! `erlang:float_to_binary/1,2`, and `erlang:list_to_float/1`, as ERTS
//! `do_float_to_charbuf` (`bif.c:3378-3445`) and `list_to_float_1`
//! (`bif.c:3699-3725`) give them.

use super::BifResult::Value;
use super::*;
use crate::term::float_text;

/// The default number of digits after the point of `float_to_list/1`,
/// `SYS_DEFAULT_FLOAT_DECIMALS` (`sys.h:890`).
const DEFAULT_DECIMALS: i32 = 20;

/// Return the bytes of a proper list of small integers from 0 to 255, as
/// ERTS reads a character list with `erts_list_length` (`utils.c:344-357`)
/// and `intlist_to_buf` (`utils.c:2828-2850`), or `None` for each other
/// term. `intlist_to_buf` also gives -1 for a byte that is not 0 to 255
/// (`is_byte`), and `erts_list_length` gives -1 for a list that is not
/// proper and for a term that is not a list.
///
/// # Examples
///
/// ```rust,ignore
/// assert_eq!(byte_text(&Term::list_from(b"1.0".iter().map(|&b| Term::Integer(b.into())))), Some(b"1.0".to_vec()));
/// assert_eq!(byte_text(&Term::list_from([Term::Integer(256)])), None);
/// assert_eq!(byte_text(&Term::static_atom("abc")), None);
/// ```
fn byte_text(list: &Term) -> Option<Vec<u8>> {
    let mut text = Vec::new();
    for element in list.proper_list()? {
        match element.small_integer() {
            Some(byte @ 0..=255) => text.push(byte as u8),
            _ => return None,
        }
    }
    Some(text)
}

/// Return the index after the digits of `text` at `index`, or `None` when
/// the text holds no digit there.
///
/// # Examples
///
/// ```rust,ignore
/// assert_eq!(skip_digits(b"10.0", 0), Some(2));
/// assert_eq!(skip_digits(b"10.0", 2), None);
/// ```
fn skip_digits(text: &[u8], index: usize) -> Option<usize> {
    let start = index;
    let mut index = index;
    while matches!(text.get(index), Some(b'0'..=b'9')) {
        index += 1;
    }
    (index > start).then_some(index)
}

/// Return the float that the byte text reads as, or `None` where ERTS
/// `sys_chars_to_double` (`sys_float.c:72-140`) gives -1.
///
/// The scan of that function reads an optional `+` or `-`, one or more
/// digits, a point or a comma, one or more digits, an optional `e` or `E`
/// with an optional sign and one or more digits, and then the end of the
/// text. ERTS converts the text with the `strtod` of the host C library,
/// first with the radix character of the current locale, and, when
/// `strtod` stops before the end, again with the other radix character in
/// place of the point (`sys_float.c:109-124`). ERTS rejects a result that
/// is not finite after each attempt (`sys_float.c:111-113` and
/// `sys_float.c:121-123`), so in the C locale the first attempt of a text
/// with a comma reads the sign and the digits before the comma only. A
/// prefix that overflows an infinity, such as 309 nines, then raises
/// `badarg` before the second attempt, whatever the full text is worth. PBeam reads the text with the Rust float parse instead, which
/// rounds to the nearest double as `strtod` does, and it reads a comma as a
/// point, which gives the ERTS result in each locale:
/// `list_to_float("1,5")` gives `1.5` and `list_to_float("1.5")` gives
/// `1.5`. A text that overflows an infinity raises `badarg`
/// (`sys_float.c:111-113`); `list_to_float("1.0e-400")` gives `0.0`, as the
/// branch of `sys_float.c:130-132` gives it.
///
/// `list_to_float_1` gives the function a C string, so a zero byte ends
/// the text and the bytes after it do not count:
/// `list_to_float([49, 46, 48, 0, 120])` gives `1.0`, and
/// `list_to_float([49, 0, 46, 48])` raises `badarg` for the text `"1"`.
///
/// # Examples
///
/// ```rust,ignore
/// assert_eq!(text_to_float(b"1.0"), Some(1.0));
/// assert_eq!(text_to_float(b"1,5"), Some(1.5));
/// assert_eq!(text_to_float(b"1"), None);
/// assert_eq!(text_to_float(b"1.5e400"), None);
/// assert_eq!(text_to_float(b"1.0\x00x"), Some(1.0));
/// let nines = "9".repeat(309);
/// assert_eq!(text_to_float(format!("{nines},5e-309").as_bytes()), None);
/// assert_eq!(text_to_float(format!("{},5e-308", &nines[..308]).as_bytes()), Some(1.0));
/// ```
fn text_to_float(text: &[u8]) -> Option<f64> {
    let end = text
        .iter()
        .position(|byte| *byte == 0)
        .unwrap_or(text.len());
    let text = &text[..end];
    let mut index = usize::from(matches!(text.first(), Some(b'+' | b'-')));
    index = skip_digits(text, index)?;
    let radix = match text.get(index) {
        Some(b'.' | b',') => index,
        _ => return None,
    };
    if text[radix] == b',' {
        // The first `strtod` call of ERTS runs with the radix character of
        // the current locale, so in the C locale it reads the sign and the
        // digits before the comma only, and ERTS rejects a result that is
        // not finite before the second attempt (`sys_float.c:109-124`).
        let prefix: f64 = std::str::from_utf8(&text[..radix]).ok()?.parse().ok()?;
        if !prefix.is_finite() {
            return None;
        }
    }
    index = skip_digits(text, index + 1)?;
    if matches!(text.get(index), Some(b'e' | b'E')) {
        let sign = usize::from(matches!(text.get(index + 1), Some(b'+' | b'-')));
        index = skip_digits(text, index + 1 + sign)?;
    }
    if index != text.len() {
        return None;
    }
    let mut text = text.to_vec();
    text[radix] = b'.';
    let value = std::str::from_utf8(&text).ok()?.parse::<f64>().ok()?;
    value.is_finite().then_some(value)
}

/// Run `erlang:list_to_float/1`: return the float that the byte list
/// `list` reads as, as ERTS `list_to_float_1` (`bif.c:3699-3725`) gives it.
/// A term that is not a proper list of bytes, and a text that the scan of
/// [`text_to_float`] rejects or that overflows an infinity, raise `badarg`
/// (`bif.c:3705-3723`).
///
/// # Examples
///
/// ```rust,ignore
/// let one = Term::list_from(b"1.0".iter().map(|&b| Term::Integer(b.into())));
/// assert!(matches!(from_list(&one), BifResult::Value(Term::Float(value)) if value == 1.0));
/// assert!(matches!(from_list(&Term::nil()), BifResult::Raise { .. }));
/// ```
pub(super) fn from_list(list: &Term) -> BifResult {
    match byte_text(list).and_then(|text| text_to_float(&text)) {
        Some(value) => Value(Term::Float(value)),
        None => BifResult::badarg(),
    }
}

/// Run `erlang:binary_to_float/1`: return the float that the bytes of a
/// binary read as, as ERTS `binary_to_float_1` (`bif.c:3727-3756`) gives it
/// with `do_charbuf_to_float` (`bif.c:3684-3697`). It gives the bytes a NUL
/// terminator, so a zero byte ends the text and the bytes after it do not
/// count: `binary_to_float(<<49,46,48,0,120>>)` gives `1.0`. A term that is
/// not a binary, an empty binary, a bit string that is not a whole number
/// of bytes, a text that the scan of [`text_to_float`] rejects, and a text
/// that overflows an infinity raise `badarg` (`bif.c:3729-3755`).
///
/// # Examples
///
/// ```rust,ignore
/// let one = Term::binary(b"1.0".to_vec());
/// assert!(matches!(from_binary(&one), BifResult::Value(Term::Float(value)) if value == 1.0));
/// assert!(matches!(from_binary(&Term::binary(Vec::new())), BifResult::Raise { .. }));
/// assert!(matches!(from_binary(&Term::Integer(1)), BifResult::Raise { .. }));
/// ```
pub(super) fn from_binary(binary: &Term) -> BifResult {
    let Term::Binary(bytes) = binary else {
        return BifResult::badarg();
    };
    if bytes.bit_size() == 0 || !bytes.bit_size().is_multiple_of(8) {
        return BifResult::badarg();
    }
    match text_to_float(bytes) {
        Some(value) => Value(Term::Float(value)),
        None => BifResult::badarg(),
    }
}

/// The text form that the options of `float_to_list/2` select.
enum Format {
    /// No `short`, `decimals`, or `scientific` option: `%.20e`.
    Legacy,
    Short,
    Fixed,
    Scientific,
}

/// Return the text of `float` with `options`, as `do_float_to_charbuf`
/// (`bif.c:3378-3445`) writes it, or `None` where ERTS raises `badarg`.
/// The options are read in order, and a later form option replaces an
/// earlier one. `compact` applies only to `{decimals, D}`. `D` is a small
/// integer that ERTS stores in a C `int`, so only its low 32 bits count:
/// `{decimals, 4294967297}` is `{decimals, 1}`. `float` must be a float,
/// each option must be `short`, `compact`, `{decimals, D}`, or
/// `{scientific, D}`, and `options` must be a proper list.
///
/// # Examples
///
/// ```rust,ignore
/// assert_eq!(float_text_of(&Term::Float(1.5), &Term::nil()), Some(b"1.50000000000000000000e+00".to_vec()));
/// let short = Term::list_from([Term::static_atom("short")]);
/// assert_eq!(float_text_of(&Term::Float(0.1), &short), Some(b"0.1".to_vec()));
/// assert_eq!(float_text_of(&Term::Integer(1), &Term::nil()), None);
/// ```
fn float_text_of(float: &Term, options: &Term) -> Option<Vec<u8>> {
    let Term::Float(value) = float else {
        return None;
    };
    let mut decimals = DEFAULT_DECIMALS;
    let mut compact = false;
    let mut format = Format::Legacy;
    for option in options.proper_list()? {
        match option {
            Term::Atom(atom) if atom.is_static("compact") => compact = true,
            Term::Atom(atom) if atom.is_static("short") => format = Format::Short,
            Term::Tuple(elements) if elements.len() == 2 => {
                let count = elements[1].small_integer()?;
                // The C assignment to `int` keeps the low 32 bits.
                decimals = count as i32;
                format = match &elements[0] {
                    Term::Atom(atom) if atom.is_static("decimals") => Format::Fixed,
                    Term::Atom(atom) if atom.is_static("scientific") => Format::Scientific,
                    _ => return None,
                };
            }
            _ => return None,
        }
    }
    match format {
        Format::Short => Some(float_text::shortest(*value)),
        Format::Fixed => float_text::fixed(*value, decimals, compact),
        Format::Legacy | Format::Scientific => float_text::scientific_option(*value, decimals),
    }
}

/// Run `erlang:float_to_list/1,2`: return the text of a float as a list of
/// bytes. A float that is not a float, or an option that is not valid,
/// raises `badarg` (`bif.c:3449-3470`).
///
/// # Examples
///
/// ```rust,ignore
/// let text = to_list(&Term::Float(0.5), &Term::list_from([Term::static_atom("short")]));
/// assert!(matches!(text, BifResult::Value(list) if list == Term::list_from(b"0.5".iter().map(|&b| Term::Integer(b.into())))));
/// ```
pub(super) fn to_list(float: &Term, options: &Term) -> BifResult {
    match float_text_of(float, options) {
        Some(text) => Value(Term::list_from(
            text.into_iter().map(|byte| Term::Integer(byte.into())),
        )),
        None => BifResult::badarg(),
    }
}

/// Run `erlang:float_to_binary/1,2`: return the text of a float as a
/// binary, with the errors of [`to_list`] (`bif.c:3474-3494`).
///
/// # Examples
///
/// ```rust,ignore
/// let text = to_binary(&Term::Float(0.5), &Term::list_from([Term::static_atom("short")]));
/// assert!(matches!(text, BifResult::Value(binary) if binary == Term::binary(b"0.5".to_vec())));
/// ```
pub(super) fn to_binary(float: &Term, options: &Term) -> BifResult {
    match float_text_of(float, options) {
        Some(text) => Value(Term::binary(text)),
        None => BifResult::badarg(),
    }
}

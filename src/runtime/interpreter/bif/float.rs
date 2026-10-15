//! Bodies of the float text BIFs `erlang:float_to_list/1,2` and
//! `erlang:float_to_binary/1,2`, as ERTS `do_float_to_charbuf`
//! (`bif.c:3378-3445`) gives them.

use super::BifResult::Value;
use super::*;
use crate::term::float_text;

/// The default number of digits after the point of `float_to_list/1`,
/// `SYS_DEFAULT_FLOAT_DECIMALS` (`sys.h:890`).
const DEFAULT_DECIMALS: i32 = 20;

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

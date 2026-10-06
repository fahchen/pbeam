//! Bodies of the `unicode` BIFs that do not fit on one table line.

use super::BifResult::Value;
use super::*;

/// Run `unicode:characters_to_binary/2`: convert `chardata` to one UTF-8
/// binary. Only the `unicode` and `utf8` encodings are supported. As in ERTS
/// (`erl_unicode.c`), they give the same result. ERTS checks the encoding
/// first, and then raises `badarg` for a term that is not chardata
/// (`erl_unicode.c:948-965`).
///
/// # Examples
///
/// ```rust,ignore
/// let chardata = Term::List(vec![Term::Integer(104), Term::Integer(105)]);
/// let result = characters_to_binary(&chardata, &Term::static_atom("unicode"))?;
/// assert!(matches!(result, BifResult::Value(Term::Binary(bytes)) if bytes == b"hi"));
/// let foo = Term::List(vec![Term::static_atom("foo")]);
/// assert!(matches!(characters_to_binary(&foo, &Term::static_atom("unicode"))?, BifResult::Raise { .. }));
/// assert!(characters_to_binary(&chardata, &Term::static_atom("latin1")).is_err());
/// ```
pub(super) fn characters_to_binary(
    chardata: &Term,
    encoding: &Term,
) -> Result<BifResult, BeamError> {
    match encoding {
        Term::Atom(encoding) if matches!(encoding.name(), "unicode" | "utf8") => {
            Ok(characters_to_utf8(chardata)?
                .map_or_else(BifResult::badarg, |bytes| Value(Term::Binary(bytes))))
        }
        // ponytail: ERTS also converts `latin1` and other encodings; see "Chardata conversion" in docs/erts-differences.md.
        _ => Err(error(format!(
            "unsupported unicode:characters_to_binary/2 encoding {encoding:?}"
        ))),
    }
}

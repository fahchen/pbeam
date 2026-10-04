//! Bodies of the `unicode` BIFs that do not fit on one table line.

use super::BifResult::Value;
use super::*;

/// Run `unicode:characters_to_binary/2`: convert `chardata` to one UTF-8
/// binary. Only the `unicode` encoding is supported.
///
/// # Examples
///
/// ```rust,ignore
/// let chardata = Term::List(vec![Term::Integer(104), Term::Integer(105)]);
/// let result = characters_to_binary(&chardata, &Term::static_atom("unicode"))?;
/// assert!(matches!(result, BifResult::Value(Term::Binary(bytes)) if bytes == b"hi"));
/// assert!(characters_to_binary(&chardata, &Term::static_atom("latin1")).is_err());
/// ```
pub(super) fn characters_to_binary(
    chardata: &Term,
    encoding: &Term,
) -> Result<BifResult, BeamError> {
    match encoding {
        Term::Atom(encoding) if encoding.name() == "unicode" => {
            Ok(Value(Term::Binary(characters_to_utf8(chardata)?)))
        }
        // ponytail: ERTS also converts `utf8`, `latin1`, and other encodings; see "Chardata conversion" in docs/erts-differences.md.
        _ => Err(error(format!(
            "unsupported unicode:characters_to_binary/2 encoding {encoding:?}"
        ))),
    }
}

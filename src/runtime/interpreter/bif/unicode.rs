//! Bodies of the `unicode` BIFs that do not fit on one table line.

use super::BifResult::{Trap, Value};
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

/// Run `unicode:characters_to_list/2`: convert `chardata` to a list of code
/// points, as `unicode_characters_to_list_2` does (`erl_unicode.c:1122-1169`).
/// The `latin1` encoding reads each byte of a binary as one code point, and
/// `unicode` and `utf8` read binaries as UTF-8. Invalid input gives
/// `{error, Converted, Rest}`, an incomplete UTF-8 sequence at the end gives
/// `{incomplete, Converted, Rest}`, and a term that is not chardata raises
/// `badarg`. Each other encoding traps to `unicode:characters_to_list_int/2`,
/// as `BIF_TRAP2` does in ERTS (`erl_unicode.c:1135-1143`).
///
/// # Examples
///
/// ```rust,ignore
/// let chardata = Term::Binary(b"hi".to_vec());
/// let result = characters_to_list(&chardata, &Term::static_atom("utf8"));
/// assert!(matches!(result, BifResult::Value(Term::List(list)) if list == [Term::Integer(104), Term::Integer(105)]));
/// let foo = Term::static_atom("foo");
/// assert!(matches!(characters_to_list(&foo, &Term::static_atom("latin1")), BifResult::Raise { .. }));
/// let result = characters_to_list(&chardata, &Term::static_atom("utf16"));
/// assert!(matches!(result, BifResult::Trap { function: "characters_to_list_int", .. }));
/// ```
pub(super) fn characters_to_list(chardata: &Term, encoding: &Term) -> BifResult {
    let latin1 = match encoding {
        Term::Atom(encoding) if encoding.name() == "latin1" => true,
        Term::Atom(encoding) if matches!(encoding.name(), "unicode" | "utf8") => false,
        _ => {
            return Trap {
                module: "unicode",
                function: "characters_to_list_int",
                arguments: vec![chardata.clone(), encoding.clone()],
            };
        }
    };
    characters_to_code_points(chardata, latin1).map_or_else(BifResult::badarg, Value)
}

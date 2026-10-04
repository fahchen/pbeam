//! Convert chardata to UTF-8, as the ERTS BIF `unicode:characters_to_binary/2`
//! does with the `unicode` encoding (`erl_unicode.c`).

use crate::{
    error::{BeamError, error},
    term::Term,
};

/// Convert chardata to its UTF-8 bytes. Chardata is a binary, or a list of
/// integers, binaries, and nested lists of the same. An integer is one
/// Unicode code point. The bytes of each run of adjacent binaries, in list
/// order, must be one valid UTF-8 sequence, so a code point can start in
/// one binary and end in the next one, as in ERTS. An integer between two
/// binaries ends a run. The UTF-8 encoding of an integer is a complete
/// sequence that does not start with a continuation byte, so the output is
/// valid UTF-8 exactly when each run is valid. Thus one check of the output
/// after the walk checks each run.
///
/// The walk uses an explicit work stack, as ERTS does with an `ESTACK`, so
/// nested lists of any depth do not use the Rust stack.
///
/// Input that ERTS does not convert returns a runtime error: a code point
/// that is not a Unicode scalar value, a run of binaries that is not valid
/// UTF-8 (ERTS returns `{error, Converted, Rest}` or `{incomplete,
/// Converted, Rest}`), and a term that is not chardata (ERTS raises
/// `badarg`).
///
/// # Examples
///
/// ```rust,ignore
/// // The chardata of Elixir `IO.puts("Hello, world!")`.
/// let chardata = Term::List(vec![Term::Binary(b"Hello, world!".to_vec()), Term::Integer(10)]);
/// assert_eq!(characters_to_utf8(&chardata)?, b"Hello, world!\n");
/// // ERTS: unicode:characters_to_binary([[<<195>>], <<169>>, 8364], unicode) is <<"é€"/utf8>>.
/// let split = Term::List(vec![
///     Term::List(vec![Term::Binary(vec![195])]),
///     Term::Binary(vec![169]),
///     Term::Integer(8364),
/// ]);
/// assert_eq!(characters_to_utf8(&split)?, "é€".as_bytes());
/// assert!(characters_to_utf8(&Term::List(vec![Term::Integer(55296)])).is_err());
/// ```
pub(super) fn characters_to_utf8(chardata: &Term) -> Result<Vec<u8>, BeamError> {
    if !matches!(chardata, Term::Binary(_) | Term::List(_)) {
        return Err(error(format!(
            "unsupported chardata {chardata:?}: not a binary or a list"
        )));
    }
    let mut bytes = Vec::new();
    let mut pending = vec![std::slice::from_ref(chardata).iter()];
    while let Some(elements) = pending.last_mut() {
        let Some(element) = elements.next() else {
            pending.pop();
            continue;
        };
        match element {
            Term::Binary(binary) => bytes.extend_from_slice(binary),
            Term::List(nested) => pending.push(nested.iter()),
            Term::Integer(code_point) => {
                let character = u32::try_from(*code_point)
                    .ok()
                    .and_then(char::from_u32)
                    .ok_or_else(|| {
                        error(format!(
                            "unsupported chardata: {code_point} is not a Unicode code point"
                        ))
                    })?;
                bytes.extend_from_slice(character.encode_utf8(&mut [0; 4]).as_bytes());
            }
            other => {
                return Err(error(format!(
                    "unsupported chardata element {other:?}: not an integer, a binary, or a list"
                )));
            }
        }
    }
    // Rust and ERTS both reject overlong forms, surrogates, code points
    // above `0x10FFFF`, and an incomplete sequence.
    std::str::from_utf8(&bytes).map_err(|reason| {
        error(format!(
            "unsupported chardata: binaries are not valid UTF-8: {reason}"
        ))
    })?;
    Ok(bytes)
}

#[cfg(test)]
mod tests {
    use super::characters_to_utf8;
    use crate::term::{Term, test_atom};

    fn binary(bytes: &[u8]) -> Term {
        Term::Binary(bytes.to_vec())
    }

    #[test]
    fn converts_binaries_code_points_and_nested_lists_as_erts() {
        let convert = |chardata: Term| characters_to_utf8(&chardata).unwrap();
        // Each expected value is the result of unicode:characters_to_binary/2
        // with the `unicode` encoding on ERTS 29.1.1.
        assert_eq!(convert(binary(b"Hello, world!\n")), b"Hello, world!\n");
        assert_eq!(
            convert(Term::List(vec![
                binary(b"Hello, world!"),
                Term::Integer(10)
            ])),
            b"Hello, world!\n"
        );
        assert_eq!(convert(Term::List(Vec::new())), b"");
        assert_eq!(
            convert(Term::List(vec![
                Term::List(vec![binary(b""), Term::List(vec![Term::Integer(104)])]),
                Term::Integer(105),
            ])),
            b"hi"
        );
        for (code_point, expected) in [
            (233, &[195, 169][..]),
            (8364, &[226, 130, 172]),
            (128512, &[240, 159, 152, 128]),
            (1114111, &[244, 143, 191, 191]),
        ] {
            assert_eq!(
                convert(Term::List(vec![Term::Integer(code_point)])),
                expected
            );
        }
        // A UTF-8 sequence can continue in the next binary, also across
        // nested and empty lists and empty binaries.
        assert_eq!(
            convert(Term::List(vec![
                binary(&[226]),
                Term::List(vec![binary(&[130]), Term::List(Vec::new())]),
                binary(b""),
                Term::List(vec![binary(&[172])]),
            ])),
            [226, 130, 172]
        );
    }

    #[test]
    fn converts_deeply_nested_lists_without_recursion() {
        let mut chardata = Term::List(vec![Term::Integer(97)]);
        for _ in 0..100_000 {
            chardata = Term::List(vec![chardata]);
        }
        assert_eq!(characters_to_utf8(&chardata).unwrap(), b"a");
    }

    #[test]
    fn rejects_chardata_that_erts_does_not_convert() {
        let error = |chardata: Term| characters_to_utf8(&chardata).unwrap_err().to_string();
        // ERTS 29.1.1 returns {error, <<>>, [Value]} for these code points.
        for code_point in [-1, 55296, 1114112] {
            assert_eq!(
                error(Term::List(vec![Term::Integer(code_point)])),
                format!("unsupported chardata: {code_point} is not a Unicode code point")
            );
        }
        // ERTS 29.1.1 returns {error, <<>>, Rest} for invalid UTF-8 and
        // {incomplete, <<>>, <<195>>} for an incomplete sequence at the end.
        assert_eq!(
            error(binary(&[255])),
            "unsupported chardata: binaries are not valid UTF-8: invalid utf-8 sequence of 1 bytes from index 0"
        );
        assert_eq!(
            error(Term::List(vec![binary(&[192, 128])])),
            "unsupported chardata: binaries are not valid UTF-8: invalid utf-8 sequence of 1 bytes from index 0"
        );
        assert_eq!(
            error(Term::List(vec![binary(&[195])])),
            "unsupported chardata: binaries are not valid UTF-8: incomplete utf-8 byte sequence from index 0"
        );
        // An integer ends a run: ERTS returns {error, <<>>, [<<195>>, [97]]}.
        assert_eq!(
            error(Term::List(vec![binary(&[195]), Term::Integer(97)])),
            "unsupported chardata: binaries are not valid UTF-8: invalid utf-8 sequence of 1 bytes from index 0"
        );
        // ERTS 29.1.1 raises badarg for these terms.
        assert_eq!(
            error(Term::Integer(97)),
            "unsupported chardata Integer(97): not a binary or a list"
        );
        assert_eq!(
            error(Term::List(vec![test_atom("foo")])),
            "unsupported chardata element Atom(\"foo\"): not an integer, a binary, or a list"
        );
    }
}

//! Convert chardata to UTF-8, as the ERTS BIF `unicode:characters_to_binary/2`
//! does with the `unicode` encoding, check and convert file names to their
//! native bytes, as ERTS does before it uses a name, and convert native
//! bytes back to a name (`erl_unicode.c`).

use super::host::FilenameEncoding;
use crate::{
    error::{BeamError, error},
    term::Term,
};

/// Convert chardata to its UTF-8 bytes, or return `None` when the term is
/// not chardata, for which ERTS raises `badarg` (`erl_unicode.c:963-965`).
/// Chardata is a binary, or a list of integers, binaries, and nested lists
/// of the same. A list can have a binary as its tail, which comes after the
/// elements of that list. An integer is one Unicode code point. The bytes
/// of each run of adjacent binaries, in list order, must be one valid UTF-8
/// sequence, so a code point can start in one binary and end in the next
/// one, as in ERTS. An integer between two binaries ends a run. The UTF-8
/// encoding of an integer is a complete sequence that does not start with a
/// continuation byte, so the output is valid UTF-8 exactly when each run is
/// valid. Thus one check of the output after the walk checks each run.
///
/// [`is_chardata`] first checks the shape of the term, as `utf8_need` does
/// before the conversion. The walks use an explicit work stack, as ERTS
/// does with an `ESTACK`, so nested lists of any depth do not use the Rust
/// stack.
///
/// Chardata that ERTS does not convert returns a runtime error: a code
/// point that is not a Unicode scalar value, and a run of binaries that is
/// not valid UTF-8. ERTS returns `{error, Converted, Rest}` or
/// `{incomplete, Converted, Rest}` for them.
///
/// # Examples
///
/// ```rust,ignore
/// // The chardata of Elixir `IO.puts("Hello, world!")`.
/// let chardata = Term::List(vec![Term::Binary(b"Hello, world!".to_vec()), Term::Integer(10)]);
/// assert_eq!(characters_to_utf8(&chardata)?, Some(b"Hello, world!\n".to_vec()));
/// // ERTS: unicode:characters_to_binary([[<<195>>], <<169>>, 8364], unicode) is <<"é€"/utf8>>.
/// let split = Term::List(vec![
///     Term::List(vec![Term::Binary(vec![195])]),
///     Term::Binary(vec![169]),
///     Term::Integer(8364),
/// ]);
/// assert_eq!(characters_to_utf8(&split)?, Some("é€".as_bytes().to_vec()));
/// // ERTS: unicode:characters_to_binary([97|<<"b">>], unicode) is <<"ab">>.
/// let improper = Term::ImproperList(vec![Term::Integer(97)], Box::new(Term::Binary(b"b".to_vec())));
/// assert_eq!(characters_to_utf8(&improper)?, Some(b"ab".to_vec()));
/// assert_eq!(characters_to_utf8(&Term::static_atom("foo"))?, None);
/// assert!(characters_to_utf8(&Term::List(vec![Term::Integer(55296)])).is_err());
/// ```
pub(super) fn characters_to_utf8(chardata: &Term) -> Result<Option<Vec<u8>>, BeamError> {
    if !is_chardata(chardata) {
        return Ok(None);
    }
    let mut bytes = Vec::new();
    let mut pending = vec![(std::slice::from_ref(chardata).iter(), None)];
    while let Some((elements, tail)) = pending.last_mut() {
        let Some(element) = elements.next().or_else(|| tail.take()) else {
            pending.pop();
            continue;
        };
        match element {
            Term::Binary(binary) => bytes.extend_from_slice(binary),
            Term::List(nested) => pending.push((nested.iter(), None)),
            Term::ImproperList(nested, tail) => pending.push((nested.iter(), Some(&**tail))),
            // `is_chardata` leaves only integers here: it stops only at an
            // integer outside the small range, and this arm stops there too.
            code_point => {
                let character = code_point
                    .small_integer()
                    .and_then(|code_point| u32::try_from(code_point).ok())
                    .and_then(char::from_u32)
                    .ok_or_else(|| {
                        error(format!(
                            "unsupported chardata: {code_point:?} is not a Unicode code point"
                        ))
                    })?;
                bytes.extend_from_slice(character.encode_utf8(&mut [0; 4]).as_bytes());
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
    Ok(Some(bytes))
}

/// Return `true` when `chardata` has the shape of chardata, as `utf8_need`
/// with the `unicode` encoding checks it (`erl_unicode.c:342-505`). A
/// binary and a list are valid. In a list, each element must be a small
/// integer, a binary, or a list of the same, and each tail must be `[]` or
/// a binary. The walk reads the term in order and returns `true` at the
/// first integer outside the small range, without a check of the rest,
/// because the conversion stops at that integer (`is_big`). The value of a
/// small integer is not checked here.
///
/// # Examples
///
/// ```rust,ignore
/// let improper = |tail| Term::ImproperList(vec![Term::Integer(-1)], Box::new(tail));
/// assert!(is_chardata(&improper(Term::Binary(vec![]))));
/// assert!(!is_chardata(&improper(Term::static_atom("x"))));
/// // [1 bsl 60 | x]: the walk stops at the big integer.
/// assert!(is_chardata(&Term::ImproperList(vec![Term::Integer(1 << 60)], Box::new(Term::static_atom("x")))));
/// assert!(!is_chardata(&Term::Integer(97)));
/// ```
fn is_chardata(chardata: &Term) -> bool {
    if !matches!(chardata, Term::List(_) | Term::ImproperList(..)) {
        return matches!(chardata, Term::Binary(_));
    }
    let mut pending = vec![(std::slice::from_ref(chardata).iter(), None)];
    while let Some((elements, tail)) = pending.last_mut() {
        let Some(element) = elements.next() else {
            // A tail is never a list, so only a binary is valid.
            if tail.is_some_and(|tail: &Term| !matches!(tail, Term::Binary(_))) {
                return false;
            }
            pending.pop();
            continue;
        };
        match element {
            Term::Binary(_) => {}
            Term::List(nested) => pending.push((nested.iter(), None)),
            Term::ImproperList(nested, tail) => pending.push((nested.iter(), Some(&**tail))),
            Term::Integer(_) if element.small_integer().is_none() => return true,
            Term::Integer(_) => {}
            _ => return false,
        }
    }
    true
}

/// Return `true` when ERTS accepts `name` as a file name in `encoding`, as
/// `erts_convert_filename_to_encoding` with `allow_empty` and without
/// `allow_atom` does (`erl_unicode.c:2057-2119`). A binary is valid with
/// each content, because ERTS copies its bytes without a check. `[]` and a
/// list are valid when [`native_filename`] accepts them. Each other term is
/// not valid, for example an atom, an integer, and a tuple.
///
/// # Examples
///
/// ```rust,ignore
/// let name = Term::List(vec![Term::List(vec![Term::Integer(97)]), Term::List(vec![]), Term::Integer(256)]);
/// assert!(is_native_filename(&name, FilenameEncoding::Utf8));
/// assert!(!is_native_filename(&name, FilenameEncoding::Latin1));
/// assert!(is_native_filename(&Term::Binary(vec![97, 0]), FilenameEncoding::Latin1));
/// assert!(!is_native_filename(&Term::List(vec![Term::Integer(0)]), FilenameEncoding::Utf8));
/// ```
pub(super) fn is_native_filename(name: &Term, encoding: FilenameEncoding) -> bool {
    matches!(name, Term::Binary(_)) || native_filename(name, encoding).is_some()
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
/// let name = Term::List(vec![Term::List(vec![Term::Integer(97)]), Term::List(vec![]), Term::Integer(233)]);
/// assert_eq!(native_filename(&name, FilenameEncoding::Latin1), Some(vec![97, 233]));
/// assert_eq!(native_filename(&name, FilenameEncoding::Utf8), Some(vec![97, 195, 169]));
/// assert_eq!(native_filename(&Term::List(vec![Term::Integer(256)]), FilenameEncoding::Latin1), None);
/// assert_eq!(native_filename(&Term::Binary(vec![97]), FilenameEncoding::Utf8), None);
/// ```
pub(super) fn native_filename(name: &Term, encoding: FilenameEncoding) -> Option<Vec<u8>> {
    let Term::List(elements) = name else {
        return None;
    };
    let mut bytes = Vec::new();
    let mut pending = vec![elements.iter()];
    while let Some(elements) = pending.last_mut() {
        let Some(element) = elements.next() else {
            pending.pop();
            continue;
        };
        if let Term::List(nested) = element {
            pending.push(nested.iter());
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
/// Latin-1, and in UTF-8 when the bytes are not valid UTF-8
/// (`erts_analyze_utf8` in `erl_unicode.c:1180-1270`), each byte is one
/// character. Valid UTF-8 gives its code points. The UTF-8 check of ERTS
/// rejects overlong forms, surrogates, code points above `0x10FFFF`, and an
/// incomplete sequence, as [`std::str::from_utf8`] does.
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
/// // ERTS on macOS: a\xc3\xa9 gives [97,233], and a\xff gives [97,255].
/// let list = |characters: &[i64]| Term::List(characters.iter().copied().map(Term::Integer).collect());
/// assert_eq!(filename_from_native(b"a\xc3\xa9", FilenameEncoding::Utf8Mac)?, list(&[97, 233]));
/// assert_eq!(filename_from_native(b"a\xc3\xa9", FilenameEncoding::Latin1)?, list(&[97, 195, 169]));
/// assert_eq!(filename_from_native(b"a\xff", FilenameEncoding::Utf8)?, list(&[97, 255]));
/// assert!(filename_from_native(b"ae\xcc\x81", FilenameEncoding::Utf8Mac).is_err());
/// ```
pub(super) fn filename_from_native(
    bytes: &[u8],
    encoding: FilenameEncoding,
) -> Result<Term, BeamError> {
    let text = match encoding {
        FilenameEncoding::Latin1 => None,
        FilenameEncoding::Utf8 | FilenameEncoding::Utf8Mac => std::str::from_utf8(bytes).ok(),
    };
    let Some(text) = text else {
        return Ok(Term::List(
            bytes
                .iter()
                .map(|&byte| Term::Integer(byte.into()))
                .collect(),
        ));
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
    Ok(Term::List(
        text.chars()
            .map(|character| Term::Integer(u32::from(character).into()))
            .collect(),
    ))
}

#[cfg(test)]
mod tests {
    use super::{
        COMPOSITION_CANDIDATES, FilenameEncoding, characters_to_utf8, filename_from_native,
        is_native_filename, native_filename,
    };
    use crate::term::{Term, test_atom};

    fn binary(bytes: &[u8]) -> Term {
        Term::Binary(bytes.to_vec())
    }

    #[test]
    fn converts_binaries_code_points_and_nested_lists_as_erts() {
        let convert = |chardata: Term| characters_to_utf8(&chardata).unwrap().unwrap();
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
        // A binary tail comes after the elements of its list, and a run of
        // binaries continues across it: ERTS gives <<"ab">> for
        // [97|<<"b">>] and <<"é"/utf8>> for [[<<195>>|<<169>>]].
        let improper =
            |elements: Vec<Term>, tail: Term| Term::ImproperList(elements, Box::new(tail));
        assert_eq!(
            convert(improper(vec![Term::Integer(97)], binary(b"b"))),
            b"ab"
        );
        assert_eq!(
            convert(Term::List(vec![improper(
                vec![binary(&[195])],
                binary(&[169])
            )])),
            [195, 169]
        );
    }

    #[test]
    fn converts_deeply_nested_lists_without_recursion() {
        let mut chardata = Term::List(vec![Term::Integer(97)]);
        for _ in 0..100_000 {
            chardata = Term::List(vec![chardata]);
        }
        assert_eq!(characters_to_utf8(&chardata).unwrap().unwrap(), b"a");
    }

    #[test]
    fn rejects_chardata_that_erts_does_not_convert() {
        let error = |chardata: Term| characters_to_utf8(&chardata).unwrap_err().to_string();
        // ERTS 29.1.1 returns {error, <<>>, [Value]} for these code points.
        for code_point in [-1, 55296, 1114112, 1 << 60] {
            assert_eq!(
                error(Term::List(vec![Term::Integer(code_point)])),
                format!("unsupported chardata: Integer({code_point}) is not a Unicode code point")
            );
        }
        // The walk of `utf8_need` stops at an integer outside the small
        // range, so the term after it is not checked: ERTS returns an error
        // tuple for [1 bsl 60 | x] and [1 bsl 60, foo], not badarg.
        let big = || Term::Integer(1 << 60);
        for chardata in [
            Term::ImproperList(vec![big()], Box::new(test_atom("x"))),
            Term::List(vec![big(), test_atom("foo")]),
        ] {
            assert_eq!(
                error(chardata),
                format!(
                    "unsupported chardata: Integer({}) is not a Unicode code point",
                    1_i64 << 60
                )
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
    }

    #[test]
    fn finds_terms_that_are_not_chardata_before_the_conversion() {
        // ERTS 29.1.1 raises badarg for each of these terms, because
        // `utf8_need` rejects them before the conversion
        // (erl_unicode.c:342-505, 963-965): a term that is not a binary or
        // a list, an element that is not an integer, a binary, or a list,
        // and a tail that is not a binary, also after a code point that is
        // not valid and after invalid UTF-8.
        let improper =
            |elements: Vec<Term>, tail: Term| Term::ImproperList(elements, Box::new(tail));
        for chardata in [
            Term::Integer(97),
            test_atom("foo"),
            Term::List(vec![test_atom("foo")]),
            Term::List(vec![Term::Integer(-1), Term::Tuple(vec![])]),
            Term::List(vec![binary(&[255]), Term::List(vec![test_atom("foo")])]),
            improper(vec![Term::Integer(97)], test_atom("x")),
            improper(vec![Term::Integer(-1)], test_atom("x")),
            improper(vec![Term::Integer(97)], Term::Integer(98)),
            Term::List(vec![
                improper(vec![binary(b"a")], test_atom("x")),
                Term::Integer(97),
            ]),
        ] {
            assert_eq!(characters_to_utf8(&chardata).unwrap(), None, "{chardata:?}");
        }
    }

    #[test]
    fn accepts_the_file_names_that_erts_accepts_in_each_encoding() {
        let list = |characters: &[i64]| {
            Term::List(characters.iter().copied().map(Term::Integer).collect())
        };
        // UTF-8 with composition checks a name as UTF-8 does, so it must
        // give the same result as UTF-8.
        let check = |name: &Term| {
            let [latin1, utf8, utf8_mac] = [
                FilenameEncoding::Latin1,
                FilenameEncoding::Utf8,
                FilenameEncoding::Utf8Mac,
            ]
            .map(|encoding| is_native_filename(name, encoding));
            assert_eq!(utf8, utf8_mac, "{name:?}");
            [latin1, utf8]
        };
        // Each expected pair is [Latin-1, UTF-8]. ERTS accepts a binary with
        // each content, also with a 0 byte, `[]`, and nested lists.
        for name in [
            binary(b"prim_tty"),
            binary(b"a\0b"),
            binary(&[255]),
            Term::List(vec![]),
            list(&[97, 1, 255]),
            Term::List(vec![list(&[97]), Term::List(vec![]), Term::Integer(98)]),
        ] {
            assert_eq!(check(&name), [true, true], "{name:?}");
        }
        // A character above 255 is valid only in UTF-8, up to 0x10FFFF and
        // not from 0xD800 to 0xDFFF.
        for character in [256, 0xD7FF, 0xE000, 0x10FFFF] {
            assert_eq!(check(&list(&[character])), [false, true], "{character}");
        }
        for character in [0, -1, 0xD800, 0xDFFF, 0x110000] {
            assert_eq!(check(&list(&[character])), [false, false], "{character}");
        }
        // A bignum, a binary, an atom, or a tuple in a list, and each term
        // that is not a binary or a list, is not valid.
        for name in [
            list(&[1 << 59]),
            Term::List(vec![binary(b"a")]),
            Term::ImproperList(vec![Term::Integer(97)], Box::new(binary(b"b"))),
            Term::List(vec![Term::ImproperList(
                vec![Term::Integer(97)],
                Box::new(Term::Integer(98)),
            )]),
            Term::List(vec![Term::Integer(97), test_atom("foo")]),
            Term::List(vec![Term::Tuple(vec![])]),
            Term::List(vec![Term::List(vec![Term::Integer(97), Term::Integer(0)])]),
            test_atom("foo"),
            Term::Integer(97),
            Term::Tuple(vec![]),
        ] {
            assert_eq!(check(&name), [false, false], "{name:?}");
        }
    }

    #[test]
    fn checks_deeply_nested_file_names_without_recursion() {
        let mut name = Term::List(vec![Term::Integer(97)]);
        for _ in 0..100_000 {
            name = Term::List(vec![name, Term::List(vec![])]);
        }
        assert!(is_native_filename(&name, FilenameEncoding::Latin1));
    }

    #[test]
    fn writes_the_native_bytes_of_a_file_name_in_each_encoding() {
        let list = |characters: &[i64]| {
            Term::List(characters.iter().copied().map(Term::Integer).collect())
        };
        let bytes = |name: &Term| {
            [
                FilenameEncoding::Latin1,
                FilenameEncoding::Utf8,
                FilenameEncoding::Utf8Mac,
            ]
            .map(|encoding| native_filename(name, encoding))
        };
        // Latin-1 writes one byte for each character, and UTF-8 writes the
        // UTF-8 bytes of each character (erl_unicode.c:2400-2555).
        let some = |bytes: &[u8]| Some(bytes.to_vec());
        assert_eq!(bytes(&list(&[])), [some(b""), some(b""), some(b"")]);
        assert_eq!(
            bytes(&Term::List(vec![
                list(&[80]),
                list(&[]),
                Term::Integer(233)
            ])),
            [
                some(&[80, 233]),
                some(&[80, 195, 169]),
                some(&[80, 195, 169])
            ]
        );
        assert_eq!(
            bytes(&list(&[1046, 0x10FFFF])),
            [
                None,
                some(&[208, 150, 244, 143, 191, 191]),
                some(&[208, 150, 244, 143, 191, 191])
            ]
        );
        // A binary is a valid file name, but it has no native bytes here.
        assert_eq!(bytes(&binary(b"a")), [None, None, None]);
    }

    #[test]
    fn converts_native_bytes_to_a_file_name_in_each_encoding() {
        let list = |characters: &[i64]| {
            Term::List(characters.iter().copied().map(Term::Integer).collect())
        };
        let convert = |bytes: &[u8], encoding| filename_from_native(bytes, encoding);
        // Each expected list comes from erl_unicode.c:2164-2220. Measured on
        // ERTS 29.1.1 on macOS: a\xc3\xa9 gives [97,233], a\xff gives
        // [97,255], and \xd0\x96 gives [1046].
        for (bytes, latin1, utf8) in [
            (&b""[..], &[][..], &[][..]),
            (b"yes", &[121, 101, 115], &[121, 101, 115]),
            (b"a\xc3\xa9", &[97, 195, 169], &[97, 233]),
            (b"a\xff", &[97, 255], &[97, 255]),
            (b"\xc0\x80", &[192, 128], &[192, 128]),
            (b"\xed\xa0\x80", &[237, 160, 128], &[237, 160, 128]),
            (b"\xd0\x96", &[208, 150], &[1046]),
            (b"ae\xcc\x81", &[97, 101, 204, 129], &[97, 101, 769]),
        ] {
            assert_eq!(convert(bytes, FilenameEncoding::Latin1), Ok(list(latin1)));
            assert_eq!(convert(bytes, FilenameEncoding::Utf8), Ok(list(utf8)));
            if !bytes.starts_with(b"ae") {
                assert_eq!(convert(bytes, FilenameEncoding::Utf8Mac), Ok(list(utf8)));
            }
        }
        // ERTS on macOS composes ae\xcc\x81 to [97,233], and PBeam has no
        // composition table. Invalid UTF-8 is not composed.
        assert_eq!(
            convert(b"ae\xcc\x81", FilenameEncoding::Utf8Mac)
                .unwrap_err()
                .to_string(),
            "unsupported composition of the native name code point 769 on macOS: see \"OS environment\" in docs/erts-differences.md"
        );
        assert_eq!(
            convert(b"\xcc\x81\xff", FilenameEncoding::Utf8Mac),
            Ok(list(&[204, 129, 255]))
        );
        // Each candidate returns the runtime error. The code points next to
        // the candidates are not candidates: measured on ERTS 29.1.1 on
        // macOS, prim_file:internal_native2name/1, which also calls
        // erts_convert_native_to_filename, gives [767], [773], [4024], and
        // [12443] for their UTF-8 bytes.
        assert!(COMPOSITION_CANDIDATES.is_sorted());
        for candidate in COMPOSITION_CANDIDATES {
            let text = char::from_u32(candidate).unwrap().to_string();
            assert!(convert(text.as_bytes(), FilenameEncoding::Utf8Mac).is_err());
        }
        for code_point in [767_i64, 773, 4024, 12443] {
            let text = char::from_u32(code_point as u32).unwrap().to_string();
            assert_eq!(
                convert(text.as_bytes(), FilenameEncoding::Utf8Mac),
                Ok(list(&[code_point]))
            );
        }
    }
}

use super::{
    COMPOSITION_CANDIDATES, FilenameEncoding, Utf8Output, Utf8State, analyze_utf8, build_utf8,
    code_points, filename_from_native, is_native_filename, native_filename, utf8_need,
};
use crate::term::{Term, test_atom};

fn binary(bytes: &[u8]) -> Term {
    Term::Binary(bytes.to_vec())
}

/// Convert chardata with a budget that does not end, and return the bytes,
/// the rest, whether the walk stopped at an error, and the bytes of an
/// unfinished sequence.
fn convert(chardata: &Term) -> (Vec<u8>, Term, bool, Vec<u8>) {
    let mut output = Utf8Output::default();
    let mut left = i64::MAX;
    let (rest, error) = build_utf8(chardata, &mut left, false, &mut output);
    (output.bytes, rest, error, output.leftover)
}

/// Convert valid chardata and return its bytes.
fn valid(chardata: Term) -> Vec<u8> {
    assert!(utf8_need(&chardata, false).is_some(), "{chardata:?}");
    let (bytes, rest, error, leftover) = convert(&chardata);
    assert_eq!((rest, error, leftover), (Term::nil(), false, Vec::new()));
    bytes
}

#[test]
fn converts_binaries_code_points_and_nested_lists_as_erts() {
    // Each expected value is the result of unicode:characters_to_binary/2
    // with the `unicode` encoding on ERTS 29.1.1.
    assert_eq!(valid(binary(b"Hello, world!\n")), b"Hello, world!\n");
    assert_eq!(
        valid(Term::list_from(vec![
            binary(b"Hello, world!"),
            Term::Integer(10)
        ])),
        b"Hello, world!\n"
    );
    assert_eq!(valid(Term::nil()), b"");
    assert_eq!(
        valid(Term::list_from(vec![
            Term::list_from(vec![binary(b""), Term::list_from(vec![Term::Integer(104)])]),
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
            valid(Term::list_from(vec![Term::Integer(code_point)])),
            expected
        );
    }
    // A UTF-8 sequence can continue in the next binary, also across
    // nested and empty lists and empty binaries.
    assert_eq!(
        valid(Term::list_from(vec![
            binary(&[226]),
            Term::list_from(vec![binary(&[130]), Term::nil()]),
            binary(b""),
            Term::list_from(vec![binary(&[172])]),
        ])),
        [226, 130, 172]
    );
    // A binary tail comes after the elements of its list, and a run of
    // binaries continues across it: ERTS gives <<"ab">> for
    // [97|<<"b">>] and <<"é"/utf8>> for [[<<195>>|<<169>>]].
    let improper = |elements: Vec<Term>, tail: Term| Term::list_with_tail(elements, tail);
    assert_eq!(
        valid(improper(vec![Term::Integer(97)], binary(b"b"))),
        b"ab"
    );
    assert_eq!(
        valid(Term::list_from(vec![improper(
            vec![binary(&[195])],
            binary(&[169])
        )])),
        [195, 169]
    );
    assert_eq!(
        code_points("aé€".as_bytes()),
        Term::list_from(vec![
            Term::Integer(97),
            Term::Integer(233),
            Term::Integer(8364)
        ])
    );
}

#[test]
fn converts_deeply_nested_lists_without_recursion() {
    let mut chardata = Term::list_from(vec![Term::Integer(97)]);
    for _ in 0..100_000 {
        chardata = Term::list_from(vec![chardata]);
    }
    assert_eq!(utf8_need(&chardata, false), Some(1));
    assert_eq!(valid(chardata), b"a");
}

#[test]
fn stops_at_chardata_that_erts_does_not_convert() {
    // ERTS 29.1.1 returns {error, <<>>, [Value]} for these code points.
    for code_point in [-1, 55296, 1114112, 1 << 60] {
        let chardata = Term::list_from(vec![Term::Integer(code_point)]);
        assert_eq!(
            convert(&chardata),
            (Vec::new(), chardata.clone(), true, Vec::new())
        );
    }
    // The walk of `utf8_need` stops at an integer outside the small
    // range, so the term after it is not checked: ERTS returns an error
    // tuple for [1 bsl 60 | x] and [1 bsl 60, foo], not badarg.
    let big = || Term::Integer(1 << 60);
    for chardata in [
        Term::list_with_tail(vec![big()], test_atom("x")),
        Term::list_from(vec![big(), test_atom("foo")]),
    ] {
        assert_eq!(utf8_need(&chardata, false), Some(0));
        assert_eq!(
            convert(&chardata),
            (Vec::new(), chardata.clone(), true, Vec::new())
        );
    }
    // ERTS 29.1.1 returns {error, <<>>, <<255>>} and
    // {error, <<>>, [<<192,128>>]} for invalid UTF-8, and
    // {incomplete, <<>>, <<195>>} for an incomplete sequence at the end.
    assert_eq!(
        convert(&binary(&[255])),
        (Vec::new(), binary(&[255]), true, Vec::new())
    );
    let overlong = Term::list_from(vec![binary(&[192, 128])]);
    assert_eq!(
        convert(&overlong),
        (Vec::new(), overlong.clone(), true, Vec::new())
    );
    let short = Term::list_from(vec![binary(&[195])]);
    assert_eq!(convert(&short), (Vec::new(), Term::nil(), false, vec![195]));
    // An integer after the start of a sequence stops the walk: ERTS
    // returns {error, <<>>, [<<195>>, [97]]}.
    let mixed = Term::list_from(vec![binary(&[195]), Term::Integer(97)]);
    assert_eq!(
        convert(&mixed),
        (
            Vec::new(),
            Term::list_from(vec![Term::Integer(97)]),
            true,
            vec![195]
        )
    );
}

#[test]
fn stops_the_walk_at_a_term_that_is_not_chardata() {
    // `utf8_need` rejects these terms first, but the walk of
    // `do_build_utf8` also stops at them with an error
    // (`erl_unicode.c:597-600`, `713-716`, `722-733`).
    let improper = Term::list_with_tail(vec![Term::Integer(97)], test_atom("x"));
    assert_eq!(
        convert(&improper),
        (b"a".to_vec(), test_atom("x"), true, Vec::new())
    );
    let list = Term::list_from(vec![Term::Integer(97), test_atom("foo")]);
    assert_eq!(
        convert(&list),
        (
            b"a".to_vec(),
            Term::list_from(vec![test_atom("foo")]),
            true,
            Vec::new()
        )
    );
    assert_eq!(
        convert(&test_atom("foo")),
        (Vec::new(), test_atom("foo"), true, Vec::new())
    );
}

#[test]
fn splits_the_walk_where_the_steps_end() {
    // A list that a step limit splits gives the rest of each entered list
    // as a cell before the rest of the enclosing list.
    let inner = Term::list_from(vec![Term::Integer(97), Term::Integer(98)]);
    let chardata = Term::list_from(vec![inner, Term::Integer(99)]);
    let mut output = Utf8Output::default();
    let mut left = 1;
    let (rest, error) = build_utf8(&chardata, &mut left, false, &mut output);
    assert_eq!(
        (rest, error, output.bytes),
        (
            Term::list_from(vec![
                Term::list_from(vec![Term::Integer(98)]),
                Term::Integer(99)
            ]),
            false,
            b"a".to_vec()
        )
    );
    // A binary splits at the next character boundary, so the steps can go
    // past 0.
    let mut output = Utf8Output::default();
    let mut left = 1;
    let (rest, error) = build_utf8(&binary("éa".as_bytes()), &mut left, false, &mut output);
    assert_eq!((rest, error, left), (binary(b"a"), false, 0));
    // With latin1, each byte is one character, and a binary splits at any byte.
    let mut output = Utf8Output::default();
    let mut left = 1;
    let (rest, _) = build_utf8(&binary(&[233, 97]), &mut left, true, &mut output);
    assert_eq!(
        (rest, output.bytes),
        (binary(&[97]), "é".as_bytes().to_vec())
    );
    // The analysis of a binary counts one step for each character.
    let mut left = 2;
    assert_eq!(
        analyze_utf8(b"abc", 0, &mut left),
        (Utf8State::AnalyzeMore, 2, 2)
    );
    let mut left = 3;
    assert_eq!(analyze_utf8(b"abc", 0, &mut left), (Utf8State::Ok, 3, 3));
    assert_eq!(left, 0);
    let mut left = 10;
    assert_eq!(
        analyze_utf8(&[97, 195], 0, &mut left),
        (Utf8State::Incomplete, 1, 1)
    );
    assert_eq!(
        analyze_utf8(&[97, 255], 0, &mut left),
        (Utf8State::Error, 1, 1)
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
    let improper = |elements: Vec<Term>, tail: Term| Term::list_with_tail(elements, tail);
    for chardata in [
        Term::Integer(97),
        test_atom("foo"),
        Term::list_from(vec![test_atom("foo")]),
        Term::list_from(vec![Term::Integer(-1), Term::Tuple(vec![])]),
        Term::list_from(vec![
            binary(&[255]),
            Term::list_from(vec![test_atom("foo")]),
        ]),
        improper(vec![Term::Integer(97)], test_atom("x")),
        improper(vec![Term::Integer(-1)], test_atom("x")),
        improper(vec![Term::Integer(97)], Term::Integer(98)),
        Term::list_from(vec![
            improper(vec![binary(b"a")], test_atom("x")),
            Term::Integer(97),
        ]),
    ] {
        assert_eq!(utf8_need(&chardata, false), None, "{chardata:?}");
    }
    // A binary costs 1, or with latin1 the bytes of its UTF-8 form.
    let list = Term::list_from(vec![Term::Integer(97), binary(&[233, 97])]);
    assert_eq!(utf8_need(&list, false), Some(2));
    assert_eq!(utf8_need(&list, true), Some(4));
}

#[test]
fn accepts_the_file_names_that_erts_accepts_in_each_encoding() {
    let list = |characters: &[i64]| Term::list_from(characters.iter().copied().map(Term::Integer));
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
        Term::nil(),
        list(&[97, 1, 255]),
        Term::list_from(vec![list(&[97]), Term::nil(), Term::Integer(98)]),
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
        Term::list_from(vec![binary(b"a")]),
        Term::list_with_tail(vec![Term::Integer(97)], binary(b"b")),
        Term::list_from(vec![Term::list_with_tail(
            vec![Term::Integer(97)],
            Term::Integer(98),
        )]),
        Term::list_from(vec![Term::Integer(97), test_atom("foo")]),
        Term::list_from(vec![Term::Tuple(vec![])]),
        Term::list_from(vec![Term::list_from(vec![
            Term::Integer(97),
            Term::Integer(0),
        ])]),
        test_atom("foo"),
        Term::Integer(97),
        Term::Tuple(vec![]),
    ] {
        assert_eq!(check(&name), [false, false], "{name:?}");
    }
}

#[test]
fn checks_deeply_nested_file_names_without_recursion() {
    let mut name = Term::list_from(vec![Term::Integer(97)]);
    for _ in 0..100_000 {
        name = Term::list_from(vec![name, Term::nil()]);
    }
    assert!(is_native_filename(&name, FilenameEncoding::Latin1));
}

#[test]
fn writes_the_native_bytes_of_a_file_name_in_each_encoding() {
    let list = |characters: &[i64]| Term::list_from(characters.iter().copied().map(Term::Integer));
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
        bytes(&Term::list_from(vec![
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
    let list = |characters: &[i64]| Term::list_from(characters.iter().copied().map(Term::Integer));
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

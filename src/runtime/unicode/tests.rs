use super::{
    COMPOSITION_CANDIDATES, FilenameEncoding, characters_to_code_points, characters_to_utf8,
    filename_from_native, is_native_filename, native_filename,
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
    let improper = |elements: Vec<Term>, tail: Term| Term::ImproperList(elements, Box::new(tail));
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
fn converts_deeply_nested_lists_to_code_points_without_recursion() {
    let mut chardata = Term::List(vec![Term::Integer(97)]);
    for _ in 0..100_000 {
        chardata = Term::List(vec![chardata]);
    }
    assert_eq!(
        characters_to_code_points(&chardata, false),
        Some(Term::List(vec![Term::Integer(97)]))
    );
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
    let improper = |elements: Vec<Term>, tail: Term| Term::ImproperList(elements, Box::new(tail));
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
    let list =
        |characters: &[i64]| Term::List(characters.iter().copied().map(Term::Integer).collect());
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
    let list =
        |characters: &[i64]| Term::List(characters.iter().copied().map(Term::Integer).collect());
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
    let list =
        |characters: &[i64]| Term::List(characters.iter().copied().map(Term::Integer).collect());
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

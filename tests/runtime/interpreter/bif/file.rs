use super::*;

#[test]
fn names_the_native_name_encoding_of_each_host_encoding() {
    // `file_native_name_encoding_0` (erl_unicode.c:2899-2916). The Latin-1
    // encoding exists only on a host other than macOS, so the integration
    // test of a macOS host does not reach it.
    for (encoding, name) in [
        (FilenameEncoding::Latin1, "latin1"),
        (FilenameEncoding::Utf8, "utf8"),
        (FilenameEncoding::Utf8Mac, "utf8"),
    ] {
        let BifResult::Value(Term::Atom(ref atom)) = native_name_encoding(encoding) else {
            panic!("native_name_encoding({encoding:?}) gives no atom");
        };
        assert_eq!(atom.name(), name, "{encoding:?}");
    }
}

#[test]
fn converts_native_name_bytes_or_gives_the_warning_tuple_in_each_encoding() {
    // `prim_file_internal_native2name_1` (erl_unicode.c:2735-2824).
    // Measured on ERTS 29.1.1 (macOS, and Linux with LC_ALL=C.UTF-8 and
    // LC_ALL=C): <<>> gives [], <<"a",255>> gives {error,warning} in UTF-8
    // and [97,255] in Latin-1, and the atom ok raises badarg. The Latin-1
    // encoding exists only on a host other than macOS.
    let convert = |name: &Term, encoding| format!("{:?}", internal_native2name(name, encoding));
    let warning = "Ok(Value(Tuple([Atom(\"error\"), Atom(\"warning\")])))";
    for (name, latin1, utf8) in [
        (
            Term::Binary(vec![]),
            "Ok(Value(List([])))",
            "Ok(Value(List([])))",
        ),
        (
            Term::Binary(b"a\xc3\xa9".to_vec()),
            "Ok(Value(List([Integer(97), Integer(195), Integer(169)])))",
            "Ok(Value(List([Integer(97), Integer(233)])))",
        ),
        (
            Term::Binary(b"a\xff".to_vec()),
            "Ok(Value(List([Integer(97), Integer(255)])))",
            warning,
        ),
        (
            Term::static_atom("ok"),
            "Ok(Raise { class: Error, reason: Atom(\"badarg\") })",
            "Ok(Raise { class: Error, reason: Atom(\"badarg\") })",
        ),
    ] {
        assert_eq!(convert(&name, FilenameEncoding::Latin1), latin1, "{name:?}");
        assert_eq!(convert(&name, FilenameEncoding::Utf8), utf8, "{name:?}");
        assert_eq!(convert(&name, FilenameEncoding::Utf8Mac), utf8, "{name:?}");
    }
    // ERTS on macOS composes ae\xcc\x81 to [97,233], and PBeam has no
    // composition table.
    let composed = internal_native2name(
        &Term::Binary(b"ae\xcc\x81".to_vec()),
        FilenameEncoding::Utf8Mac,
    );
    assert!(
        composed
            .unwrap_err()
            .to_string()
            .starts_with("unsupported composition of the native name code point 769 on macOS")
    );
}

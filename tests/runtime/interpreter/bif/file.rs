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

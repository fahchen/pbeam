//! Bodies of the `file` and `prim_file` BIFs that read the native file name
//! encoding of the host (`erl_unicode.c`).

use super::BifResult::Value;
use super::*;
use crate::runtime::host::{FilenameEncoding, filename_encoding};
use crate::runtime::unicode::native_filename;

/// Run `file:native_name_encoding()` for the file name encoding of the
/// host, `encoding`: return `latin1` for Latin-1, else `utf8`, as
/// `file_native_name_encoding_0` (`erl_unicode.c:2899-2916`) does. The
/// encoding is UTF-8 on macOS, and on another host it comes from the
/// `LC_CTYPE` codeset (see [`filename_encoding`]).
///
/// # Examples
///
/// ```rust,ignore
/// assert!(matches!(native_name_encoding(FilenameEncoding::Utf8Mac), BifResult::Value(Term::Atom(atom)) if atom.name() == "utf8"));
/// assert!(matches!(native_name_encoding(FilenameEncoding::Latin1), BifResult::Value(Term::Atom(atom)) if atom.name() == "latin1"));
/// ```
pub(super) fn native_name_encoding(encoding: FilenameEncoding) -> BifResult {
    Value(Term::static_atom(match encoding {
        FilenameEncoding::Latin1 => "latin1",
        FilenameEncoding::Utf8 | FilenameEncoding::Utf8Mac => "utf8",
    }))
}

/// Run `prim_file:internal_name2native(Name)`: return the native bytes of
/// the file name `Name` with one NUL byte at the end, as
/// `prim_file_internal_name2native_1` (`erl_unicode.c:2629-2734`) does on a
/// host that is not Windows.
///
/// An atom raises `badarg` (`erl_unicode.c:2645-2647`). A binary gives its
/// bytes as they are, and a NUL byte in it raises `badarg`
/// (`erl_unicode.c:2648-2675`, `2709-2711`). Each other term gives the bytes
/// of `erts_native_filename_put` in the file name encoding of the host (see
/// [`native_filename`]), or raises `badarg` when
/// `erts_native_filename_need` rejects it (`erl_unicode.c:2715-2717`).
///
/// # Examples
///
/// ```rust,ignore
/// let name = Term::list_from(vec![Term::Integer(97), Term::list_from(vec![Term::Integer(98)])]);
/// assert!(matches!(internal_name2native(&name), BifResult::Value(Term::Binary(bytes)) if bytes == b"ab\0"));
/// assert!(matches!(internal_name2native(&Term::Binary(b"a".to_vec())), BifResult::Value(Term::Binary(bytes)) if bytes == b"a\0"));
/// assert!(matches!(internal_name2native(&Term::Binary(b"a\0".to_vec())), BifResult::Raise { .. }));
/// assert!(matches!(internal_name2native(&Term::static_atom("a")), BifResult::Raise { .. }));
/// ```
pub(super) fn internal_name2native(name: &Term) -> BifResult {
    let bytes = match name {
        Term::Atom(_) => None,
        Term::Binary(bytes) => (!bytes.contains(&0)).then(|| bytes.clone()),
        _ => native_filename(name, filename_encoding()),
    };
    match bytes {
        Some(mut bytes) => {
            bytes.push(0);
            Value(Term::Binary(bytes))
        }
        None => BifResult::badarg(),
    }
}

#[cfg(test)]
#[path = "../../../../tests/runtime/interpreter/bif/file.rs"]
mod tests;

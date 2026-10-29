//! Bodies of the `file` and `prim_file` BIFs that read the native file name
//! encoding of the host (`erl_unicode.c`).

use super::BifResult::Value;
use super::{BeamError, BifResult, Term};
use crate::runtime::host::{FilenameEncoding, filename_encoding};
use crate::runtime::unicode::{name_from_native, native_filename};

/// Run `file:native_name_encoding()` for the file name encoding of the
/// host, `encoding`: return `latin1` for Latin-1, else `utf8`, as
/// `file_native_name_encoding_0` (`erl_unicode.c:2899-2916`) does. The
/// encoding is UTF-8 on macOS, and on another host it comes from the
/// `LC_CTYPE` codeset (see [`filename_encoding`]).
///
/// # Examples
///
/// ```rust,ignore
/// assert!(matches!(native_name_encoding(FilenameEncoding::Utf8Mac), BifResult::Value(Term::Atom(atom)) if atom.is_static("utf8"));
/// assert!(matches!(native_name_encoding(FilenameEncoding::Latin1), BifResult::Value(Term::Atom(atom)) if atom.is_static("latin1"));
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
/// assert!(matches!(internal_name2native(&Term::binary(b"a".to_vec())), BifResult::Value(Term::Binary(bytes)) if bytes == b"a\0"));
/// assert!(matches!(internal_name2native(&Term::binary(b"a\0".to_vec())), BifResult::Raise { .. }));
/// assert!(matches!(internal_name2native(&Term::static_atom("a")), BifResult::Raise { .. }));
/// ```
pub(super) fn internal_name2native(name: &Term) -> BifResult {
    let bytes = match name {
        Term::Atom(_) => None,
        Term::Binary(bytes) => bytes
            .bytes()
            .filter(|bytes| !bytes.contains(&0))
            .map(<[u8]>::to_vec),
        _ => native_filename(name, filename_encoding()),
    };
    match bytes {
        Some(mut bytes) => {
            bytes.push(0);
            Value(Term::binary(bytes))
        }
        None => BifResult::badarg(),
    }
}

/// Run `prim_file:internal_native2name(Name)` for the file name encoding of
/// the host, `encoding`: return the characters of the native bytes `Name`,
/// as `prim_file_internal_native2name_1` (`erl_unicode.c:2735-2824`) does
/// on a host that is not Windows (see [`name_from_native`]).
///
/// A term that is not a binary raises `badarg`, and `<<>>` gives `[]`
/// (`erl_unicode.c:2750-2756`). Bytes that are not valid UTF-8 in a UTF-8
/// encoding give `{error, warning}`: PBeam has no `+fn` emulator flag, so
/// the warning type is the ERTS default (`erl_unicode.c:2774-2788`).
///
/// # Examples
///
/// ```rust,ignore
/// // [97, 233]
/// let result = internal_native2name(&Term::binary(b"a\xc3\xa9".to_vec()), FilenameEncoding::Utf8)?;
/// // {error, warning}
/// let result = internal_native2name(&Term::binary(b"a\xff".to_vec()), FilenameEncoding::Utf8)?;
/// assert!(matches!(internal_native2name(&Term::static_atom("a"), FilenameEncoding::Utf8)?, BifResult::Raise { .. }));
/// ```
///
/// # Errors
///
/// The runtime error of [`name_from_native`] for a composition on macOS.
pub(super) fn internal_native2name(
    name: &Term,
    encoding: FilenameEncoding,
) -> Result<BifResult, BeamError> {
    let Some(bytes) = name.binary_bytes() else {
        return Ok(BifResult::badarg());
    };
    Ok(Value(name_from_native(bytes, encoding)?.unwrap_or_else(
        || {
            Term::tuple(vec![
                Term::static_atom("error"),
                Term::static_atom("warning"),
            ])
        },
    )))
}

#[cfg(test)]
#[path = "../../../../tests/runtime/interpreter/bif/file.rs"]
mod tests;

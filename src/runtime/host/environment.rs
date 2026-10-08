//! Read the OS environment of the host process as ERTS reads its copy of
//! the environment (`sys/unix/sys_env.c`, `sys/common/erl_osenv.c`).

use std::ffi::{CStr, c_char};

/// Return the value of the environment variable `key` as raw bytes, or
/// `None` when the environment has no entry with the key. The lookup
/// follows the ERTS copy of the environment, which `import_initial_env`
/// (`sys_env.c:40-46`, `134-154`) makes from `environ` at emulator start:
/// each entry splits at its first `=`, an entry without `=` is skipped, and
/// a later entry replaces an earlier entry with the same key
/// (`erts_osenv_put_native` in `erl_osenv.c:360-385`). The empty key is
/// also a key. Thus the result can be different from libc `getenv` and
/// `std::env::var_os`, which return the first entry of a key.
///
/// PBeam reads `environ` at each call, not a copy from the run start. The
/// result is the same, because PBeam never changes its environment: it
/// has no `os:putenv/2` and no `os:unsetenv/1`, and it does not call
/// `std::env::set_var` or `std::env::remove_var`.
///
/// # Examples
///
/// ```rust,ignore
/// // PBEAM_GETENV_EQUALS=x=y
/// assert_eq!(variable(b"PBEAM_GETENV_EQUALS"), Some(b"x=y".to_vec()));
/// assert_eq!(variable(b"PBEAM_GETENV_MISSING"), None);
/// ```
pub(crate) fn variable(key: &[u8]) -> Option<Vec<u8>> {
    find(entries(), key).map(<[u8]>::to_vec)
}

/// Return the entries of `environ` in their order, each as its bytes
/// without the NUL byte.
///
/// # Examples
///
/// ```rust,ignore
/// // HOME=/root and PATH=/bin
/// assert_eq!(entries().collect::<Vec<_>>(), [&b"HOME=/root"[..], b"PATH=/bin"]);
/// ```
fn entries() -> impl Iterator<Item = &'static [u8]> {
    let mut entry = environ();
    std::iter::from_fn(move || {
        // SAFETY: `environ` is an array of pointers to NUL-terminated
        // strings, and a null pointer ends it. PBeam never changes its
        // environment, so the array and its strings stay valid and do not
        // change while the process runs. Thus the walk stops at the null
        // pointer, and each string stays valid for `'static`.
        unsafe {
            let variable = *entry;
            if variable.is_null() {
                return None;
            }
            entry = entry.add(1);
            Some(CStr::from_ptr(variable).to_bytes())
        }
    })
}

/// Return the `environ` array of macOS, which a dynamic library reads
/// with `_NSGetEnviron`.
///
/// # Examples
///
/// ```rust,ignore
/// assert!(!environ().is_null());
/// ```
#[cfg(target_os = "macos")]
fn environ() -> *const *const c_char {
    // SAFETY: `_NSGetEnviron` returns a valid pointer to the `environ`
    // variable of the process.
    unsafe { (*libc::_NSGetEnviron()).cast() }
}

/// Return the `environ` array of the C library.
///
/// # Examples
///
/// ```rust,ignore
/// assert!(!environ().is_null());
/// ```
#[cfg(not(target_os = "macos"))]
fn environ() -> *const *const c_char {
    // SAFETY: PBeam never changes its environment, so no other thread
    // writes `environ` while this function reads it.
    unsafe { libc::environ.cast() }
}

/// Return the value of the last entry of `entries` whose bytes before the
/// first `=` are `key`. An entry without `=` has no key, so it is skipped.
///
/// # Examples
///
/// ```rust,ignore
/// let entries = [&b"A=1"[..], b"A=2", b"=lead=x", b"NOEQ"];
/// assert_eq!(find(entries, b"A"), Some(&b"2"[..]));
/// assert_eq!(find(entries, b""), Some(&b"lead=x"[..]));
/// assert_eq!(find(entries, b"NOEQ"), None);
/// ```
fn find<'a>(entries: impl IntoIterator<Item = &'a [u8]>, key: &[u8]) -> Option<&'a [u8]> {
    entries
        .into_iter()
        .filter_map(|entry| {
            let separator = entry.iter().position(|&byte| byte == b'=')?;
            (&entry[..separator] == key).then(|| &entry[separator + 1..])
        })
        .last()
}

#[cfg(test)]
mod tests {
    use super::find;

    #[test]
    fn finds_the_last_entry_of_a_key_as_the_erts_copy_does() {
        // Measured on ERTS 29.1.1: `erlexec` started with the raw
        // environment `PBEAM_DUP=first`, `PBEAM_DUP=second`, and
        // `=lead=x` gives "second" for os:getenv("PBEAM_DUP") and "lead=x"
        // for os:getenv(""). A `Command` cannot make a duplicate entry,
        // and the `erl` script removes these entries, so this test gives
        // the raw entries to `find`.
        let entries = [&b"A=1"[..], b"A=2", b"=lead=x", b"NOEQ", b"=x"];
        assert_eq!(find(entries, b"A"), Some(&b"2"[..]));
        assert_eq!(find(entries, b""), Some(&b"x"[..]));
        assert_eq!(
            find(entries[..3].iter().copied(), b""),
            Some(&b"lead=x"[..])
        );
        assert_eq!(find(entries, b"NOEQ"), None);
        assert_eq!(find(entries, b"B"), None);
    }
}

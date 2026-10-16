//! Read and change the environment of the run as ERTS keeps its copy of the
//! environment of the host (`sys/unix/sys_env.c`, `sys/common/erl_osenv.c`).

use std::ffi::{CStr, c_char};
use std::sync::{LazyLock, RwLock};

/// The environment of the run: the ERTS global environment copy, which
/// `import_initial_env` (`sys_env.c:134-154`) makes from `environ` at
/// emulator start. One entry holds a key and its value, and a later entry
/// of a key replaces an earlier entry (`erts_osenv_put_native`,
/// `erl_osenv.c:360-385`). `os:getenv/1` and `os:env/0` read this copy, and
/// `os:putenv/2` and `os:unsetenv/1` change it, as `erts_sys_global_osenv`
/// gives it to `os_getenv_1` and `os_putenv_2`
/// (`erts/emulator/beam/erl_bif_os.c:82-142`). ERTS holds a read-write lock
/// for the copy (`sysenv_rwmtx`, `sys_env.c:63-82`), which PBeam matches.
///
/// ERTS gives the child of a port a new environment from this copy
/// (`erts_osenv_export`). PBeam gives each child the `environ` of the
/// process instead (see [`environ`]), so a child of PBeam does not see the
/// change of `os:putenv/2` (row "OS environment" of
/// `docs/erts-differences.md`).
static ENVIRONMENT: LazyLock<RwLock<Entries>> = LazyLock::new(|| RwLock::new(import()));

/// The key and the value of one entry of the environment.
type Entry = (Vec<u8>, Vec<u8>);

/// The entries of the environment, one for each key.
type Entries = Vec<Entry>;

/// Return the value of the environment variable `key` as raw bytes, or
/// `None` when the environment has no entry with the key. Each entry of the
/// copy splits at its first `=`, an entry without `=` is skipped, and a
/// later entry replaces an earlier entry with the same key
/// (`import_initial_env`, `sys_env.c:134-154`; `erts_osenv_put_native`,
/// `erl_osenv.c:360-385`). Thus the result can be different from libc
/// `getenv` and `std::env::var_os`, which return the first entry of a key.
///
/// # Examples
///
/// ```rust,ignore
/// // PBEAM_GETENV_EQUALS=x=y
/// assert_eq!(variable(b"PBEAM_GETENV_EQUALS"), Some(b"x=y".to_vec()));
/// assert_eq!(variable(b"PBEAM_GETENV_MISSING"), None);
/// ```
pub(crate) fn variable(key: &[u8]) -> Option<Vec<u8>> {
    entries()
        .iter()
        .find(|(name, _)| name == key)
        .map(|(_, value)| value.clone())
}

/// Return the key and the value of each entry of the environment, in the
/// order of [`ENVIRONMENT`]. ERTS iterates its copy with an unordered walk
/// of a red-black tree (`ERTS_RBT_NEED_FOREACH_UNORDERED__`,
/// `erl_rbtree.h:535-539`), so `os:env/0` does not give an order either.
///
/// # Examples
///
/// ```rust,ignore
/// // HOME=/root and PATH=/bin
/// assert_eq!(variables(), [(b"HOME".to_vec(), b"/root".to_vec()), (b"PATH".to_vec(), b"/bin".to_vec())]);
/// ```
pub(crate) fn variables() -> Entries {
    entries().clone()
}

/// Set the environment variable `key` to `value`, as `os_putenv_2`
/// (`erl_bif_os.c:113-127`) does with `erts_osenv_put_native`
/// (`erl_osenv.c:360-385`): a known key takes the new value in its place,
/// and a new key joins the end of the copy. The caller checks `key` and
/// `value` first.
///
/// # Examples
///
/// ```rust,ignore
/// put(b"PBEAM_PUT", b"1");
/// assert_eq!(variable(b"PBEAM_PUT"), Some(b"1".to_vec()));
/// ```
pub(crate) fn put(key: &[u8], value: &[u8]) {
    set(&mut write(), key, value.to_vec());
}

/// Give `key` the value `value` in `entries`: a known key takes the value
/// in its place, and a new key joins the end, as `erts_osenv_put_native`
/// (`erl_osenv.c:360-385`) changes a node of its tree.
///
/// # Examples
///
/// ```rust,ignore
/// let mut entries = vec![(b"A".to_vec(), b"1".to_vec())];
/// set(&mut entries, b"A", b"2".to_vec());
/// assert_eq!(entries, [(b"A".to_vec(), b"2".to_vec())]);
/// ```
fn set(entries: &mut Entries, key: &[u8], value: Vec<u8>) {
    match entries.iter_mut().find(|(name, _)| name == key) {
        Some(entry) => entry.1 = value,
        None => entries.push((key.to_vec(), value)),
    }
}

/// Remove the environment variable `key`, as `os_unsetenv_1`
/// (`erl_bif_os.c:129-142`) does with `erts_osenv_unset_native`. A key that
/// the environment does not hold changes nothing.
///
/// # Examples
///
/// ```rust,ignore
/// unset(b"PBEAM_PUT");
/// assert_eq!(variable(b"PBEAM_PUT"), None);
/// ```
pub(crate) fn unset(key: &[u8]) {
    write().retain(|(name, _)| name != key);
}

/// Borrow the entries of [`ENVIRONMENT`] for a read.
///
/// # Panics
///
/// Panic when another thread held the lock during a panic, which the
/// runtime does not do.
fn entries() -> std::sync::RwLockReadGuard<'static, Entries> {
    ENVIRONMENT
        .read()
        .expect("the environment lock is not poisoned")
}

/// Borrow the entries of [`ENVIRONMENT`] for a write.
///
/// # Panics
///
/// Panic when another thread held the lock during a panic, which the
/// runtime does not do.
fn write() -> std::sync::RwLockWriteGuard<'static, Entries> {
    ENVIRONMENT
        .write()
        .expect("the environment lock is not poisoned")
}

/// Return the copy of `environ` that [`ENVIRONMENT`] holds: one entry for
/// each key in the order of the first entry of that key, as
/// `import_initial_env` (`sys_env.c:134-154`) makes it with
/// `erts_osenv_put_native` (`erl_osenv.c:360-385`).
///
/// # Examples
///
/// ```rust,ignore
/// // HOME=/root and PATH=/bin
/// assert_eq!(import(), [(b"HOME".to_vec(), b"/root".to_vec()), (b"PATH".to_vec(), b"/bin".to_vec())]);
/// ```
fn import() -> Entries {
    let mut entries = Vec::new();
    for (key, value) in entries_of_environ().filter_map(split) {
        set(&mut entries, &key, value);
    }
    entries
}

/// Return the key and the value of one entry of `environ`, or `None` for an
/// entry without `=`.
///
/// # Examples
///
/// ```rust,ignore
/// assert_eq!(split(b"A=1"), Some((b"A".to_vec(), b"1".to_vec())));
/// assert_eq!(split(b"NOEQ"), None);
/// ```
fn split(entry: &[u8]) -> Option<Entry> {
    let separator = entry.iter().position(|&byte| byte == b'=')?;
    Some((entry[..separator].to_vec(), entry[separator + 1..].to_vec()))
}

/// Return the entries of `environ` in their order, each as its bytes
/// without the NUL byte.
///
/// # Examples
///
/// ```rust,ignore
/// // HOME=/root and PATH=/bin
/// assert_eq!(entries_of_environ().collect::<Vec<_>>(), [&b"HOME=/root"[..], b"PATH=/bin"]);
/// ```
fn entries_of_environ() -> impl Iterator<Item = &'static [u8]> {
    let mut entry = environ();
    std::iter::from_fn(move || {
        // SAFETY: `environ` is an array of pointers to NUL-terminated
        // strings, and a null pointer ends it. PBeam changes the copy of the
        // environment and not `environ`, so the array and its strings stay
        // valid and do not change while the process runs. Thus the walk
        // stops at the null pointer, and each string stays valid for
        // `'static`.
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
pub(super) fn environ() -> *const *const c_char {
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
pub(super) fn environ() -> *const *const c_char {
    // SAFETY: PBeam changes the copy of the environment and not `environ`,
    // so no other thread writes `environ` while this function reads it.
    unsafe { libc::environ.cast() }
}

#[cfg(test)]
#[path = "../../../tests/runtime/host/environment.rs"]
mod tests;

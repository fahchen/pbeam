//! Read the host system information that ERTS reads at emulator start
//! (`sys/unix/sys.c`), and the host terminal that the `prim_tty` NIFs use.

use std::ffi::CStr;

pub(crate) mod errno;
pub(crate) mod terminal;

/// Return the OS flavor of the host: the `uname` system name with each
/// ASCII uppercase letter in lowercase, as ERTS `os_flavor`
/// (`sys/unix/sys.c:855-865`) does. ERTS makes the second element of
/// `erlang:system_info(os_type)` from this name, as a Latin-1 atom
/// (`os_info_init` in `erl_bif_info.c:6418-6440`), so each byte gives one
/// character.
///
/// ERTS ignores a failure of `uname`. Here a failure gives an empty name,
/// because the buffer starts with zeros.
///
/// # Examples
///
/// ```rust,ignore
/// // macOS: `uname -s` prints `Darwin`.
/// assert_eq!(os_flavor(), "darwin");
/// // Linux: `uname -s` prints `Linux`.
/// assert_eq!(os_flavor(), "linux");
/// ```
pub(crate) fn os_flavor() -> String {
    // SAFETY: `utsname` is a C struct of `c_char` arrays, so all zeros is a
    // valid value. `uname` writes only into the struct that the pointer
    // gives, and each name that it writes ends with a NUL byte. When `uname`
    // fails, the zeros of `sysname` give an empty NUL-terminated string. So
    // `CStr::from_ptr` reads inside `sysname`.
    let sysname = unsafe {
        let mut name: libc::utsname = std::mem::zeroed();
        libc::uname(&raw mut name);
        CStr::from_ptr(name.sysname.as_ptr()).to_bytes().to_vec()
    };
    sysname
        .into_iter()
        .map(|byte| char::from(byte.to_ascii_lowercase()))
        .collect()
}

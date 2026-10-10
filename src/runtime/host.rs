//! Read the host system information that ERTS reads at emulator start
//! (`sys/unix/sys.c`, `sys/common/erl_sys_common_misc.c`), the host
//! terminal that the `prim_tty` NIFs use, read, and write to, the host signal
//! dispositions that `os:set_signal/2` sets, the host clocks that the
//! time BIFs read, the OS environment that `os:getenv/1` reads, the
//! external programs that `open_port/2` starts, and the file information
//! that the `prim_file` NIFs read.

use std::{ffi::CStr, sync::OnceLock};

pub(crate) mod clock;
pub(crate) mod environment;
pub(crate) mod errno;
pub(crate) mod file;
pub(crate) mod input;
pub(crate) mod output;
pub(crate) mod signal;
pub(crate) mod spawn;
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

/// The encoding of file names on the host, as the ERTS
/// `filename_encoding` (`erl_sys_common_misc.c:50-56`). ERTS on Windows
/// uses `ERL_FILENAME_WIN_WCHAR`. PBeam supports neither Windows nor the
/// `+fnl`, `+fnu`, and `+fna` emulator flags.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum FilenameEncoding {
    /// `ERL_FILENAME_LATIN1`: each character of a name is one byte. Only a
    /// host other than macOS uses it.
    #[cfg_attr(target_os = "macos", allow(dead_code))]
    Latin1,
    /// `ERL_FILENAME_UTF8`: a name is UTF-8. Only a host other than macOS
    /// uses it.
    #[cfg_attr(target_os = "macos", allow(dead_code))]
    Utf8,
    /// `ERL_FILENAME_UTF8_MAC`: a name is UTF-8, and only macOS uses it.
    /// ERTS checks and writes a name as `ERL_FILENAME_UTF8` does
    /// (`erl_unicode.c:2222-2555`), but it composes the code points of a
    /// native name that it reads (`erl_unicode.c:2177-2190`).
    #[cfg_attr(not(target_os = "macos"), allow(dead_code))]
    Utf8Mac,
}

/// Return the file name encoding of the host. ERTS sets it once at
/// emulator start (`erts_init_sys_common_misc` in
/// `erl_sys_common_misc.c:89-126`), so the first call reads it and each
/// later call returns the same value.
///
/// On macOS the encoding is always UTF-8 with composition
/// (`erl_sys_common_misc.c:54-56`, `120-124`). On another host it is UTF-8 when the `LC_CTYPE` locale of
/// the environment exists and its codeset is `UTF-8`, else Latin-1
/// (`erl_sys_common_misc.c:95-119`). ERTS reads the locale with
/// `setlocale(LC_CTYPE, "")`, which also sets the locale of the process.
/// Here `newlocale` reads the same environment variables and does not
/// change the process locale; `prim_tty:setlocale/1` changes it.
///
/// # Examples
///
/// ```rust,ignore
/// // macOS, also with `LC_ALL=C`.
/// assert_eq!(filename_encoding(), FilenameEncoding::Utf8Mac);
/// // Linux with `LC_ALL=C` or a locale that does not exist.
/// assert_eq!(filename_encoding(), FilenameEncoding::Latin1);
/// ```
pub(crate) fn filename_encoding() -> FilenameEncoding {
    static ENCODING: OnceLock<FilenameEncoding> = OnceLock::new();
    *ENCODING.get_or_init(read_filename_encoding)
}

/// Return the file name encoding of macOS, `ERL_FILENAME_UTF8_MAC`
/// (`erl_sys_common_misc.c:54-56`, `120-124`).
///
/// # Examples
///
/// ```rust,ignore
/// assert_eq!(read_filename_encoding(), FilenameEncoding::Utf8Mac);
/// ```
#[cfg(target_os = "macos")]
fn read_filename_encoding() -> FilenameEncoding {
    FilenameEncoding::Utf8Mac
}

/// Return the file name encoding from the `LC_CTYPE` codeset of the
/// environment (`erl_sys_common_misc.c:95-119`). A locale that does not
/// exist gives Latin-1, as a failed `setlocale` does in ERTS.
///
/// # Examples
///
/// ```rust,ignore
/// // LC_ALL=C.UTF-8
/// assert_eq!(read_filename_encoding(), FilenameEncoding::Utf8);
/// // LC_ALL=pbeam-unknown
/// assert_eq!(read_filename_encoding(), FilenameEncoding::Latin1);
/// ```
#[cfg(not(target_os = "macos"))]
fn read_filename_encoding() -> FilenameEncoding {
    // SAFETY: the locale name is a valid C string, and a null base asks
    // `newlocale` for a new locale object. `nl_langinfo_l` returns a valid
    // C string for the locale object, which `CStr::from_ptr` reads before
    // `freelocale` frees the object.
    let utf8 = unsafe {
        let locale = libc::newlocale(libc::LC_CTYPE_MASK, c"".as_ptr(), std::ptr::null_mut());
        if locale.is_null() {
            return FilenameEncoding::Latin1;
        }
        let utf8 = CStr::from_ptr(libc::nl_langinfo_l(libc::CODESET, locale)) == c"UTF-8";
        libc::freelocale(locale);
        utf8
    };
    if utf8 {
        FilenameEncoding::Utf8
    } else {
        FilenameEncoding::Latin1
    }
}

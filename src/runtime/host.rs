//! Read the host system information that ERTS reads at emulator start
//! (`sys/unix/sys.c`, `sys/common/erl_sys_common_misc.c`), the host
//! terminal that the `prim_tty` NIFs use, read, and write to, the host signal
//! dispositions that `os:set_signal/2` sets, the host clocks that the
//! time BIFs read, the OS environment that `os:getenv/1` reads, the
//! external programs that `open_port/2` starts, and the file information
//! that the `prim_file` NIFs read.

use std::sync::OnceLock;

pub(crate) mod clock;
pub(crate) mod environment;
pub(crate) mod errno;
pub(crate) mod file;
pub(crate) mod gethost;
pub(crate) mod ifaddrs;
pub(crate) mod input;
pub(crate) mod netdb;
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
    name_bytes(&uname().sysname)
        .into_iter()
        .map(|byte| char::from(byte.to_ascii_lowercase()))
        .collect()
}

/// Return the name of the host, which the `GETHOSTNAME` request of
/// `inet_drv.c` reads with `gethostname` (inet_drv.c:11219-11233): the
/// `uname` node name. macOS and Linux give `gethostname` and the node name
/// from the same kernel value (`kern.hostname` on macOS, the UTS name on
/// Linux), so the bytes are the same. A failure of `uname` gives an empty
/// name, as for [`os_flavor`].
///
/// # Examples
///
/// ```rust,ignore
/// // `hostname` prints `philz-m1`.
/// assert_eq!(host_name(), b"philz-m1");
/// ```
pub(crate) fn host_name() -> Vec<u8> {
    name_bytes(&uname().nodename)
}

/// Return the `uname` release of the host as the numbers `{Major, Minor,
/// Build}` of `erlang:system_info(os_version)`, as `os_version` and
/// `get_number` of `sys/unix/sys.c:838-885` read it: each number is the
/// `atoi` value of the text before the next dot, and a part that does not
/// start with a digit gives 0 and does not move past its text.
pub(crate) fn os_version() -> [i32; 3] {
    release_numbers(&name_bytes(&uname().release))
}

/// Return the three numbers of the `uname` release text `release`, as
/// `get_number` of `sys/unix/sys.c:860-885` reads them.
fn release_numbers(release: &[u8]) -> [i32; 3] {
    let mut rest = release;
    [(); 3].map(|()| {
        if !rest.first().is_some_and(u8::is_ascii_digit) {
            return 0;
        }
        let (part, after) = match rest.iter().position(|&byte| byte == b'.') {
            Some(dot) => (&rest[..dot], &rest[dot + 1..]),
            None => (rest, &rest[rest.len()..]),
        };
        rest = after;
        // `atoi` reads the leading digits into a C `int`.
        part.iter()
            .take_while(|byte| byte.is_ascii_digit())
            .fold(0i32, |value, &digit| {
                value.wrapping_mul(10).wrapping_add(i32::from(digit - b'0'))
            })
    })
}

/// Return the `uname` names of the host. A failure of `uname` leaves each
/// name empty.
fn uname() -> libc::utsname {
    // SAFETY: `utsname` is a C struct of `c_char` arrays, so all zeros is a
    // valid value. `uname` writes only into the struct that the pointer
    // gives.
    unsafe {
        let mut name: libc::utsname = std::mem::zeroed();
        libc::uname(&raw mut name);
        name
    }
}

/// Return the bytes of a `uname` name before its first NUL byte, or all
/// bytes of a name without a NUL byte.
fn name_bytes(name: &[libc::c_char]) -> Vec<u8> {
    name.iter()
        .map(|&byte| byte.to_ne_bytes()[0])
        .take_while(|&byte| byte != 0)
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
    #[cfg_attr(
        all(target_os = "macos", not(test)),
        expect(dead_code, reason = "only a host other than macOS uses it")
    )]
    Latin1,
    /// `ERL_FILENAME_UTF8`: a name is UTF-8. Only a host other than macOS
    /// uses it.
    #[cfg_attr(
        all(target_os = "macos", not(test)),
        expect(dead_code, reason = "only a host other than macOS uses it")
    )]
    Utf8,
    /// `ERL_FILENAME_UTF8_MAC`: a name is UTF-8, and only macOS uses it.
    /// ERTS checks and writes a name as `ERL_FILENAME_UTF8` does
    /// (`erl_unicode.c:2222-2555`), but it composes the code points of a
    /// native name that it reads (`erl_unicode.c:2177-2190`).
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
        let utf8 = std::ffi::CStr::from_ptr(libc::nl_langinfo_l(libc::CODESET, locale)) == c"UTF-8";
        libc::freelocale(locale);
        utf8
    };
    if utf8 {
        FilenameEncoding::Utf8
    } else {
        FilenameEncoding::Latin1
    }
}

#[cfg(test)]
#[path = "../../tests/runtime/host.rs"]
mod tests;

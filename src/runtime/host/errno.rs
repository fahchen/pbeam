//! Name an errno value as the ERTS `erl_errno_id`
//! (`erts/emulator/beam/erl_errno_str.c:1228-1309`) does.

use std::ffi::c_int;

/// The cases of the ERTS `errno_name` switch (`erl_errno_str.c:986-1105`),
/// in its order. Some names have the same value on a host: `EWOULDBLOCK` is
/// `EAGAIN` on macOS and Linux, and `EOPNOTSUPP` is `ENOTSUP` on Linux.
/// ERTS compiles only the first case of such a pair, so the first match
/// gives its name.
const COLLISIONS: [(c_int, &str); 23] = [
    (libc::EAGAIN, "EAGAIN"),
    (libc::EALREADY, "EALREADY"),
    (libc::EBUSY, "EBUSY"),
    (libc::ECONNREFUSED, "ECONNREFUSED"),
    (libc::EDEADLK, "EDEADLK"),
    (libc::EEXIST, "EEXIST"),
    (libc::EIDRM, "EIDRM"),
    (libc::EINPROGRESS, "EINPROGRESS"),
    (libc::ELOOP, "ELOOP"),
    (libc::ENAMETOOLONG, "ENAMETOOLONG"),
    (libc::ENOBUFS, "ENOBUFS"),
    (libc::ENOSTR, "ENOSTR"),
    (libc::ENODATA, "ENODATA"),
    (libc::ENOENT, "ENOENT"),
    (libc::ENOLCK, "ENOLCK"),
    (libc::ENOSR, "ENOSR"),
    (libc::ENOTEMPTY, "ENOTEMPTY"),
    (libc::ENOTTY, "ENOTTY"),
    (libc::ENOTSUP, "ENOTSUP"),
    (libc::EOPNOTSUPP, "EOPNOTSUPP"),
    (libc::ETIME, "ETIME"),
    (libc::ETIMEDOUT, "ETIMEDOUT"),
    (libc::EWOULDBLOCK, "EWOULDBLOCK"),
];

/// The cases of the ERTS `errno_name_fallback` switch
/// (`erl_errno_str.c:397-975`) that macOS defines, in its order. ERTS uses
/// this switch on a host without `strerrorname_np`.
#[cfg(target_os = "macos")]
const FALLBACK: [(c_int, &str); 85] = [
    (libc::EPERM, "EPERM"),
    (libc::ESRCH, "ESRCH"),
    (libc::EINTR, "EINTR"),
    (libc::EIO, "EIO"),
    (libc::ENXIO, "ENXIO"),
    (libc::E2BIG, "E2BIG"),
    (libc::ENOEXEC, "ENOEXEC"),
    (libc::EBADF, "EBADF"),
    (libc::ECHILD, "ECHILD"),
    (libc::ENOMEM, "ENOMEM"),
    (libc::EACCES, "EACCES"),
    (libc::EFAULT, "EFAULT"),
    (libc::ENOTBLK, "ENOTBLK"),
    (libc::EXDEV, "EXDEV"),
    (libc::ENODEV, "ENODEV"),
    (libc::ENOTDIR, "ENOTDIR"),
    (libc::EISDIR, "EISDIR"),
    (libc::EINVAL, "EINVAL"),
    (libc::ENFILE, "ENFILE"),
    (libc::EMFILE, "EMFILE"),
    (libc::ETXTBSY, "ETXTBSY"),
    (libc::EFBIG, "EFBIG"),
    (libc::ENOSPC, "ENOSPC"),
    (libc::ESPIPE, "ESPIPE"),
    (libc::EROFS, "EROFS"),
    (libc::EMLINK, "EMLINK"),
    (libc::EPIPE, "EPIPE"),
    (libc::EDOM, "EDOM"),
    (libc::ERANGE, "ERANGE"),
    (libc::ENOTSOCK, "ENOTSOCK"),
    (libc::EDESTADDRREQ, "EDESTADDRREQ"),
    (libc::EMSGSIZE, "EMSGSIZE"),
    (libc::EPROTOTYPE, "EPROTOTYPE"),
    (libc::ENOPROTOOPT, "ENOPROTOOPT"),
    (libc::EPROTONOSUPPORT, "EPROTONOSUPPORT"),
    (libc::ESOCKTNOSUPPORT, "ESOCKTNOSUPPORT"),
    (libc::EPFNOSUPPORT, "EPFNOSUPPORT"),
    (libc::EAFNOSUPPORT, "EAFNOSUPPORT"),
    (libc::EADDRINUSE, "EADDRINUSE"),
    (libc::EADDRNOTAVAIL, "EADDRNOTAVAIL"),
    (libc::ENETDOWN, "ENETDOWN"),
    (libc::ENETUNREACH, "ENETUNREACH"),
    (libc::ENETRESET, "ENETRESET"),
    (libc::ECONNABORTED, "ECONNABORTED"),
    (libc::ECONNRESET, "ECONNRESET"),
    (libc::EISCONN, "EISCONN"),
    (libc::ENOTCONN, "ENOTCONN"),
    (libc::ESHUTDOWN, "ESHUTDOWN"),
    (libc::ETOOMANYREFS, "ETOOMANYREFS"),
    (libc::EHOSTDOWN, "EHOSTDOWN"),
    (libc::EHOSTUNREACH, "EHOSTUNREACH"),
    (libc::EPROCLIM, "EPROCLIM"),
    (libc::EUSERS, "EUSERS"),
    (libc::EDQUOT, "EDQUOT"),
    (libc::ESTALE, "ESTALE"),
    (libc::EREMOTE, "EREMOTE"),
    (libc::EBADRPC, "EBADRPC"),
    (libc::ERPCMISMATCH, "ERPCMISMATCH"),
    (libc::EPROGUNAVAIL, "EPROGUNAVAIL"),
    (libc::EPROGMISMATCH, "EPROGMISMATCH"),
    (libc::EPROCUNAVAIL, "EPROCUNAVAIL"),
    (libc::ENOSYS, "ENOSYS"),
    (libc::EFTYPE, "EFTYPE"),
    (libc::EAUTH, "EAUTH"),
    (libc::ENEEDAUTH, "ENEEDAUTH"),
    (libc::EPWROFF, "EPWROFF"),
    (libc::EDEVERR, "EDEVERR"),
    (libc::EOVERFLOW, "EOVERFLOW"),
    (libc::EBADEXEC, "EBADEXEC"),
    (libc::EBADARCH, "EBADARCH"),
    (libc::ESHLIBVERS, "ESHLIBVERS"),
    (libc::EBADMACHO, "EBADMACHO"),
    (libc::ECANCELED, "ECANCELED"),
    (libc::ENOMSG, "ENOMSG"),
    (libc::EILSEQ, "EILSEQ"),
    (libc::ENOATTR, "ENOATTR"),
    (libc::EBADMSG, "EBADMSG"),
    (libc::EMULTIHOP, "EMULTIHOP"),
    (libc::ENOLINK, "ENOLINK"),
    (libc::EPROTO, "EPROTO"),
    (libc::ENOPOLICY, "ENOPOLICY"),
    (libc::ENOTRECOVERABLE, "ENOTRECOVERABLE"),
    (libc::EOWNERDEAD, "EOWNERDEAD"),
    (libc::EQFULL, "EQFULL"),
    (libc::ENOTCAPABLE, "ENOTCAPABLE"),
];

/// Return the ERTS atom name of `errno`: `errno_0` for 0, the lowercase
/// errno name, or `errno_N` for a value without a name
/// (`erl_errno_str.c:47`, `106-124`, `1161-1163`). ERTS takes a name from
/// its `errno_name` switch first, then from `strerrorname_np` on glibc or
/// from `errno_name_fallback` on macOS.
///
/// # Examples
///
/// ```rust,ignore
/// assert_eq!(errno_id(0), "errno_0");
/// assert_eq!(errno_id(libc::EWOULDBLOCK), "eagain");
/// assert_eq!(errno_id(libc::EPERM), "eperm");
/// assert_eq!(errno_id(-1), "errno_-1");
/// ```
pub(crate) fn errno_id(errno: c_int) -> String {
    // ERTS writes `errno_0` for 0 and does not look up a name.
    let name = match errno {
        0 => None,
        _ => COLLISIONS
            .iter()
            .find(|(value, _)| *value == errno)
            .map(|(_, name)| *name)
            .or_else(|| host_name(errno)),
    };
    name.map_or_else(|| format!("errno_{errno}"), str::to_ascii_lowercase)
}

/// Return the glibc name of `errno` from `strerrorname_np`, or `None` for a
/// value without a name.
///
/// # Examples
///
/// ```rust,ignore
/// assert_eq!(host_name(libc::EPERM), Some("EPERM"));
/// assert_eq!(host_name(10000), None);
/// ```
#[cfg(all(target_os = "linux", target_env = "gnu"))]
fn host_name(errno: c_int) -> Option<&'static str> {
    unsafe extern "C" {
        fn strerrorname_np(errnum: c_int) -> *const std::ffi::c_char;
    }
    // SAFETY: `strerrorname_np` returns null or a static NUL-terminated
    // string.
    let name = unsafe { strerrorname_np(errno) };
    // SAFETY: a name that is not null is a static C string.
    (!name.is_null()).then(|| unsafe { std::ffi::CStr::from_ptr(name) }.to_str().ok())?
}

/// Return the `errno_name_fallback` name of `errno`, or `None` for a value
/// without a name.
///
/// # Examples
///
/// ```rust,ignore
/// assert_eq!(host_name(libc::EPERM), Some("EPERM"));
/// assert_eq!(host_name(10000), None);
/// ```
#[cfg(target_os = "macos")]
fn host_name(errno: c_int) -> Option<&'static str> {
    FALLBACK
        .iter()
        .find(|(value, _)| *value == errno)
        .map(|(_, name)| *name)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_errno_values_as_erts() {
        // `errno_0` is the name of 0 (erl_errno_str.c:1161-1163). ENOENT and
        // EAGAIN are cases of `errno_name`, and EWOULDBLOCK has the value of
        // EAGAIN, so it gives the first name. EPERM is not a case of
        // `errno_name`, so the host lookup gives it. 10000 and -1 have no
        // name, so they give `errno_N` (erl_errno_str.c:47).
        for (errno, name) in [
            (0, "errno_0"),
            (libc::ENOENT, "enoent"),
            (libc::EAGAIN, "eagain"),
            (libc::EWOULDBLOCK, "eagain"),
            (libc::EPERM, "eperm"),
            (10000, "errno_10000"),
            (-1, "errno_-1"),
        ] {
            assert_eq!(errno_id(errno), name, "errno {errno}");
        }
    }
}

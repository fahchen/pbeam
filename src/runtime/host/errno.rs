//! Read the `errno` of the last host call, and name an errno value as the
//! ERTS `erl_errno_id` (`erts/emulator/beam/erl_errno_str.c:1228-1309`)
//! does.

use std::ffi::c_int;

/// Return the `errno` of the last host call of this thread.
///
/// # Examples
///
/// ```rust,ignore
/// // SAFETY: `isatty` of fd -1 sets `errno` to `EBADF`.
/// unsafe { libc::isatty(-1) };
/// assert_eq!(errno(), libc::EBADF);
/// ```
pub(crate) fn errno() -> c_int {
    std::io::Error::last_os_error()
        .raw_os_error()
        .unwrap_or_default()
}

/// The cases of the ERTS `errno_name` switch (`erl_errno_str.c:986-1105`),
/// in its order, each with the names of its `#if` guard. Some names have
/// the same value on a host: `EWOULDBLOCK` is `EAGAIN` on macOS and Linux,
/// and `EOPNOTSUPP` is `ENOTSUP` on Linux. ERTS does not compile a case
/// when one of its guard names has the same value, so the other name of
/// the pair gives the name. The guard does not always drop the later case:
/// for `ENOSTR` and `ENOTTY`, ERTS drops `ENOSTR`.
const COLLISIONS: [(c_int, &str, &[c_int]); 23] = [
    (libc::EAGAIN, "EAGAIN", &[]),
    (libc::EALREADY, "EALREADY", &[libc::EBUSY]),
    (libc::EBUSY, "EBUSY", &[]),
    (libc::ECONNREFUSED, "ECONNREFUSED", &[]),
    (libc::EDEADLK, "EDEADLK", &[libc::EWOULDBLOCK]),
    (libc::EEXIST, "EEXIST", &[]),
    (libc::EIDRM, "EIDRM", &[libc::EINPROGRESS]),
    (libc::EINPROGRESS, "EINPROGRESS", &[]),
    (libc::ELOOP, "ELOOP", &[libc::ENOENT]),
    (libc::ENAMETOOLONG, "ENAMETOOLONG", &[]),
    (libc::ENOBUFS, "ENOBUFS", &[libc::ENOSR]),
    (libc::ENOSTR, "ENOSTR", &[libc::ENOTTY]),
    (libc::ENODATA, "ENODATA", &[libc::ECONNREFUSED]),
    (libc::ENOENT, "ENOENT", &[]),
    (libc::ENOLCK, "ENOLCK", &[]),
    (libc::ENOSR, "ENOSR", &[libc::ENAMETOOLONG]),
    (libc::ENOTEMPTY, "ENOTEMPTY", &[libc::EEXIST]),
    (libc::ENOTTY, "ENOTTY", &[]),
    (libc::ENOTSUP, "ENOTSUP", &[]),
    (libc::EOPNOTSUPP, "EOPNOTSUPP", &[libc::ENOTSUP]),
    (libc::ETIME, "ETIME", &[libc::ELOOP]),
    (libc::ETIMEDOUT, "ETIMEDOUT", &[libc::ENOSTR, libc::EAGAIN]),
    (libc::EWOULDBLOCK, "EWOULDBLOCK", &[libc::EAGAIN]),
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
        _ => case_name(&COLLISIONS, errno).or_else(|| host_name(errno)),
    };
    name.map_or_else(|| format!("errno_{errno}"), str::to_ascii_lowercase)
}

/// Return the name of `errno` from `cases`, a table in the form of
/// `COLLISIONS`, as the `errno_name` switch does. A case is not in the
/// switch when one of its guard names has its value. `None` is the
/// `default` branch.
///
/// # Examples
///
/// ```rust,ignore
/// assert_eq!(case_name(&[(5, "ENOSTR", &[5]), (5, "ENOTTY", &[])], 5), Some("ENOTTY"));
/// assert_eq!(case_name(&COLLISIONS, libc::EPERM), None);
/// ```
fn case_name(cases: &[(c_int, &'static str, &[c_int])], errno: c_int) -> Option<&'static str> {
    cases
        .iter()
        .find(|(value, _, guards)| *value == errno && !guards.contains(value))
        .map(|(_, name, _)| *name)
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

    #[test]
    fn drops_a_guarded_case_whose_guard_name_has_its_value() {
        // On a host where `ENOTTY` has the value of `ENOSTR`, ERTS does not
        // compile the earlier `ENOSTR` case, so the name is `ENOTTY`
        // (erl_errno_str.c:1042-1046). Without a collision, `ENOSTR` stays.
        let collision: [(c_int, &str, &[c_int]); 2] = [(5, "ENOSTR", &[5]), (5, "ENOTTY", &[])];
        assert_eq!(case_name(&collision, 5), Some("ENOTTY"));
        let distinct: [(c_int, &str, &[c_int]); 2] = [(5, "ENOSTR", &[6]), (6, "ENOTTY", &[])];
        assert_eq!(case_name(&distinct, 5), Some("ENOSTR"));
        assert_eq!(case_name(&distinct, 7), None);
    }
}

//! Read the host terminal state of an fd, the locale, and the terminfo
//! entry, as the Unix part of the ERTS `prim_tty` NIFs does
//! (`erts/emulator/nifs/common/prim_tty_nif.c`).

use std::{
    ffi::{CStr, c_char, c_int, c_void},
    ptr,
    sync::atomic::{AtomicPtr, Ordering},
};

// The curses library that the pinned ERTS links:
// `/usr/lib/libncurses.5.4.dylib` on macOS, and `libtinfo.so.6` in the
// Linux build that mise installs. The Linux name is verbatim, because a
// host without the ncurses development package has no `libtinfo.so` link.
#[cfg_attr(target_os = "macos", link(name = "ncurses"))]
#[cfg_attr(
    target_os = "linux",
    link(name = "libtinfo.so.6", kind = "dylib", modifiers = "+verbatim")
)]
unsafe extern "C" {
    #[link_name = "setupterm"]
    fn curses_setupterm(term: *const c_char, fildes: c_int, errret: *mut c_int) -> c_int;
    fn del_curterm(terminal: *mut c_void) -> c_int;
    /// The current terminal of curses, which `setupterm` sets.
    static mut cur_term: *mut c_void;
}

/// The terminal of the last successful `setupterm`, as `saved_term` of
/// `prim_tty_nif.c:775`. The next successful `setupterm` frees it.
static SAVED_TERMINAL: AtomicPtr<c_void> = AtomicPtr::new(ptr::null_mut());

/// The result of `isatty` for one fd, in the three classes that
/// `isatty_nif` gives (`prim_tty_nif.c:270-278`).
#[derive(Debug, PartialEq, Eq)]
pub(crate) enum TerminalCheck {
    /// The fd is a terminal.
    Terminal,
    /// `isatty` fails with `EINVAL` or `ENOTTY`: the fd is open, but it is
    /// not a terminal.
    NotTerminal,
    /// `isatty` fails with each other errno, such as `EBADF` for an fd that
    /// is not open.
    Error,
}

/// Check whether `fd` is a terminal with `isatty`. ERTS reads `errno`
/// directly after the call (`prim_tty_nif.c:272-277`).
///
/// # Examples
///
/// ```rust,ignore
/// // A file on stdout.
/// assert_eq!(check(libc::STDOUT_FILENO), TerminalCheck::NotTerminal);
/// assert_eq!(check(-1), TerminalCheck::Error);
/// ```
pub(crate) fn check(fd: libc::c_int) -> TerminalCheck {
    // SAFETY: `isatty` reads only the fd number, and an fd that is not open
    // gives an error, not undefined behavior.
    if unsafe { libc::isatty(fd) } != 0 {
        return TerminalCheck::Terminal;
    }
    match errno() {
        libc::EINVAL | libc::ENOTTY => TerminalCheck::NotTerminal,
        _ => TerminalCheck::Error,
    }
}

/// Set the `LC_CTYPE` locale from the environment, and return whether its
/// codeset is UTF-8, as `setlocale_nif` (`prim_tty_nif.c:766-771`) does:
/// `true` when `setlocale(LC_CTYPE, "")` succeeds and `nl_langinfo(CODESET)`
/// is `UTF-8`.
///
/// # Examples
///
/// ```rust,ignore
/// // LC_ALL=C.UTF-8
/// assert!(setlocale_utf8());
/// // LC_ALL=C
/// assert!(!setlocale_utf8());
/// ```
pub(crate) fn setlocale_utf8() -> bool {
    // SAFETY: both arguments are valid C strings. `nl_langinfo` returns a
    // NUL-terminated string that stays valid until the next locale call.
    unsafe {
        !libc::setlocale(libc::LC_CTYPE, c"".as_ptr()).is_null()
            && CStr::from_ptr(libc::nl_langinfo(libc::CODESET)) == c"UTF-8"
    }
}

/// Read the terminfo entry of `TERM` with `setupterm`, as
/// `tty_setupterm_nif` (`prim_tty_nif.c:774-791`) does. On success, free
/// the terminal of the previous success with `del_curterm`, keep the new
/// current terminal, and return `Ok`. A failure returns the `errno` that
/// `setupterm` leaves, which ERTS reads without a reset to 0.
///
/// # Examples
///
/// ```rust,ignore
/// // TERM=dumb
/// assert_eq!(setupterm(), Ok(()));
/// // TERM=pbeam-unknown-term on macOS
/// assert_eq!(setupterm(), Err(libc::ENOENT));
/// ```
pub(crate) fn setupterm() -> Result<(), c_int> {
    let mut status = 0;
    // SAFETY: a null name makes `setupterm` read `TERM`. With a status
    // pointer, it reports a failure there and does not exit. ERTS gives the
    // fd -1 too.
    if unsafe { curses_setupterm(ptr::null(), -1, &raw mut status) } < 0 {
        return Err(errno());
    }
    let saved = SAVED_TERMINAL.load(Ordering::Relaxed);
    // SAFETY: `saved` is null or the terminal of an earlier success, which
    // only this function frees, after it replaces it. The new success made
    // a new `cur_term`, so `saved` is not the current terminal.
    unsafe {
        if !saved.is_null() {
            del_curterm(saved);
        }
        SAVED_TERMINAL.store(cur_term, Ordering::Relaxed);
    }
    Ok(())
}

/// Return the `errno` of the last host call of this thread.
///
/// # Examples
///
/// ```rust,ignore
/// // SAFETY: `isatty` of fd -1 sets `errno` to `EBADF`.
/// unsafe { libc::isatty(-1) };
/// assert_eq!(errno(), libc::EBADF);
/// ```
fn errno() -> c_int {
    std::io::Error::last_os_error()
        .raw_os_error()
        .unwrap_or_default()
}

/// Return the window size of the terminal as `(columns, rows)`, as
/// `tty_window_size_nif` (`prim_tty_nif.c:1179-1189`) reads it: the
/// `TIOCGWINSZ` size of fd 0, or of `ofd` only when fd 0 fails. A count of
/// 0 and a failure of both fds give -1.
///
/// # Examples
///
/// ```rust,ignore
/// // fd 0 is `/dev/null`, and stdout is an 80 by 0 pseudo-terminal.
/// assert_eq!(window_size(libc::STDOUT_FILENO), (80, -1));
/// // Neither fd is a terminal.
/// assert_eq!(window_size(libc::STDOUT_FILENO), (-1, -1));
/// ```
pub(crate) fn window_size(ofd: libc::c_int) -> (libc::c_int, libc::c_int) {
    let count = |value: libc::c_ushort| if value > 0 { value.into() } else { -1 };
    read_window_size(libc::STDIN_FILENO)
        .or_else(|| read_window_size(ofd))
        .map_or((-1, -1), |size| (count(size.ws_col), count(size.ws_row)))
}

/// Read the `TIOCGWINSZ` window size of `fd`. Return `None` when the
/// `ioctl` fails, for example for an fd that is not a terminal.
///
/// # Examples
///
/// ```rust,ignore
/// assert!(read_window_size(-1).is_none());
/// ```
fn read_window_size(fd: libc::c_int) -> Option<libc::winsize> {
    let mut size = libc::winsize {
        ws_row: 0,
        ws_col: 0,
        ws_xpixel: 0,
        ws_ypixel: 0,
    };
    // SAFETY: `TIOCGWINSZ` writes one `winsize` to the pointer, and an fd
    // that is not a terminal gives an error, not undefined behavior.
    (unsafe { libc::ioctl(fd, libc::TIOCGWINSZ, &raw mut size) } == 0).then_some(size)
}

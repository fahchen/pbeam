//! Read the host terminal state of an fd, as the Unix part of the ERTS
//! `prim_tty` NIFs does (`erts/emulator/nifs/common/prim_tty_nif.c`).

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
    match std::io::Error::last_os_error().raw_os_error() {
        Some(libc::EINVAL | libc::ENOTTY) => TerminalCheck::NotTerminal,
        _ => TerminalCheck::Error,
    }
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

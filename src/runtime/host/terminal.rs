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

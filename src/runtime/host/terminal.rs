//! Read and set the host terminal state of an fd, and read the locale, the
//! terminfo entry, and its capabilities, as the Unix part of the ERTS
//! `prim_tty` NIFs does (`erts/emulator/nifs/common/prim_tty_nif.c`).

use std::{
    cell::RefCell,
    ffi::{CStr, c_char, c_int, c_long, c_void},
    ptr,
    sync::{
        Mutex,
        atomic::{AtomicPtr, Ordering},
    },
};

use super::errno::errno;

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
    fn tgetnum(id: *const c_char) -> c_int;
    fn tgetflag(id: *const c_char) -> c_int;
    fn wcwidth(wc: libc::wchar_t) -> c_int;
    fn tigetstr(cap: *const c_char) -> *mut c_char;
    /// `tparm` is variadic on the supported hosts (`NCURSES_TPARM_VARARGS`
    /// is defined in the curses header of macOS and of the Linux build that
    /// mise installs).
    fn tparm(cap: *const c_char, ...) -> *mut c_char;
    fn tputs(cap: *const c_char, count: c_int, putc: extern "C" fn(c_int) -> c_int) -> c_int;
    /// The name tables of ncurses, each one terminated by a null pointer.
    /// Each table is an array of `char *`, so the declaration uses a
    /// zero-length array: the address of the symbol is the address of the
    /// first entry, and a load from it would read that entry.
    static boolnames: [*const c_char; 0];
    static boolcodes: [*const c_char; 0];
    static boolfnames: [*const c_char; 0];
    static numnames: [*const c_char; 0];
    static numcodes: [*const c_char; 0];
    static numfnames: [*const c_char; 0];
    static strnames: [*const c_char; 0];
    static strcodes: [*const c_char; 0];
    static strfnames: [*const c_char; 0];
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

/// Return the numeric capability `code` of the current terminal with the
/// curses `tgetnum`, as `tty_tigetnum_nif` (`prim_tty_nif.c:841-850`)
/// does. ERTS calls the termcap function `tgetnum`, not `tigetnum`, so
/// `code` is a termcap code, such as `co` for the columns. An absent
/// capability, and each capability before a successful `setupterm`, give
/// -1.
///
/// # Examples
///
/// ```rust,ignore
/// // TERM=vt100, after setupterm()
/// assert_eq!(termcap_number(c"co"), 80);
/// assert_eq!(termcap_number(c"zz"), -1);
/// ```
pub(crate) fn termcap_number(code: &CStr) -> c_int {
    // SAFETY: `code` is a NUL-terminated string. `tgetnum` reads the
    // current terminal of curses, and gives -1 when there is none.
    unsafe { tgetnum(code.as_ptr()) }
}

/// Return whether the current terminal has the boolean capability `code`,
/// with the curses `tgetflag`, as `tty_tigetflag_nif`
/// (`prim_tty_nif.c:852-863`) does. ERTS calls the termcap function
/// `tgetflag`, not `tigetflag`, so `code` is a termcap code, such as `xn`
/// for the newline glitch. An absent capability, and each capability
/// before a successful `setupterm`, give `false`.
///
/// # Examples
///
/// ```rust,ignore
/// // TERM=vt100, after setupterm()
/// assert!(termcap_flag(c"xn"));
/// assert!(!termcap_flag(c"zz"));
/// ```
pub(crate) fn termcap_flag(code: &CStr) -> bool {
    // SAFETY: `code` is a NUL-terminated string. `tgetflag` reads the
    // current terminal of curses, and gives 0 when there is none.
    unsafe { tgetflag(code.as_ptr()) != 0 }
}

/// The result of `tigetstr` for one termcap code, as `tty_tigetstr_nif`
/// (`prim_tty_nif.c:865-881`) gives it.
#[derive(Debug, PartialEq, Eq)]
pub(crate) enum TermcapString {
    /// The current terminal has no such capability: `false`.
    Missing,
    /// The capability code is invalid: `badarg`.
    Invalid,
    /// The capability string: `{ok, Binary}`.
    Value(Vec<u8>),
}

/// Return the string capability `code` of the current terminal with the
/// curses `tigetstr`, as `tty_tigetstr_nif` (`prim_tty_nif.c:865-881`)
/// does. A null result is `false`, the pointer -1 is `badarg`, and each
/// other result is the bytes before the first NUL.
///
/// # Examples
///
/// ```rust,ignore
/// // TERM=vt100, after setupterm()
/// assert!(matches!(termcap_string(c"cup"), TermcapString::Value(_)));
/// assert_eq!(termcap_string(c"zz"), TermcapString::Missing);
/// ```
pub(crate) fn termcap_string(code: &CStr) -> TermcapString {
    // SAFETY: `code` is a NUL-terminated string. `tigetstr` reads the
    // current terminal, and its result is null, -1, or a NUL-terminated
    // string.
    unsafe {
        let result = tigetstr(code.as_ptr());
        if result.is_null() {
            TermcapString::Missing
        } else if result as isize == -1 {
            TermcapString::Invalid
        } else {
            TermcapString::Value(CStr::from_ptr(result).to_bytes().to_vec())
        }
    }
}

/// The name, the code, and the full name of one capability of a curses
/// table, as `tty_tinfo_make_map` (`prim_tty_nif.c:794-815`) writes them.
pub(crate) type TermcapEntry = (Vec<u8>, Vec<u8>, Vec<u8>);

/// Return the three capability lists of the curses library, as
/// `tty_tinfo_nif` (`prim_tty_nif.c:818-839`) does: the boolean, numeric,
/// and string capabilities, each as a [`TermcapEntry`]. The list is in the
/// reverse order of the ncurses table, as `enif_make_list_cell` prepends
/// each entry.
///
/// # Examples
///
/// ```rust,ignore
/// let [booleans, numbers, strings] = tinfo();
/// assert_eq!(booleans.len(), 44);
/// assert_eq!(numbers.len(), 39);
/// assert!(strings.len() > 400);
/// ```
pub(crate) fn tinfo() -> [Vec<TermcapEntry>; 3] {
    // SAFETY: the statics are the ncurses capability tables, which are
    // read-only after the library starts. `as_ptr` gives the address of each
    // table, and `tinfo_list` stops at the first null of each table and
    // copies the strings.
    let tables = unsafe {
        [
            (boolnames.as_ptr(), boolcodes.as_ptr(), boolfnames.as_ptr()),
            (numnames.as_ptr(), numcodes.as_ptr(), numfnames.as_ptr()),
            (strnames.as_ptr(), strcodes.as_ptr(), strfnames.as_ptr()),
        ]
    };
    tables.map(|(names, codes, full_names)| tinfo_list(names, codes, full_names))
}

/// Return the entries of the three ncurses tables `names`, `codes`, and
/// `full_names` up to the first null of each table, in the reverse order of
/// the tables.
///
/// # Examples
///
/// ```rust,ignore
/// assert!(tinfo_list(boolnames, boolcodes, boolfnames).len() == 44);
/// ```
fn tinfo_list(
    names: *const *const c_char,
    codes: *const *const c_char,
    full_names: *const *const c_char,
) -> Vec<TermcapEntry> {
    let mut entries = Vec::new();
    let mut index = 0;
    // SAFETY: the three pointers are the null-terminated ncurses tables,
    // and the loop stops at the first null of each of them.
    unsafe {
        loop {
            let name = *names.add(index);
            let code = *codes.add(index);
            let full_name = *full_names.add(index);
            if name.is_null() || code.is_null() || full_name.is_null() {
                break;
            }
            entries.push((
                CStr::from_ptr(name).to_bytes().to_vec(),
                CStr::from_ptr(code).to_bytes().to_vec(),
                CStr::from_ptr(full_name).to_bytes().to_vec(),
            ));
            index += 1;
        }
    }
    entries.reverse();
    entries
}

// The bytes that the `tputs` callback writes.
thread_local! {
    static TPUTC_BUFFER: RefCell<Vec<u8>> = const { RefCell::new(Vec::new()) };
}

/// The lock of `tparm` and `tputs`, because neither function is thread safe
/// (`prim_tty_nif.c:947-948`, `980`).
static TPUTC_LOCK: Mutex<()> = Mutex::new(());

/// The `putc` callback of `tputs` (`prim_tty_nif.c:898-919`): append the
/// byte to [`TPUTC_BUFFER`] and give 0.
extern "C" fn tputs_putc(byte: c_int) -> c_int {
    TPUTC_BUFFER.with_borrow_mut(|buffer| buffer.push(byte as u8));
    0
}

/// Emit the expansion `expanded` with `tputs`, as `tty_tputs_nif`
/// (`prim_tty_nif.c:956-986`) does: a null result of `tparm` gives its
/// `errno`, and each other result writes its bytes through the `putc`
/// callback. ERTS ignores the result of `tputs`, which fails only for a
/// null string.
///
/// # Examples
///
/// ```rust,ignore
/// assert!(emit_expanded(std::ptr::null_mut()).is_err());
/// assert_eq!(emit_expanded(c"cup".as_ptr().cast_mut()), Ok(b"cup".to_vec()));
/// ```
///
/// # Errors
///
/// The `errno` of a failed `tparm`.
fn emit_expanded(expanded: *mut c_char) -> Result<Vec<u8>, c_int> {
    if expanded.is_null() {
        return Err(errno());
    }
    // SAFETY: `expanded` is a NUL-terminated string from `tparm` or from
    // the capability code. `tputs` writes through the `putc` callback,
    // which only appends to `TPUTC_BUFFER`, and `TPUTC_LOCK` serializes it.
    unsafe {
        TPUTC_BUFFER.with_borrow_mut(Vec::clear);
        let _ = tputs(expanded, 1, tputs_putc);
        Ok(TPUTC_BUFFER.with_borrow(Clone::clone))
    }
}

/// Expand and emit the capability `code` with `params`, as
/// `tty_tputs_nif` (`prim_tty_nif.c:922-986`) does with `tparm` and
/// `tputs`: the first nine `params` replace the `tparm` parameters, an
/// empty list emits the bytes of `code` unchanged, and the result is the
/// bytes that `tputs` writes. A failed `tparm` gives its `errno`.
///
/// # Examples
///
/// ```rust,ignore
/// // TERM=vt100, after setupterm()
/// assert!(termcap_expand(c"cup", &[1, 2]).is_ok());
/// ```
///
/// # Errors
///
/// The `errno` of a failed `tparm`.
pub(crate) fn termcap_expand(code: &CStr, params: &[i64]) -> Result<Vec<u8>, c_int> {
    let _guard = TPUTC_LOCK.lock().expect("the tputs lock is not poisoned");
    let mut values = [0 as c_long; 9];
    for (value, param) in values.iter_mut().zip(params) {
        *value = *param as c_long;
    }
    // SAFETY: `code` is a NUL-terminated string. `tparm` reads the current
    // terminal and gives null or a NUL-terminated string that stays valid
    // until the next `tparm`.
    let expanded = unsafe {
        if params.is_empty() {
            code.as_ptr().cast_mut()
        } else {
            tparm(
                code.as_ptr(),
                values[0],
                values[1],
                values[2],
                values[3],
                values[4],
                values[5],
                values[6],
                values[7],
                values[8],
            )
        }
    };
    emit_expanded(expanded)
}

/// Return the width of the code point `codepoint` with `wcwidth`, as
/// `wcwidth_nif` (`prim_tty_nif.c:309-325`) does. A width of -1 is a code
/// point that is not printable.
///
/// # Examples
///
/// ```rust,ignore
/// // LC_CTYPE=C.UTF-8
/// assert_eq!(char_width('a' as c_int), 1);
/// assert_eq!(char_width(0x4e2d), 2);
/// ```
pub(crate) fn char_width(codepoint: c_int) -> c_int {
    // SAFETY: `wcwidth` reads the current locale and its tables, and it
    // only reads the argument.
    unsafe { wcwidth(codepoint as libc::wchar_t) }
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

/// The mode of a TTY resource whose output fd is not a terminal: all zeros,
/// as the `memset` of `tty_create_nif` leaves it when `tcgetattr` fails
/// (`prim_tty_nif.c:992`, `1006-1010`).
// SAFETY: `termios` is a C struct of integers and integer arrays, so all
// zeros is a valid value.
pub(crate) const EMPTY_MODE: libc::termios = unsafe { std::mem::zeroed() };

/// Read the mode of the terminal `fd` with `tcgetattr`, as `tty_create_nif`
/// (`prim_tty_nif.c:1007`) does. Return `None` when `tcgetattr` fails, for
/// example for an fd that is not a terminal.
///
/// # Examples
///
/// ```rust,ignore
/// // A file on stdout.
/// assert!(get_mode(libc::STDOUT_FILENO).is_none());
/// // A pseudo-terminal on stderr.
/// assert!(get_mode(libc::STDERR_FILENO).is_some());
/// ```
pub(crate) fn get_mode(fd: c_int) -> Option<libc::termios> {
    let mut mode = EMPTY_MODE;
    // SAFETY: `tcgetattr` writes one `termios` to the pointer, and an fd
    // that is not a terminal gives an error, not undefined behavior.
    (unsafe { libc::tcgetattr(fd, &raw mut mode) } >= 0).then_some(mode)
}

/// Set the mode of the terminal `fd` at once with `tcsetattr(fd, TCSANOW,
/// mode)`, as `tty_init_nif` (`prim_tty_nif.c:1131`) and `tty_monitor_down`
/// (`prim_tty_nif.c:1239`) do. A failure returns the `errno` of the call.
///
/// # Examples
///
/// ```rust,ignore
/// assert_eq!(set_mode(-1, &EMPTY_MODE), Err(libc::EBADF));
/// ```
pub(crate) fn set_mode(fd: c_int, mode: &libc::termios) -> Result<(), c_int> {
    // SAFETY: `tcsetattr` reads one `termios` from the pointer, and an fd
    // that is not a terminal gives an error, not undefined behavior.
    if unsafe { libc::tcsetattr(fd, libc::TCSANOW, mode) } < 0 {
        return Err(errno());
    }
    Ok(())
}

/// Return whether two modes are equal, as the `sys_memcmp` of the two
/// modes in `tty_init_nif` (`prim_tty_nif.c:1103`) finds. The comparison
/// is field by field, because a Rust copy of a mode does not keep its
/// padding bytes. Each mode that PBeam keeps comes from `tcgetattr` or from
/// a change of its fields, so the result is the same.
///
/// # Examples
///
/// ```rust,ignore
/// assert!(same_mode(&EMPTY_MODE, &EMPTY_MODE));
/// assert!(!same_mode(&EMPTY_MODE, &raw_mode(&EMPTY_MODE)));
/// ```
pub(crate) fn same_mode(left: &libc::termios, right: &libc::termios) -> bool {
    #[cfg(target_os = "linux")]
    let same_line = left.c_line == right.c_line;
    #[cfg(not(target_os = "linux"))]
    let same_line = true;
    same_line
        && left.c_iflag == right.c_iflag
        && left.c_oflag == right.c_oflag
        && left.c_cflag == right.c_cflag
        && left.c_lflag == right.c_lflag
        && left.c_cc == right.c_cc
        && left.c_ispeed == right.c_ispeed
        && left.c_ospeed == right.c_ospeed
}

/// Return the raw mode of `mode`, as `tty_init_nif` makes it for the
/// input `raw` (`prim_tty_nif.c:1108-1128`): no `ISTRIP`, `ICRNL`,
/// `ICANON`, `OPOST`, or `ECHO`, `VMIN` 1, `VTIME` 0, and `VDSUSP` 0 where
/// the host has `VDSUSP`. Linux has no `VDSUSP`.
///
/// # Examples
///
/// ```rust,ignore
/// let raw = raw_mode(&EMPTY_MODE);
/// assert_eq!((raw.c_cc[libc::VMIN], raw.c_cc[libc::VTIME]), (1, 0));
/// ```
pub(crate) fn raw_mode(mode: &libc::termios) -> libc::termios {
    let mut raw = *mode;
    raw.c_iflag &= !(libc::ISTRIP | libc::ICRNL);
    raw.c_lflag &= !(libc::ICANON | libc::ECHO);
    raw.c_oflag &= !libc::OPOST;
    raw.c_cc[libc::VMIN] = 1;
    raw.c_cc[libc::VTIME] = 0;
    #[cfg(not(target_os = "linux"))]
    {
        raw.c_cc[libc::VDSUSP] = 0;
    }
    raw
}

#[cfg(test)]
#[path = "../../../tests/runtime/host/terminal.rs"]
mod tests;

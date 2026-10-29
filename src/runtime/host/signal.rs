//! Set the host disposition of a signal, as `erts_set_signal` does for
//! `os:set_signal/2` (`sys/unix/sys.c:437-460`, `697-767`), set the
//! dispositions of the emulator start (`sys.c:799-823`), and pass each
//! handled signal to the scheduler through a notify pipe, as the ERTS
//! handlers pass it to the signal dispatcher thread (`sys.c:1106-1201`).

use std::{
    ffi::c_int,
    sync::{
        OnceLock,
        atomic::{AtomicBool, Ordering},
    },
};

use super::errno::errno;

/// The read end and the write end of the notify pipe, as `sig_notify_fds`
/// (`sys.c:1189`). The first handler install makes the pipe, and the pipe
/// stays open until the OS process ends. Both ends are non-blocking and
/// close-on-exec.
static NOTIFY_PIPE: OnceLock<[c_int; 2]> = OnceLock::new();

/// True when a handler wrote to the notify pipe after the last
/// [`take_notified`]. The scheduler reads it at each scheduling step, so a
/// signal reaches its target without a system call for each step.
static NOTIFIED: AtomicBool = AtomicBool::new(false);

/// Whether a break is active, as `erts_break_requested` marks one
/// (`sys.c:611-624`). The scheduler of PBeam takes the dispatcher role of
/// ERTS: it sets the flag before the menu, and clears it after the menu
/// returns (`sys.c:913`). A second request stops the run while the flag is
/// set (`break.c:602-604`).
static BREAK_REQUESTED: AtomicBool = AtomicBool::new(false);

/// Whether a `SIGINT` arrived while a break was active. The handler sets it
/// when [`BREAK_REQUESTED`] is set, so a request that arrives at the end of
/// the menu cannot become a new menu (`sys.c:611-624`).
static SECOND_BREAK: AtomicBool = AtomicBool::new(false);

/// The disposition that `os:set_signal/2` gives a signal: its `Mode`
/// argument `ignore`, `default`, or `handle`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Disposition {
    /// `SIG_IGN`: the host ignores the signal.
    Ignore,
    /// `SIG_DFL`: the default action of the host, for example a stop of the
    /// process for `SIGHUP`.
    Default,
    /// A handler, so the signal does not do its default action. The
    /// handler writes the signal to the notify pipe, and the scheduler sends
    /// `{notify, Signal}` to `erl_signal_server`, as ERTS does
    /// (sys.c:589-607, 749-752, 1163-1176).
    Handle,
}

/// Set the dispositions of the emulator start, as `erts_sys_init_float`
/// (sys_float.c:31-37), `init_break_handler` (sys.c:799-803), and
/// `erts_sys_unix_later_init` (sys.c:812-823) do: `SIG_IGN` for `SIGFPE`,
/// and handlers for `SIGINT`, `SIGQUIT`, `SIGTERM`, and `SIGUSR1`. ERTS
/// also sets `SIG_IGN` for `SIGCHLD`, because `erl_child_setup`, a
/// separate process, reaps its port programs (erl_child_setup.c:396-420).
/// PBeam keeps `SIG_DFL`: a thread of this process reaps each port program
/// (see `host::spawn`), and with `SIG_IGN` the host reaps the program
/// first, so the exit status of the port is lost. ERTS installs `request_break` for `SIGINT`,
/// which writes to the notify pipe as the other handler does
/// (sys.c:626-629). The Rust runtime already sets `SIGPIPE` to `SIG_IGN`,
/// as `erl_sys_late_init` does (`sys_drivers.c:175`). The `pbeam` binary
/// calls this function before it loads the BEAM files.
///
/// # Examples
///
/// ```rust,ignore
/// install_start_dispositions();
/// assert!(notify_fd().is_some());
/// ```
pub fn install_start_dispositions() {
    set_disposition(libc::SIGFPE, Disposition::Ignore);
    for signal in [libc::SIGINT, libc::SIGQUIT, libc::SIGTERM, libc::SIGUSR1] {
        set_disposition(signal, Disposition::Handle);
    }
}

/// Set the disposition of the host signal `signal` with `sigaction`, as
/// `sys_signal` (sys.c:437-460) does. The notify pipe exists before a
/// handler is installed. `os:set_signal/2` calls it with the number of
/// [`signal_number`], as `erts_set_signal` (sys.c:754-767) does.
///
/// As `sys_signal` on the pinned aarch64 build, the `sigaction` call has an
/// empty mask and no flags, and its result is ignored. Thus `sigstop`,
/// which the host does not let a process change, also gives `ok`. With no
/// `SA_RESTART`, a handled signal stops a `select` of the scheduler with
/// `EINTR`.
///
/// # Examples
///
/// ```rust,ignore
/// set_disposition(libc::SIGUSR2, Disposition::Handle);
/// set_disposition(libc::SIGSTOP, Disposition::Ignore);
/// ```
pub(crate) fn set_disposition(signal: c_int, disposition: Disposition) {
    let handler = match disposition {
        Disposition::Ignore => libc::SIG_IGN,
        Disposition::Default => libc::SIG_DFL,
        Disposition::Handle => {
            NOTIFY_PIPE.get_or_init(open_notify_pipe);
            notify_signal as extern "C" fn(c_int) as libc::sighandler_t
        }
    };
    // SAFETY: `sigaction` is a C struct of integers and a signal set, so
    // all zeros is a valid value. `sigemptyset` writes only the mask of the
    // struct. `sigaction` reads the struct and accepts a null pointer for
    // the old action. `notify_signal` only calls async-signal-safe
    // functions.
    unsafe {
        let mut action: libc::sigaction = std::mem::zeroed();
        libc::sigemptyset(&raw mut action.sa_mask);
        action.sa_sigaction = handler;
        libc::sigaction(signal, &raw const action, std::ptr::null_mut());
    }
}

/// Make the notify pipe, as `init_smp_sig_notify` (sys.c:1182-1201) does.
/// Both ends are non-blocking: one thread runs the handler and reads the
/// pipe, so a handler that waits for a full pipe would wait without end.
/// Both ends are close-on-exec, so a child process does not keep them.
///
/// # Examples
///
/// ```rust,ignore
/// let [read_end, write_end] = open_notify_pipe();
/// ```
///
/// # Panics
///
/// Panics when `pipe` fails, as ERTS stops with `Failed to create
/// signal-dispatcher pipe`.
fn open_notify_pipe() -> [c_int; 2] {
    let mut fds = [-1; 2];
    // SAFETY: `pipe` writes two new fds to `fds`. `fcntl` changes only the
    // flags of each new fd.
    unsafe {
        assert_eq!(
            libc::pipe(fds.as_mut_ptr()),
            0,
            "Failed to create signal-dispatcher pipe"
        );
        for fd in fds {
            libc::fcntl(fd, libc::F_SETFL, libc::O_NONBLOCK);
            libc::fcntl(fd, libc::F_SETFD, libc::FD_CLOEXEC);
        }
    }
    fds
}

/// Return the read end of the notify pipe, or `None` before the first
/// handler install. The scheduler waits for it when no process can run.
///
/// # Examples
///
/// ```rust,ignore
/// assert!(notify_fd().is_some());
/// ```
pub(crate) fn notify_fd() -> Option<c_int> {
    NOTIFY_PIPE.get().map(|[read_end, _]| *read_end)
}

/// Return the signals that the handlers wrote to the notify pipe, in the
/// order of their arrival, when a handler ran after the last call. Return
/// an empty list without a system call otherwise.
///
/// # Examples
///
/// ```rust,ignore
/// // SAFETY: the handler of SIGUSR2 writes to the notify pipe.
/// unsafe { libc::raise(libc::SIGUSR2) };
/// assert_eq!(take_notified(), vec![libc::SIGUSR2]);
/// assert_eq!(take_notified(), vec![]);
/// ```
pub(crate) fn take_notified() -> Vec<c_int> {
    if NOTIFIED.swap(false, Ordering::Acquire) {
        read_notified()
    } else {
        Vec::new()
    }
}

/// Mark a break, as `break_requested` does before it pokes the break
/// handling (`sys.c:611-624`). The signal handler marks a second break when
/// it finds this flag set.
///
/// # Examples
///
/// ```rust,ignore
/// request_break();
/// assert!(take_break_requested());
/// ```
pub(crate) fn request_break() {
    BREAK_REQUESTED.store(true, Ordering::Release);
}

/// Return whether a break is active, and clear the flag. The scheduler
/// clears it after the menu returns, as ERTS does (`sys.c:913`).
///
/// # Examples
///
/// ```rust,ignore
/// request_break();
/// assert!(take_break_requested());
/// assert!(!take_break_requested());
/// ```
pub(crate) fn take_break_requested() -> bool {
    BREAK_REQUESTED.swap(false, Ordering::Acquire)
}

/// Return whether a `SIGINT` arrived while a break was active, without a
/// change to the flag.
///
/// # Examples
///
/// ```rust,ignore
/// assert!(!second_break());
/// ```
pub(crate) fn second_break() -> bool {
    SECOND_BREAK.load(Ordering::Acquire)
}

/// Mark a second break for a test. The signal handler marks the second
/// break of a run; no production code calls this function.
#[cfg(test)]
pub(crate) fn mark_second_break() {
    SECOND_BREAK.store(true, Ordering::Release);
}

/// Return whether a `SIGINT` arrived while a break was active, and clear
/// the flag.
///
/// # Examples
///
/// ```rust,ignore
/// assert!(!take_second_break());
/// ```
pub(crate) fn take_second_break() -> bool {
    SECOND_BREAK.swap(false, Ordering::Acquire)
}

/// Read all signals in the notify pipe, as the signal dispatcher thread
/// reads each signal number (sys.c:1128-1144). A wait of the scheduler that
/// the pipe ends does not read it: the next [`take_notified`] does. Each handler writes one
/// `int` with one `write` of at most `PIPE_BUF` bytes, so a `read` gets only
/// whole numbers.
///
/// # Examples
///
/// ```rust,ignore
/// // SAFETY: the handler of SIGUSR2 writes to the notify pipe.
/// unsafe { libc::raise(libc::SIGUSR2) };
/// assert_eq!(read_notified(), vec![libc::SIGUSR2]);
/// assert_eq!(read_notified(), vec![]);
/// ```
fn read_notified() -> Vec<c_int> {
    let read_end = notify_fd().expect("a handler install makes the pipe");
    let mut signals = Vec::new();
    let mut buffer = [0_u8; 256];
    loop {
        // SAFETY: `read` writes at most the length of `buffer` into it.
        let count = unsafe { libc::read(read_end, buffer.as_mut_ptr().cast(), buffer.len()) };
        let Ok(count @ 1..) = usize::try_from(count) else {
            return signals;
        };
        let (numbers, _) = buffer[..count].as_chunks::<{ size_of::<c_int>() }>();
        signals.extend(numbers.iter().map(|bytes| c_int::from_ne_bytes(*bytes)));
    }
}

/// Return the atom name of a host signal number, as `signum_to_signalterm`
/// (sys.c:723-747) does, or `None` for `SIGINT` and each other number that
/// it maps to `error`.
///
/// # Examples
///
/// ```rust,ignore
/// assert_eq!(signal_name(libc::SIGTERM), Some("sigterm"));
/// assert_eq!(signal_name(libc::SIGINT), None);
/// ```
pub(crate) fn signal_name(signal: c_int) -> Option<&'static str> {
    SIGNALS
        .iter()
        .find(|(_, number)| *number == signal)
        .map(|(name, _)| *name)
}

/// Return the host number of a signal atom name, as `signalterm_to_signum`
/// (sys.c:697-722) does. ERTS has no case for `sigint`, `sigill`, and
/// `sigsegv`, and it has `siginfo` only on a host with `SIGINFO`, which of
/// the PBeam hosts is only macOS.
///
/// # Examples
///
/// ```rust,ignore
/// assert_eq!(signal_number("sigwinch"), Some(libc::SIGWINCH));
/// assert_eq!(signal_number("sigint"), None);
/// ```
pub(crate) fn signal_number(name: &str) -> Option<c_int> {
    SIGNALS
        .iter()
        .find(|(signal, _)| *signal == name)
        .map(|(_, number)| *number)
}

/// Each signal of `signalterm_to_signum` and `signum_to_signalterm`
/// (sys.c:697-747), with its atom name.
const SIGNALS: &[(&str, c_int)] = &[
    ("sighup", libc::SIGHUP),
    ("sigquit", libc::SIGQUIT),
    ("sigabrt", libc::SIGABRT),
    ("sigalrm", libc::SIGALRM),
    ("sigterm", libc::SIGTERM),
    ("sigusr1", libc::SIGUSR1),
    ("sigusr2", libc::SIGUSR2),
    ("sigchld", libc::SIGCHLD),
    ("sigstop", libc::SIGSTOP),
    ("sigtstp", libc::SIGTSTP),
    ("sigcont", libc::SIGCONT),
    ("sigwinch", libc::SIGWINCH),
    #[cfg(target_os = "macos")]
    ("siginfo", libc::SIGINFO),
];

/// The handler of the `handle` disposition and of the start dispositions:
/// write the signal number to the notify pipe, as `smp_sig_notify`
/// (sys.c:1105-1120) does, then mark the pipe for [`take_notified`]. The
/// mark comes after the write, so a reader that sees the mark finds the
/// number. The handler keeps `errno`, so the code that the signal stops
/// reads its own error.
///
/// ERTS writes to a blocking pipe that its dispatcher thread reads, and it
/// aborts when the write fails. In PBeam the scheduler thread reads the
/// pipe, so the write end is non-blocking, and a write to a full pipe drops
/// the signal. A full pipe holds 4096 or more signals that the scheduler
/// did not read.
///
/// # Examples
///
/// ```rust,ignore
/// set_disposition(libc::SIGUSR2, Disposition::Handle);
/// // SAFETY: the handler of SIGUSR2 writes to the notify pipe.
/// unsafe { libc::raise(libc::SIGUSR2) };
/// ```
extern "C" fn notify_signal(signal: c_int) {
    let [_, write_end] = *NOTIFY_PIPE.get().expect("a handler install makes the pipe");
    let saved = errno();
    if signal == libc::SIGINT && BREAK_REQUESTED.load(Ordering::Acquire) {
        SECOND_BREAK.store(true, Ordering::Release);
    }
    let bytes = signal.to_ne_bytes();
    // SAFETY: `write` reads the bytes of `bytes`. It is async-signal-safe.
    // A write to a non-blocking pipe does not wait, so a signal cannot stop
    // it with `EINTR`.
    unsafe { libc::write(write_end, bytes.as_ptr().cast(), bytes.len()) };
    NOTIFIED.store(true, Ordering::Release);
    set_errno(saved);
}

/// Set `errno` of the calling thread.
///
/// # Examples
///
/// ```rust,ignore
/// set_errno(libc::EINTR);
/// assert_eq!(errno(), libc::EINTR);
/// ```
fn set_errno(value: c_int) {
    // SAFETY: the errno location of the calling thread is valid for writes.
    unsafe {
        #[cfg(target_os = "macos")]
        let location = libc::__error();
        #[cfg(not(target_os = "macos"))]
        let location = libc::__errno_location();
        *location = value;
    }
}

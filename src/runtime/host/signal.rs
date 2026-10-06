//! Set the host disposition of a signal, as `erts_set_signal` does for
//! `os:set_signal/2` (`sys/unix/sys.c:437-460`, `697-767`).

use std::ffi::c_int;

/// The disposition that `os:set_signal/2` gives a signal: its `Mode`
/// argument `ignore`, `default`, or `handle`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Disposition {
    /// `SIG_IGN`: the host ignores the signal.
    Ignore,
    /// `SIG_DFL`: the default action of the host, for example a stop of the
    /// process for `SIGHUP`.
    Default,
    /// A handler, so the signal does not do its default action. The ERTS
    /// handler sends `{notify, Signal}` to `erl_signal_server` (sys.c:589-607,
    /// 749-752). The PBeam handler does nothing (see "OS signals" in
    /// `docs/erts-differences.md`).
    Handle,
}

/// Set the disposition of the signal with the atom name `name`, and return
/// whether the name is a signal of `signalterm_to_signum`, as
/// `erts_set_signal` (sys.c:754-767) does. A name that is not a signal
/// changes nothing.
///
/// As `sys_signal` (sys.c:437-460) on the pinned aarch64 build, the
/// `sigaction` call has an empty mask and no flags, and its result is
/// ignored. Thus `sigstop`, which the host does not let a process change,
/// also returns `true`.
///
/// # Examples
///
/// ```rust,ignore
/// assert!(set_signal("sighup", Disposition::Ignore));
/// assert!(set_signal("sigstop", Disposition::Handle));
/// assert!(!set_signal("sigint", Disposition::Ignore));
/// ```
pub(crate) fn set_signal(name: &str, disposition: Disposition) -> bool {
    let Some(signal) = signal_number(name) else {
        return false;
    };
    let handler = match disposition {
        Disposition::Ignore => libc::SIG_IGN,
        Disposition::Default => libc::SIG_DFL,
        Disposition::Handle => handle_signal as extern "C" fn(c_int) as libc::sighandler_t,
    };
    // SAFETY: `sigaction` is a C struct of integers and a signal set, so
    // all zeros is a valid value. `sigemptyset` writes only the mask of the
    // struct. `sigaction` reads the struct and accepts a null pointer for
    // the old action. `handle_signal` does nothing, so it is safe in a
    // signal handler.
    unsafe {
        let mut action: libc::sigaction = std::mem::zeroed();
        libc::sigemptyset(&raw mut action.sa_mask);
        action.sa_sigaction = handler;
        libc::sigaction(signal, &raw const action, std::ptr::null_mut());
    }
    true
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
fn signal_number(name: &str) -> Option<c_int> {
    Some(match name {
        "sighup" => libc::SIGHUP,
        "sigquit" => libc::SIGQUIT,
        "sigabrt" => libc::SIGABRT,
        "sigalrm" => libc::SIGALRM,
        "sigterm" => libc::SIGTERM,
        "sigusr1" => libc::SIGUSR1,
        "sigusr2" => libc::SIGUSR2,
        "sigchld" => libc::SIGCHLD,
        "sigstop" => libc::SIGSTOP,
        "sigtstp" => libc::SIGTSTP,
        "sigcont" => libc::SIGCONT,
        "sigwinch" => libc::SIGWINCH,
        #[cfg(target_os = "macos")]
        "siginfo" => libc::SIGINFO,
        _ => return None,
    })
}

/// The handler of the `handle` disposition. It does nothing, so the signal
/// does not do its default action.
///
/// # Examples
///
/// ```rust,ignore
/// assert!(set_signal("sigusr2", Disposition::Handle));
/// // SAFETY: the handler of SIGUSR2 does nothing.
/// unsafe { libc::raise(libc::SIGUSR2) };
/// ```
extern "C" fn handle_signal(_signal: c_int) {}

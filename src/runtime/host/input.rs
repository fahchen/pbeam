//! Read a host input fd and wait until it has input, as the Unix part of
//! the ERTS `prim_tty` NIF `tty_read_nif`
//! (`erts/emulator/nifs/common/prim_tty_nif.c`) and the ERTS IO poll of a
//! NIF read select (`erts/emulator/sys/common/erl_check_io.c`) do.

use std::{ffi::c_int, time::Duration};

use super::errno::errno;

/// What [`wait_readable`] found on its fds.
#[derive(Debug, PartialEq, Eq)]
pub(crate) enum Readiness {
    /// A `read` of each of these fds does not block: the fd has input, its
    /// end of file, or an error.
    Ready(Vec<c_int>),
    /// The timeout ended, or a signal stopped the wait.
    Pending,
    /// An fd is not open, as `ERTS_POLL_EV_NVAL`.
    Invalid,
}

impl Readiness {
    /// Return `true` when the wait ended for `fd`: `fd` is ready, or an fd
    /// is not open.
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// assert!(Readiness::Ready(vec![0]).ended(0));
    /// assert!(Readiness::Invalid.ended(0));
    /// assert!(!Readiness::Pending.ended(0));
    /// ```
    pub(crate) fn ended(&self, fd: c_int) -> bool {
        match self {
            Readiness::Ready(fds) => fds.contains(&fd),
            Readiness::Pending => false,
            Readiness::Invalid => true,
        }
    }

    /// Return the ready fds: none when the timeout ended or an fd is not
    /// open.
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// assert_eq!(Readiness::Ready(vec![5]).into_ready(), vec![5]);
    /// assert!(Readiness::Invalid.into_ready().is_empty());
    /// ```
    pub(crate) fn into_ready(self) -> Vec<c_int> {
        match self {
            Readiness::Ready(fds) => fds,
            Readiness::Pending | Readiness::Invalid => Vec::new(),
        }
    }
}

/// Read at most `size` bytes from `fd` with one `read`, as `tty_read_nif`
/// (`prim_tty_nif.c:646-657`) does. Return the bytes, which are empty at
/// the end of file and when `size` is 0, or the `errno` of a failed
/// `read`.
///
/// # Examples
///
/// ```rust,ignore
/// // stdin is a pipe that holds "hi\n".
/// assert_eq!(read(libc::STDIN_FILENO, 1024), Ok(b"hi\n".to_vec()));
/// assert_eq!(read(-1, 1024), Err(libc::EBADF));
/// ```
pub(crate) fn read(fd: c_int, size: usize) -> Result<Vec<u8>, c_int> {
    let mut bytes = vec![0; size];
    // SAFETY: `read` writes at most `size` bytes to the buffer of `bytes`,
    // which holds `size` bytes.
    let count = unsafe { libc::read(fd, bytes.as_mut_ptr().cast(), size) };
    let count = usize::try_from(count).map_err(|_| errno())?;
    bytes.truncate(count);
    Ok(bytes)
}

/// Wait until a `read` of one of `fds` does not block, for at most
/// `timeout`, or without end for `None`, with `select`. The ERTS poll
/// thread waits for the read select of a NIF and for the input of a port in
/// the same way (`erl_process.c:3372`, `erl_check_io.c:2121-2152`), and its
/// poll set also holds the read end of its wake pipe (`erl_poll.c:489-525`).
/// ERTS uses `kqueue` on macOS and `epoll` on Linux, with a `select` or
/// `poll` set for each fd that the kernel poll does not accept
/// (`erl_poll.h:100-135`, `214-225`). `select` reads each fd type on both
/// hosts. The macOS `poll` gives `POLLNVAL` for a character
/// device such as `/dev/null`. An fd can be at or above `FD_SETSIZE`, as in
/// the ERTS poll: the sets grow with the largest fd (see [`FdSets`]).
///
/// # Examples
///
/// ```rust,ignore
/// // stdin is `/dev/null`.
/// assert_eq!(wait_readable(&[libc::STDIN_FILENO], None), Readiness::Ready(vec![0]));
/// assert_eq!(wait_readable(&[1000], Some(Duration::ZERO)), Readiness::Invalid);
/// ```
pub(crate) fn wait_readable(fds: &[c_int], timeout: Option<Duration>) -> Readiness {
    let limit = fds.iter().max().map_or(0, |fd| fd + 1);
    let mut sets = FdSets::new(limit);
    for &fd in fds {
        sets.set(fd);
    }
    // `select` takes microseconds; round up, so the wait does not end
    // before a timer end.
    let mut time = timeout.map(|timeout| {
        let micros = timeout.as_nanos().div_ceil(1000);
        libc::timeval {
            tv_sec: libc::time_t::try_from(micros / 1_000_000).unwrap_or(libc::time_t::MAX),
            tv_usec: libc::suseconds_t::try_from(micros % 1_000_000).expect("below 1000000"),
        }
    });
    let time = time
        .as_mut()
        .map_or(std::ptr::null_mut(), |time| &raw mut *time);
    // SAFETY: `select` reads and writes the bits of the fds below `limit`
    // in `sets`, which holds them, and reads `time`, which is null or points
    // to `timeval`. It reads no other set.
    let count = unsafe {
        select(
            limit,
            sets.0.as_mut_ptr(),
            std::ptr::null_mut(),
            std::ptr::null_mut(),
            time,
        )
    };
    match count {
        1.. => Readiness::Ready(fds.iter().copied().filter(|&fd| sets.is_set(fd)).collect()),
        0 => Readiness::Pending,
        _ if errno() == libc::EINTR => Readiness::Pending,
        _ => Readiness::Invalid,
    }
}

unsafe extern "C" {
    /// The `select` of the host. On macOS, it is the variant of
    /// `_DARWIN_UNLIMITED_SELECT`, as ERTS uses (`erl_poll.c:69-74`): the
    /// other variant gives `EINVAL` for `nfds` above `FD_SETSIZE`. The Linux
    /// `select` reads `nfds` bits of each set.
    #[cfg_attr(target_os = "macos", link_name = "select$DARWIN_EXTSN")]
    fn select(
        nfds: c_int,
        readfds: *mut libc::fd_set,
        writefds: *mut libc::fd_set,
        errorfds: *mut libc::fd_set,
        timeout: *mut libc::timeval,
    ) -> c_int;
}

/// A read set of `select` for the fds below a limit, also above
/// `FD_SETSIZE`, as `ERTS_fd_set` (`erl_poll.c:148-196`). The sets are
/// consecutive `fd_set` values, so their bits are one bit array: fd `n` is
/// bit `n % FD_SETSIZE` of set `n / FD_SETSIZE`.
struct FdSets(Vec<libc::fd_set>);

impl FdSets {
    /// Make an empty set for the fds below `limit`.
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// let sets = FdSets::new(2000);
    /// assert!(!sets.is_set(1999));
    /// ```
    fn new(limit: c_int) -> Self {
        let size = usize::try_from(limit)
            .unwrap_or(0)
            .div_ceil(libc::FD_SETSIZE);
        // SAFETY: `fd_set` is a C struct of integers, so all zeros is a
        // valid value and an empty set.
        Self(vec![unsafe { std::mem::zeroed() }; size.max(1)])
    }

    /// Add `fd`, a non-negative fd below the limit, to the set.
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// let mut sets = FdSets::new(2000);
    /// sets.set(1999);
    /// assert!(sets.is_set(1999));
    /// ```
    fn set(&mut self, fd: c_int) {
        let (set, bit) = Self::position(fd);
        // SAFETY: `FD_SET` writes only bit `bit` of `set`, and `bit` is
        // below `FD_SETSIZE`.
        unsafe { libc::FD_SET(bit, &raw mut self.0[set]) };
    }

    /// Return whether `fd`, a non-negative fd below the limit, is in the set.
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// assert!(!FdSets::new(1).is_set(0));
    /// ```
    fn is_set(&self, fd: c_int) -> bool {
        let (set, bit) = Self::position(fd);
        // SAFETY: `FD_ISSET` reads only bit `bit` of `set`, and `bit` is
        // below `FD_SETSIZE`.
        unsafe { libc::FD_ISSET(bit, &raw const self.0[set]) }
    }

    /// Return the index of the set and the bit in it for `fd`.
    fn position(fd: c_int) -> (usize, c_int) {
        let fd = usize::try_from(fd).expect("a non-negative fd");
        let bit = c_int::try_from(fd % libc::FD_SETSIZE).expect("below FD_SETSIZE");
        (fd / libc::FD_SETSIZE, bit)
    }
}

#[cfg(test)]
#[path = "../../../tests/runtime/host/input.rs"]
mod tests;

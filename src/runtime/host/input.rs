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
/// `timeout`, or without end for `None`, with `select`. The ERTS poll thread
/// waits for the read select of a NIF in the same way (`erl_process.c:3372`,
/// `erl_check_io.c:2121-2152`). ERTS uses `kqueue` on macOS and `epoll` on
/// Linux, with a `select` or `poll` set for each fd that the kernel poll
/// does not accept (`erl_poll.h:100-135`, `214-225`). `select` reads each fd
/// type on both hosts. The macOS `poll` gives `POLLNVAL` for a character
/// device such as `/dev/null`.
///
/// # Examples
///
/// ```rust,ignore
/// // stdin is `/dev/null`.
/// assert_eq!(wait_readable(&[libc::STDIN_FILENO], None), Readiness::Ready(vec![0]));
/// assert_eq!(wait_readable(&[1000], Some(Duration::ZERO)), Readiness::Invalid);
/// ```
pub(crate) fn wait_readable(fds: &[c_int], timeout: Option<Duration>) -> Readiness {
    // SAFETY: `fd_set` is a C struct of integers, so all zeros is a valid
    // value. `FD_ZERO` and `FD_SET` write only into `set`, and each fd is a
    // standard fd or a runtime pipe fd, below `FD_SETSIZE`.
    let mut set = unsafe {
        let mut set: libc::fd_set = std::mem::zeroed();
        libc::FD_ZERO(&raw mut set);
        for &fd in fds {
            libc::FD_SET(fd, &raw mut set);
        }
        set
    };
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
    let limit = fds.iter().max().map_or(0, |fd| fd + 1);
    // SAFETY: `select` reads and writes `set` and reads `time`, which is
    // null or points to `timeval`, and it reads no other set.
    let count = unsafe {
        libc::select(
            limit,
            &raw mut set,
            std::ptr::null_mut(),
            std::ptr::null_mut(),
            time,
        )
    };
    match count {
        // SAFETY: `FD_ISSET` only reads `set`.
        1.. => Readiness::Ready(
            fds.iter()
                .copied()
                .filter(|&fd| unsafe { libc::FD_ISSET(fd, &raw const set) })
                .collect(),
        ),
        0 => Readiness::Pending,
        _ if errno() == libc::EINTR => Readiness::Pending,
        _ => Readiness::Invalid,
    }
}

#[cfg(test)]
#[path = "../../../tests/runtime/host/input.rs"]
mod tests;

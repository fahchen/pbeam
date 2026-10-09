//! Write to a host output fd, as the Unix part of the ERTS `prim_tty`
//! NIF `tty_write_nif` does (`erts/emulator/nifs/common/prim_tty_nif.c`).

use std::{ffi::c_int, io::IoSlice};

use super::errno::errno;

/// Write all bytes of `buffers` to `fd` with `writev`, as `tty_write_nif`
/// (`prim_tty_nif.c:376-423`) writes one group of an iovec. The function
/// calls `writev` again while it fails with `EINTR` or `EAGAIN`. After a
/// partial write, it removes the written bytes from `buffers` and writes
/// the remaining bytes. Each other failure returns its `errno`.
///
/// The function calls `writev` also when `buffers` is empty, as ERTS does.
/// On macOS, `writev` with a count of 0 fails with `EINVAL`. On Linux, it
/// writes 0 bytes, so the result is `Ok`.
///
/// # Examples
///
/// ```rust,ignore
/// let mut buffers = [IoSlice::new(b"Hello, "), IoSlice::new(b"world!\n")];
/// assert_eq!(write_all(libc::STDOUT_FILENO, &mut buffers), Ok(()));
/// assert_eq!(write_all(-1, &mut [IoSlice::new(b"x")]), Err(libc::EBADF));
/// // macOS
/// assert_eq!(write_all(libc::STDOUT_FILENO, &mut []), Err(libc::EINVAL));
/// ```
pub(crate) fn write_all(fd: c_int, mut buffers: &mut [IoSlice<'_>]) -> Result<(), c_int> {
    let mut size: usize = buffers.iter().map(|buffer| buffer.len()).sum();
    loop {
        let count = c_int::try_from(buffers.len()).unwrap_or(c_int::MAX);
        // SAFETY: `IoSlice` is ABI compatible with `iovec` on Unix, and
        // `writev` reads at most `count` entries of `buffers`, each of which
        // points to bytes that `buffers` borrows. ERTS calls `writev` in the
        // same way (prim_tty_nif.c:376-380).
        let written = unsafe { libc::writev(fd, buffers.as_ptr().cast(), count) };
        let Ok(written) = usize::try_from(written) else {
            match errno() {
                libc::EINTR | libc::EAGAIN => continue,
                errno => return Err(errno),
            }
        };
        if written == size {
            return Ok(());
        }
        size -= written;
        IoSlice::advance_slices(&mut buffers, written);
    }
}

#[cfg(test)]
#[path = "../../../tests/runtime/host/output.rs"]
mod tests;

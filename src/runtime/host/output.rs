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

/// The writes of one `prim_tty:write_nif/2` call: the groups of its iovec
/// that `tty_write_nif` writes, in order, and whether a bad group follows
/// them (`prim_tty_nif.c:366-426`). Each group holds the binaries of at
/// most `MAXIOV` iovec elements that are not empty. The bytes are copies,
/// so a dirty IO thread owns them while it writes.
#[derive(Debug)]
pub(crate) struct IovecWrite {
    /// The output fd of the TTY resource, or -1 when the term is not a TTY
    /// resource and the call writes nothing.
    pub(crate) fd: c_int,
    /// The groups to write, each one with one `writev` loop.
    pub(crate) groups: Vec<Vec<Vec<u8>>>,
    /// True when the call raises `badarg` after it writes `groups`: the TTY
    /// term or the iovec is not valid, or a later group has an element that
    /// is not a binary or the tail of an improper list
    /// (`examine_iovec_term`, `erl_nif.c:4092-4150`).
    pub(crate) bad_group: bool,
}

/// The end of an [`IovecWrite`].
#[derive(Debug, PartialEq, Eq)]
pub(crate) enum IovecWritten {
    /// All groups are written, and no bad group follows.
    All,
    /// The write of a group failed with this `errno`. The later groups are
    /// not written.
    Failed(c_int),
    /// All groups are written, and a bad group follows.
    BadGroup,
}

impl IovecWrite {
    /// Write each group to `fd` with [`write_all`], in order, as
    /// `tty_write_nif` (`prim_tty_nif.c:366-426`) does. Stop at the first
    /// failed write. This call can wait without end for a full fd, so a
    /// dirty IO thread runs it, not the scheduler thread.
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// let write = IovecWrite { fd: libc::STDOUT_FILENO, groups: vec![vec![b"hi\n".to_vec()]], bad_group: false };
    /// assert_eq!(write.run(), IovecWritten::All);
    /// let write = IovecWrite { fd: -1, groups: vec![vec![b"x".to_vec()]], bad_group: true };
    /// assert_eq!(write.run(), IovecWritten::Failed(libc::EBADF));
    /// ```
    pub(crate) fn run(&self) -> IovecWritten {
        for group in &self.groups {
            let mut buffers: Vec<IoSlice<'_>> =
                group.iter().map(|bytes| IoSlice::new(bytes)).collect();
            if let Err(errno) = write_all(self.fd, &mut buffers) {
                return IovecWritten::Failed(errno);
            }
        }
        if self.bad_group {
            IovecWritten::BadGroup
        } else {
            IovecWritten::All
        }
    }
}

#[cfg(test)]
#[path = "../../../tests/runtime/host/output.rs"]
mod tests;

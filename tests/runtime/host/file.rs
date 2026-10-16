use super::*;

/// A writer that takes at most `limit` bytes for each write and gives 0
/// bytes after `total` bytes. It gives `EINTR` for its first write when
/// `interrupt` is set, `EINVAL` for each vectored write when
/// `invalid_vectored` is set, and `EINVAL` for each write of more than
/// `invalid_above` bytes.
struct Partial {
    written: Vec<u8>,
    limit: usize,
    total: usize,
    interrupt: bool,
    invalid_vectored: bool,
    invalid_above: usize,
}

impl Partial {
    /// Return a writer that takes `limit` bytes for each write, with no
    /// other limit and no error.
    fn new(limit: usize) -> Self {
        Partial {
            written: Vec::new(),
            limit,
            total: usize::MAX,
            interrupt: false,
            invalid_vectored: false,
            invalid_above: usize::MAX,
        }
    }

    /// Take the bytes of `buffers`, as a write of them does.
    fn take(&mut self, buffers: &[IoSlice<'_>]) -> io::Result<usize> {
        if std::mem::take(&mut self.interrupt) {
            return Err(io::Error::from_raw_os_error(libc::EINTR));
        }
        let room = self.limit.min(self.total - self.written.len());
        let bytes: Vec<u8> = buffers
            .iter()
            .flat_map(|buffer| buffer.iter().copied())
            .take(room)
            .collect();
        self.written.extend_from_slice(&bytes);
        Ok(bytes.len())
    }
}

impl Write for Partial {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        if bytes.len() > self.invalid_above {
            return Err(io::Error::from_raw_os_error(libc::EINVAL));
        }
        self.take(&[IoSlice::new(bytes)])
    }

    fn write_vectored(&mut self, buffers: &[IoSlice<'_>]) -> io::Result<usize> {
        if self.invalid_vectored {
            return Err(io::Error::from_raw_os_error(libc::EINVAL));
        }
        self.take(buffers)
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

#[test]
fn writes_the_rest_of_the_buffers_after_a_partial_write_as_efile_writev() {
    // `efile_writev` calls `writev` again after `EINTR` and after a partial
    // write with the iovec after the written bytes, and it stops at a write
    // of 0 bytes (unix_prim_file.c:343-394).
    let mut writer = Partial {
        interrupt: true,
        ..Partial::new(3)
    };
    let mut buffers = [
        IoSlice::new(b"ab"),
        IoSlice::new(b"cdefg"),
        IoSlice::new(b"h"),
    ];
    write_vectored(&mut writer, &mut buffers, 4).unwrap();
    assert_eq!(writer.written, b"abcdefgh");
    let mut writer = Partial {
        total: 5,
        ..Partial::new(3)
    };
    let mut buffers = [IoSlice::new(b"ab"), IoSlice::new(b"cdefg")];
    write_vectored(&mut writer, &mut buffers, 4).unwrap();
    assert_eq!(writer.written, b"abcde");
    writer.flush().unwrap();
}

#[test]
fn falls_back_to_write_after_einval_as_efile_writev() {
    // After `EINVAL` from `writev`, `efile_writev` writes the first iovec
    // entry with `write`, and after `EINVAL` from that write of an entry of
    // more than `FALLBACK_RW_LENGTH` bytes, only its first
    // `FALLBACK_RW_LENGTH` bytes (unix_prim_file.c:360-376).
    let mut writer = Partial {
        invalid_vectored: true,
        invalid_above: 4,
        ..Partial::new(usize::MAX)
    };
    let mut buffers = [IoSlice::new(b"abcdefgh"), IoSlice::new(b"ij")];
    write_vectored(&mut writer, &mut buffers, 4).unwrap();
    assert_eq!(writer.written, b"abcdefghij");
    // An entry that is not longer than the limit gives the error of `write`.
    let mut writer = Partial {
        invalid_vectored: true,
        invalid_above: 1,
        ..Partial::new(usize::MAX)
    };
    let mut buffers = [IoSlice::new(b"abc")];
    let error = write_vectored(&mut writer, &mut buffers, 4).unwrap_err();
    assert_eq!(error.raw_os_error(), Some(libc::EINVAL));
}

use super::*;

/// A writer that takes at most `limit` bytes for each write and then gives
/// 0 bytes after `total` bytes, or gives `EINTR` once first.
struct Partial {
    written: Vec<u8>,
    limit: usize,
    total: usize,
    interrupt: bool,
}

impl Write for Partial {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        self.write_vectored(&[IoSlice::new(bytes)])
    }

    fn write_vectored(&mut self, buffers: &[IoSlice<'_>]) -> io::Result<usize> {
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

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

#[test]
fn writes_the_rest_of_the_buffers_after_a_partial_write_as_efile_writev() {
    // `efile_writev` calls `writev` again after `EINTR` and after a partial
    // write with the iovec after the written bytes, and it stops at a write
    // of 0 bytes (unix_prim_file.c:360-394).
    let mut writer = Partial {
        written: Vec::new(),
        limit: 3,
        total: usize::MAX,
        interrupt: true,
    };
    let mut buffers = [
        IoSlice::new(b"ab"),
        IoSlice::new(b"cdefg"),
        IoSlice::new(b"h"),
    ];
    write_vectored(&mut writer, &mut buffers).unwrap();
    assert_eq!(writer.written, b"abcdefgh");
    let mut writer = Partial {
        written: Vec::new(),
        limit: 3,
        total: 5,
        interrupt: false,
    };
    let mut buffers = [IoSlice::new(b"ab"), IoSlice::new(b"cdefg")];
    write_vectored(&mut writer, &mut buffers).unwrap();
    assert_eq!(writer.written, b"abcde");
    assert_eq!(writer.write(b"x").unwrap(), 0);
    writer.flush().unwrap();
}

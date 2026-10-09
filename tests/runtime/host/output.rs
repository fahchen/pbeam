use super::*;
use std::{io::Read, os::fd::AsRawFd, thread, time::Duration};

#[test]
fn writes_the_remaining_bytes_after_a_partial_write_and_eagain() {
    // A pipe holds at most 64 KiB on macOS and Linux, and the reader
    // waits 20 ms before it reads. So the first `writev` of 1 MiB to
    // the non-blocking write end is partial, and a later `writev`
    // fails with `EAGAIN` while the pipe is full.
    let (mut reader, writer) = std::io::pipe().unwrap();
    // SAFETY: `fcntl` changes only the status flags of an open fd.
    unsafe {
        let flags = libc::fcntl(writer.as_raw_fd(), libc::F_GETFL);
        assert_eq!(
            libc::fcntl(writer.as_raw_fd(), libc::F_SETFL, flags | libc::O_NONBLOCK),
            0
        );
    }
    let first: Vec<u8> = (0..512 * 1024).map(|index| (index % 251) as u8).collect();
    let second: Vec<u8> = (0..512 * 1024).map(|index| (index % 241) as u8).collect();
    let reading = thread::spawn(move || {
        thread::sleep(Duration::from_millis(20));
        let mut bytes = Vec::new();
        reader.read_to_end(&mut bytes).unwrap();
        bytes
    });
    let mut buffers = [IoSlice::new(&first), IoSlice::new(&second)];
    assert_eq!(write_all(writer.as_raw_fd(), &mut buffers), Ok(()));
    drop(writer);
    assert_eq!(reading.join().unwrap(), [first, second].concat());
}

#[test]
fn gives_the_errno_of_a_failed_write() {
    assert_eq!(write_all(-1, &mut [IoSlice::new(b"x")]), Err(libc::EBADF));
}

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
    let first: Vec<u8> = (0..512 * 1024)
        .map(|index| u8::try_from(index % 251).unwrap())
        .collect();
    let second: Vec<u8> = (0..512 * 1024)
        .map(|index| u8::try_from(index % 241).unwrap())
        .collect();
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

#[test]
fn writes_each_group_in_order_until_a_failed_write() {
    let (mut reader, writer) = std::io::pipe().unwrap();
    let fd = writer.as_raw_fd();
    let groups = vec![
        vec![b"Hello, ".to_vec(), b"world".to_vec()],
        vec![],
        vec![b"!\n".to_vec()],
    ];
    let mut write = IovecWrite {
        fd,
        groups: vec![groups[0].clone(), groups[2].clone()],
        bad_group: false,
    };
    assert_eq!(write.run(), IovecWritten::All);
    write.bad_group = true;
    assert_eq!(write.run(), IovecWritten::BadGroup);
    write.groups.clear();
    assert_eq!(write.run(), IovecWritten::BadGroup);
    // `writev` with a count of 0: `EINVAL` on macOS, 0 bytes on Linux.
    write.groups = groups;
    write.bad_group = false;
    let expected = if cfg!(target_os = "macos") {
        IovecWritten::Failed(libc::EINVAL)
    } else {
        IovecWritten::All
    };
    assert_eq!(write.run(), expected);
    write.fd = -1;
    assert_eq!(write.run(), IovecWritten::Failed(libc::EBADF));
    drop(writer);
    let mut bytes = Vec::new();
    reader.read_to_end(&mut bytes).unwrap();
    let written = if cfg!(target_os = "macos") {
        "Hello, world!\nHello, world!\nHello, world"
    } else {
        "Hello, world!\nHello, world!\nHello, world!\n"
    };
    assert_eq!(String::from_utf8(bytes).unwrap(), written);
}

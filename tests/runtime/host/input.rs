use super::*;
use std::{io::Write, os::fd::AsRawFd};

#[test]
fn reads_the_bytes_of_one_read_and_the_end_of_file() {
    let (reader, mut writer) = std::io::pipe().unwrap();
    writer.write_all(b"hello\nworld\n").unwrap();
    assert_eq!(read(reader.as_raw_fd(), 3), Ok(b"hel".to_vec()));
    assert_eq!(read(reader.as_raw_fd(), 0), Ok(Vec::new()));
    assert_eq!(read(reader.as_raw_fd(), 1024), Ok(b"lo\nworld\n".to_vec()));
    drop(writer);
    assert_eq!(read(reader.as_raw_fd(), 1024), Ok(Vec::new()));
}

#[test]
fn gives_the_errno_of_a_failed_read() {
    assert_eq!(read(-1, 1024), Err(libc::EBADF));
}

#[test]
fn waits_until_a_pipe_has_input_or_its_end_of_file() {
    let (reader, mut writer) = std::io::pipe().unwrap();
    let fd = reader.as_raw_fd();
    assert_eq!(
        wait_readable(&[fd], Some(Duration::from_millis(1))),
        Some(vec![])
    );
    writer.write_all(b"x").unwrap();
    assert_eq!(wait_readable(&[fd], None), Some(vec![fd]));
    assert_eq!(read(fd, 1024), Ok(b"x".to_vec()));
    drop(writer);
    assert_eq!(wait_readable(&[fd], Some(Duration::ZERO)), Some(vec![fd]));
}

#[test]
fn waits_until_the_timeout_ends() {
    let (reader, _writer) = std::io::pipe().unwrap();
    let start = std::time::Instant::now();
    assert_eq!(
        wait_readable(
            &[reader.as_raw_fd()],
            Some(Duration::from_nanos(20_000_001))
        ),
        Some(vec![])
    );
    assert!(start.elapsed() >= Duration::from_millis(20));
}

#[test]
fn finds_an_fd_that_is_not_open() {
    let (reader, _writer) = std::io::pipe().unwrap();
    let fd = reader.as_raw_fd();
    drop(reader);
    assert_eq!(wait_readable(&[fd], Some(Duration::ZERO)), None);
}

#[test]
fn gives_each_ready_fd_of_the_set_in_request_order() {
    let (first, mut first_writer) = std::io::pipe().unwrap();
    let (second, mut second_writer) = std::io::pipe().unwrap();
    let fds = [second.as_raw_fd(), first.as_raw_fd()];
    first_writer.write_all(b"x").unwrap();
    assert_eq!(wait_readable(&fds, None), Some(vec![fds[1]]));
    second_writer.write_all(b"y").unwrap();
    assert_eq!(wait_readable(&fds, None), Some(fds.to_vec()));
}

#[test]
#[should_panic(expected = "select cannot hold fd 1024: FD_SETSIZE is 1024")]
fn refuses_an_fd_that_select_cannot_hold() {
    let fd = libc::c_int::try_from(libc::FD_SETSIZE).unwrap();
    wait_readable(&[fd], Some(Duration::ZERO));
}

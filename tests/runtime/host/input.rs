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
        Readiness::Pending
    );
    writer.write_all(b"x").unwrap();
    assert_eq!(wait_readable(&[fd], None), Readiness::Ready(vec![fd]));
    assert_eq!(read(fd, 1024), Ok(b"x".to_vec()));
    drop(writer);
    assert_eq!(
        wait_readable(&[fd], Some(Duration::ZERO)),
        Readiness::Ready(vec![fd])
    );
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
        Readiness::Pending
    );
    assert!(start.elapsed() >= Duration::from_millis(20));
}

#[test]
fn finds_an_fd_that_is_not_open() {
    let (reader, _writer) = std::io::pipe().unwrap();
    let fd = reader.as_raw_fd();
    drop(reader);
    assert_eq!(
        wait_readable(&[fd], Some(Duration::ZERO)),
        Readiness::Invalid
    );
}

#[test]
fn gives_each_fd_that_has_input() {
    let (first, _first_writer) = std::io::pipe().unwrap();
    let (second, mut second_writer) = std::io::pipe().unwrap();
    let (third, mut third_writer) = std::io::pipe().unwrap();
    second_writer.write_all(b"x").unwrap();
    third_writer.write_all(b"y").unwrap();
    let fds = [third.as_raw_fd(), first.as_raw_fd(), second.as_raw_fd()];
    assert_eq!(
        wait_readable(&fds, None),
        Readiness::Ready(vec![third.as_raw_fd(), second.as_raw_fd()])
    );
}

#[test]
fn tells_whether_the_wait_ended_for_an_fd() {
    assert!(Readiness::Ready(vec![0, 5]).ended(5));
    assert!(!Readiness::Ready(vec![5]).ended(0));
    assert!(!Readiness::Pending.ended(0));
    assert!(Readiness::Invalid.ended(0));
    assert_eq!(Readiness::Ready(vec![5]).into_ready(), vec![5]);
    assert!(Readiness::Pending.into_ready().is_empty());
    assert!(Readiness::Invalid.into_ready().is_empty());
}

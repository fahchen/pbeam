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
    assert_eq!(wait_ready(fd), Readiness::Ready(vec![fd]));
    assert_eq!(read(fd, 1024), Ok(b"x".to_vec()));
    drop(writer);
    assert_eq!(wait_ready(fd), Readiness::Ready(vec![fd]));
}

/// Wait until `fd` is ready.
///
/// A signal of another test stops the `select` with `EINTR`, which
/// [`wait_readable`] reports as [`Readiness::Pending`], as the scheduler
/// step does (`sys/unix/erl_poll.c`). Retry the wait, so the test checks the
/// fd and not the arrival of a signal.
fn wait_ready(fd: libc::c_int) -> Readiness {
    loop {
        match wait_readable(&[fd], None) {
            Readiness::Pending => {}
            readiness => return readiness,
        }
    }
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
    crate::isolated::run("runtime::host::input::tests::isolated_finds_an_fd_that_is_not_open");
}

#[test]
#[ignore = "finds_an_fd_that_is_not_open runs it in a child process"]
fn isolated_finds_an_fd_that_is_not_open() {
    crate::isolated::requires();
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
    let fds = [first.as_raw_fd(), second.as_raw_fd()];
    second_writer.write_all(b"x").unwrap();
    assert_eq!(
        wait_readable(&fds, None),
        Readiness::Ready(vec![second.as_raw_fd()])
    );
}

#[test]
fn gives_each_ready_fd_of_the_set_in_request_order() {
    let (first, mut first_writer) = std::io::pipe().unwrap();
    let (second, mut second_writer) = std::io::pipe().unwrap();
    let fds = [second.as_raw_fd(), first.as_raw_fd()];
    first_writer.write_all(b"x").unwrap();
    assert_eq!(wait_readable(&fds, None), Readiness::Ready(vec![fds[1]]));
    second_writer.write_all(b"y").unwrap();
    assert_eq!(wait_readable(&fds, None), Readiness::Ready(fds.to_vec()));
}

#[test]
fn waits_for_an_fd_at_or_above_fd_setsize() {
    // nextest runs each test in its own process, so the raised soft limit
    // of open fds reaches no other test.
    let mut limit = libc::rlimit {
        rlim_cur: 0,
        rlim_max: 0,
    };
    // SAFETY: `getrlimit` and `setrlimit` read and write only `limit`.
    unsafe {
        assert_eq!(libc::getrlimit(libc::RLIMIT_NOFILE, &raw mut limit), 0);
        limit.rlim_cur = limit.rlim_cur.max(2048).min(limit.rlim_max);
        assert_eq!(libc::setrlimit(libc::RLIMIT_NOFILE, &raw const limit), 0);
    }
    let (low, _low_writer) = std::io::pipe().unwrap();
    let (reader, mut writer) = std::io::pipe().unwrap();
    let setsize = c_int::try_from(libc::FD_SETSIZE).unwrap();
    // SAFETY: `fcntl` makes a new fd of `reader` at or above `FD_SETSIZE`.
    let high = unsafe { libc::fcntl(reader.as_raw_fd(), libc::F_DUPFD_CLOEXEC, setsize) };
    assert!(high >= setsize, "{high}");
    let fds = [low.as_raw_fd(), high];
    assert_eq!(
        wait_readable(&fds, Some(Duration::ZERO)),
        Readiness::Pending
    );
    writer.write_all(b"x").unwrap();
    assert_eq!(wait_readable(&fds, None), Readiness::Ready(vec![high]));
    // SAFETY: `high` is an fd of this test, which no other code uses.
    unsafe { libc::close(high) };
}

#[test]
fn tells_whether_the_wait_ended_for_an_fd() {
    assert!(Readiness::Ready(vec![0, 5]).ended(5));
    assert!(!Readiness::Ready(vec![5]).ended(0));
    assert!(!Readiness::Pending.ended(0));
    assert!(Readiness::Invalid.ended(0));
}

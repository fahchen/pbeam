use super::*;
use crate::runtime::host::input::{Readiness, wait_readable};
use std::{
    io::{PipeReader, PipeWriter},
    time::Duration,
};

/// The time that a dirty IO thread has to complete a call that does not
/// wait.
const COMPLETION_LIMIT: Duration = Duration::from_secs(10);

/// Return the write of `bytes` to `writer` in one group.
fn write_to(writer: &PipeWriter, bytes: &[u8]) -> IovecWrite {
    IovecWrite {
        fd: writer.as_raw_fd(),
        groups: vec![vec![bytes.to_vec()]],
        bad_group: false,
    }
}

/// Return a pipe whose write end holds as many bytes as the pipe takes, so
/// a blocking write to it waits until the test reads the pipe.
fn full_pipe() -> (PipeReader, PipeWriter) {
    let (reader, mut writer) = std::io::pipe().unwrap();
    let fd = writer.as_raw_fd();
    // SAFETY: `fcntl` changes only the status flags of an open fd.
    let flags = unsafe { libc::fcntl(fd, libc::F_GETFL) };
    // SAFETY: as above.
    assert_eq!(
        unsafe { libc::fcntl(fd, libc::F_SETFL, flags | libc::O_NONBLOCK) },
        0
    );
    while writer.write(&[0; 4096]).is_ok() {}
    // SAFETY: as above.
    assert_eq!(unsafe { libc::fcntl(fd, libc::F_SETFL, flags) }, 0);
    (reader, writer)
}

/// Read all bytes that are in `reader` now.
fn drain(reader: &mut PipeReader) {
    let mut bytes = [0; 4096];
    while wait_readable(&[reader.as_raw_fd()], Some(Duration::ZERO)) != Readiness::Pending {
        assert!(reader.read(&mut bytes).unwrap() > 0);
    }
}

#[test]
fn gives_each_call_a_new_identifier() {
    let mut dirty_io = DirtyIo::new();
    assert_eq!(dirty_io.next_call(), 0);
    assert_eq!(dirty_io.next_call(), 1);
    assert!(!dirty_io.running());
    assert!(dirty_io.wake_fd() > libc::STDERR_FILENO);
}

#[test]
fn runs_a_call_on_a_dirty_io_thread_and_wakes_the_scheduler() {
    let mut dirty_io = DirtyIo::new();
    let (mut reader, writer) = std::io::pipe().unwrap();
    let wake = [dirty_io.wake_fd()];
    dirty_io.submit(ProcessId(3), 7, write_to(&writer, b"hello\n"));
    let failed = IovecWrite {
        fd: -1,
        groups: vec![vec![b"x".to_vec()]],
        bad_group: false,
    };
    dirty_io.submit(ProcessId(4), 8, failed);
    assert!(dirty_io.running());
    let mut completions = Vec::new();
    while completions.len() < 2 {
        assert_eq!(
            wait_readable(&wake, Some(COMPLETION_LIMIT)),
            Readiness::Ready(wake.to_vec())
        );
        dirty_io.clear_wake();
        completions.extend(std::iter::from_fn(|| dirty_io.take_completion()));
    }
    completions.sort_by_key(|completion| completion.call);
    assert_eq!(
        completions,
        [
            Completion {
                process: ProcessId(3),
                call: 7,
                written: IovecWritten::All
            },
            Completion {
                process: ProcessId(4),
                call: 8,
                written: IovecWritten::Failed(libc::EBADF)
            }
        ]
    );
    assert!(!dirty_io.running());
    assert_eq!(dirty_io.take_completion(), None);
    assert_eq!(
        wait_readable(&wake, Some(Duration::ZERO)),
        Readiness::Pending
    );
    drop(writer);
    let mut bytes = Vec::new();
    reader.read_to_end(&mut bytes).unwrap();
    assert_eq!(bytes, b"hello\n");
}

#[test]
fn completes_a_later_call_while_a_call_waits_for_a_full_pipe() {
    let mut dirty_io = DirtyIo::new();
    let (mut full, full_writer) = full_pipe();
    let (mut other, other_writer) = std::io::pipe().unwrap();
    let wake = [dirty_io.wake_fd()];
    dirty_io.submit(ProcessId(0), 0, write_to(&full_writer, &[1; 4096]));
    assert_eq!(
        wait_readable(&wake, Some(Duration::from_millis(20))),
        Readiness::Pending
    );
    assert_eq!(dirty_io.take_completion(), None);
    // The second call runs on another thread while the first one waits.
    dirty_io.submit(ProcessId(1), 1, write_to(&other_writer, b"next"));
    assert_eq!(
        wait_readable(&wake, Some(COMPLETION_LIMIT)),
        Readiness::Ready(wake.to_vec())
    );
    dirty_io.clear_wake();
    let second = Completion {
        process: ProcessId(1),
        call: 1,
        written: IovecWritten::All,
    };
    assert_eq!(dirty_io.take_completion(), Some(second));
    assert_eq!(dirty_io.take_completion(), None);
    assert!(dirty_io.running());
    drain(&mut full);
    assert_eq!(
        wait_readable(&wake, Some(COMPLETION_LIMIT)),
        Readiness::Ready(wake.to_vec())
    );
    let first = Completion {
        process: ProcessId(0),
        call: 0,
        written: IovecWritten::All,
    };
    assert_eq!(dirty_io.take_completion(), Some(first));
    assert!(!dirty_io.running());
    drop(other_writer);
    let mut bytes = Vec::new();
    other.read_to_end(&mut bytes).unwrap();
    assert_eq!(bytes, b"next");
}

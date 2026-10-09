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
    while writer.write(&[0]).is_ok() {}
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
    let claim = Claim::default();
    dirty_io.submit(ProcessId(3), 7, &claim, write_to(&writer, b"hello\n"));
    let failed = IovecWrite {
        fd: -1,
        groups: vec![vec![b"x".to_vec()]],
        bad_group: false,
    };
    dirty_io.submit(ProcessId(4), 8, &claim, failed);
    assert!(dirty_io.running());
    // A thread writes its wake byte after it sends its completion, so the
    // inbox has both completions after the second byte.
    let mut wakes = 0;
    while wakes < 2 {
        assert_eq!(
            wait_readable(&wake, Some(COMPLETION_LIMIT)),
            Readiness::Ready(wake.to_vec())
        );
        wakes += (&dirty_io.wake.0).read(&mut [0; 2]).unwrap();
    }
    let mut completions: Vec<_> = std::iter::from_fn(|| dirty_io.take_completion()).collect();
    completions.sort_by_key(|completion| completion.call);
    assert_eq!(
        completions,
        [
            Completion {
                process: ProcessId(3),
                call: 7,
                written: Some(IovecWritten::All)
            },
            Completion {
                process: ProcessId(4),
                call: 8,
                written: Some(IovecWritten::Failed(libc::EBADF))
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
    let claim = Claim::default();
    dirty_io.submit(ProcessId(0), 0, &claim, write_to(&full_writer, &[1; 4096]));
    assert_eq!(
        wait_readable(&wake, Some(Duration::from_millis(20))),
        Readiness::Pending
    );
    assert_eq!(dirty_io.take_completion(), None);
    // The second call runs on another thread while the first one waits.
    dirty_io.submit(ProcessId(1), 1, &claim, write_to(&other_writer, b"next"));
    assert_eq!(
        wait_readable(&wake, Some(COMPLETION_LIMIT)),
        Readiness::Ready(wake.to_vec())
    );
    dirty_io.clear_wake();
    let second = Completion {
        process: ProcessId(1),
        call: 1,
        written: Some(IovecWritten::All),
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
        written: Some(IovecWritten::All),
    };
    assert_eq!(dirty_io.take_completion(), Some(first));
    assert!(!dirty_io.running());
    drop(other_writer);
    let mut bytes = Vec::new();
    other.read_to_end(&mut bytes).unwrap();
    assert_eq!(bytes, b"next");
}

#[test]
fn runs_no_call_of_a_dropped_claim_and_ends_the_threads_after_the_run() {
    // The 10 threads run 10 calls that wait for full pipes, and an 11th
    // call waits in the queue. At the end of the run, the run drops the
    // processes with their claims, and then the dirty IO state. The calls
    // that run continue to their end, the 11th call does not run, and each
    // thread then stops. A dirty IO scheduler does not run the NIF of an
    // exiting process (erl_process.c:10252-10258), and ERTS `halt` does not
    // wait for the dirty NIF of `prim_tty` (erl_nif.c:470-497).
    let mut dirty_io = DirtyIo::new();
    let pipes: Vec<_> = (0..10).map(|_| std::io::pipe().unwrap()).collect();
    let (queued, queued_writer) = std::io::pipe().unwrap();
    let claims: Vec<_> = (0..=10).map(|_| Claim::default()).collect();
    let bytes = vec![b'r'; 1 << 20];
    for ((call, claim), (_, writer)) in (0..10).zip(&claims).zip(&pipes) {
        dirty_io.submit(ProcessId(call), call, claim, write_to(writer, &bytes));
    }
    dirty_io.submit(
        ProcessId(10),
        10,
        &claims[10],
        write_to(&queued_writer, b"q"),
    );
    // Each of the 10 calls runs when its pipe has bytes.
    for (reader, _) in &pipes {
        assert_eq!(
            wait_readable(&[reader.as_raw_fd()], Some(COMPLETION_LIMIT)),
            Readiness::Ready(vec![reader.as_raw_fd()])
        );
    }
    let wake = dirty_io.wake.0.try_clone().unwrap();
    drop(claims);
    drop(dirty_io);
    for (reader, _) in &pipes {
        let mut written = vec![0; bytes.len()];
        (&*reader).read_exact(&mut written).unwrap();
        assert!(written == bytes);
    }
    // Each thread writes to the wake socket after each call. The end of
    // file comes when each thread stopped and dropped the write end.
    let mut wakes = [0; 64];
    loop {
        assert_eq!(
            wait_readable(&[wake.as_raw_fd()], Some(COMPLETION_LIMIT)),
            Readiness::Ready(vec![wake.as_raw_fd()])
        );
        if (&wake).read(&mut wakes).unwrap() == 0 {
            break;
        }
    }
    assert_eq!(
        wait_readable(&[queued.as_raw_fd()], Some(Duration::ZERO)),
        Readiness::Pending
    );
}

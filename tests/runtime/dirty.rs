use super::*;
use crate::runtime::host::input::{Readiness, wait_readable};
use std::{
    io::{PipeReader, PipeWriter},
    sync::Condvar,
    time::Duration,
};

/// The time that a dirty thread has to complete a call that does not wait.
const COMPLETION_LIMIT: Duration = Duration::from_secs(10);

/// Return the process, the call, and the end of the host write of a
/// completion.
fn written(completion: Completion) -> (ProcessId, u64, Option<IovecWritten>) {
    let written = completion.output.map(|output| match output {
        Output::Written(written) => written,
        other => panic!("a write gives no other output: {other:?}"),
    });
    (completion.process, completion.call, written)
}

/// Submit the host write `write` of `process` to the dirty IO pool.
fn submit_write(
    dirty: &mut DirtyCalls,
    pools: &Pools,
    process: u64,
    claim: &Claim,
    write: IovecWrite,
) -> u64 {
    dirty
        .submit(
            pools,
            Job::IoBound,
            ProcessId(process),
            Priority::Normal,
            claim,
            Work::Write(write),
        )
        .unwrap()
}

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
    assert_eq!(
        // SAFETY: as above. `flags` came from `F_GETFL` of the same open fd,
        // and `fcntl` with `F_SETFL` reads no pointer.
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
fn gives_each_call_a_new_identifier_and_no_pool_to_a_normal_job() {
    let pools = Pools::new(1);
    let mut dirty = DirtyCalls::new();
    assert!(!dirty.running());
    assert!(dirty.wake_fd() > libc::STDERR_FILENO);
    let claim = Claim::default();
    let error = dirty
        .submit(
            &pools,
            Job::Normal,
            ProcessId(0),
            Priority::Normal,
            &claim,
            Work::Write(IovecWrite {
                fd: -1,
                groups: Vec::new(),
                bad_group: false,
            }),
        )
        .unwrap_err();
    assert_eq!(
        error.to_string(),
        "invariant error: a NIF with the job Normal switches out to a dirty pool"
    );
    assert!(!dirty.running());
    let empty = || IovecWrite {
        fd: -1,
        groups: Vec::new(),
        bad_group: false,
    };
    assert_eq!(submit_write(&mut dirty, &pools, 0, &claim, empty()), 0);
    assert_eq!(submit_write(&mut dirty, &pools, 1, &claim, empty()), 1);
    assert!(dirty.running());
}

#[test]
fn runs_a_call_on_a_dirty_io_thread_and_wakes_the_scheduler() {
    let pools = Pools::new(1);
    let mut dirty_io = DirtyCalls::new();
    dirty_io.next_call = 7;
    let (mut reader, writer) = std::io::pipe().unwrap();
    let wake = [dirty_io.wake_fd()];
    let claim = Claim::default();
    submit_write(
        &mut dirty_io,
        &pools,
        3,
        &claim,
        write_to(&writer, b"hello\n"),
    );
    let failed = IovecWrite {
        fd: -1,
        groups: vec![vec![b"x".to_vec()]],
        bad_group: false,
    };
    submit_write(&mut dirty_io, &pools, 4, &claim, failed);
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
    let mut completions: Vec<_> = std::iter::from_fn(|| dirty_io.take_completion())
        .map(written)
        .collect();
    completions.sort_by_key(|completion| completion.1);
    assert_eq!(
        completions,
        [
            (ProcessId(3), 7, Some(IovecWritten::All)),
            (ProcessId(4), 8, Some(IovecWritten::Failed(libc::EBADF)))
        ]
    );
    assert!(claim.started());
    assert!(!dirty_io.running());
    assert!(dirty_io.take_completion().is_none());
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
    let pools = Pools::new(1);
    let mut dirty_io = DirtyCalls::new();
    let (mut full, full_writer) = full_pipe();
    let (mut other, other_writer) = std::io::pipe().unwrap();
    let wake = [dirty_io.wake_fd()];
    let claim = Claim::default();
    submit_write(
        &mut dirty_io,
        &pools,
        0,
        &claim,
        write_to(&full_writer, &[1; 4096]),
    );
    assert_eq!(
        wait_readable(&wake, Some(Duration::from_millis(20))),
        Readiness::Pending
    );
    assert!(dirty_io.take_completion().is_none());
    // The second call runs on another thread while the first one waits.
    submit_write(
        &mut dirty_io,
        &pools,
        1,
        &claim,
        write_to(&other_writer, b"next"),
    );
    assert_eq!(
        wait_readable(&wake, Some(COMPLETION_LIMIT)),
        Readiness::Ready(wake.to_vec())
    );
    dirty_io.clear_wake();
    assert_eq!(
        dirty_io.take_completion().map(written),
        Some((ProcessId(1), 1, Some(IovecWritten::All)))
    );
    assert!(dirty_io.take_completion().is_none());
    assert!(dirty_io.running());
    drain(&mut full);
    assert_eq!(
        wait_readable(&wake, Some(COMPLETION_LIMIT)),
        Readiness::Ready(wake.to_vec())
    );
    assert_eq!(
        dirty_io.take_completion().map(written),
        Some((ProcessId(0), 0, Some(IovecWritten::All)))
    );
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
    // processes with their claims, the dirty call state of the scheduler,
    // and then the node with its pools. The calls
    // that run continue to their end, the 11th call does not run, and each
    // thread then stops. A dirty IO scheduler does not run the NIF of an
    // exiting process (erl_process.c:10252-10258), and ERTS `halt` does not
    // wait for the dirty NIF of `prim_tty` (erl_nif.c:470-497).
    let pools = Pools::new(1);
    let mut dirty_io = DirtyCalls::new();
    let pipes: Vec<_> = (0..10).map(|_| std::io::pipe().unwrap()).collect();
    let (queued, queued_writer) = std::io::pipe().unwrap();
    let claims: Vec<_> = (0..=10).map(|_| Claim::default()).collect();
    let bytes = vec![b'r'; 1 << 20];
    for ((call, claim), (_, writer)) in (0..10).zip(&claims).zip(&pipes) {
        assert_eq!(
            submit_write(&mut dirty_io, &pools, call, claim, write_to(writer, &bytes)),
            call
        );
    }
    submit_write(
        &mut dirty_io,
        &pools,
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
    assert!(claims[..10].iter().all(Claim::started));
    assert!(!claims[10].started());
    drop(claims);
    drop(dirty_io);
    drop(pools);
    for (reader, _) in &pipes {
        let mut written = vec![0; bytes.len()];
        (&*reader).read_exact(&mut written).unwrap();
        assert_eq!(written, bytes);
    }
    // Each thread writes to the wake socket after each call. The end of
    // file comes when each call dropped its copy of the write end.
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

/// The gate of [`gated_thread_name`]: the number of calls that started, and
/// whether the test opened the gate.
static GATE: (Mutex<(usize, bool)>, Condvar) = (Mutex::new((0, false)), Condvar::new());

/// Count the start of the call, wait until the test opens [`GATE`], and
/// return the name of the thread as a binary.
#[expect(
    clippy::unnecessary_wraps,
    reason = "NIF function pointer; every NIF returns `Result<NativeResult, BeamError>`"
)]
fn gated_thread_name(_: &mut NifContext<'_>, _: &[Term]) -> Result<NativeResult, BeamError> {
    let (state, changed) = &GATE;
    let mut state = state.lock().unwrap();
    state.0 += 1;
    changed.notify_all();
    while !state.1 {
        state = changed.wait(state).unwrap();
    }
    let name = thread::current().name().unwrap().as_bytes().to_vec();
    Ok(NativeResult::Value(Term::binary(name)))
}

#[test]
fn runs_cpu_bound_calls_on_one_dirty_cpu_thread_for_each_scheduler() {
    // A node of 2 schedulers has 2 dirty CPU threads (erl_init.c:865),
    // named `erts_dcpus_1` and `erts_dcpus_2` (erl_process.c:8958). Two
    // calls that wait for the gate start on the two threads, and the third
    // waits in the queue until a thread is free.
    let pools = Pools::new(2);
    let mut dirty = DirtyCalls::new();
    let claims: Vec<_> = (0..3).map(|_| Claim::default()).collect();
    for (process, claim) in (0..).zip(&claims) {
        let work = Work::Native {
            function: gated_thread_name,
            arguments: Vec::new(),
        };
        dirty
            .submit(
                &pools,
                Job::CpuBound,
                ProcessId(process),
                Priority::Normal,
                claim,
                work,
            )
            .unwrap();
    }
    let (state, changed) = &GATE;
    let mut started = state.lock().unwrap();
    while started.0 < 2 {
        started = changed.wait(started).unwrap();
    }
    assert_eq!(claims.iter().filter(|claim| claim.started()).count(), 2);
    started.1 = true;
    changed.notify_all();
    drop(started);
    let mut names = Vec::new();
    while names.len() < 3 {
        assert_ne!(
            wait_readable(&[dirty.wake_fd()], Some(COMPLETION_LIMIT)),
            Readiness::Pending
        );
        dirty.clear_wake();
        while let Some(completion) = dirty.take_completion() {
            let Some(Output::Native(Ok(NativeResult::Value(Term::Binary(ref name))))) =
                completion.output
            else {
                panic!("the call returns the thread name: {completion:?}");
            };
            names.push(String::from_utf8(name.bit_bytes().to_vec()).unwrap());
        }
    }
    assert!(claims.iter().all(Claim::started));
    names.sort();
    names.dedup();
    assert_eq!(names, ["erts_dcpus_1", "erts_dcpus_2"]);
}

/// A value that sends the name of the thread that drops it.
struct DropProbe(mpsc::Sender<Option<String>>);

impl Drop for DropProbe {
    fn drop(&mut self) {
        _ = self.0.send(thread::current().name().map(str::to_owned));
    }
}

/// Return the host part of a static binding whose finish owns `probe`, as
/// the finish of `prim_file:open_nif/2` owns the open file.
fn probe_host(probe: DropProbe) -> Work {
    Work::Host(Host(Box::new(move || {
        let finish: Box<FinishFn> = Box::new(move |_| {
            drop(probe);
            Ok(BifResult::Value(Term::nil()))
        });
        finish
    })))
}

#[test]
fn runs_the_host_part_of_a_static_binding_and_gives_its_finish() {
    let (sender, _dropped) = mpsc::channel();
    let work = probe_host(DropProbe(sender));
    assert_eq!(format!("{work:?}"), "Host(Host)");
    assert_eq!(format!("{:?}", work.run(&Arc::default())), "Host(Finish)");
    let discard = Discard(Box::new(|_| Ok(BifResult::Value(Term::nil()))));
    assert_eq!(format!("{discard:?}"), "Discard");
}

#[test]
fn drops_a_finish_that_did_not_run_on_a_dirty_io_thread() {
    // A caller that exits before its finish runs leaves the finish to the
    // scheduler, which drops it. A dirty IO thread then drops the function
    // and the host resource that it owns, such as the open file of
    // `open_nif/2`, so a `close` that waits does not stop the scheduler.
    let pools = Pools::new(1);
    let mut dirty_io = DirtyCalls::new();
    let wake = [dirty_io.wake_fd()];
    let claim = Claim::default();
    let (sender, dropped) = mpsc::channel();
    let work = probe_host(DropProbe(sender));
    dirty_io
        .submit(
            &pools,
            Job::IoBound,
            ProcessId(3),
            Priority::Normal,
            &claim,
            work,
        )
        .unwrap();
    assert_eq!(
        wait_readable(&wake, Some(COMPLETION_LIMIT)),
        Readiness::Ready(wake.to_vec())
    );
    let completion = dirty_io.take_completion().unwrap();
    assert_eq!(format!("{:?}", completion.output), "Some(Host(Finish))");
    drop(completion);
    let name = dropped.recv_timeout(COMPLETION_LIMIT).unwrap().unwrap();
    assert!(name.starts_with("erts_dios_"), "{name}");
    // After the node dropped the pool, the current thread drops the
    // function.
    let (sender, dropped) = mpsc::channel();
    let output = probe_host(DropProbe(sender)).run(&pools.io.shared);
    drop(pools);
    drop(output);
    assert_eq!(
        dropped.try_recv().unwrap(),
        thread::current().name().map(str::to_owned)
    );
}

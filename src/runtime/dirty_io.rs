//! The dirty IO threads of a run: the host calls that ERTS runs on a dirty
//! IO scheduler run here, off the scheduler thread, so a call that waits
//! for the host does not stop the other processes.
//!
//! ERTS starts 10 dirty IO schedulers (`erl_init.c:867`). A process that
//! calls a dirty IO NIF switches out of its normal scheduler and goes to
//! the dirty IO run queue (`static_schedule_dirty_nif`,
//! `erl_nif.c:3437-3477`; `erl_process.c:10244-10262`). A dirty IO
//! scheduler calls the NIF (`erts_call_dirty_nif`, `erl_nif.c:426-543`),
//! and the process then goes back to a normal run queue, where it returns
//! the result or raises the exception of the call (`dirty_nif_finalizer`
//! and `dirty_nif_exception`, `erl_nif.c:3386-3435`).
//!
//! PBeam has one scheduler thread. The scheduler registers the call on the
//! process before it gives the call to [`DirtyIo::submit`], so a completion
//! always finds a registered call. A dirty IO thread sends each completion
//! to the scheduler inbox and then writes one byte to the wake socket, so
//! an idle scheduler that waits on the socket wakes up.

use std::{
    ffi::c_int,
    io::{Read, Write},
    os::{fd::AsRawFd, unix::net::UnixStream},
    sync::{
        Arc, Mutex,
        mpsc::{self, Receiver, Sender},
    },
    thread,
};

use super::host::output::{IovecWrite, IovecWritten};
use crate::term::ProcessId;

/// The number of dirty IO threads, as the default number of ERTS dirty IO
/// schedulers (`dirty_io_scheds = 10`, `erl_init.c:867`).
///
/// # Examples
///
/// ```rust,ignore
/// assert_eq!(DIRTY_IO_THREADS, 10);
/// ```
const DIRTY_IO_THREADS: usize = 10;

/// One host call for a dirty IO thread.
#[derive(Debug)]
struct Job {
    /// The process that waits for the call.
    process: ProcessId,
    /// The identifier of the call in the run.
    call: u64,
    /// The host work of the call.
    write: IovecWrite,
}

/// The end of one host call, which a dirty IO thread sends to the scheduler
/// inbox.
#[derive(Debug, PartialEq, Eq)]
pub(super) struct Completion {
    /// The process that waits for the call.
    pub(super) process: ProcessId,
    /// The identifier of the call in the run.
    pub(super) call: u64,
    /// The result of the host work.
    pub(super) written: IovecWritten,
}

/// The dirty IO threads of a run, the queue of their calls, and the
/// scheduler inbox for the completions.
#[derive(Debug)]
pub(super) struct DirtyIo {
    /// The send end of the call queue. `None` until the first call starts
    /// the threads. When the run drops it, the threads still run the calls
    /// in the queue, and each thread stops when the queue is empty. The
    /// end of the run with calls that run or wait is task #20.
    jobs: Option<Sender<Job>>,
    /// The send end of the scheduler inbox. Each thread holds a copy.
    completion_sender: Sender<Completion>,
    /// The scheduler inbox: the completions that the scheduler did not take.
    completions: Receiver<Completion>,
    /// The non-blocking socket pair that wakes an idle scheduler: a thread
    /// writes one byte to the second socket after each completion, and the
    /// scheduler waits until the first socket can be read. The socket pair
    /// is open until the run drops it.
    wake: (UnixStream, Arc<UnixStream>),
    /// The number of calls whose completion the scheduler did not take.
    running: usize,
    /// The identifier of the next call.
    next_call: u64,
}

impl DirtyIo {
    /// Return the dirty IO state of a new run: no thread and no call.
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// let dirty_io = DirtyIo::new();
    /// assert!(!dirty_io.running());
    /// ```
    ///
    /// # Panics
    ///
    /// The host cannot make the wake socket pair, for example when the OS
    /// process has no free fd. ERTS also stops when it cannot make its wake
    /// pipe (`create_wakeup_pipe`, `erl_poll.c:489-505`).
    pub(super) fn new() -> Self {
        let (completion_sender, completions) = mpsc::channel();
        // `try_clone` gives an fd of 3 or more (`F_DUPFD_CLOEXEC`), so the
        // socket pair does not take a closed standard fd, which the run
        // reads or writes as fd 0, 1, or 2.
        let (read, write) = UnixStream::pair()
            .and_then(|(read, write)| Ok((read.try_clone()?, write.try_clone()?)))
            .expect("the host makes a socket pair");
        for socket in [&read, &write] {
            socket
                .set_nonblocking(true)
                .expect("the host sets a socket to non-blocking");
        }
        DirtyIo {
            jobs: None,
            completion_sender,
            completions,
            wake: (read, Arc::new(write)),
            running: 0,
            next_call: 0,
        }
    }

    /// Return the identifier of the next call. Each identifier is new in
    /// the run.
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// assert_eq!(dirty_io.next_call(), 0);
    /// assert_eq!(dirty_io.next_call(), 1);
    /// ```
    pub(super) fn next_call(&mut self) -> u64 {
        let call = self.next_call;
        self.next_call += 1;
        call
    }

    /// Give the call `call` of `process` with its host work `write` to the
    /// dirty IO threads. The first call starts the threads. The threads
    /// take the calls in order.
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// let call = dirty_io.next_call();
    /// dirty_io.submit(ProcessId(0), call, write);
    /// assert!(dirty_io.running());
    /// ```
    ///
    /// # Panics
    ///
    /// The host cannot start a thread. ERTS stops with `Failed to create
    /// dirty io scheduler thread` in this case (`erl_process.c:8964-8971`).
    pub(super) fn submit(&mut self, process: ProcessId, call: u64, write: IovecWrite) {
        self.running += 1;
        let jobs = self.jobs.get_or_insert_with(|| {
            let (sender, receiver) = mpsc::channel::<Job>();
            let receiver = Arc::new(Mutex::new(receiver));
            for index in 1..=DIRTY_IO_THREADS {
                let receiver = Arc::clone(&receiver);
                let completions = self.completion_sender.clone();
                let wake = Arc::clone(&self.wake.1);
                thread::Builder::new()
                    .name(format!("erts_dios_{index}"))
                    .spawn(move || run_jobs(&receiver, &completions, &wake))
                    .expect("the host starts a dirty IO thread");
            }
            sender
        });
        jobs.send(Job {
            process,
            call,
            write,
        })
        .expect("the dirty IO threads run until the run ends");
    }

    /// Return true while a call has a completion that the scheduler did not
    /// take.
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// assert!(!DirtyIo::new().running());
    /// ```
    pub(super) fn running(&self) -> bool {
        self.running > 0
    }

    /// Return the fd that an idle scheduler waits on: it can be read when a
    /// completion arrived after the last [`Self::clear_wake`].
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// let fds = [libc::STDIN_FILENO, dirty_io.wake_fd()];
    /// ```
    pub(super) fn wake_fd(&self) -> c_int {
        self.wake.0.as_raw_fd()
    }

    /// Read all bytes from the wake socket, so the next wait on
    /// [`Self::wake_fd`] waits for a new completion.
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// dirty_io.clear_wake();
    /// ```
    pub(super) fn clear_wake(&self) {
        let mut bytes = [0; 64];
        while (&self.wake.0).read(&mut bytes).is_ok_and(|count| count > 0) {}
    }

    /// Take the next completion from the scheduler inbox, or `None` when
    /// the inbox is empty. The take does not wait.
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// while let Some(completion) = dirty_io.take_completion() {
    ///     // resume completion.process
    /// }
    /// ```
    pub(super) fn take_completion(&mut self) -> Option<Completion> {
        let completion = self.completions.try_recv().ok()?;
        self.running -= 1;
        Some(completion)
    }
}

/// Run the calls of the queue until the run drops the queue and the queue
/// is empty: write each call, send its completion to the scheduler inbox,
/// then write one byte to the wake socket, as `wake_poller` writes to the
/// ERTS wake pipe (`erl_poll.c:429-462`). A full wake socket already wakes the scheduler,
/// so the thread ignores a write that fails with `WouldBlock`.
///
/// # Examples
///
/// ```rust,ignore
/// thread::spawn(move || run_jobs(&receiver, &completions, &wake));
/// ```
fn run_jobs(jobs: &Mutex<Receiver<Job>>, completions: &Sender<Completion>, wake: &UnixStream) {
    loop {
        let job = jobs.lock().map(|jobs| jobs.recv());
        let Ok(Ok(Job {
            process,
            call,
            write,
        })) = job
        else {
            return;
        };
        let written = write.run();
        let completion = Completion {
            process,
            call,
            written,
        };
        // The send fails only after the run dropped the inbox. The run then
        // also dropped the queue, so a receive ends the thread when the
        // queue is empty.
        _ = completions.send(completion);
        _ = (&*wake).write(&[0]);
    }
}

#[cfg(test)]
#[path = "../../tests/runtime/dirty_io.rs"]
mod tests;

//! The dirty schedulers of a run: the NIF calls that ERTS runs on a dirty
//! CPU or a dirty IO scheduler run here, off the scheduler thread, so a
//! long call does not stop the other processes.
//!
//! ERTS starts one dirty CPU scheduler for each scheduler and 10 dirty IO
//! schedulers (`erl_init.c:865-867`), with the thread names `erts_dcpus_N`
//! and `erts_dios_N` (`erl_process.c:8956-8971`). A process that calls a
//! dirty NIF switches out of its normal scheduler and goes to the dirty run
//! queue of the job of the NIF (`static_schedule_dirty_nif`,
//! `erl_nif.c:3437-3477`; `erl_process.c:10244-10262`). A dirty scheduler
//! calls the NIF (`erts_call_dirty_nif`, `erl_nif.c:426-543`), and the
//! process then goes back to a normal run queue, where it returns the
//! result or raises the exception of the call (`dirty_nif_finalizer` and
//! `dirty_nif_exception`, `erl_nif.c:3386-3435`).
//!
//! The [`Pools`] of the threads belong to the node, so the schedulers of a
//! run share them. Each scheduler has its own [`DirtyCalls`]: the inbox of
//! the completions of its calls and its wake socket. The scheduler
//! registers the call on the process before the scheduling step that can
//! take its completion, so a completion always finds a registered call. A
//! dirty thread sends each completion to the inbox of the scheduler of the
//! call and then writes one byte to its wake socket, so an idle scheduler
//! that waits on the socket wakes up.
//!
//! A call does not stop when its caller exits. A thread starts a call only
//! while the caller holds the [`Claim`] of the call. The caller drops the
//! claim when it starts to exit, and the run drops the claims of all
//! processes when it ends. A call in the queue then does not run, as a
//! dirty scheduler does not call the NIF of an exiting process
//! (`erl_process.c:10252-10258`). A call that runs continues until its
//! work ends, as ERTS cannot stop a dirty NIF, and the scheduler drops its
//! result (`erl_nif.c:507-542`).

use std::{
    ffi::c_int,
    io::{Read, Write},
    os::{fd::AsRawFd, unix::net::UnixStream},
    sync::{
        Arc, Mutex, OnceLock, Weak,
        atomic::{AtomicBool, Ordering},
        mpsc::{self, Receiver, Sender},
    },
    thread,
};

use super::{
    Job, NativeFn, NativeResult,
    host::output::{IovecWrite, IovecWritten},
};
use crate::{
    error::{BeamError, error},
    term::{ProcessId, Term},
};

/// The number of dirty IO threads, as the default number of ERTS dirty IO
/// schedulers (`dirty_io_scheds = 10`, `erl_init.c:867`).
///
/// # Examples
///
/// ```rust,ignore
/// assert_eq!(DIRTY_IO_THREADS, 10);
/// ```
const DIRTY_IO_THREADS: usize = 10;

/// The work of one dirty NIF call, which a dirty thread runs.
#[derive(Debug)]
pub(super) enum Work {
    /// The host write of `prim_tty:write_nif/2`.
    Write(IovecWrite),
    /// A registered native function with the arguments of its call.
    Native {
        /// The function of the registration.
        function: NativeFn,
        /// The arguments of the call.
        arguments: Vec<Term>,
    },
}

impl Work {
    /// Run the work on the current thread and return its output.
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// let output = Work::Native { function, arguments: vec![Term::Integer(1)] }.run();
    /// assert!(matches!(output, Output::Native(Ok(_))));
    /// ```
    fn run(self) -> Output {
        match self {
            Work::Write(write) => Output::Written(write.run()),
            Work::Native {
                function,
                arguments,
            } => Output::Native(function(&arguments)),
        }
    }
}

/// The output of the work of one dirty NIF call. The caller turns it into
/// the result of the call in its next turn.
#[derive(Debug)]
pub(super) enum Output {
    /// The end of a host write.
    Written(IovecWritten),
    /// The result of a registered native function.
    Native(Result<NativeResult, BeamError>),
}

/// One call for a dirty thread.
#[derive(Debug)]
struct Call {
    /// The process that waits for the call.
    process: ProcessId,
    /// The identifier of the call in the inbox of its scheduler.
    call: u64,
    /// The claim of the caller. The call runs only when the claim still
    /// exists.
    claim: Weak<AtomicBool>,
    /// The work of the call.
    work: Work,
    /// The inbox of the scheduler of the call.
    completions: Sender<Completion>,
    /// The wake socket pair of the scheduler of the call. The call holds
    /// both ends, so its wake byte never goes to a socket whose read end is
    /// closed, which gives `EPIPE` and `SIGPIPE`.
    wake: Arc<(UnixStream, UnixStream)>,
}

/// The claim of a caller on its dirty call. The caller holds it from the
/// call until the call completes or the caller starts to exit. A dirty
/// thread runs the call only while the claim exists, as a dirty scheduler
/// runs the NIF only for a process that does not exit
/// (`erl_process.c:10252-10258`). The flag of the claim is true after a
/// thread started the call.
#[derive(Debug, Default)]
pub(super) struct Claim(Arc<AtomicBool>);

impl Claim {
    /// Return true after a dirty thread started the call, as the
    /// `ERTS_PSFLG_DIRTY_RUNNING` state of a process that runs on a dirty
    /// scheduler (`erl_process.c:10244-10262`).
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// assert!(!Claim::default().started());
    /// ```
    pub(super) fn started(&self) -> bool {
        self.0.load(Ordering::Acquire)
    }
}

/// The end of one call, which a dirty thread sends to the inbox of the
/// scheduler of the call.
#[derive(Debug)]
pub(super) struct Completion {
    /// The process that waits for the call.
    pub(super) process: ProcessId,
    /// The identifier of the call in the inbox of its scheduler.
    pub(super) call: u64,
    /// The output of the work, or `None` when the caller dropped the
    /// [`Claim`] before a thread took the call, so the call did not run.
    pub(super) output: Option<Output>,
}

/// The threads of one dirty pool and the send end of their queue.
#[derive(Debug)]
struct Pool {
    /// The prefix of the thread names, as ERTS names its dirty scheduler
    /// threads (`erl_process.c:8958`, `8966`).
    name: &'static str,
    /// The number of threads.
    threads: usize,
    /// The send end of the call queue, which the first call sets when it
    /// starts the threads. When the node drops it, the threads take the
    /// calls that are still in the queue, but they do not run them,
    /// because the run dropped the claims. Each thread stops when the
    /// queue is empty.
    calls: OnceLock<Sender<Call>>,
}

impl Pool {
    /// Give `call` to the threads of the pool. The first call starts the
    /// threads.
    ///
    /// # Panics
    ///
    /// The host cannot start a thread. ERTS stops with `Failed to create
    /// dirty cpu scheduler thread` or `Failed to create dirty io scheduler
    /// thread` in this case (`erl_process.c:8962`, `8970`).
    fn submit(&self, call: Call) {
        let calls = self.calls.get_or_init(|| {
            let (sender, receiver) = mpsc::channel::<Call>();
            let receiver = Arc::new(Mutex::new(receiver));
            for index in 1..=self.threads {
                let receiver = Arc::clone(&receiver);
                thread::Builder::new()
                    .name(format!("{}_{index}", self.name))
                    .spawn(move || run_calls(&receiver))
                    .expect("the host starts a dirty scheduler thread");
            }
            sender
        });
        calls
            .send(call)
            .expect("the dirty threads run until the node ends");
    }
}

/// The dirty CPU pool and the dirty IO pool of a node.
#[derive(Debug)]
pub(in crate::runtime) struct Pools {
    /// The pool of the `CpuBound` calls.
    cpu: Pool,
    /// The pool of the `IoBound` calls.
    io: Pool,
}

impl Pools {
    /// Return the pools of a node with `schedulers` schedulers: as many
    /// dirty CPU threads as schedulers and 10 dirty IO threads
    /// (`erl_init.c:865-867`). No thread starts before the first call.
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// let pools = Pools::new(2);
    /// ```
    pub(in crate::runtime) fn new(schedulers: usize) -> Self {
        Pools {
            cpu: Pool {
                name: "erts_dcpus",
                threads: schedulers,
                calls: OnceLock::new(),
            },
            io: Pool {
                name: "erts_dios",
                threads: DIRTY_IO_THREADS,
                calls: OnceLock::new(),
            },
        }
    }

    /// Return the pool of `job`.
    ///
    /// # Errors
    ///
    /// A `Normal` job has no pool: its NIF runs on the scheduler thread.
    fn pool(&self, job: Job) -> Result<&Pool, BeamError> {
        match job {
            Job::CpuBound => Ok(&self.cpu),
            Job::IoBound => Ok(&self.io),
            Job::Normal => Err(error(
                "invariant error: a NIF with the job Normal switches out to a dirty pool",
            )),
        }
    }
}

/// The dirty calls of one scheduler: the inbox of their completions, the
/// wake socket, and the call count.
#[derive(Debug)]
pub(super) struct DirtyCalls {
    /// The send end of the inbox. Each call holds a copy.
    completion_sender: Sender<Completion>,
    /// The inbox: the completions that the scheduler did not take.
    completions: Receiver<Completion>,
    /// The non-blocking socket pair that wakes an idle scheduler: a thread
    /// writes one byte to the second socket after each completion, and the
    /// scheduler waits until the first socket can be read. The socket pair
    /// is open until the scheduler and each call drop it.
    wake: Arc<(UnixStream, UnixStream)>,
    /// The number of calls whose completion the scheduler did not take.
    running: usize,
    /// The identifier of the next call.
    next_call: u64,
}

impl DirtyCalls {
    /// Return the dirty call state of a new scheduler: no call.
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// let dirty = DirtyCalls::new();
    /// assert!(!dirty.running());
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
        DirtyCalls {
            completion_sender,
            completions,
            wake: Arc::new((read, write)),
            running: 0,
            next_call: 0,
        }
    }

    /// Give the work `work` of a call of `process` to the pool of `job` in
    /// `pools`, and return the identifier of the call, which is new in the
    /// inbox. The threads of a pool take the calls in order, and run a call
    /// only while `claim` exists.
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// let claim = Claim::default();
    /// assert_eq!(dirty.submit(&pools, Job::IoBound, ProcessId(0), &claim, Work::Write(write))?, 0);
    /// assert!(dirty.running());
    /// ```
    ///
    /// # Errors
    ///
    /// The error of a `Normal` job, which has no pool.
    pub(super) fn submit(
        &mut self,
        pools: &Pools,
        job: Job,
        process: ProcessId,
        claim: &Claim,
        work: Work,
    ) -> Result<u64, BeamError> {
        let pool = pools.pool(job)?;
        let call = self.next_call;
        self.next_call += 1;
        self.running += 1;
        pool.submit(Call {
            process,
            call,
            claim: Arc::downgrade(&claim.0),
            work,
            completions: self.completion_sender.clone(),
            wake: Arc::clone(&self.wake),
        });
        Ok(call)
    }

    /// Return true while a call has a completion that the scheduler did not
    /// take.
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// assert!(!DirtyCalls::new().running());
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
    /// let fds = [libc::STDIN_FILENO, dirty.wake_fd()];
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
    /// dirty.clear_wake();
    /// ```
    pub(super) fn clear_wake(&self) {
        let mut bytes = [0; 64];
        while (&self.wake.0).read(&mut bytes).is_ok_and(|count| count > 0) {}
    }

    /// Take the next completion from the inbox, or `None` when the inbox is
    /// empty. The take does not wait.
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// while let Some(completion) = dirty.take_completion() {
    ///     // resume completion.process
    /// }
    /// ```
    pub(super) fn take_completion(&mut self) -> Option<Completion> {
        let completion = self.completions.try_recv().ok()?;
        self.running -= 1;
        Some(completion)
    }
}

/// Run the calls of the queue until the node drops the queue and the queue
/// is empty: run each call whose claim exists, send its completion to the
/// inbox of its scheduler, then write one byte to the wake socket of the
/// scheduler, as `wake_poller` writes to the ERTS wake pipe
/// (`erl_poll.c:429-462`). A call whose claim is gone does not run, and its
/// completion has no output. A full wake socket already wakes the
/// scheduler, so the thread ignores a write that fails with `WouldBlock`.
///
/// # Examples
///
/// ```rust,ignore
/// thread::spawn(move || run_calls(&receiver));
/// ```
fn run_calls(calls: &Mutex<Receiver<Call>>) {
    loop {
        let call = calls.lock().map(|calls| calls.recv());
        let Ok(Ok(Call {
            process,
            call,
            claim,
            work,
            completions,
            wake,
        })) = call
        else {
            return;
        };
        // The thread does not keep the claim: a caller that exits while the
        // call runs drops it, and the scheduler then drops the result.
        let output = claim.upgrade().map(|claim| {
            claim.store(true, Ordering::Release);
            drop(claim);
            work.run()
        });
        let completion = Completion {
            process,
            call,
            output,
        };
        // The send fails only after the scheduler of the call dropped its
        // inbox at the end of the run.
        _ = completions.send(completion);
        _ = (&wake.1).write(&[0]);
    }
}

#[cfg(test)]
#[path = "../../tests/runtime/dirty.rs"]
mod tests;

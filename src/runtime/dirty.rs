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
    collections::HashMap,
    ffi::c_int,
    io::{Read, Write},
    os::{fd::AsRawFd, unix::net::UnixStream},
    sync::{
        Arc, Condvar, Mutex, OnceLock, Weak,
        atomic::{AtomicBool, Ordering},
        mpsc::{self, Receiver, Sender},
    },
    thread,
};

use super::{
    Job, NativeFn, NativeResult, NifContext,
    host::output::{IovecWrite, IovecWritten},
    interpreter::BifResult,
    nif::abi,
    process::Priority,
    scheduler::{Scheduler, run_queue::RunQueue},
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

/// The function of a [`Finish`].
pub(super) type FinishFn = dyn FnOnce(&mut Scheduler) -> Result<BifResult, BeamError> + Send;

/// The last part of a static binding that ran on a dirty thread: it makes
/// the result of the call on the scheduler thread of the caller, with the
/// atoms and the resources of the scheduler.
///
/// The function can own a host resource, such as the open file of
/// `prim_file:open_nif/2`. When the scheduler drops a finish that did not
/// run, because its caller exits or the run ends, the drop gives the
/// function back to the dirty pool of the call, and a dirty thread drops
/// it. Thus a `close` that waits does not stop the scheduler thread, as
/// ERTS closes the file of an open whose caller exited on the dirty
/// scheduler (`create_ref_or_error_tuple`, `prim_file_nif.c:547-553`).
pub(super) struct Finish {
    /// The function, until it runs or goes back to the pool.
    run: Option<Box<FinishFn>>,
    /// The shared queue of the pool of the call.
    pool: Arc<Shared>,
}

impl Finish {
    /// Run the finish on the scheduler thread of the caller.
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// let result = finish.run(&mut scheduler)?;
    /// ```
    ///
    /// # Errors
    ///
    /// The error of the function of the finish.
    pub(super) fn run(mut self, scheduler: &mut Scheduler) -> Result<BifResult, BeamError> {
        let run = self.run.take().expect("a finish runs once");
        run(scheduler)
    }
}

impl Drop for Finish {
    fn drop(&mut self) {
        if let Some(run) = self.run.take() {
            self.pool.discard(run);
        }
    }
}

/// The host part of a static binding: the host calls of the NIF, which run
/// on a dirty thread and give the function of the [`Finish`] of the call.
/// The binding reads the terms and the resources of its arguments on the
/// scheduler thread first, because they belong to that thread.
pub(super) struct Host(pub(super) Box<dyn FnOnce() -> Box<FinishFn> + Send>);

impl std::fmt::Debug for Finish {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("Finish")
    }
}

impl std::fmt::Debug for Host {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("Host")
    }
}

/// The work of one dirty NIF call, which a dirty thread runs.
#[derive(Debug)]
pub(super) enum Work {
    /// The host write of `prim_tty:write_nif/2`.
    Write(IovecWrite),
    /// The host part of a static binding, such as a `prim_file` NIF.
    Host(Host),
    /// An extension binding with the arguments of its call.
    Native {
        /// The run function of the binding.
        function: NativeFn,
        /// The arguments of the call.
        arguments: Vec<Term>,
    },
    /// A function of a NIF shared library with the arguments of its call.
    /// The work keeps the library open until the call ends.
    Shared {
        /// The library of the function.
        library: Arc<abi::Library>,
        /// The index of the function in the functions of the library.
        index: usize,
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
    /// let output = Work::Native { function, arguments: vec![Term::Integer(1)] }.run(&pool);
    /// assert!(matches!(output, Output::Native(Ok(_))));
    /// ```
    fn run(self, pool: &Arc<Shared>) -> Output {
        match self {
            Work::Write(write) => Output::Written(write.run()),
            Work::Host(Host(host)) => Output::Host(Finish {
                run: Some(host()),
                pool: Arc::clone(pool),
            }),
            Work::Native {
                function,
                arguments,
            } => Output::Native(function(&mut NifContext::dirty(), &arguments)),
            Work::Shared {
                library,
                index,
                arguments,
            } => Output::Native(library.functions()[index].call(&arguments)),
        }
    }
}

/// The output of the work of one dirty NIF call. The caller turns it into
/// the result of the call in its next turn.
#[derive(Debug)]
pub(super) enum Output {
    /// The end of a host write.
    Written(IovecWritten),
    /// The last part of a static binding.
    Host(Finish),
    /// The result of an extension binding.
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

/// The run queue of one dirty pool: the calls that no thread took yet, in
/// the order of the ERTS dirty run queue of the pool.
#[derive(Debug, Default)]
struct Queue {
    /// The order of the callers, as an ERTS run queue: by the priority of
    /// the caller at the call, with `normal` and `low` in one queue
    /// (`select_enqueue_run_queue`, `erl_process.c:6630-6660`). A dirty
    /// scheduler picks as a normal scheduler does (`erl_process.c:9993-10024`;
    /// `check_requeue_process`, `erl_process.c:4039-4052`).
    order: RunQueue,
    /// The call of each caller in `order`. A process has at most one dirty
    /// call, and process identifiers are not used again in a run.
    calls: HashMap<ProcessId, Call>,
    /// The functions of the finishes that the scheduler dropped before they
    /// ran. A thread drops them before it takes a call.
    discards: Vec<Discard>,
    /// True after the node dropped the pool.
    closed: bool,
}

/// The function of a [`Finish`] that did not run, which a dirty thread
/// drops.
struct Discard(Box<FinishFn>);

impl std::fmt::Debug for Discard {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("Discard")
    }
}

/// The next job of a dirty thread.
enum Taken {
    /// A call to run.
    Call(Call),
    /// A function of a finish to drop.
    Discard(Discard),
}

/// The queue of a pool and the condition that wakes its idle threads.
#[derive(Debug, Default)]
struct Shared {
    /// The queue of the calls.
    queue: Mutex<Queue>,
    /// The condition that a thread waits on while the queue is empty.
    ready: Condvar,
}

impl Shared {
    /// Give the function `run` of a finish that did not run to a thread of
    /// the pool, which drops it. After the node dropped the pool, the
    /// current thread drops it, because the threads stop when the queue is
    /// empty.
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// shared.discard(run);
    /// ```
    fn discard(&self, run: Box<FinishFn>) {
        let mut queue = self.queue.lock().expect("the dirty queue is not poisoned");
        if queue.closed {
            drop(queue);
            drop(run);
            return;
        }
        queue.discards.push(Discard(run));
        drop(queue);
        self.ready.notify_one();
    }

    /// Return the next job of the queue: a discarded function first, then
    /// the next call. Wait while the queue is empty and the pool is open.
    /// Return `None` when the pool is closed and the queue is empty.
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// while let Some(taken) = shared.take() { /* run or drop */ }
    /// ```
    fn take(&self) -> Option<Taken> {
        let mut queue = self.queue.lock().expect("the dirty queue is not poisoned");
        loop {
            if let Some(discard) = queue.discards.pop() {
                return Some(Taken::Discard(discard));
            }
            if let Some(process) = queue.order.pop() {
                let call = queue.calls.remove(&process);
                return Some(Taken::Call(
                    call.expect("each caller in the order has its call"),
                ));
            }
            if queue.closed {
                return None;
            }
            queue = self
                .ready
                .wait(queue)
                .expect("the dirty queue is not poisoned");
        }
    }
}

/// The threads of one dirty pool and their queue.
#[derive(Debug)]
struct Pool {
    /// The prefix of the thread names, as ERTS names its dirty scheduler
    /// threads (`erl_process.c:8958`, `8966`).
    name: &'static str,
    /// The number of threads.
    threads: usize,
    /// The queue, which the threads share.
    shared: Arc<Shared>,
    /// Set when the first call started the threads.
    started: OnceLock<()>,
}

impl Pool {
    /// Return the pool of `threads` threads named `name_N`, with no thread
    /// started.
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// let pool = Pool::new("erts_dios", 10);
    /// ```
    fn new(name: &'static str, threads: usize) -> Self {
        Pool {
            name,
            threads,
            shared: Arc::default(),
            started: OnceLock::new(),
        }
    }

    /// Put `call` of a caller with `priority` into the queue of the pool.
    /// The first call starts the threads.
    ///
    /// # Panics
    ///
    /// The host cannot start a thread. ERTS stops with `Failed to create
    /// dirty cpu scheduler thread` or `Failed to create dirty io scheduler
    /// thread` in this case (`erl_process.c:8962`, `8970`).
    fn submit(&self, priority: Priority, call: Call) {
        self.started.get_or_init(|| {
            for index in 1..=self.threads {
                let shared = Arc::clone(&self.shared);
                thread::Builder::new()
                    .name(format!("{}_{index}", self.name))
                    .spawn(move || run_calls(&shared))
                    .expect("the host starts a dirty scheduler thread");
            }
        });
        let mut queue = self
            .shared
            .queue
            .lock()
            .expect("the dirty queue is not poisoned");
        queue.order.push(call.process, priority);
        queue.calls.insert(call.process, call);
        drop(queue);
        self.shared.ready.notify_one();
    }
}

impl Drop for Pool {
    /// Close the queue. The threads take the calls that are still in the
    /// queue, but they do not run them, because the run dropped the claims.
    /// Each thread stops when the queue is empty.
    fn drop(&mut self) {
        let mut queue = self
            .shared
            .queue
            .lock()
            .expect("the dirty queue is not poisoned");
        queue.closed = true;
        drop(queue);
        self.shared.ready.notify_all();
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
            cpu: Pool::new("erts_dcpus", schedulers),
            io: Pool::new("erts_dios", DIRTY_IO_THREADS),
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

    /// Give the work `work` of a call of `process`, whose priority is
    /// `priority`, to the pool of `job` in `pools`, and return the
    /// identifier of the call, which is new in the inbox. The threads of a
    /// pool take the calls in the order of an ERTS run queue, and run a
    /// call only while `claim` exists.
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// let claim = Claim::default();
    /// assert_eq!(dirty.submit(&pools, Job::IoBound, ProcessId(0), Priority::Normal, &claim, Work::Write(write))?, 0);
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
        priority: Priority,
        claim: &Claim,
        work: Work,
    ) -> Result<u64, BeamError> {
        let pool = pools.pool(job)?;
        let call = self.next_call;
        self.next_call += 1;
        self.running += 1;
        pool.submit(
            priority,
            Call {
                process,
                call,
                claim: Arc::downgrade(&claim.0),
                work,
                completions: self.completion_sender.clone(),
                wake: Arc::clone(&self.wake),
            },
        );
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

/// Run the calls of the queue until the node drops the pool and the queue
/// is empty: run each call whose claim exists, send its completion to the
/// inbox of its scheduler, then write one byte to the wake socket of the
/// scheduler, as `wake_poller` writes to the ERTS wake pipe
/// (`erl_poll.c:429-462`). A call whose claim is gone does not run, and its
/// completion has no output. The thread drops each discarded function of
/// a [`Finish`] before it takes a call. A full wake socket already wakes the
/// scheduler, so the thread ignores a write that fails with `WouldBlock`.
///
/// # Examples
///
/// ```rust,ignore
/// thread::spawn(move || run_calls(&shared));
/// ```
fn run_calls(shared: &Arc<Shared>) {
    while let Some(taken) = shared.take() {
        let Call {
            process,
            call,
            claim,
            work,
            completions,
            wake,
        } = match taken {
            Taken::Call(call) => call,
            Taken::Discard(Discard(run)) => {
                drop(run);
                continue;
            }
        };
        // The thread does not keep the claim: a caller that exits while the
        // call runs drops it, and the scheduler then drops the result.
        let output = claim.upgrade().map(|claim| {
            claim.store(true, Ordering::Release);
            drop(claim);
            work.run(shared)
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

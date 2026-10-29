//! The process and message functions of the NIF C API: the process-
//! independent env of `enif_alloc_env`, `enif_self`, `enif_get_local_pid`,
//! `enif_is_process_alive`, and `enif_send`.
//!
//! A NIF call and a load callback run in a [`Context`] of their thread: the
//! atom table of the call, the node of the runtime, and the messages of
//! `enif_send`. The functions find it through a thread-local value, because
//! `enif_alloc_env` and `enif_send` with a NULL env get no env of the call.
//! The scheduler of the caller sends the messages when the call returns.
//! Thus a message of a dirty NIF arrives when the NIF returns, where ERTS
//! puts it into the queue of the receiver at once (`erl_nif.c:1077-1080`).
//!
//! An `ErlNifPid` holds the number of a local process, not a handle of an
//! env, because the C code keeps it after the call (`erl_nif.h:201-203`).

use std::cell::RefCell;
use std::ffi::{c_int, c_void};
use std::sync::{Arc, RwLock};

use super::env::Env;
use super::layout::{ErlNifMonitor, ErlNifPid, NifTerm};
use crate::AtomTable;
use crate::Term;
use crate::runtime::scheduler::node::Node;
use crate::runtime::signal::Signal;
use crate::term::{ProcessId, Reference};

thread_local! {
    /// The context of the NIF call or of the load callback that runs on the
    /// thread, or `None`.
    static CONTEXT: RefCell<Option<Context>> = const { RefCell::new(None) };
}

/// The run that a thread of a NIF library finds: the atom table and the
/// node of the run that loaded the library. ERTS keeps its atom table and
/// its process registry for the whole life of the emulator
/// (`erl_atom.c`, `erl_process.c`), so `enif_alloc_env`
/// (`erl_nif.c:669-675`) and `enif_send` (`erl_nif.c:2296-2338`) run from
/// a thread that the emulator does not manage. PBeam keeps the run of the
/// last NIF library load here, and never clears it, as ERTS never clears
/// its tables.
static RUN: RwLock<Option<RunContext>> = RwLock::new(None);

/// The run of the loaded NIF libraries.
#[derive(Debug)]
struct RunContext {
    atoms: Arc<AtomTable>,
    node: Arc<Node>,
}

/// Record the run of `atoms` and `node` as the run that a thread of a NIF
/// library finds.
pub(super) fn enter_run(atoms: &Arc<AtomTable>, node: &Arc<Node>) {
    *RUN.write().expect("the run lock is not poisoned") = Some(RunContext {
        atoms: Arc::clone(atoms),
        node: Arc::clone(node),
    });
}

/// Return the result of `read` with the run of the loaded NIF libraries,
/// or `None` when no library has loaded.
fn with_run<T>(read: impl FnOnce(&RunContext) -> T) -> Option<T> {
    RUN.read()
        .expect("the run lock is not poisoned")
        .as_ref()
        .map(read)
}

/// The context of one NIF call or load callback.
#[derive(Debug)]
pub(super) struct Context {
    /// The atom table of the process of the call.
    atoms: Arc<AtomTable>,
    /// The node of the process of the call.
    node: Arc<Node>,
    /// The address of the env of the call.
    env: usize,
    /// The messages of `enif_send`: the receiver and the message, in send
    /// order.
    sends: Vec<(ProcessId, Term)>,
    /// The text of the runtime error of the call: the first broken
    /// contract of `enif_send`, or the name of the first `enif_*` function
    /// that PBeam does not implement, or `None`.
    unsupported: Option<String>,
}

/// The messages and the runtime error of a call that ended.
#[derive(Debug)]
pub(super) struct Ended {
    /// The messages of `enif_send`, in send order.
    pub(super) sends: Vec<(ProcessId, Term)>,
    /// The text of the runtime error of the call, or `None`.
    pub(super) unsupported: Option<String>,
}

/// Run `call` in a new context with the atom table `atoms`, the node
/// `node`, and the env `env` of the call, and return its result and the end
/// of the context.
///
/// # Examples
///
/// ```rust,ignore
/// let (value, ended) = within(&atoms, &node, &raw mut env, || 7);
/// assert!(ended.sends.is_empty());
/// ```
pub(super) fn within<T>(
    atoms: &Arc<AtomTable>,
    node: &Arc<Node>,
    env: *mut Env,
    call: impl FnOnce() -> T,
) -> (T, Ended) {
    CONTEXT.set(Some(Context {
        atoms: Arc::clone(atoms),
        node: Arc::clone(node),
        env: env.expose_provenance(),
        sends: Vec::new(),
        unsupported: None,
    }));
    let value = call();
    let context = CONTEXT.take().expect("the call keeps its context");
    (
        value,
        Ended {
            sends: context.sends,
            unsupported: context.unsupported,
        },
    )
}

/// Return the result of `read` with the context of the thread, or `None`
/// on a thread that runs no NIF call.
fn with_context<T>(read: impl FnOnce(&mut Context) -> T) -> Option<T> {
    CONTEXT.with_borrow_mut(|context| context.as_mut().map(read))
}

/// Record the name of an unsupported `enif_*` function of the call that
/// runs on the thread, so the call gives a runtime error when it returns.
/// A thread that runs no NIF call records nothing.
pub(super) fn record_unsupported(name: &str) {
    with_context(|context| {
        context.unsupported.get_or_insert_with(|| {
            format!("the NIF called {name}, which PBeam does not implement yet")
        });
    });
}

/// Return the node of the NIF call that runs on the thread, or else the
/// node of the loaded NIF libraries ([`RUN`]), as ERTS finds its process
/// table from each thread.
fn node() -> Option<Arc<Node>> {
    with_context(|context| Arc::clone(&context.node))
        .or_else(|| with_run(|run| Arc::clone(&run.node)))
}

/// Return true when the local process `pid` exists and does not exit, as
/// `erts_proc_lookup` and `erts_pid2proc_opt` find it
/// (`erl_nif.c:2323-2338`, `877-901`).
fn alive(pid: ProcessId) -> bool {
    node().is_some_and(|node| node.live_owner(pid).is_some())
}

/// `ErlNifEnv* enif_alloc_env(void)`: return a new process-independent env
/// (`erl_nif.c:669-675`). Its terms stay valid until `enif_free_env`.
///
/// A thread that runs no NIF call finds the run of the loaded libraries
/// through [`RUN`], as ERTS finds its global tables from each thread. A
/// thread of a run with no loaded library has no atom table for the env
/// and gets NULL.
#[unsafe(no_mangle)]
pub extern "C" fn enif_alloc_env() -> *mut Env {
    with_context(|context| Box::into_raw(Box::new(Env::new(&context.atoms, None, &[]))))
        .or_else(|| with_run(|run| Box::into_raw(Box::new(Env::new(&run.atoms, None, &[])))))
        .unwrap_or(std::ptr::null_mut())
}

/// `void enif_free_env(ErlNifEnv*)`: free an env of `enif_alloc_env` and
/// its terms (`erl_nif.c:676-680`).
///
/// # Safety
///
/// `env` is an env of `enif_alloc_env` that no call freed, as the C API
/// requires.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn enif_free_env(env: *mut Env) {
    // SAFETY: `enif_alloc_env` made the env from a box, and the C API gives
    // it back once.
    drop(unsafe { Box::from_raw(env) });
}

/// `ErlNifPid* enif_self(ErlNifEnv*, ErlNifPid*)`: write the calling
/// process to `pid` and return `pid`, or return NULL for an env with no
/// process (`erl_nif.c:1206-1212`).
///
/// # Safety
///
/// `env` is the env of the current call or an env of `enif_alloc_env`, and
/// `pid` points to a writable `ErlNifPid`, as the C API requires.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn enif_self(env_pointer: *mut Env, pid: *mut ErlNifPid) -> *mut ErlNifPid {
    // SAFETY: the C API gives a valid env.
    let env = unsafe { super::env::env(env_pointer) };
    let Some(process) = env.process() else {
        return std::ptr::null_mut();
    };
    // SAFETY: the C API requires that `pid` points to a writable
    // `ErlNifPid`.
    unsafe { pid.write(ErlNifPid::of(process)) };
    pid
}

/// `int enif_get_local_pid(ErlNifEnv*, ERL_NIF_TERM, ErlNifPid*)`: write
/// the process of a local pid `term` to `pid` and return 1, or return 0
/// (`erl_nif.c:1214-1221`).
///
/// # Safety
///
/// `env` is a valid env, and `pid` points to a writable `ErlNifPid`, as the
/// C API requires.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn enif_get_local_pid(
    env_pointer: *mut Env,
    term: NifTerm,
    pid: *mut ErlNifPid,
) -> c_int {
    // SAFETY: the C API gives a valid env.
    let env = unsafe { super::env::env(env_pointer) };
    let Some(Term::Pid(process)) = env.get(term) else {
        return 0;
    };
    // SAFETY: the C API requires that `pid` points to a writable
    // `ErlNifPid`.
    unsafe { pid.write(ErlNifPid::of(process)) };
    1
}

/// `int enif_is_process_alive(ErlNifEnv*, ErlNifPid*)`: return 1 when the
/// process exists and does not exit, else 0 (`erl_nif.c:2323-2338`).
///
/// # Safety
///
/// `pid` points to an `ErlNifPid` that `enif_self` or `enif_get_local_pid`
/// wrote, as the C API requires.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn enif_is_process_alive(_env: *mut Env, pid: *const ErlNifPid) -> c_int {
    // SAFETY: the C API gives a valid `ErlNifPid`.
    let pid = unsafe { pid.read() };
    c_int::from(alive(pid.process()))
}

/// `int enif_send(ErlNifEnv*, const ErlNifPid*, ErlNifEnv*, ERL_NIF_TERM)`:
/// send the term `msg` of `msg_env`, or of the env of the call when
/// `msg_env` is NULL, to the process `to_pid` and return 1, or return 0
/// when the process does not exist or exits (`erl_nif.c:860-1094`). ERTS
/// copies a term of the call also when `env` is NULL (`erl_nif.c:965-993`).
/// The scheduler of the caller sends the message when the call returns.
///
/// A `msg` that the env does not have breaks the C API contract, and the
/// call gives a runtime error. A thread, a down callback, or a destructor
/// that runs no NIF call sends through the inbox of the scheduler of the
/// receiver. With no message env and no env, it sends only an atom, and
/// gets 0 for another term.
///
/// # Safety
///
/// `env` is the env of the current call or NULL, `msg_env` is an env of
/// `enif_alloc_env` or NULL, and `to_pid` points to a valid `ErlNifPid`, as
/// the C API requires.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn enif_send(
    env: *mut Env,
    to_pid: *const ErlNifPid,
    msg_env: *mut Env,
    msg: NifTerm,
) -> c_int {
    // SAFETY: the C API gives a valid `ErlNifPid`.
    let target = unsafe { to_pid.read() }.process();
    if !alive(target) {
        return 0;
    }
    // The message term of a call comes from the env of the call when the
    // NIF gives no message env, as ERTS reads it
    // (`erl_nif.c:1100-1106`). A thread of a library that gives no message
    // env sends an atom, whose handle is the same in each env.
    let in_call = with_context(|context| context.env);
    let term = if !msg_env.is_null() {
        // SAFETY: the C API gives an env of `enif_alloc_env`.
        unsafe { super::env::env(msg_env) }.get(msg)
    } else if let Some(env) = in_call {
        // SAFETY: the env of the running call is alive until it returns.
        unsafe { super::env::env(std::ptr::with_exposed_provenance_mut(env)) }.get(msg)
    } else if !env.is_null() {
        // A down callback or a destructor gives its env, which has no
        // process, and ERTS copies the term from it (`erl_nif.c:965-993`).
        // SAFETY: the C API gives the env of the running callback.
        unsafe { super::env::env(env) }.get(msg)
    } else {
        with_run(|run| Env::new(&run.atoms, None, &[]).get(msg)).flatten()
    };
    let Some(term) = term else {
        with_context(|context| {
            context.unsupported.get_or_insert_with(|| {
                format!("enif_send sent the term {msg:#x}, which its env does not have")
            });
        });
        return 0;
    };
    // A call queues its messages for the caller's scheduler to send when
    // the call returns. A thread of a library, which runs no call, sends
    // through the inbox of the scheduler of the receiver at once, as ERTS
    // queues the message from each thread (`erl_nif.c:877-901`, `996`).
    // The sender is `None`: a thread of a library is no process, and ERTS
    // gives the message the sender `am_undefined`.
    match with_context(|context| context.sends.push((target, term.clone()))) {
        Some(()) => 1,
        None => node().map_or(0, |node| {
            node.send_message(0, None, term, target, None);
            1
        }),
    }
}

/// `int enif_monitor_process(ErlNifEnv*, void*, const ErlNifPid*,
/// ErlNifMonitor*)`: monitor the process `to_pid` for the resource object
/// `obj` and return 0, or return -1 when the type of the object has no
/// down callback, and 1 when the process does not exist, as ERTS gives for
/// a monitor signal that it cannot send (`erl_nif.c:3933-3990`).
///
/// The monitor signal goes through the inbox of the scheduler of the
/// process, as ERTS sends it with the sender `am_system`
/// (`erl_nif.c:3968-3969`). The exit of the process runs the down callback
/// ([`super::resource::monitor_down`]).
///
/// # Safety
///
/// `obj` is the data of a live resource object, `to_pid` points to a valid
/// `ErlNifPid`, and `monitor` is NULL or points to a writable
/// `ErlNifMonitor`, as the C API requires.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn enif_monitor_process(
    _env: *mut Env,
    obj: *mut c_void,
    to_pid: *const ErlNifPid,
    monitor: *mut ErlNifMonitor,
) -> c_int {
    // SAFETY: the C API gives a valid `ErlNifPid`.
    let target = unsafe { to_pid.read() }.process();
    let data = obj.expose_provenance();
    let Some(down) = super::resource::down_of(data) else {
        return -1;
    };
    // ERTS sends the signal to a process that exits too, and the exit runs
    // the down callback (`erl_proc_sig_queue.c`, `erl_process.c:13829-13831`).
    let Some(node) = node().filter(|node| node.owner(target).is_some()) else {
        return 1;
    };
    // ERTS takes the monitor reference from the counter of `make_ref/0`
    // (`erl_nif.c:3957`), so it is never the reference of another monitor
    // of the target.
    let reference = Reference(node.next_reference());
    super::resource::add_monitor(data, down, reference);
    node.send_signal(
        0,
        target,
        None,
        Signal::NifMonitor { monitor: reference },
        None,
    );
    if !monitor.is_null() {
        // SAFETY: the C API requires that a `monitor` that is not NULL
        // points to a writable `ErlNifMonitor`.
        unsafe { monitor.write(ErlNifMonitor::of(reference)) };
    }
    0
}

/// `int enif_demonitor_process(ErlNifEnv*, void*, const ErlNifMonitor*)`:
/// remove the monitor `monitor` of the resource object `obj` and return 0,
/// or return 1 when the object has no such monitor, because the down
/// callback ran or a demonitor removed it before (`erl_nif.c:3998-4031`).
///
/// ERTS also sends a demonitor signal to the process. PBeam keeps the
/// target monitor in the process, and its exit finds no monitor and runs
/// no down callback.
///
/// # Safety
///
/// `obj` is the data of a live resource object and `monitor` points to a
/// valid `ErlNifMonitor`, as the C API requires.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn enif_demonitor_process(
    _env: *mut Env,
    obj: *mut c_void,
    monitor: *const ErlNifMonitor,
) -> c_int {
    // SAFETY: the C API gives a valid `ErlNifMonitor`.
    let reference = unsafe { monitor.read() }.reference();
    c_int::from(!super::resource::demonitor(
        obj.expose_provenance(),
        reference,
    ))
}

/// `int enif_compare_monitors(const ErlNifMonitor*, const ErlNifMonitor*)`:
/// compare the bytes of two monitors as `sys_memcmp` does
/// (`erl_nif.c:4033-4038`), and return a negative number, 0, or a positive
/// number.
///
/// # Safety
///
/// `monitor1` and `monitor2` point to valid `ErlNifMonitor`s, as the C
/// API requires.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn enif_compare_monitors(
    monitor1: *const ErlNifMonitor,
    monitor2: *const ErlNifMonitor,
) -> c_int {
    // SAFETY: the C API gives valid `ErlNifMonitor`s.
    let [one, two] = [monitor1, monitor2].map(|monitor| unsafe { monitor.read() }.words);
    let bytes = |words: [usize; 4]| words.map(usize::to_ne_bytes);
    bytes(one).cmp(&bytes(two)) as c_int
}

#[cfg(test)]
#[path = "../../../../tests/runtime/nif/abi/process.rs"]
mod tests;

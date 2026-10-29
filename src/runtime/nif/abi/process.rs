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
use std::sync::Arc;

use super::env::Env;
use super::layout::{ErlNifMonitor, ErlNifPid, NifTerm};
use crate::AtomTable;
use crate::Term;
use crate::runtime::scheduler::node::Node;
use crate::term::ProcessId;

thread_local! {
    /// The context of the NIF call or of the load callback that runs on the
    /// thread, or `None`.
    static CONTEXT: RefCell<Option<Context>> = const { RefCell::new(None) };
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

/// Return true when the local process `pid` exists and does not exit, as
/// `erts_proc_lookup` and `erts_pid2proc_opt` find it
/// (`erl_nif.c:2323-2338`). A thread that runs no NIF call finds no process.
fn alive(pid: ProcessId) -> bool {
    with_context(|context| context.node.live_owner(pid).is_some()).unwrap_or(false)
}

/// `ErlNifEnv* enif_alloc_env(void)`: return a new process-independent env
/// (`erl_nif.c:669-675`). Its terms stay valid until `enif_free_env`.
///
/// On a thread that runs no NIF call, PBeam has no atom table for the env
/// and returns NULL.
#[unsafe(no_mangle)]
pub extern "C" fn enif_alloc_env() -> *mut Env {
    with_context(|context| Box::into_raw(Box::new(Env::new(&context.atoms, None, &[]))))
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
/// call gives a runtime error. On a thread that runs no NIF call, the call
/// returns 0.
///
/// # Safety
///
/// `env` is the env of the current call or NULL, `msg_env` is an env of
/// `enif_alloc_env` or NULL, and `to_pid` points to a valid `ErlNifPid`, as
/// the C API requires.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn enif_send(
    _env: *mut Env,
    to_pid: *const ErlNifPid,
    msg_env: *mut Env,
    msg: NifTerm,
) -> c_int {
    // SAFETY: the C API gives a valid `ErlNifPid`.
    let target = unsafe { to_pid.read() }.process();
    if !alive(target) {
        return 0;
    }
    with_context(|context| {
        let source = if msg_env.is_null() {
            std::ptr::with_exposed_provenance_mut(context.env)
        } else {
            msg_env
        };
        // SAFETY: `source` is an env of `enif_alloc_env` or the env of the
        // running call, which is alive until the call returns.
        let Some(term) = unsafe { super::env::env(source) }.get(msg) else {
            context.unsupported.get_or_insert_with(|| {
                format!("enif_send sent the term {msg:#x}, which its env does not have")
            });
            return 0;
        };
        context.sends.push((target, term));
        1
    })
    .unwrap_or(0)
}

/// `int enif_monitor_process(ErlNifEnv*, void*, const ErlNifPid*,
/// ErlNifMonitor*)`: watch the process `to_pid` for the resource object
/// `obj` and return 0, or return -1 when the type of the object has no
/// down callback, and 1 when the process does not exist, as ERTS gives for
/// a monitor signal that it cannot send (`erl_nif.c:3933-3990`).
///
/// PBeam does not run the down callback of the type yet: it writes a
/// monitor of zero words, which `enif_demonitor_process` and
/// `enif_compare_monitors` do not use.
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
    with_context(|context| {
        if !super::resource::has_down(obj.expose_provenance()) {
            return -1;
        }
        if context.node.live_owner(target).is_none() {
            return 1;
        }
        if !monitor.is_null() {
            // SAFETY: the C API requires that a `monitor` that is not NULL
            // points to a writable `ErlNifMonitor`.
            unsafe { monitor.write(ErlNifMonitor { words: [0; 4] }) };
        }
        0
    })
    .unwrap_or(1)
}

#[cfg(test)]
#[path = "../../../../tests/runtime/nif/abi/process.rs"]
mod tests;

//! The `ErlNifEnv` of a NIF call and the `enif_*` functions that PBeam
//! exports to a NIF library.
//!
//! An `ERL_NIF_TERM` is a handle of the terms of one call: the handle `N`
//! is the term `N - 1` of [`Env`], and the handle 0 is `THE_NON_VALUE`,
//! which `enif_make_badarg` returns. The arguments of the call are the
//! handles `1..=argc`, and each `enif_make_*` call adds a term. A handle is
//! valid only in the call of its env, as an ERTS process-bound term is
//! valid only until the NIF returns (`erl_nif.h`, "ERL_NIF_TERM").
//!
//! The handle of an atom is the exception: it is [`ATOM_HANDLE`] plus the
//! index of the atom, for an atom of `enif_make_atom`, an argument, and an
//! element that an `enif_get_*` call reads. It names the same atom in each
//! env of the node, and two handles of one atom are equal, as an ERTS
//! atom is an immediate term
//! (`make_atom`, `erl_term.h:300`). A library can thus keep the atoms of
//! its load callback in static variables and return them from later
//! calls, as exqlite 0.37.0 does (`sqlite3_nif.c:13-20`, `1428`). A handle
//! never reaches [`ATOM_HANDLE`], because no env has that many terms.
//!
//! The env of a dirty NIF is the same: the dirty thread makes it for the
//! call, and its terms go back to the process with the result, as the
//! terms of a dirty NIF are on the heap of its process in ERTS
//! (`erl_nif.c:3386-3435`).
//!
//! The executable exports each function, so the dynamic linker binds the
//! undefined `enif_*` symbols of a library to them (`build.rs` adds the
//! export of the `enif_*` symbols to the link of each executable).

use std::ffi::{CStr, c_char, c_double, c_int, c_long, c_uint, c_ulong, c_void};
use std::sync::Arc;

use super::Handle;
use super::layout::{ErlNifBinary, ErlNifMonitor, ErlNifPid, ErlNifResourceTypeInit, NifTerm};
use super::resource::{self, DtorFn, ResourceType};
use super::{mutex, process, thread};
use crate::BeamError;
use crate::Term;
use crate::error::error;
use crate::runtime::NativeResult;
use crate::runtime::interpreter::bif::iodata_bytes;
use crate::term::Map;
use crate::term::{MAX_ATOM_CHARACTERS, ProcessId};
use crate::{Atom, AtomTable};

/// `THE_NON_VALUE`: the handle that no term has.
const NON_VALUE: NifTerm = 0;

/// The bit of the handle of an atom of `enif_make_atom`: the other bits
/// are the index of the atom in the atom table of the node.
const ATOM_HANDLE: NifTerm = 1 << (NifTerm::BITS - 1);

/// `ERL_NIF_LATIN1` of `ErlNifCharEncoding` (`erl_nif.h:195-198`).
const LATIN1: c_int = 1;

/// `ERL_NIF_UTF8` of `ErlNifCharEncoding` (`erl_nif.h:195-198`).
const UTF8: c_int = 2;

/// The exported `enif_*` functions. The linker removes a function that no
/// code of the executable refers to, and a NIF library refers to it only
/// at run time, so this static keeps each function in each executable.
#[used]
static EXPORTS: Exports = Exports {
    get_int: enif_get_int,
    get_uint: enif_get_uint,
    inspect_binary: enif_inspect_binary,
    get_string: enif_get_string,
    make_int: enif_make_int,
    make_new_binary: enif_make_new_binary,
    make_badarg: enif_make_badarg,
    alloc: enif_alloc,
    realloc: enif_realloc,
    free: enif_free,
    alloc_binary: enif_alloc_binary,
    release_binary: enif_release_binary,
    make_binary: enif_make_binary,
    inspect_iolist_as_binary: enif_inspect_iolist_as_binary,
    get_atom: enif_get_atom,
    get_double: enif_get_double,
    get_long: enif_get_long,
    get_list_cell: enif_get_list_cell,
    get_list_length: enif_get_list_length,
    make_atom: enif_make_atom,
    make_double: enif_make_double,
    make_long: enif_make_long,
    make_list: enif_make_list,
    make_list_cell: enif_make_list_cell,
    make_list_from_array: enif_make_list_from_array,
    make_tuple: enif_make_tuple,
    raise_exception: enif_raise_exception,
    open_resource_type: enif_open_resource_type,
    alloc_resource: enif_alloc_resource,
    release_resource: enif_release_resource,
    keep_resource: enif_keep_resource,
    make_resource: enif_make_resource,
    get_resource: enif_get_resource,
    alloc_env: process::enif_alloc_env,
    free_env: process::enif_free_env,
    self_pid: process::enif_self,
    get_local_pid: process::enif_get_local_pid,
    is_process_alive: process::enif_is_process_alive,
    send: process::enif_send,
    mutex_create: mutex::enif_mutex_create,
    mutex_destroy: mutex::enif_mutex_destroy,
    mutex_lock: mutex::enif_mutex_lock,
    mutex_unlock: mutex::enif_mutex_unlock,
    clear_env: enif_clear_env,
    is_atom: enif_is_atom,
    is_empty_list: enif_is_empty_list,
    is_list: enif_is_list,
    get_ulong: enif_get_ulong,
    make_new_atom_len: enif_make_new_atom_len,
    make_tuple_from_array: enif_make_tuple_from_array,
    get_map_value: enif_get_map_value,
    make_map_from_arrays: enif_make_map_from_arrays,
    init_resource_type: enif_init_resource_type,
    monitor_process: process::enif_monitor_process,
    demonitor_process: process::enif_demonitor_process,
    compare_monitors: process::enif_compare_monitors,
    thread_type: thread::enif_thread_type,
};

/// The type of [`EXPORTS`]: one field for each exported function.
#[expect(dead_code, reason = "only the linker reads the fields")]
struct Exports {
    get_int: unsafe extern "C" fn(*mut Env, NifTerm, *mut c_int) -> c_int,
    get_uint: unsafe extern "C" fn(*mut Env, NifTerm, *mut c_uint) -> c_int,
    inspect_binary: unsafe extern "C" fn(*mut Env, NifTerm, *mut ErlNifBinary) -> c_int,
    get_string: unsafe extern "C" fn(*mut Env, NifTerm, *mut c_char, c_uint, c_int) -> c_int,
    make_int: unsafe extern "C" fn(*mut Env, c_int) -> NifTerm,
    make_new_binary: unsafe extern "C" fn(*mut Env, usize, *mut NifTerm) -> *mut u8,
    make_badarg: unsafe extern "C" fn(*mut Env) -> NifTerm,
    alloc: unsafe extern "C" fn(usize) -> *mut c_void,
    realloc: unsafe extern "C" fn(*mut c_void, usize) -> *mut c_void,
    free: unsafe extern "C" fn(*mut c_void),
    alloc_binary: unsafe extern "C" fn(usize, *mut ErlNifBinary) -> c_int,
    release_binary: unsafe extern "C" fn(*mut ErlNifBinary),
    make_binary: unsafe extern "C" fn(*mut Env, *mut ErlNifBinary) -> NifTerm,
    inspect_iolist_as_binary: unsafe extern "C" fn(*mut Env, NifTerm, *mut ErlNifBinary) -> c_int,
    get_atom: unsafe extern "C" fn(*mut Env, NifTerm, *mut c_char, c_uint, c_int) -> c_int,
    get_double: unsafe extern "C" fn(*mut Env, NifTerm, *mut c_double) -> c_int,
    get_long: unsafe extern "C" fn(*mut Env, NifTerm, *mut c_long) -> c_int,
    get_list_cell: unsafe extern "C" fn(*mut Env, NifTerm, *mut NifTerm, *mut NifTerm) -> c_int,
    get_list_length: unsafe extern "C" fn(*mut Env, NifTerm, *mut c_uint) -> c_int,
    make_atom: unsafe extern "C" fn(*mut Env, *const c_char) -> NifTerm,
    make_double: unsafe extern "C" fn(*mut Env, c_double) -> NifTerm,
    make_long: unsafe extern "C" fn(*mut Env, c_long) -> NifTerm,
    make_list: unsafe extern "C" fn(*mut Env, c_uint, ...) -> NifTerm,
    make_list_cell: unsafe extern "C" fn(*mut Env, NifTerm, NifTerm) -> NifTerm,
    make_list_from_array: unsafe extern "C" fn(*mut Env, *const NifTerm, c_uint) -> NifTerm,
    make_tuple: unsafe extern "C" fn(*mut Env, c_uint, ...) -> NifTerm,
    raise_exception: unsafe extern "C" fn(*mut Env, NifTerm) -> NifTerm,
    open_resource_type: unsafe extern "C" fn(
        *mut Env,
        *const c_char,
        *const c_char,
        Option<DtorFn>,
        c_int,
        *mut c_int,
    ) -> *const ResourceType,
    alloc_resource: unsafe extern "C" fn(*const ResourceType, usize) -> *mut c_void,
    release_resource: unsafe extern "C" fn(*mut c_void),
    keep_resource: unsafe extern "C" fn(*mut c_void),
    make_resource: unsafe extern "C" fn(*mut Env, *mut c_void) -> NifTerm,
    get_resource:
        unsafe extern "C" fn(*mut Env, NifTerm, *const ResourceType, *mut *mut c_void) -> c_int,
    alloc_env: extern "C" fn() -> *mut Env,
    free_env: unsafe extern "C" fn(*mut Env),
    self_pid: unsafe extern "C" fn(*mut Env, *mut ErlNifPid) -> *mut ErlNifPid,
    get_local_pid: unsafe extern "C" fn(*mut Env, NifTerm, *mut ErlNifPid) -> c_int,
    is_process_alive: unsafe extern "C" fn(*mut Env, *const ErlNifPid) -> c_int,
    send: unsafe extern "C" fn(*mut Env, *const ErlNifPid, *mut Env, NifTerm) -> c_int,
    mutex_create: extern "C" fn(*mut c_char) -> *mut c_void,
    mutex_destroy: unsafe extern "C" fn(*mut c_void),
    mutex_lock: unsafe extern "C" fn(*mut c_void),
    mutex_unlock: unsafe extern "C" fn(*mut c_void),
    clear_env: unsafe extern "C" fn(*mut Env),
    is_atom: unsafe extern "C" fn(*mut Env, NifTerm) -> c_int,
    is_empty_list: unsafe extern "C" fn(*mut Env, NifTerm) -> c_int,
    is_list: unsafe extern "C" fn(*mut Env, NifTerm) -> c_int,
    get_ulong: unsafe extern "C" fn(*mut Env, NifTerm, *mut c_ulong) -> c_int,
    make_new_atom_len:
        unsafe extern "C" fn(*mut Env, *const c_char, usize, *mut NifTerm, c_int) -> c_int,
    make_tuple_from_array: unsafe extern "C" fn(*mut Env, *const NifTerm, c_uint) -> NifTerm,
    get_map_value: unsafe extern "C" fn(*mut Env, NifTerm, NifTerm, *mut NifTerm) -> c_int,
    make_map_from_arrays: unsafe extern "C" fn(
        *mut Env,
        *const NifTerm,
        *const NifTerm,
        usize,
        *mut NifTerm,
    ) -> c_int,
    init_resource_type: unsafe extern "C" fn(
        *mut Env,
        *const c_char,
        *const ErlNifResourceTypeInit,
        c_int,
        *mut c_int,
    ) -> *const ResourceType,
    monitor_process:
        unsafe extern "C" fn(*mut Env, *mut c_void, *const ErlNifPid, *mut ErlNifMonitor) -> c_int,
    demonitor_process: unsafe extern "C" fn(*mut Env, *mut c_void, *const ErlNifMonitor) -> c_int,
    compare_monitors: unsafe extern "C" fn(*const ErlNifMonitor, *const ErlNifMonitor) -> c_int,
    thread_type: extern "C" fn() -> c_int,
}

/// The term of one handle of an [`Env`].
#[derive(Debug)]
enum Slot {
    /// A term.
    Term(Term),
    /// The bytes of a binary of `enif_make_new_binary`, which the NIF can
    /// write until it returns. The vector keeps its size, so its bytes do
    /// not move while the env is alive.
    Buffer(Vec<u8>),
}

/// The env of one NIF call or one load callback: the terms of its handles,
/// the exception of `enif_raise_exception`, and the first call of a part of the
/// C API that PBeam does not support yet. The C code gets it as an opaque
/// `ErlNifEnv*`.
#[derive(Debug)]
pub(crate) struct Env {
    /// The atom table of the process of the call.
    atoms: Arc<AtomTable>,
    /// The process of the call, or `None` for an env with no process: an
    /// env of `enif_alloc_env` or of a destructor.
    process: Option<ProcessId>,
    /// The term of each handle, in handle order.
    slots: Vec<Slot>,
    /// The reason of `enif_raise_exception`, or `None`.
    exception: Option<Term>,
    /// The text of the first unsupported call or of the first broken
    /// contract that PBeam detects, or `None`.
    unsupported: Option<String>,
    /// The bytes of each iolist of `enif_inspect_iolist_as_binary`, which
    /// the NIF can read until the call returns, as ERTS keeps them in the
    /// temporary objects of the env (`erl_nif.c:1401`).
    copies: Vec<Vec<u8>>,
    /// The address of each block of `enif_alloc_binary` that
    /// `enif_make_binary` took. The NIF can read the bytes until the call
    /// returns (`erl_nif.c:1708-1714`), so drop frees the blocks.
    taken: Vec<usize>,
    /// The library of a load callback, or `None` in another env. Only the
    /// env of a load callback opens a resource type (`erl_nif.c:2708`).
    library: Option<Arc<Handle>>,
}

impl Drop for Env {
    fn drop(&mut self) {
        for &address in &self.taken {
            // SAFETY: each address is the `ref_bin` of a block of
            // `enif_alloc_binary`, which `malloc` gave, and
            // `enif_make_binary` took its ownership from the NIF, so no
            // other code frees it.
            unsafe { libc::free(std::ptr::with_exposed_provenance_mut(address)) };
        }
    }
}

impl Env {
    /// Return the env of a call with `arguments` in its slots
    /// `1..=arguments.len()`. The handle of an argument is its slot number,
    /// except for an atom argument, which has its atom handle.
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// let env = Env::new(&atoms, Some(caller), &[Term::Integer(1)]);
    /// assert_eq!(env.handles(), [1]);
    /// ```
    pub(super) fn new(
        atoms: &Arc<AtomTable>,
        process: Option<ProcessId>,
        arguments: &[Term],
    ) -> Self {
        Self {
            atoms: Arc::clone(atoms),
            process,
            slots: arguments.iter().cloned().map(Slot::Term).collect(),
            exception: None,
            unsupported: None,
            copies: Vec::new(),
            taken: Vec::new(),
            library: None,
        }
    }

    /// Return the env of the load callback of the library `library` in the
    /// process `process`, with the load info `info` in the slot 1.
    pub(super) fn load(
        atoms: &Arc<AtomTable>,
        process: ProcessId,
        info: &Term,
        library: &Arc<Handle>,
    ) -> Self {
        let mut env = Self::new(atoms, Some(process), std::slice::from_ref(info));
        env.library = Some(Arc::clone(library));
        env
    }

    /// Return the process of the env, or `None` for an env with no process.
    pub(super) fn process(&self) -> Option<ProcessId> {
        self.process
    }

    /// Return the handles of the arguments, the `argv` of the call. The
    /// handle of an atom argument is its atom handle.
    pub(super) fn handles(&self) -> Vec<NifTerm> {
        let handle = |(index, slot): (usize, &Slot)| match slot {
            Slot::Term(Term::Atom(atom)) => atom_handle(atom),
            _ => index + 1,
        };
        self.slots.iter().enumerate().map(handle).collect()
    }

    /// Add `slot` and return its handle.
    fn put(&mut self, slot: Slot) -> NifTerm {
        self.slots.push(slot);
        self.slots.len()
    }

    /// Return the slot of `handle`, or `None` for `THE_NON_VALUE` and for a
    /// handle that no term of the env has.
    fn slot(&self, handle: NifTerm) -> Option<&Slot> {
        self.slots.get(handle.wrapping_sub(1))
    }

    /// Return the term of `handle`, or `None` for `THE_NON_VALUE` and for a
    /// handle that no term of the env has. The term of a binary of
    /// `enif_make_new_binary` has the bytes that the NIF wrote so far. An
    /// atom handle gives its atom in each env of the node.
    pub(super) fn get(&self, handle: NifTerm) -> Option<Term> {
        if handle & ATOM_HANDLE != 0 {
            let index = u32::try_from(handle ^ ATOM_HANDLE).ok();
            return index.and_then(|index| self.atoms.atom_at(index).map(Term::Atom));
        }
        self.slot(handle).map(|slot| match slot {
            Slot::Term(term) => term.clone(),
            Slot::Buffer(bytes) => Term::binary(bytes.clone()),
        })
    }

    /// Return the term of `handle` for a constructor of a term. A handle
    /// that no term of the env has breaks the C API contract, and ERTS
    /// makes an undefined term (`ASSERT_IN_ENV`, `erl_nif.c:2154`): the
    /// call gives a runtime error, and the function uses `[]`.
    fn term(&mut self, handle: NifTerm) -> Term {
        self.get(handle).unwrap_or_else(|| {
            self.unsupported.get_or_insert_with(|| {
                format!("the NIF used the term {handle:#x}, which no enif call of the call made")
            });
            Term::Nil
        })
    }

    /// Free each term of the env and each block and copy that a term of
    /// the env took, as `enif_clear_env` frees the temporary objects and
    /// the off-heap of its env (`erl_nif.c:700-716`). Each handle of the
    /// env then has no term.
    pub(super) fn clear(&mut self) {
        self.slots.clear();
        self.copies.clear();
        for address in self.taken.drain(..) {
            // SAFETY: each address is a block of `enif_alloc_binary` that
            // `enif_make_binary` took, which `malloc` gave.
            unsafe { libc::free(std::ptr::with_exposed_provenance_mut(address)) };
        }
    }

    /// Add the term `term` and return its handle. An atom adds no term:
    /// its handle is its atom handle.
    fn make(&mut self, term: Term) -> NifTerm {
        if let Term::Atom(atom) = &term {
            return atom_handle(atom);
        }
        self.put(Slot::Term(term))
    }

    /// Return the atom of the C string `name`, as `enif_make_atom` makes it
    /// with the Latin-1 encoding (`erl_nif.c:2080-2114`): each byte is one
    /// character, and a name of more than 255 characters raises
    /// `error:badarg` and gives `THE_NON_VALUE` (`atom.c:271-277`). The
    /// handle is the atom handle of the atom, which adds no term to the env.
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// let handle = env.make_atom(b"\xe9t\xe9");
    /// assert_eq!(env.get(handle), Some(Term::Atom(atoms.put("été").unwrap())));
    /// ```
    fn make_atom(&mut self, name: &[u8]) -> NifTerm {
        if name.len() > MAX_ATOM_CHARACTERS {
            return self.raise(Term::static_atom("badarg"));
        }
        let name: String = name.iter().copied().map(char::from).collect();
        match self.atoms.put(&name) {
            Ok(atom) => self.make(Term::Atom(atom)),
            // ERTS stops the node when the atom table is full
            // (`index.c:86-91`); PBeam gives a runtime error.
            Err(failure) => {
                self.unsupported.get_or_insert(failure.to_string());
                NON_VALUE
            }
        }
    }

    /// Raise `error:Reason` when the NIF returns and return
    /// `THE_NON_VALUE`, as `enif_raise_exception` does: a later call
    /// replaces the reason (`erl_nif.c:1796-1801`).
    fn raise(&mut self, reason: Term) -> NifTerm {
        self.exception = Some(reason);
        NON_VALUE
    }

    /// Return the result of a NIF that returned `handle`, as ERTS gives it
    /// after the call: after `enif_raise_exception`, the call raises the
    /// exception for each handle (`erl_nif.c:1791-1801`,
    /// `beam_jit_common.cpp:650-652`), and else the handle is the value of
    /// the call.
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// let env = Env::new(&[Term::Integer(1)]);
    /// assert!(matches!(env.finish(1), Ok(NativeResult::Value(Term::Integer(1)))));
    /// ```
    ///
    /// # Errors
    ///
    /// A call of a part of the C API that PBeam does not support gives a
    /// runtime error. A handle that no term of the env has, also
    /// `THE_NON_VALUE`, with no exception gives a runtime error. In ERTS
    /// the result is undefined.
    pub(super) fn finish(mut self, handle: NifTerm) -> Result<NativeResult, BeamError> {
        if let Some(text) = self.unsupported.take() {
            return Err(error(text));
        }
        if let Some(reason) = self.exception.take() {
            return Ok(NativeResult::Raise { reason });
        }
        self.get(handle).map(NativeResult::Value).ok_or_else(|| {
            error(format!(
                "the NIF returned the term {handle:#x}, which no enif call of the call made"
            ))
        })
    }
}

/// Return the atom handle of `atom`, the same in each env of the node.
fn atom_handle(atom: &Atom) -> NifTerm {
    ATOM_HANDLE | atom.index() as NifTerm
}

/// Return the bytes that `enif_get_string` writes for `list` with the
/// `ERL_NIF_LATIN1` encoding to a buffer of `len` bytes, and its result
/// (`erl_nif.c:1602-1627`):
///
/// - A buffer of 0 bytes gives 0 and no byte.
/// - A proper list of bytes gives the bytes, a NUL byte, and the number of
///   bytes with the NUL byte.
/// - When the bytes with the NUL byte do not fit, the buffer gets the first
///   `len - 1` bytes and a NUL byte, and the result is `-len`.
/// - A term that is not a list, an element that is not a byte, and the
///   tail of an improper list give 0, with the bytes before it and a NUL
///   byte.
///
/// # Examples
///
/// ```rust,ignore
/// let list = Term::list_from([Term::Integer(97), Term::Integer(98)]);
/// assert_eq!(latin1_string(&list, 3), (b"ab\0".to_vec(), 3));
/// assert_eq!(latin1_string(&list, 2), (b"a\0".to_vec(), -2));
/// ```
fn latin1_string(list: &Term, len: c_uint) -> (Vec<u8>, c_int) {
    let mut bytes = Vec::new();
    if len < 1 {
        return (bytes, 0);
    }
    let limit = usize::try_from(len).unwrap_or(usize::MAX);
    let Some(mut elements) = list.list_iter() else {
        bytes.push(0);
        return (bytes, 0);
    };
    for element in elements.by_ref() {
        let Some(byte) = byte_of(element) else {
            bytes.push(0);
            return (bytes, 0);
        };
        bytes.push(byte);
        if bytes.len() >= limit {
            // ERTS returns the negated `unsigned` length as a C `int`.
            bytes.pop();
            bytes.push(0);
            return (bytes, 0_u32.wrapping_sub(len).cast_signed());
        }
    }
    let proper = elements.rest().is_nil();
    let count = c_int::try_from(bytes.len() + 1).unwrap_or(c_int::MAX);
    bytes.push(0);
    (bytes, if proper { count } else { 0 })
}

/// Return the byte of an integer element from 0 to 255, or `None`.
fn byte_of(element: &Term) -> Option<u8> {
    match element {
        Term::Integer(value) => u8::try_from(*value).ok(),
        _ => None,
    }
}

/// Return the `Env` of the opaque `env` of a NIF call.
///
/// # Safety
///
/// `env` is the `ErlNifEnv*` that the loader gave to the running NIF call
/// or load callback, which is alive and has no other reference while the
/// call runs. The C API allows an `enif_*` call only with the env of the
/// current call (`erl_nif.h`, "ErlNifEnv").
pub(super) unsafe fn env<'a>(env: *mut Env) -> &'a mut Env {
    // SAFETY: the caller gives a valid and unique `Env`, as the function
    // documents.
    unsafe { &mut *env }
}

/// `int enif_get_int(ErlNifEnv*, ERL_NIF_TERM, int*)`: write the integer of
/// `term` to `ip` and return 1, or return 0 when the term is not an
/// integer in the C `int` range (`erl_nif.c:1843-1858`).
///
/// # Safety
///
/// `env` is the env of the current call, and `ip` points to a writable C
/// `int`, as the C API requires.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn enif_get_int(
    env_pointer: *mut Env,
    term: NifTerm,
    ip: *mut c_int,
) -> c_int {
    // SAFETY: the C API gives the env of the current call.
    let env = unsafe { env(env_pointer) };
    let Some(Term::Integer(value)) = env.get(term) else {
        return 0;
    };
    let Ok(value) = c_int::try_from(value) else {
        return 0;
    };
    // SAFETY: the C API requires that `ip` points to a writable C `int`.
    unsafe { ip.write(value) };
    1
}

/// `int enif_get_uint(ErlNifEnv*, ERL_NIF_TERM, unsigned*)`: write the
/// integer of `term` to `ip` and return 1, or return 0 when the term is not
/// an integer in the C `unsigned int` range (`erl_nif.c:1860-1873`).
///
/// # Safety
///
/// `env` is the env of the current call, and `ip` points to a writable C
/// `unsigned int`, as the C API requires.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn enif_get_uint(
    env_pointer: *mut Env,
    term: NifTerm,
    ip: *mut c_uint,
) -> c_int {
    // SAFETY: the C API gives the env of the current call.
    let env = unsafe { env(env_pointer) };
    let Some(Term::Integer(value)) = env.get(term) else {
        return 0;
    };
    let Ok(value) = c_uint::try_from(value) else {
        return 0;
    };
    // SAFETY: the C API requires that `ip` points to a writable C
    // `unsigned int`.
    unsafe { ip.write(value) };
    1
}

/// `int enif_inspect_binary(ErlNifEnv*, ERL_NIF_TERM, ErlNifBinary*)`:
/// write the size and the address of the bytes of the binary `term` to
/// `bin` and return 1, or return 0 when the term is not a bitstring of a
/// whole number of bytes (`erl_nif.c:1343-1383`). The bytes of a binary of
/// `enif_make_new_binary` are its buffer, as ERTS gives the bytes of the
/// binary itself. ERTS sets `ref_bin` to NULL and does not write the spare
/// fields.
///
/// # Safety
///
/// `env` is the env of the current call, and `bin` points to a writable
/// `ErlNifBinary`, as the C API requires.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn enif_inspect_binary(
    env_pointer: *mut Env,
    term: NifTerm,
    bin: *mut ErlNifBinary,
) -> c_int {
    // SAFETY: the C API gives the env of the current call.
    let env = unsafe { env(env_pointer) };
    let (data, size) = match env.slot(term) {
        Some(Slot::Term(Term::Binary(binary))) if let Some(bytes) = binary.bytes() => {
            (bytes.as_ptr(), bytes.len())
        }
        // `Vec::as_ptr` makes no reference to the bytes, so the address
        // stays valid when the NIF writes the bytes through the address of
        // `enif_make_new_binary` after this call.
        Some(Slot::Buffer(bytes)) => (bytes.as_ptr(), bytes.len()),
        _ => return 0,
    };
    // SAFETY: the C API requires that `bin` points to a writable
    // `ErlNifBinary`. The bytes stay at their address until the env drops
    // after the call: the env keeps the term or the buffer, and neither
    // moves its bytes. The C API allows the NIF to read the bytes only in
    // the call and not to write them (`erl_nif.c:1376`).
    unsafe {
        (&raw mut (*bin).size).write(size);
        (&raw mut (*bin).data).write(data.cast_mut());
        (&raw mut (*bin).ref_bin).write(std::ptr::null_mut());
    }
    1
}

/// `int enif_get_string(ErlNifEnv*, ERL_NIF_TERM, char*, unsigned,
/// ErlNifCharEncoding)`: write the bytes of the string `list` to `buf` and
/// return the result of [`latin1_string`] (`erl_nif.c:1598-1627`). The
/// `ERL_NIF_UTF8` encoding is not supported yet: the call returns 0 and
/// writes no byte, and the NIF call gives a runtime error.
///
/// # Safety
///
/// `env` is the env of the current call, and `buf` points to `len`
/// writable bytes, as the C API requires.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn enif_get_string(
    env_pointer: *mut Env,
    list: NifTerm,
    buf: *mut c_char,
    len: c_uint,
    encoding: c_int,
) -> c_int {
    // SAFETY: the C API gives the env of the current call.
    let env = unsafe { env(env_pointer) };
    if encoding != LATIN1 {
        env.unsupported.get_or_insert_with(|| {
            format!("unsupported: enif_get_string with the encoding {encoding}")
        });
        return 0;
    }
    // A handle that no term has is not a list, as `THE_NON_VALUE` is not.
    let term = env.get(list).unwrap_or(Term::Integer(0));
    let (bytes, result) = latin1_string(&term, len);
    if bytes.is_empty() {
        // A buffer of 0 bytes can be NULL, and ERTS does not touch it
        // (`erl_nif.c:1602-1604`).
        return result;
    }
    // SAFETY: the C API requires that `buf` points to `len` writable
    // bytes, and `latin1_string` gives at most `len` bytes.
    unsafe { std::ptr::copy_nonoverlapping(bytes.as_ptr(), buf.cast::<u8>(), bytes.len()) };
    result
}

/// `ERL_NIF_TERM enif_make_int(ErlNifEnv*, int)`: return the integer `i`
/// (`erl_nif.c:1993-2000`).
///
/// # Safety
///
/// `env` is the env of the current call, as the C API requires.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn enif_make_int(env_pointer: *mut Env, i: c_int) -> NifTerm {
    // SAFETY: the C API gives the env of the current call.
    let env = unsafe { env(env_pointer) };
    env.put(Slot::Term(Term::Integer(i.into())))
}

/// `unsigned char* enif_make_new_binary(ErlNifEnv*, size_t, ERL_NIF_TERM*)`:
/// write the handle of a new binary of `size` bytes to `termp` and return
/// the address of its bytes, which the NIF writes in the call
/// (`erl_nif.c:1463-1473`). The bytes are 0 until the NIF writes them;
/// ERTS does not set them.
///
/// # Safety
///
/// `env` is the env of the current call, and `termp` points to a writable
/// `ERL_NIF_TERM`, as the C API requires.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn enif_make_new_binary(
    env_pointer: *mut Env,
    size: usize,
    termp: *mut NifTerm,
) -> *mut u8 {
    // SAFETY: the C API gives the env of the current call.
    let env = unsafe { env(env_pointer) };
    let mut bytes = vec![0; size];
    // The heap bytes of a vector do not move when the vector moves, and the
    // env does not change the size of a buffer, so the address stays valid
    // for the writes of the NIF until the env drops after the call.
    let data = bytes.as_mut_ptr();
    let handle = env.put(Slot::Buffer(bytes));
    // SAFETY: the C API requires that `termp` points to a writable
    // `ERL_NIF_TERM`.
    unsafe { termp.write(handle) };
    data
}

/// `ERL_NIF_TERM enif_make_badarg(ErlNifEnv*)`: raise `error:badarg` when
/// the NIF returns, and return `THE_NON_VALUE` (`erl_nif.c:1791-1801`).
///
/// # Safety
///
/// `env` is the env of the current call, as the C API requires.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn enif_make_badarg(env_pointer: *mut Env) -> NifTerm {
    // SAFETY: the C API gives the env of the current call.
    let env = unsafe { env(env_pointer) };
    env.raise(Term::static_atom("badarg"))
}

/// `ERL_NIF_TERM enif_raise_exception(ErlNifEnv*, ERL_NIF_TERM)`: raise
/// `error:Reason` when the NIF returns, and return `THE_NON_VALUE`
/// (`erl_nif.c:1796-1801`).
///
/// # Safety
///
/// `env` is the env of the current call, as the C API requires.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn enif_raise_exception(env_pointer: *mut Env, reason: NifTerm) -> NifTerm {
    // SAFETY: the C API gives the env of the current call.
    let env = unsafe { env(env_pointer) };
    let reason = env.term(reason);
    env.raise(reason)
}

/// `void* enif_alloc(size_t)`: return a block of `size` bytes of the C
/// heap, or NULL when the heap has no block of that size, as
/// `erts_alloc_fnf` does (`erl_nif.c:614-617`).
///
/// # Safety
///
/// The C API has no precondition. `enif_free` frees the block.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn enif_alloc(size: usize) -> *mut c_void {
    // SAFETY: `malloc` accepts each size and gives NULL on failure.
    unsafe { libc::malloc(size) }
}

/// `void* enif_realloc(void*, size_t)`: return the block `ptr` resized to
/// `size` bytes, or NULL and keep `ptr` when the heap has no block of that
/// size (`erl_nif.c:619-622`).
///
/// # Safety
///
/// `ptr` is NULL or a block of `enif_alloc` or `enif_realloc` that the
/// NIF did not free, as the C API requires.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn enif_realloc(ptr: *mut c_void, size: usize) -> *mut c_void {
    // SAFETY: the caller gives NULL or a live block of `malloc`.
    unsafe { libc::realloc(ptr, size) }
}

/// `void enif_free(void*)`: free a block of `enif_alloc` or `enif_realloc`
/// (`erl_nif.c:624-627`).
///
/// # Safety
///
/// `ptr` is NULL or a block of `enif_alloc` or `enif_realloc` that the
/// NIF did not free, as the C API requires.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn enif_free(ptr: *mut c_void) {
    // SAFETY: the caller gives NULL or a live block of `malloc`.
    unsafe { libc::free(ptr) }
}

/// `int enif_alloc_binary(size_t, ErlNifBinary*)`: write a new block of
/// `size` bytes to `bin` and return 1, or return 0 when the heap has no
/// block of that size (`erl_nif.c:1409-1422`). The `ref_bin` of the block
/// is its address, which is never NULL, so `enif_release_binary` and
/// `enif_make_binary` know that the NIF owns it.
///
/// # Safety
///
/// `bin` points to a writable `ErlNifBinary`, as the C API requires.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn enif_alloc_binary(size: usize, bin: *mut ErlNifBinary) -> c_int {
    // SAFETY: `malloc` accepts each size and gives NULL on failure. A block
    // of 1 byte for a binary of 0 bytes gives an address that is not NULL.
    let data = unsafe { libc::malloc(size.max(1)) };
    if data.is_null() {
        return 0;
    }
    // SAFETY: the C API requires that `bin` points to a writable
    // `ErlNifBinary`.
    unsafe {
        (&raw mut (*bin).size).write(size);
        (&raw mut (*bin).data).write(data.cast());
        (&raw mut (*bin).ref_bin).write(data);
    }
    1
}

/// `void enif_release_binary(ErlNifBinary*)`: free the block of a binary
/// of `enif_alloc_binary` that the NIF owns, and do nothing for a binary
/// with no `ref_bin`, such as one of `enif_inspect_binary`
/// (`erl_nif.c:1451-1461`). ERTS does not change the fields.
///
/// # Safety
///
/// `bin` points to an `ErlNifBinary` of `enif_alloc_binary` that the NIF
/// owns, or one with a NULL `ref_bin`, as the C API requires.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn enif_release_binary(bin: *mut ErlNifBinary) {
    // SAFETY: the C API requires that `bin` points to an `ErlNifBinary`.
    // A `ref_bin` that is not NULL is a live block of `malloc` of
    // `enif_alloc_binary`, and `free` of NULL does nothing.
    unsafe { libc::free((*bin).ref_bin) }
}

/// `ERL_NIF_TERM enif_make_binary(ErlNifEnv*, ErlNifBinary*)`: return the
/// binary of the `size` bytes at `data` (`erl_nif.c:1698-1757`). For a
/// block of `enif_alloc_binary`, the env takes the block from the NIF and
/// sets `ref_bin` to NULL; the NIF can read the bytes until the call
/// returns, as in ERTS. For another binary, the term is a copy of the
/// bytes.
///
/// # Safety
///
/// `env` is the env of the current call, and `bin` points to an
/// `ErlNifBinary` of `enif_alloc_binary` that the NIF owns, of
/// `enif_inspect_binary`, or of `enif_inspect_iolist_as_binary`, whose
/// `size` bytes at `data` are readable, as the C API requires.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn enif_make_binary(
    env_pointer: *mut Env,
    bin: *mut ErlNifBinary,
) -> NifTerm {
    // SAFETY: the C API gives the env of the current call.
    let env = unsafe { env(env_pointer) };
    // SAFETY: the C API requires that `bin` points to an `ErlNifBinary`
    // with `size` readable bytes at `data`.
    let (bytes, block) = unsafe {
        let bytes = std::slice::from_raw_parts((*bin).data, (*bin).size).to_vec();
        (bytes, (*bin).ref_bin)
    };
    if !block.is_null() {
        env.taken.push(block.expose_provenance());
        // SAFETY: as above, `bin` points to a writable `ErlNifBinary`.
        unsafe { (&raw mut (*bin).ref_bin).write(std::ptr::null_mut()) };
    }
    env.make(Term::binary(bytes))
}

/// `int enif_inspect_iolist_as_binary(ErlNifEnv*, ERL_NIF_TERM,
/// ErlNifBinary*)`: write the bytes of the iodata `term` to `bin` and
/// return 1, or return 0 when the term is not iodata
/// (`erl_nif.c:1385-1407`). A bitstring gives the result of
/// [`enif_inspect_binary`]. The bytes of an iolist are a copy that the env
/// keeps until the call returns, and `ref_bin` is NULL.
///
/// # Safety
///
/// `env` is the env of the current call, and `bin` points to a writable
/// `ErlNifBinary`, as the C API requires.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn enif_inspect_iolist_as_binary(
    env_pointer: *mut Env,
    term: NifTerm,
    bin: *mut ErlNifBinary,
) -> c_int {
    // SAFETY: the C API gives the env of the current call.
    let env = unsafe { env(env_pointer) };
    if matches!(
        env.slot(term),
        Some(Slot::Buffer(_) | Slot::Term(Term::Binary(_)))
    ) {
        // SAFETY: the caller gives the env and the `bin` of this call.
        return unsafe { enif_inspect_binary(env_pointer, term, bin) };
    }
    let Some(bytes) = env.get(term).as_ref().and_then(iodata_bytes) else {
        return 0;
    };
    // The heap bytes of a vector do not move when the vector moves into
    // the env, so the address stays valid until the env drops.
    let (data, size) = (bytes.as_ptr(), bytes.len());
    env.copies.push(bytes);
    // SAFETY: the C API requires that `bin` points to a writable
    // `ErlNifBinary`. The C API allows the NIF to read the bytes only in
    // the call and not to write them (`erl_nif.c:1405`).
    unsafe {
        (&raw mut (*bin).size).write(size);
        (&raw mut (*bin).data).write(data.cast_mut());
        (&raw mut (*bin).ref_bin).write(std::ptr::null_mut());
    }
    1
}

/// Return the bytes of the atom name `name` in `encoding` with no NUL
/// byte, or `None` for the Latin-1 encoding of a name with a character
/// above 255, whose `latin1_chars` is -1 in ERTS, and for an encoding
/// that is neither `ERL_NIF_LATIN1` nor `ERL_NIF_UTF8`
/// (`erl_nif.c:1820-1840`).
fn atom_bytes(name: &str, encoding: c_int) -> Option<Vec<u8>> {
    match encoding {
        LATIN1 => name.chars().map(|char| u8::try_from(char).ok()).collect(),
        UTF8 => Some(name.as_bytes().to_vec()),
        _ => None,
    }
}

/// `int enif_get_atom(ErlNifEnv*, ERL_NIF_TERM, char*, unsigned,
/// ErlNifCharEncoding)`: write the name of the atom `atom` in `encoding`
/// and a NUL byte to `buf`, and return the number of bytes with the NUL
/// byte (`erl_nif.c:1810-1841`). A term that is not an atom, a name that
/// does not fit with its NUL byte, also in a buffer of 0 bytes, the Latin-1
/// encoding of a name with a character above 255, and another encoding
/// give 0 and write no byte.
///
/// # Safety
///
/// `env` is the env of the current call, and `buf` points to `len`
/// writable bytes, as the C API requires.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn enif_get_atom(
    env_pointer: *mut Env,
    atom: NifTerm,
    buf: *mut c_char,
    len: c_uint,
    encoding: c_int,
) -> c_int {
    // SAFETY: the C API gives the env of the current call.
    let env = unsafe { env(env_pointer) };
    let bytes = env.get(atom).and_then(|term| match &term {
        Term::Atom(atom) => atom_bytes(env.atoms.name_of(atom), encoding),
        _ => None,
    });
    let Some(mut bytes) = bytes else {
        return 0;
    };
    bytes.push(0);
    // A name has at most 1020 bytes, so the count fits in a C `int`.
    let count = c_int::try_from(bytes.len()).unwrap_or(c_int::MAX);
    if count.cast_unsigned() > len {
        return 0;
    }
    // SAFETY: the C API requires that `buf` points to `len` writable
    // bytes, and the bytes are at most `len`.
    unsafe { std::ptr::copy_nonoverlapping(bytes.as_ptr(), buf.cast::<u8>(), bytes.len()) };
    count
}

/// `int enif_get_double(ErlNifEnv*, ERL_NIF_TERM, double*)`: write the
/// float `term` to `dp` and return 1, or return 0 when the term is not a
/// float (`erl_nif.c:1924-1933`).
///
/// # Safety
///
/// `env` is the env of the current call, and `dp` points to a writable C
/// `double`, as the C API requires.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn enif_get_double(
    env_pointer: *mut Env,
    term: NifTerm,
    dp: *mut c_double,
) -> c_int {
    // SAFETY: the C API gives the env of the current call.
    let env = unsafe { env(env_pointer) };
    let Some(Term::Float(value)) = env.get(term) else {
        return 0;
    };
    // SAFETY: the C API requires that `dp` points to a writable C `double`.
    unsafe { dp.write(value) };
    1
}

/// `int enif_get_long(ErlNifEnv*, ERL_NIF_TERM, long*)`: write the integer
/// of `term` to `ip` and return 1, or return 0 when the term is not an
/// integer in the C `long` range, the signed 64-bit range of each PBeam
/// host (`erl_nif.c:1875-1891`). An integer outside that range is a
/// `Term::BigInt`.
///
/// # Safety
///
/// `env` is the env of the current call, and `ip` points to a writable C
/// `long`, as the C API requires.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn enif_get_long(
    env_pointer: *mut Env,
    term: NifTerm,
    ip: *mut c_long,
) -> c_int {
    // SAFETY: the C API gives the env of the current call.
    let env = unsafe { env(env_pointer) };
    let Some(Term::Integer(value)) = env.get(term) else {
        return 0;
    };
    // SAFETY: the C API requires that `ip` points to a writable C `long`.
    unsafe { ip.write(value) };
    1
}

/// `int enif_get_list_cell(ErlNifEnv*, ERL_NIF_TERM, ERL_NIF_TERM*,
/// ERL_NIF_TERM*)`: write the head and the tail of the list cell `term` to
/// `head` and `tail` and return 1, or return 0 when the term is not a list
/// cell (`erl_nif.c:1971-1979`).
///
/// # Safety
///
/// `env` is the env of the current call, and `head` and `tail` point to
/// writable `ERL_NIF_TERM`s, as the C API requires.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn enif_get_list_cell(
    env_pointer: *mut Env,
    term: NifTerm,
    head: *mut NifTerm,
    tail: *mut NifTerm,
) -> c_int {
    // SAFETY: the C API gives the env of the current call.
    let env = unsafe { env(env_pointer) };
    let cell = env.get(term).and_then(|list| {
        let mut elements = list.list_iter()?;
        let first = elements.next()?.clone();
        Some((first, elements.rest().clone()))
    });
    let Some((first, rest)) = cell else {
        return 0;
    };
    let (first, rest) = (env.make(first), env.make(rest));
    // SAFETY: the C API requires that `head` and `tail` point to writable
    // `ERL_NIF_TERM`s.
    unsafe {
        head.write(first);
        tail.write(rest);
    }
    1
}

/// `int enif_get_list_length(ErlNifEnv*, ERL_NIF_TERM, unsigned*)`: write
/// the length of the proper list `term` to `len` and return 1, or return 0
/// for another term and for a length above the C `unsigned` range
/// (`erl_nif.c:1981-1991`).
///
/// # Safety
///
/// `env` is the env of the current call, and `len` points to a writable C
/// `unsigned`, as the C API requires.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn enif_get_list_length(
    env_pointer: *mut Env,
    term: NifTerm,
    len: *mut c_uint,
) -> c_int {
    // SAFETY: the C API gives the env of the current call.
    let env = unsafe { env(env_pointer) };
    let length = env.get(term).and_then(|list| {
        let count = list.proper_list()?.count();
        c_uint::try_from(count).ok()
    });
    let Some(length) = length else {
        return 0;
    };
    // SAFETY: the C API requires that `len` points to a writable C
    // `unsigned`.
    unsafe { len.write(length) };
    1
}

/// `ERL_NIF_TERM enif_make_atom(ErlNifEnv*, const char*)`: return the
/// atom of the C string `name` in the Latin-1 encoding, or raise
/// `error:badarg` (`erl_nif.c:2080-2092`, [`Env::make_atom`]).
///
/// # Safety
///
/// `env` is the env of the current call, and `name` is a C string, as the
/// C API requires.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn enif_make_atom(env_pointer: *mut Env, name: *const c_char) -> NifTerm {
    // SAFETY: the C API gives the env of the current call.
    let env = unsafe { env(env_pointer) };
    // SAFETY: the C API requires that `name` is a C string.
    let name = unsafe { CStr::from_ptr(name) };
    env.make_atom(name.to_bytes())
}

/// `ERL_NIF_TERM enif_make_double(ErlNifEnv*, double)`: return the float
/// `d`, or raise `error:badarg` for an infinity and a NaN
/// (`erl_nif.c:2067-2078`).
///
/// # Safety
///
/// `env` is the env of the current call, as the C API requires.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn enif_make_double(env_pointer: *mut Env, d: c_double) -> NifTerm {
    // SAFETY: the C API gives the env of the current call.
    let env = unsafe { env(env_pointer) };
    if !d.is_finite() {
        return env.raise(Term::static_atom("badarg"));
    }
    env.make(Term::Float(d))
}

/// `ERL_NIF_TERM enif_make_long(ErlNifEnv*, long)`: return the integer
/// `i` (`erl_nif.c:2013-2030`).
///
/// # Safety
///
/// `env` is the env of the current call, as the C API requires.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn enif_make_long(env_pointer: *mut Env, i: c_long) -> NifTerm {
    // SAFETY: the C API gives the env of the current call.
    let env = unsafe { env(env_pointer) };
    env.make(Term::Integer(i))
}

/// `ERL_NIF_TERM enif_make_list(ErlNifEnv*, unsigned, ...)`: return the
/// list of the `cnt` terms of the variable arguments
/// (`erl_nif.c:2195-2222`).
///
/// # Safety
///
/// `env` is the env of the current call, and the call has `cnt` variable
/// arguments of the type `ERL_NIF_TERM`, as the C API requires.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn enif_make_list(
    env_pointer: *mut Env,
    cnt: c_uint,
    mut args: ...
) -> NifTerm {
    // SAFETY: the C API gives the env of the current call.
    let env = unsafe { env(env_pointer) };
    // SAFETY: the C API requires `cnt` variable arguments of the type
    // `ERL_NIF_TERM`.
    let handles: Vec<NifTerm> = (0..cnt).map(|_| unsafe { args.next_arg() }).collect();
    let elements: Vec<Term> = handles.into_iter().map(|handle| env.term(handle)).collect();
    env.make(Term::list_from(elements))
}

/// `ERL_NIF_TERM enif_make_list_cell(ErlNifEnv*, ERL_NIF_TERM,
/// ERL_NIF_TERM)`: return the list cell `[car | cdr]`
/// (`erl_nif.c:2183-2193`).
///
/// # Safety
///
/// `env` is the env of the current call, as the C API requires.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn enif_make_list_cell(
    env_pointer: *mut Env,
    car: NifTerm,
    cdr: NifTerm,
) -> NifTerm {
    // SAFETY: the C API gives the env of the current call.
    let env = unsafe { env(env_pointer) };
    let (car, cdr) = (env.term(car), env.term(cdr));
    env.make(Term::cons(car, cdr))
}

/// `ERL_NIF_TERM enif_make_list_from_array(ErlNifEnv*, const
/// ERL_NIF_TERM[], unsigned)`: return the list of the `cnt` terms of `arr`
/// (`erl_nif.c:2224-2244`). A list of 0 terms reads no handle, so `arr`
/// can be NULL then, as in ERTS.
///
/// # Safety
///
/// `env` is the env of the current call, and `arr` points to `cnt`
/// readable `ERL_NIF_TERM`s, as the C API requires.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn enif_make_list_from_array(
    env_pointer: *mut Env,
    arr: *const NifTerm,
    cnt: c_uint,
) -> NifTerm {
    // SAFETY: the C API gives the env of the current call.
    let env = unsafe { env(env_pointer) };
    // SAFETY: the C API requires that `arr` points to `cnt` readable
    // `ERL_NIF_TERM`s; the walk reads no handle for 0 terms.
    let handles: Vec<NifTerm> = (0..cnt as usize)
        .map(|index| unsafe { arr.add(index).read() })
        .collect();
    let elements: Vec<Term> = handles.into_iter().map(|handle| env.term(handle)).collect();
    env.make(Term::list_from(elements))
}

/// `ERL_NIF_TERM enif_make_tuple(ErlNifEnv*, unsigned, ...)`: return the
/// tuple of the `cnt` terms of the variable arguments
/// (`erl_nif.c:2138-2160`).
///
/// # Safety
///
/// `env` is the env of the current call, and the call has `cnt` variable
/// arguments of the type `ERL_NIF_TERM`, as the C API requires.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn enif_make_tuple(
    env_pointer: *mut Env,
    cnt: c_uint,
    mut args: ...
) -> NifTerm {
    // SAFETY: the C API gives the env of the current call.
    let env = unsafe { env(env_pointer) };
    // SAFETY: the C API requires `cnt` variable arguments of the type
    // `ERL_NIF_TERM`.
    let handles: Vec<NifTerm> = (0..cnt).map(|_| unsafe { args.next_arg() }).collect();
    let elements: Vec<Term> = handles.into_iter().map(|handle| env.term(handle)).collect();
    env.make(Term::tuple(elements))
}

/// `ErlNifResourceType* enif_open_resource_type(ErlNifEnv*, const char*,
/// const char*, ErlNifResourceDtor*, ErlNifResourceFlags,
/// ErlNifResourceFlags*)`: return a new resource type with the destructor
/// `dtor` and write `ERL_NIF_RT_CREATE` to `tried`, as ERTS does in a load
/// callback with `ERL_NIF_RT_CREATE` in `flags` (`erl_nif.c:2696-2785`).
/// In another env, and without `ERL_NIF_RT_CREATE`, it returns NULL and
/// writes `flags` to `tried`. A NULL `tried` gets no write.
///
/// PBeam loads a module once, so no type of the module exists before its
/// load callback, and `ERL_NIF_RT_TAKEOVER` finds no type
/// (`erl_nif.c:2715-2740`). A type lives until the end of the run, as
/// PBeam does not purge a module.
///
/// # Safety
///
/// `env` is the env of the current call, and `tried` is NULL or points to
/// a writable `ErlNifResourceFlags`, as the C API requires.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn enif_open_resource_type(
    env_pointer: *mut Env,
    _module_str: *const c_char,
    _name_str: *const c_char,
    dtor: Option<DtorFn>,
    flags: c_int,
    tried: *mut c_int,
) -> *const ResourceType {
    // SAFETY: the C API gives the env of the current call.
    let env = unsafe { env(env_pointer) };
    let (kind, op) = match &env.library {
        Some(library) if flags & resource::CREATE != 0 => {
            let kind = ResourceType::new(dtor, Arc::clone(&env.atoms), Arc::clone(library));
            (Arc::into_raw(Arc::new(kind)), resource::CREATE)
        }
        _ => (std::ptr::null(), flags),
    };
    if !tried.is_null() {
        // SAFETY: the C API requires that a `tried` that is not NULL
        // points to a writable `ErlNifResourceFlags`.
        unsafe { tried.write(op) };
    }
    kind
}

/// `void* enif_alloc_resource(ErlNifResourceType*, size_t)`: return the
/// data of `size` bytes of a new object of `type` with one C reference
/// (`erl_nif.c:3121-3162`).
///
/// # Safety
///
/// `kind` is a type of `enif_open_resource_type`, as the C API requires.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn enif_alloc_resource(
    kind: *const ResourceType,
    size: usize,
) -> *mut c_void {
    // SAFETY: `enif_open_resource_type` made `kind` with `Arc::into_raw`
    // and never takes it back, so its count stays above 0 and one more
    // `Arc` is valid.
    let kind = unsafe {
        Arc::increment_strong_count(kind);
        Arc::from_raw(kind)
    };
    std::ptr::with_exposed_provenance_mut(resource::allocate(kind, size))
}

/// `void enif_release_resource(void*)`: remove a C reference of the
/// object `obj`; the drop of its last reference calls its destructor
/// (`erl_nif.c:3164-3175`). PBeam calls the destructor at once, where ERTS
/// schedules it as aux work of the scheduler (`erl_nif.c:2961-2972`).
///
/// # Safety
///
/// `obj` is the data of a resource object with a C reference, as the C
/// API requires.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn enif_release_resource(obj: *mut c_void) {
    resource::release(obj.expose_provenance());
}

/// `void enif_keep_resource(void*)`: add a C reference to the object
/// `obj` (`erl_nif.c:3177-3188`).
///
/// # Safety
///
/// `obj` is the data of a live resource object, as the C API requires.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn enif_keep_resource(obj: *mut c_void) {
    resource::keep(obj.expose_provenance());
}

/// `ERL_NIF_TERM enif_make_resource(ErlNifEnv*, void*)`: return the
/// resource term of the object `obj`, a magic reference
/// (`erl_nif.c:3197-3204`). Each term of one object is the same reference.
/// An `obj` that is not the data of a live object breaks the C API
/// contract: the call gives a runtime error.
///
/// # Safety
///
/// `env` is the env of the current call, as the C API requires.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn enif_make_resource(env_pointer: *mut Env, obj: *mut c_void) -> NifTerm {
    // SAFETY: the C API gives the env of the current call.
    let env = unsafe { env(env_pointer) };
    let Some(term) = resource::term(obj.expose_provenance()) else {
        env.unsupported.get_or_insert_with(|| {
            format!("the NIF made a resource term of {obj:p}, which is not a live resource object")
        });
        return NON_VALUE;
    };
    env.make(term)
}

/// `int enif_get_resource(ErlNifEnv*, ERL_NIF_TERM, ErlNifResourceType*,
/// void**)`: write the data of the object of the resource term `term` to
/// `objp` and return 1, or return 0 when the term is not a resource term
/// of `type` (`erl_nif.c:3225-3262`). A magic reference of another kind,
/// such as an ETS table identifier, gives 0.
///
/// # Safety
///
/// `env` is the env of the current call, and `objp` points to a writable
/// `void*`, as the C API requires.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn enif_get_resource(
    env_pointer: *mut Env,
    term: NifTerm,
    kind: *const ResourceType,
    objp: *mut *mut c_void,
) -> c_int {
    // SAFETY: the C API gives the env of the current call.
    let env = unsafe { env(env_pointer) };
    let found = env.get(term);
    let Some(Term::Resource(object)) = &found else {
        return 0;
    };
    let Some(data) = object.data_of(kind) else {
        return 0;
    };
    // SAFETY: the C API requires that `objp` points to a writable `void*`.
    // The term of the env keeps the object alive until the call returns.
    unsafe { objp.write(std::ptr::with_exposed_provenance_mut(data)) };
    1
}

/// `void enif_clear_env(ErlNifEnv*)`: free each term of the env of
/// `enif_alloc_env`, as ERTS frees the temporary objects and the message
/// buffer of its env (`erl_nif.c:700-716`). Each handle of the env then
/// has no term.
///
/// # Safety
///
/// `env` is an env of `enif_alloc_env`, as the C API requires.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn enif_clear_env(env_pointer: *mut Env) {
    // SAFETY: the C API gives an env of `enif_alloc_env`, which the NIF
    // keeps until it calls `enif_free_env`.
    unsafe { env(env_pointer) }.clear();
}

/// `int enif_is_atom(ErlNifEnv*, ERL_NIF_TERM)`: return 1 when `term` is an
/// atom, else 0 (`erl_nif.c:1243-1246`).
///
/// # Safety
///
/// `env` is the env of the current call, as the C API requires.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn enif_is_atom(env_pointer: *mut Env, term: NifTerm) -> c_int {
    // SAFETY: the C API gives the env of the current call.
    let env = unsafe { env(env_pointer) };
    c_int::from(matches!(env.get(term), Some(Term::Atom(_))))
}

/// `int enif_is_empty_list(ErlNifEnv*, ERL_NIF_TERM)`: return 1 when `term`
/// is `[]`, else 0 (`erl_nif.c:1253-1256`).
///
/// # Safety
///
/// `env` is the env of the current call, as the C API requires.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn enif_is_empty_list(env_pointer: *mut Env, term: NifTerm) -> c_int {
    // SAFETY: the C API gives the env of the current call.
    let env = unsafe { env(env_pointer) };
    c_int::from(matches!(env.get(term), Some(Term::Nil)))
}

/// `int enif_is_list(ErlNifEnv*, ERL_NIF_TERM)`: return 1 when `term` is a
/// list, also `[]`, else 0 (`erl_nif.c:1283-1286`). An improper list is a
/// list.
///
/// # Safety
///
/// `env` is the env of the current call, as the C API requires.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn enif_is_list(env_pointer: *mut Env, term: NifTerm) -> c_int {
    // SAFETY: the C API gives the env of the current call.
    let env = unsafe { env(env_pointer) };
    c_int::from(matches!(env.get(term), Some(Term::Nil | Term::Cons(_))))
}

/// `int enif_get_ulong(ErlNifEnv*, ERL_NIF_TERM, unsigned long*)`: write the
/// integer of `term` to `ip` and return 1, or return 0 when the term is not
/// an integer in the C `unsigned long` range, the unsigned 64-bit range of
/// each PBeam host (`erl_nif.c:1893-1910`).
///
/// # Safety
///
/// `env` is the env of the current call, and `ip` points to a writable C
/// `unsigned long`, as the C API requires.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn enif_get_ulong(
    env_pointer: *mut Env,
    term: NifTerm,
    ip: *mut c_ulong,
) -> c_int {
    // SAFETY: the C API gives the env of the current call.
    let env = unsafe { env(env_pointer) };
    let term = env.get(term);
    // `term_to_Uint` takes each integer from 0 to the largest `Uint`, also
    // a bignum above `i64::MAX` (`erl_nif.c:1893-1896`).
    let value = match &term {
        Some(Term::Integer(value)) => c_ulong::try_from(*value).ok(),
        Some(Term::BigInt(value)) => c_ulong::try_from(&**value).ok(),
        _ => None,
    };
    let Some(value) = value else {
        return 0;
    };
    // SAFETY: the C API requires that `ip` points to a writable C
    // `unsigned long`.
    unsafe { ip.write(value) };
    1
}

/// `int enif_make_new_atom_len(ErlNifEnv*, const char*, size_t,
/// ERL_NIF_TERM*, ErlNifCharEncoding)`: write the atom of the `len` bytes at
/// `name` in `encoding` to `atom` and return 1, or return 0 when PBeam
/// cannot make the atom: a name of more than 255 characters, a name that is
/// not UTF-8 in the `ERL_NIF_UTF8` encoding, or an encoding that is neither
/// `ERL_NIF_LATIN1` nor `ERL_NIF_UTF8` (`erl_nif.c:2100-2114`,
/// `atom.c:271-277`). The atom goes into the table of the run, as
/// `enif_make_atom` puts it.
///
/// # Safety
///
/// `env` is the env of the current call, `name` points to `len` readable
/// bytes, and `atom` points to a writable `ERL_NIF_TERM`, as the C API
/// requires.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn enif_make_new_atom_len(
    env_pointer: *mut Env,
    name: *const c_char,
    len: usize,
    atom: *mut NifTerm,
    encoding: c_int,
) -> c_int {
    // SAFETY: the C API gives the env of the current call.
    let env = unsafe { env(env_pointer) };
    // SAFETY: the C API requires that `name` points to `len` readable
    // bytes.
    let bytes = unsafe { std::slice::from_raw_parts(name.cast::<u8>(), len) };
    let name = match encoding {
        LATIN1 => Some(bytes.iter().copied().map(char::from).collect::<String>()),
        UTF8 => std::str::from_utf8(bytes).ok().map(str::to_owned),
        _ => None,
    };
    let Some(name) = name else {
        return 0;
    };
    if name.chars().count() > MAX_ATOM_CHARACTERS {
        return 0;
    }
    let Ok(made) = env.atoms.put(&name) else {
        return 0;
    };
    let handle = env.make(Term::Atom(made));
    // SAFETY: the C API requires that `atom` points to a writable
    // `ERL_NIF_TERM`.
    unsafe { atom.write(handle) };
    1
}

/// `ERL_NIF_TERM enif_make_tuple_from_array(ErlNifEnv*, const
/// ERL_NIF_TERM[], unsigned)`: return the tuple of the `cnt` terms of `arr`
/// (`erl_nif.c:2162-2180`). A tuple of 0 terms reads no handle, so `arr`
/// can be NULL then, as in ERTS.
///
/// # Safety
///
/// `env` is the env of the current call, and `arr` points to `cnt`
/// readable `ERL_NIF_TERM`s, as the C API requires.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn enif_make_tuple_from_array(
    env_pointer: *mut Env,
    arr: *const NifTerm,
    cnt: c_uint,
) -> NifTerm {
    // SAFETY: the C API gives the env of the current call.
    let env = unsafe { env(env_pointer) };
    let mut elements = Vec::with_capacity(cnt as usize);
    for index in 0..cnt as usize {
        // SAFETY: the C API requires that `arr` points to `cnt` readable
        // `ERL_NIF_TERM`s; the walk reads no handle for 0 terms.
        let handle = unsafe { arr.add(index).read() };
        elements.push(env.term(handle));
    }
    env.make(Term::tuple(elements))
}

/// `int enif_get_map_value(ErlNifEnv*, ERL_NIF_TERM, ERL_NIF_TERM,
/// ERL_NIF_TERM*)`: write the value of the key `key` of the map `map` to
/// `value` and return 1, or return 0 when the term is not a map or the map
/// has no such key (`erl_nif.c:3700-3715`).
///
/// # Safety
///
/// `env` is the env of the current call, and `value` points to a writable
/// `ERL_NIF_TERM`, as the C API requires.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn enif_get_map_value(
    env_pointer: *mut Env,
    map: NifTerm,
    key: NifTerm,
    value: *mut NifTerm,
) -> c_int {
    // SAFETY: the C API gives the env of the current call.
    let env = unsafe { env(env_pointer) };
    let found = env.get(map);
    let Some(Term::Map(map)) = &found else {
        return 0;
    };
    let Some(key) = env.get(key) else {
        return 0;
    };
    let Some(found) = map.get(&key).cloned() else {
        return 0;
    };
    let handle = env.make(found);
    // SAFETY: the C API requires that `value` points to a writable
    // `ERL_NIF_TERM`.
    unsafe { value.write(handle) };
    1
}

/// `int enif_make_map_from_arrays(ErlNifEnv*, ERL_NIF_TERM[],
/// ERL_NIF_TERM[], size_t, ERL_NIF_TERM*)`: write the map of the `cnt`
/// pairs of `keys` and `values` to `map_out` and return 1, or return 0 when
/// two keys are equal, as ERTS rejects a duplicate key of a flatmap and of
/// a hashmap (`erl_map.c:673-714`, `erts_hashmap_from_ks_and_vs_extra` with
/// `reject_dupkeys`). A key that the map form of PBeam does not support yet
/// gives 0 and the runtime error of the call.
///
/// # Safety
///
/// `env` is the env of the current call, `keys` and `values` point to `cnt`
/// readable `ERL_NIF_TERM`s, and `map_out` points to a writable
/// `ERL_NIF_TERM`, as the C API requires.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn enif_make_map_from_arrays(
    env_pointer: *mut Env,
    keys: *const NifTerm,
    values: *const NifTerm,
    cnt: usize,
    map_out: *mut NifTerm,
) -> c_int {
    // SAFETY: the C API gives the env of the current call.
    let env = unsafe { env(env_pointer) };
    let mut pairs = Vec::with_capacity(cnt);
    for index in 0..cnt {
        // SAFETY: the C API requires that `keys` and `values` point to
        // `cnt` readable `ERL_NIF_TERM`s.
        let (key, value) = unsafe { (keys.add(index).read(), values.add(index).read()) };
        pairs.push((env.term(key), env.term(value)));
    }
    let duplicate =
        (1..pairs.len()).any(|index| pairs[..index].iter().any(|(key, _)| *key == pairs[index].0));
    if duplicate {
        return 0;
    }
    let map = match Map::from_pairs(pairs, &env.atoms) {
        Ok(map) => map,
        Err(key) => {
            env.unsupported.get_or_insert_with(|| {
                format!("the NIF made a map with the key {key:?}, which PBeam does not support yet")
            });
            return 0;
        }
    };
    let handle = env.make(Term::Map(map));
    // SAFETY: the C API requires that `map_out` points to a writable
    // `ERL_NIF_TERM`.
    unsafe { map_out.write(handle) };
    1
}

/// `ErlNifResourceType* enif_init_resource_type(ErlNifEnv*, const char*,
/// const ErlNifResourceTypeInit*, ErlNifResourceFlags,
/// ErlNifResourceFlags*)`: return a new resource type with the callbacks of
/// `init` and write `ERL_NIF_RT_CREATE` to `tried`, as
/// `enif_open_resource_type` does with its destructor, for a library of NIF
/// 2.16 or newer (`erl_nif.c:2797-2805`, `2697-2745`). In another env, and
/// without `ERL_NIF_RT_CREATE`, it returns NULL and writes `flags` to
/// `tried`, as `enif_open_resource_type` does. A NULL `tried` gets no
/// write.
///
/// # Safety
///
/// `env` is the env of the current call, `init` points to a readable
/// `ErlNifResourceTypeInit`, and `tried` is NULL or points to a writable
/// `ErlNifResourceFlags`, as the C API requires.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn enif_init_resource_type(
    env_pointer: *mut Env,
    _name_str: *const c_char,
    init: *const ErlNifResourceTypeInit,
    flags: c_int,
    tried: *mut c_int,
) -> *const ResourceType {
    // SAFETY: the C API gives the env of the current call.
    let env = unsafe { env(env_pointer) };
    // SAFETY: the C API requires that `init` points to a readable
    // `ErlNifResourceTypeInit`.
    let init = unsafe { &*init };
    let (kind, op) = match &env.library {
        Some(library) if flags & resource::CREATE != 0 => (
            Arc::into_raw(Arc::new(ResourceType::with_init(
                init,
                Arc::clone(&env.atoms),
                Arc::clone(library),
            ))),
            resource::CREATE,
        ),
        _ => (std::ptr::null(), flags),
    };
    if !tried.is_null() {
        // SAFETY: the C API requires that a `tried` that is not NULL
        // points to a writable `ErlNifResourceFlags`.
        unsafe { tried.write(op) };
    }
    kind
}

#[cfg(test)]
#[path = "../../../../tests/runtime/nif/abi/env.rs"]
mod tests;

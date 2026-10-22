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
//! The env of a dirty NIF is the same: the dirty thread makes it for the
//! call, and its terms go back to the process with the result, as the
//! terms of a dirty NIF are on the heap of its process in ERTS
//! (`erl_nif.c:3386-3435`).
//!
//! The executable exports each function, so the dynamic linker binds the
//! undefined `enif_*` symbols of a library to them (`build.rs` adds the
//! export of the `enif_*` symbols to the link of each executable).

use std::ffi::{c_char, c_int, c_uint};

use super::layout::{ErlNifBinary, NifTerm};
use crate::BeamError;
use crate::Term;
use crate::error::error;
use crate::runtime::NativeResult;

/// `THE_NON_VALUE`: the handle that no term has.
const NON_VALUE: NifTerm = 0;

/// `ERL_NIF_LATIN1` of `ErlNifCharEncoding` (`erl_nif.h:195-198`).
const LATIN1: c_int = 1;

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
/// the exception of `enif_make_badarg`, and the first call of a part of the
/// C API that PBeam does not support yet. The C code gets it as an opaque
/// `ErlNifEnv*`.
#[derive(Debug)]
pub(crate) struct Env {
    /// The term of each handle, in handle order.
    slots: Vec<Slot>,
    /// The reason of `enif_make_badarg`, or `None`.
    exception: Option<Term>,
    /// The text of the first unsupported call, or `None`.
    unsupported: Option<String>,
}

impl Env {
    /// Return the env of a call with `arguments` as the handles
    /// `1..=arguments.len()`.
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// let env = Env::new(&[Term::Integer(1)]);
    /// assert_eq!(env.handles(), [1]);
    /// ```
    pub(super) fn new(arguments: &[Term]) -> Self {
        Self {
            slots: arguments.iter().cloned().map(Slot::Term).collect(),
            exception: None,
            unsupported: None,
        }
    }

    /// Return the handles of the arguments, the `argv` of the call.
    pub(super) fn handles(&self) -> Vec<NifTerm> {
        (1..=self.slots.len()).collect()
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
    /// `enif_make_new_binary` has the bytes that the NIF wrote so far.
    fn get(&self, handle: NifTerm) -> Option<Term> {
        self.slot(handle).map(|slot| match slot {
            Slot::Term(term) => term.clone(),
            Slot::Buffer(bytes) => Term::binary(bytes.clone()),
        })
    }

    /// Return the result of a NIF that returned `handle`, as ERTS gives it
    /// after the call: after `enif_make_badarg`, the call raises the
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
unsafe fn env<'a>(env: *mut Env) -> &'a mut Env {
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
        Some(Slot::Term(Term::Binary(binary))) if binary.bit_size() % 8 == 0 => {
            (binary.as_ptr(), binary.len())
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
    env.exception = Some(Term::static_atom("badarg"));
    NON_VALUE
}

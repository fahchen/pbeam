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
//! The executable exports each function, so the dynamic linker binds the
//! undefined `enif_*` symbols of a library to them (`build.rs` adds the
//! export of the `enif_*` symbols to the link of each executable).

use std::ffi::c_int;

use super::layout::NifTerm;
use crate::BeamError;
use crate::Term;
use crate::error::error;
use crate::runtime::NativeResult;

/// `THE_NON_VALUE`: the handle that no term has.
const NON_VALUE: NifTerm = 0;

/// The exported `enif_*` functions. The linker removes a function that no
/// code of the executable refers to, and a NIF library refers to it only
/// at run time, so this static keeps each function in each executable.
#[used]
static EXPORTS: (
    unsafe extern "C" fn(*mut Env, NifTerm, *mut c_int) -> c_int,
    unsafe extern "C" fn(*mut Env, c_int) -> NifTerm,
    unsafe extern "C" fn(*mut Env) -> NifTerm,
) = (enif_get_int, enif_make_int, enif_make_badarg);

/// The env of one NIF call or one load callback: the terms of its handles
/// and the exception of `enif_make_badarg`. The C code gets it as an
/// opaque `ErlNifEnv*`.
#[derive(Debug)]
pub(crate) struct Env {
    /// The term of each handle, in handle order.
    terms: Vec<Term>,
    /// The reason of `enif_make_badarg`, or `None`.
    exception: Option<Term>,
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
            terms: arguments.to_vec(),
            exception: None,
        }
    }

    /// Return the handles of the arguments, the `argv` of the call.
    pub(super) fn handles(&self) -> Vec<NifTerm> {
        (1..=self.terms.len()).collect()
    }

    /// Add `term` and return its handle.
    fn put(&mut self, term: Term) -> NifTerm {
        self.terms.push(term);
        self.terms.len()
    }

    /// Return the term of `handle`, or `None` for `THE_NON_VALUE` and for a
    /// handle that no term of the env has.
    fn get(&self, handle: NifTerm) -> Option<&Term> {
        self.terms.get(handle.wrapping_sub(1))
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
    /// A handle that no term of the env has, also `THE_NON_VALUE`, with no
    /// exception gives a runtime error. In ERTS the result is undefined.
    pub(super) fn finish(mut self, handle: NifTerm) -> Result<NativeResult, BeamError> {
        if let Some(reason) = self.exception.take() {
            return Ok(NativeResult::Raise { reason });
        }
        let term = self.get(handle).cloned();
        term.map(NativeResult::Value).ok_or_else(|| {
            error(format!(
                "the NIF returned the term {handle:#x}, which no enif call of the call made"
            ))
        })
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
    let Some(&Term::Integer(value)) = env.get(term) else {
        return 0;
    };
    let Ok(value) = c_int::try_from(value) else {
        return 0;
    };
    // SAFETY: the C API requires that `ip` points to a writable C `int`.
    unsafe { ip.write(value) };
    1
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
    env.put(Term::Integer(i.into()))
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

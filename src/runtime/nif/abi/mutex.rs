//! The mutex functions of the NIF C API. ERTS gives an `ErlNifMutex` of
//! the driver API, an `ethr_mutex` on a POSIX mutex (`erl_nif.c:2405-2409`,
//! `erl_drv_thread.c:154-245`). PBeam gives a POSIX mutex with the default
//! attributes. The name is only for the lock checker of a debug ERTS, so
//! PBeam does not keep it.

use std::ffi::{c_char, c_void};

/// `ErlNifMutex* enif_mutex_create(char*)`: return a new unlocked mutex
/// (`erl_nif.c:2405`).
#[unsafe(no_mangle)]
pub extern "C" fn enif_mutex_create(_name: *mut c_char) -> *mut c_void {
    // A static initializer gives a valid mutex until its first use, and the
    // box keeps its address from then on.
    Box::into_raw(Box::new(libc::PTHREAD_MUTEX_INITIALIZER)).cast()
}

/// `void enif_mutex_destroy(ErlNifMutex*)`: free an unlocked mutex
/// (`erl_nif.c:2406`).
///
/// # Safety
///
/// `mtx` is a mutex of `enif_mutex_create` that no thread holds and that
/// no call destroyed, as the C API requires.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn enif_mutex_destroy(mtx: *mut c_void) {
    let mutex = mtx.cast::<libc::pthread_mutex_t>();
    // SAFETY: `mutex` is an unlocked mutex of `enif_mutex_create`, which
    // made it from a box, and the C API destroys it once.
    unsafe {
        libc::pthread_mutex_destroy(mutex);
        drop(Box::from_raw(mutex));
    }
}

/// `void enif_mutex_lock(ErlNifMutex*)`: lock the mutex, and wait while
/// another thread holds it (`erl_nif.c:2408`).
///
/// # Safety
///
/// `mtx` is a live mutex of `enif_mutex_create` that the thread does not
/// hold, as the C API requires.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn enif_mutex_lock(mtx: *mut c_void) {
    // SAFETY: `mtx` is a live mutex of `enif_mutex_create`.
    unsafe { libc::pthread_mutex_lock(mtx.cast()) };
}

/// `void enif_mutex_unlock(ErlNifMutex*)`: unlock the mutex that the thread
/// holds (`erl_nif.c:2409`).
///
/// # Safety
///
/// `mtx` is a live mutex of `enif_mutex_create` that the thread holds, as
/// the C API requires.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn enif_mutex_unlock(mtx: *mut c_void) {
    // SAFETY: `mtx` is a live mutex of `enif_mutex_create` that the thread
    // holds.
    unsafe { libc::pthread_mutex_unlock(mtx.cast()) };
}

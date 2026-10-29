//! The kind of the thread that a NIF runs on, which `enif_thread_type`
//! reports.
//!
//! ERTS reads the scheduler data of the thread: a thread that the emulator
//! does not manage has none and gets `ERL_NIF_THR_UNDEFINED`
//! (`erl_nif.c:3586-3600`). PBeam keeps the kind of the thread in a
//! thread-local value: a scheduler thread sets [`NORMAL`] when it starts,
//! and each thread of a dirty pool sets the kind of its pool, so a thread
//! of the library, which PBeam does not manage, keeps [`UNDEFINED`] and
//! gets the ERTS result for it.

use std::cell::Cell;
use std::ffi::c_int;

/// `ERL_NIF_THR_UNDEFINED` (`erl_nif.h:334`).
pub(crate) const UNDEFINED: c_int = 0;
/// `ERL_NIF_THR_NORMAL_SCHEDULER` (`erl_nif.h:335`).
pub(crate) const NORMAL: c_int = 1;
/// `ERL_NIF_THR_DIRTY_CPU_SCHEDULER` (`erl_nif.h:336`).
pub(crate) const DIRTY_CPU: c_int = 2;
/// `ERL_NIF_THR_DIRTY_IO_SCHEDULER` (`erl_nif.h:337`).
pub(crate) const DIRTY_IO: c_int = 3;

thread_local! {
    /// The kind of the thread, or [`UNDEFINED`] for a thread that PBeam
    /// does not manage.
    static KIND: Cell<c_int> = const { Cell::new(UNDEFINED) };
}

/// Make the current thread a thread of the kind `kind`, as a scheduler
/// thread and a thread of a dirty pool do when they start.
pub(crate) fn set(kind: c_int) {
    KIND.set(kind);
}

/// `int enif_thread_type(void)`: return the kind of the thread of the
/// caller: `ERL_NIF_THR_NORMAL_SCHEDULER` on a scheduler,
/// `ERL_NIF_THR_DIRTY_CPU_SCHEDULER` or `ERL_NIF_THR_DIRTY_IO_SCHEDULER` on
/// a thread of a dirty pool, and `ERL_NIF_THR_UNDEFINED` on each other
/// thread, such as a thread of the library (`erl_nif.c:3586-3600`).
#[unsafe(no_mangle)]
pub extern "C" fn enif_thread_type() -> c_int {
    KIND.get()
}

#[cfg(test)]
#[path = "../../../../tests/runtime/nif/abi/thread.rs"]
mod tests;

//! The `#[repr(C)]` mirrors of the `erl_nif.h` types that the loader reads,
//! as the vendored `vendor/erts/include/erl_nif.h` of OTP 29.1.1 defines
//! them (`erl_nif.h:108-169`), with the C function types of the entry.
//!
//! The layout is the LP64 layout of the vendored `erl_int_sizes_config.h`:
//! `ERL_NIF_TERM` is an `unsigned long` of 8 bytes, as a pointer is. The
//! constant assertions below hold the size and the offset of each field,
//! and the test of the module compares them with the `sizeof` and
//! `offsetof` values of the vendored header.

use std::ffi::{c_char, c_int, c_uint, c_void};
use std::mem::{offset_of, size_of};

use super::env::Env;

/// `ERL_NIF_TERM`: an `ErlNifUInt`, which is an `unsigned long` on LP64.
pub(super) type NifTerm = usize;

/// The `fptr` of an `ErlNifFunc`: `ERL_NIF_TERM (*)(ErlNifEnv*, int, const
/// ERL_NIF_TERM[])`.
pub(super) type NifFn = unsafe extern "C" fn(*mut Env, c_int, *const NifTerm) -> NifTerm;

/// The `load` and `reload` callbacks of an `ErlNifEntry`: `int (*)(ErlNifEnv*,
/// void**, ERL_NIF_TERM)`.
pub(super) type LoadFn = unsafe extern "C" fn(*mut Env, *mut *mut c_void, NifTerm) -> c_int;

/// The `upgrade` callback of an `ErlNifEntry`: `int (*)(ErlNifEnv*, void**,
/// void**, ERL_NIF_TERM)`.
pub(super) type UpgradeFn =
    unsafe extern "C" fn(*mut Env, *mut *mut c_void, *mut *mut c_void, NifTerm) -> c_int;

/// The `unload` callback of an `ErlNifEntry`: `void (*)(ErlNifEnv*, void*)`.
pub(super) type UnloadFn = unsafe extern "C" fn(*mut Env, *mut c_void);

/// The `nif_init` function of a NIF library: `ErlNifEntry* nif_init(void)`
/// (`ERL_NIF_INIT_DECL`, `erl_nif.h:407-408`).
pub(super) type InitFn = unsafe extern "C" fn() -> *const ErlNifEntry;

/// `ErlNifFunc` (`erl_nif.h:126-132`): one NIF of the library.
#[repr(C)]
pub(super) struct ErlNifFunc {
    /// The NUL-terminated Latin-1 name of the function.
    pub(super) name: *const c_char,
    /// The arity of the function.
    pub(super) arity: c_uint,
    /// The C function of the NIF. The C API requires a function, so the
    /// type has no NULL value.
    pub(super) fptr: NifFn,
    /// 0, `ERL_NIF_DIRTY_JOB_CPU_BOUND` (1), or `ERL_NIF_DIRTY_JOB_IO_BOUND`
    /// (2) (`erl_nif.h:240-243`, `erl_drv_nif.h:49-52`).
    pub(super) flags: c_uint,
}

/// `ErlNifEntry` of NIF version 2.18 (`erl_nif.h:135-158`). An older
/// library has a shorter entry: `vm_variant` comes with 2.1, `options`
/// with 2.7, `sizeof_ErlNifResourceTypeInit` with 2.12, and `min_erts` with
/// 2.14, so the loader reads a field only of a version that has it.
#[repr(C)]
pub(super) struct ErlNifEntry {
    /// The major NIF version of the library.
    pub(super) major: c_int,
    /// The minor NIF version of the library.
    pub(super) minor: c_int,
    /// The NUL-terminated Latin-1 module name of the library.
    pub(super) name: *const c_char,
    /// The number of entries of `funcs`.
    pub(super) num_of_funcs: c_int,
    /// The first of `num_of_funcs` functions.
    pub(super) funcs: *const ErlNifFunc,
    /// The load callback, or NULL.
    pub(super) load: Option<LoadFn>,
    /// The reload callback, which ERTS does not call since OTP 20.
    pub(super) reload: Option<LoadFn>,
    /// The upgrade callback, or NULL.
    pub(super) upgrade: Option<UpgradeFn>,
    /// The unload callback, which ERTS calls when it purges the module.
    pub(super) unload: Option<UnloadFn>,
    /// The NUL-terminated vm variant, `beam.vanilla` for ERTS (2.1).
    pub(super) vm_variant: *const c_char,
    /// Unused by ERTS (2.7).
    pub(super) options: c_uint,
    /// `sizeof(ErlNifResourceTypeInit)` of the library (2.12).
    pub(super) sizeof_resource_type_init: usize,
    /// The NUL-terminated oldest ERTS version for the library (2.14).
    pub(super) min_erts: *const c_char,
}

/// `ErlNifBinary` (`erl_nif.h:160-169`): the bytes of a binary that
/// `enif_inspect_binary` gives to a NIF.
#[repr(C)]
pub(super) struct ErlNifBinary {
    /// The number of bytes.
    pub(super) size: usize,
    /// The first byte. The NIF reads the bytes and does not write them.
    pub(super) data: *mut u8,
    /// The reference of the ERTS binary, NULL for the binary of an inspect.
    pub(super) ref_bin: *mut c_void,
    /// Reserved; ERTS does not write it.
    pub(super) spare: [*mut c_void; 2],
}

const _: () = {
    assert!(size_of::<NifTerm>() == 8);
    assert!(size_of::<ErlNifFunc>() == 32);
    assert!(offset_of!(ErlNifFunc, name) == 0);
    assert!(offset_of!(ErlNifFunc, arity) == 8);
    assert!(offset_of!(ErlNifFunc, fptr) == 16);
    assert!(offset_of!(ErlNifFunc, flags) == 24);
    assert!(size_of::<ErlNifEntry>() == 96);
    assert!(offset_of!(ErlNifEntry, major) == 0);
    assert!(offset_of!(ErlNifEntry, minor) == 4);
    assert!(offset_of!(ErlNifEntry, name) == 8);
    assert!(offset_of!(ErlNifEntry, num_of_funcs) == 16);
    assert!(offset_of!(ErlNifEntry, funcs) == 24);
    assert!(offset_of!(ErlNifEntry, load) == 32);
    assert!(offset_of!(ErlNifEntry, reload) == 40);
    assert!(offset_of!(ErlNifEntry, upgrade) == 48);
    assert!(offset_of!(ErlNifEntry, unload) == 56);
    assert!(offset_of!(ErlNifEntry, vm_variant) == 64);
    assert!(offset_of!(ErlNifEntry, options) == 72);
    assert!(offset_of!(ErlNifEntry, sizeof_resource_type_init) == 80);
    assert!(offset_of!(ErlNifEntry, min_erts) == 88);
    assert!(size_of::<ErlNifBinary>() == 40);
    assert!(offset_of!(ErlNifBinary, size) == 0);
    assert!(offset_of!(ErlNifBinary, data) == 8);
    assert!(offset_of!(ErlNifBinary, ref_bin) == 16);
    assert!(offset_of!(ErlNifBinary, spare) == 24);
};

#[cfg(test)]
#[path = "../../../../tests/runtime/nif/abi/layout.rs"]
mod tests;

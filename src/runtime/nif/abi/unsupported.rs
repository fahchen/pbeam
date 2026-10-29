//! The exported `enif_*` functions that PBeam does not implement yet.
//!
//! Rustler 0.38.0, which fs_notify 0.1.2 uses, resolves every name of its
//! NIF 2.18 API at load, inside the generated `nif_init`, and aborts the
//! process on the first name that `dlsym` does not find
//! (`rustler-0.38.0/src/sys/nif_filler.rs:35`). Each executable of PBeam
//! therefore exports each name of the 130-name API that another module of
//! the ABI layer implements or that this module stubs. A call of one of these stubs records the runtime error `the NIF
//! called <name>, which PBeam does not implement yet`, which the NIF call
//! gives when it returns, and returns the default value that the C ABI
//! gives a caller that ignores the result (`THE_NON_VALUE` for a term, 0
//! for a flag, NULL for a pointer). The project rule for an unimplemented
//! part is a clear runtime error and a record in `docs/erts-differences.md`
//! (row "NIF shared libraries").
//!
//! A call on a thread that runs no NIF call, such as a thread of the
//! library, has no call to give the error to; it only returns the default
//! value.

use std::ffi::{c_char, c_int, c_uint, c_ulong, c_void};

use super::env::Env;
use super::layout::{
    DlErrHandler, ErlNifBinary, ErlNifMapIterator, ErlNifMonitor, ErlNifPid, ErlNifPort,
    ErlNifResourceTypeInit, ErlNifSysInfo, NifFn, NifTerm,
};
use super::resource::ResourceType;

/// `THE_NON_VALUE`: the term that no env has, which each term-valued stub
/// returns (`erl_nif.h`, "THE_NON_VALUE").
const THE_NON_VALUE: NifTerm = 0;

/// Generate the exported stubs: one `#[unsafe(no_mangle)]` function for
/// each table entry, with the signature, the return type, and the default
/// value of the C API (`vendor/erts/include/erl_nif_api_funcs.h`), and one
/// `#[used]` static that keeps the function in each executable. A
/// variadic parameter has the pattern `_`, which ignores the arguments
/// (`erl_nif_api_funcs.h:97`, `179`, `227`).
macro_rules! unsupported_functions {
    ($($name:ident($($argument:tt)*) $(-> $result:ty)? = $default:expr;)*) => {
        $(
            #[unsafe(no_mangle)]
            pub unsafe extern "C" fn $name($($argument)*) $(-> $result)? {
                super::process::record_unsupported(stringify!($name));
                $default
            }

            const _: () = {
                #[used]
                static KEEP: unsafe extern "C" fn($($argument)*) $(-> $result)? = $name;
            };
        )*
    };
}

unsupported_functions! {
    enif_binary_to_term(_env: *mut Env, _data: *const u8, _sz: usize, _term: *mut NifTerm, _opts: c_uint) -> usize = 0;
    enif_compare(_lhs: NifTerm, _rhs: NifTerm) -> c_int = 0;
    enif_consume_timeslice(_env: *mut Env, _percent: c_int) -> c_int = 0;
    enif_convert_time_unit(_time: i64, _from: c_int, _to: c_int) -> i64 = 0;
    enif_cpu_time(_env: *mut Env) -> NifTerm = THE_NON_VALUE;
    enif_dlopen(_lib: *const c_char, _err_handler: Option<DlErrHandler>, _err_arg: *mut c_void) -> *mut c_void = std::ptr::null_mut();
    enif_dlsym(_handle: *mut c_void, _symbol: *const c_char, _err_handler: Option<DlErrHandler>, _err_arg: *mut c_void) -> *mut c_void = std::ptr::null_mut();
    enif_dynamic_resource_call(_env: *mut Env, _module: NifTerm, _name: NifTerm, _rsrc: NifTerm, _call_data: *const c_void) -> c_int = 0;
    enif_fprintf(_filep: *mut c_void, _format: *const c_char, _: ...) -> c_int = 0;
    enif_get_atom_cache_index(_env: *mut Env, _atom: NifTerm, _index: *mut c_uint) -> c_int = 0;
    enif_get_atom_length(_env: *mut Env, _atom: NifTerm, _len: *mut c_uint, _encoding: c_int) -> c_int = 0;
    enif_get_local_port(_env: *mut Env, _port: NifTerm, _port_id: *mut ErlNifPort) -> c_int = 0;
    enif_get_map_size(_env: *mut Env, _term: NifTerm, _size: *mut usize) -> c_int = 0;
    enif_get_string_length(_env: *mut Env, _list: NifTerm, _len: *mut c_uint, _encoding: c_int) -> c_int = 0;
    enif_get_tuple(_env: *mut Env, _tpl: NifTerm, _arity: *mut c_int, _array: *mut *const NifTerm) -> c_int = 0;
    enif_getenv(_key: *const c_char, _value: *mut c_char, _value_size: *mut usize) -> c_int = 0;
    enif_has_pending_exception(_env: *mut Env, _reason: *mut NifTerm) -> c_int = 0;
    enif_hash(_type: c_int, _term: NifTerm, _salt: u64) -> u64 = 0;
    enif_is_binary(_env: *mut Env, _term: NifTerm) -> c_int = 0;
    enif_is_current_process_alive(_env: *mut Env) -> c_int = 0;
    enif_is_exception(_env: *mut Env, _term: NifTerm) -> c_int = 0;
    enif_is_fun(_env: *mut Env, _term: NifTerm) -> c_int = 0;
    enif_is_identical(_lhs: NifTerm, _rhs: NifTerm) -> c_int = 0;
    enif_is_map(_env: *mut Env, _term: NifTerm) -> c_int = 0;
    enif_is_number(_env: *mut Env, _term: NifTerm) -> c_int = 0;
    enif_is_pid(_env: *mut Env, _term: NifTerm) -> c_int = 0;
    enif_is_pid_undefined(_pid: *const ErlNifPid) -> c_int = 0;
    enif_is_port(_env: *mut Env, _term: NifTerm) -> c_int = 0;
    enif_is_port_alive(_env: *mut Env, _port_id: *mut ErlNifPort) -> c_int = 0;
    enif_is_ref(_env: *mut Env, _term: NifTerm) -> c_int = 0;
    enif_is_tuple(_env: *mut Env, _term: NifTerm) -> c_int = 0;
    enif_make_atom_len(_env: *mut Env, _name: *const c_char, _len: usize) -> NifTerm = THE_NON_VALUE;
    enif_make_copy(_dst_env: *mut Env, _src_term: NifTerm) -> NifTerm = THE_NON_VALUE;
    enif_make_existing_atom(_env: *mut Env, _name: *const c_char, _atom: *mut NifTerm, _encoding: c_int) -> c_int = 0;
    enif_make_existing_atom_len(_env: *mut Env, _name: *const c_char, _len: usize, _atom: *mut NifTerm, _encoding: c_int) -> c_int = 0;
    enif_make_map_put(_env: *mut Env, _map_in: NifTerm, _key: NifTerm, _value: NifTerm, _map_out: *mut NifTerm) -> c_int = 0;
    enif_make_map_remove(_env: *mut Env, _map_in: NifTerm, _key: NifTerm, _map_out: *mut NifTerm) -> c_int = 0;
    enif_make_map_update(_env: *mut Env, _map_in: NifTerm, _key: NifTerm, _value: NifTerm, _map_out: *mut NifTerm) -> c_int = 0;
    enif_make_monitor_term(_env: *mut Env, _monitor: *const ErlNifMonitor) -> NifTerm = THE_NON_VALUE;
    enif_make_new_atom(_env: *mut Env, _name: *const c_char, _atom: *mut NifTerm, _encoding: c_int) -> c_int = 0;
    enif_make_new_map(_env: *mut Env) -> NifTerm = THE_NON_VALUE;
    enif_make_ref(_env: *mut Env) -> NifTerm = THE_NON_VALUE;
    enif_make_resource_binary(_env: *mut Env, _obj: *mut c_void, _data: *const c_void, _size: usize) -> NifTerm = THE_NON_VALUE;
    enif_make_reverse_list(_env: *mut Env, _term: NifTerm, _list: *mut NifTerm) -> c_int = 0;
    enif_make_string(_env: *mut Env, _string: *const c_char, _encoding: c_int) -> NifTerm = THE_NON_VALUE;
    enif_make_string_len(_env: *mut Env, _string: *const c_char, _len: usize, _encoding: c_int) -> NifTerm = THE_NON_VALUE;
    enif_make_sub_binary(_env: *mut Env, _bin_term: NifTerm, _pos: usize, _size: usize) -> NifTerm = THE_NON_VALUE;
    enif_make_uint(_env: *mut Env, _i: c_uint) -> NifTerm = THE_NON_VALUE;
    enif_make_ulong(_env: *mut Env, _i: c_ulong) -> NifTerm = THE_NON_VALUE;
    enif_make_unique_integer(_env: *mut Env, _properties: c_int) -> NifTerm = THE_NON_VALUE;
    enif_map_iterator_create(_env: *mut Env, _map: NifTerm, _iter: *mut ErlNifMapIterator, _entry: c_int) -> c_int = 0;
    enif_map_iterator_destroy(_env: *mut Env, _iter: *mut ErlNifMapIterator) = ();
    enif_map_iterator_get_pair(_env: *mut Env, _iter: *mut ErlNifMapIterator, _key: *mut NifTerm, _value: *mut NifTerm) -> c_int = 0;
    enif_map_iterator_is_head(_env: *mut Env, _iter: *mut ErlNifMapIterator) -> c_int = 0;
    enif_map_iterator_is_tail(_env: *mut Env, _iter: *mut ErlNifMapIterator) -> c_int = 0;
    enif_map_iterator_next(_env: *mut Env, _iter: *mut ErlNifMapIterator) -> c_int = 0;
    enif_map_iterator_prev(_env: *mut Env, _iter: *mut ErlNifMapIterator) -> c_int = 0;
    enif_max_atom_cache_index() -> c_uint = 0;
    enif_monotonic_time(_unit: c_int) -> i64 = 0;
    enif_now_time(_env: *mut Env) -> NifTerm = THE_NON_VALUE;
    enif_open_resource_type_x(_env: *mut Env, _name: *const c_char, _init: *const ErlNifResourceTypeInit, _flags: c_int, _tried: *mut c_int) -> *mut ResourceType = std::ptr::null_mut();
    enif_port_command(_env: *mut Env, _to_port: *const ErlNifPort, _msg_env: *mut Env, _msg: NifTerm) -> c_int = 0;
    enif_priv_data(_env: *mut Env) -> *mut c_void = std::ptr::null_mut();
    enif_realloc_binary(_bin: *mut ErlNifBinary, _size: usize) -> c_int = 0;
    enif_schedule_nif(_env: *mut Env, _name: *const c_char, _flags: c_int, _fp: Option<NifFn>, _argc: c_int, _argv: *const NifTerm) -> NifTerm = THE_NON_VALUE;
    enif_select(_env: *mut Env, _event: c_int, _flags: c_int, _obj: *mut c_void, _pid: *const ErlNifPid, _ref: NifTerm) -> c_int = 0;
    enif_set_option(_env: *mut Env, _opt: c_int, _: ...) -> c_int = 0;
    enif_set_pid_undefined(_pid: *mut ErlNifPid) = ();
    enif_sizeof_resource(_obj: *mut c_void) -> usize = 0;
    enif_snprintf(_buffer: *mut c_char, _size: usize, _format: *const c_char, _: ...) -> c_int = 0;
    enif_system_info(_sip: *mut ErlNifSysInfo, _si_size: usize) = ();
    enif_term_size(_term: NifTerm) -> usize = 0;
    enif_term_to_binary(_env: *mut Env, _term: NifTerm, _bin: *mut ErlNifBinary) -> c_int = 0;
    enif_term_type(_env: *mut Env, _term: NifTerm) -> c_int = 0;
    enif_time_offset(_unit: c_int) -> i64 = 0;
    enif_whereis_pid(_env: *mut Env, _name: NifTerm, _pid: *mut ErlNifPid) -> c_int = 0;
    enif_whereis_port(_env: *mut Env, _name: NifTerm, _port: *mut ErlNifPort) -> c_int = 0;
}

#[cfg(test)]
#[path = "../../../../tests/runtime/nif/abi/unsupported.rs"]
mod tests;

//! The unsupported `enif_*` stubs: each exports its symbol and records the
//! runtime error of its call.

use std::ffi::c_int;
use std::ptr::{null, null_mut};
use std::sync::Arc;

use super::super::process;
use super::*;
use crate::AtomTable;
use crate::runtime::scheduler::node::Node;

/// Call each unsupported function with the arguments 0 and `NULL`, which
/// every stub ignores, and return the default of one term-valued stub.
/// The calls run on a thread with no NIF call, so they record nothing.
#[test]
fn gives_the_default_value_of_each_unimplemented_function() {
    let mut result = 0;
    // SAFETY: each stub ignores all of its arguments and reads no pointer,
    // so the null pointers and the 0 values cannot cause a fault. The
    // variadic stubs ignore the arguments after the fixed ones. The test
    // thread runs no NIF call, so a stub only returns its default value.
    unsafe {
        result |= enif_binary_to_term(null_mut(), null(), 0, null_mut(), 0) as c_int;
        result |= enif_compare(0, 0);
        result |= enif_consume_timeslice(null_mut(), 0);
        result |= enif_convert_time_unit(0, 0, 0) as c_int;
        result |= enif_cpu_time(null_mut()) as c_int;
        result |= (!enif_dlopen(null(), None, null_mut()).is_null()) as c_int;
        result |= (!enif_dlsym(null_mut(), null(), None, null_mut()).is_null()) as c_int;
        result |= enif_dynamic_resource_call(null_mut(), 0, 0, 0, null());
        result |= enif_fprintf(null_mut(), null());
        result |= enif_get_atom_cache_index(null_mut(), 0, null_mut());
        result |= enif_get_atom_length(null_mut(), 0, null_mut(), 0);
        result |= enif_get_local_port(null_mut(), 0, null_mut());
        result |= enif_get_map_size(null_mut(), 0, null_mut());
        result |= enif_get_string_length(null_mut(), 0, null_mut(), 0);
        result |= enif_get_tuple(null_mut(), 0, null_mut(), null_mut());
        result |= enif_getenv(null(), null_mut(), null_mut());
        result |= enif_has_pending_exception(null_mut(), null_mut());
        result |= enif_hash(0, 0, 0) as c_int;
        result |= enif_is_binary(null_mut(), 0);
        result |= enif_is_current_process_alive(null_mut());
        result |= enif_is_exception(null_mut(), 0);
        result |= enif_is_fun(null_mut(), 0);
        result |= enif_is_identical(0, 0);
        result |= enif_is_map(null_mut(), 0);
        result |= enif_is_number(null_mut(), 0);
        result |= enif_is_pid(null_mut(), 0);
        result |= enif_is_pid_undefined(null());
        result |= enif_is_port(null_mut(), 0);
        result |= enif_is_port_alive(null_mut(), null_mut());
        result |= enif_is_ref(null_mut(), 0);
        result |= enif_is_tuple(null_mut(), 0);
        result |= enif_make_atom_len(null_mut(), null(), 0) as c_int;
        result |= enif_make_copy(null_mut(), 0) as c_int;
        result |= enif_make_existing_atom(null_mut(), null(), null_mut(), 0);
        result |= enif_make_existing_atom_len(null_mut(), null(), 0, null_mut(), 0);
        result |= enif_make_map_put(null_mut(), 0, 0, 0, null_mut());
        result |= enif_make_map_remove(null_mut(), 0, 0, null_mut());
        result |= enif_make_map_update(null_mut(), 0, 0, 0, null_mut());
        result |= enif_make_monitor_term(null_mut(), null()) as c_int;
        result |= enif_make_new_atom(null_mut(), null(), null_mut(), 0);
        result |= enif_make_new_map(null_mut()) as c_int;
        result |= enif_make_ref(null_mut()) as c_int;
        result |= enif_make_resource_binary(null_mut(), null_mut(), null(), 0) as c_int;
        result |= enif_make_reverse_list(null_mut(), 0, null_mut());
        result |= enif_make_string(null_mut(), null(), 0) as c_int;
        result |= enif_make_string_len(null_mut(), null(), 0, 0) as c_int;
        result |= enif_make_sub_binary(null_mut(), 0, 0, 0) as c_int;
        result |= enif_make_uint(null_mut(), 0) as c_int;
        result |= enif_make_ulong(null_mut(), 0) as c_int;
        result |= enif_make_unique_integer(null_mut(), 0) as c_int;
        result |= enif_map_iterator_create(null_mut(), 0, null_mut(), 0);
        enif_map_iterator_destroy(null_mut(), null_mut());
        result |= enif_map_iterator_get_pair(null_mut(), null_mut(), null_mut(), null_mut());
        result |= enif_map_iterator_is_head(null_mut(), null_mut());
        result |= enif_map_iterator_is_tail(null_mut(), null_mut());
        result |= enif_map_iterator_next(null_mut(), null_mut());
        result |= enif_map_iterator_prev(null_mut(), null_mut());
        result |= enif_max_atom_cache_index() as c_int;
        result |= enif_monotonic_time(0) as c_int;
        result |= enif_now_time(null_mut()) as c_int;
        result |= (!enif_open_resource_type_x(null_mut(), null(), null(), 0, null_mut()).is_null())
            as c_int;
        result |= enif_port_command(null_mut(), null(), null_mut(), 0);
        result |= (!enif_priv_data(null_mut()).is_null()) as c_int;
        result |= enif_realloc_binary(null_mut(), 0);
        result |= enif_schedule_nif(null_mut(), null(), 0, None, 0, null()) as c_int;
        result |= enif_select(null_mut(), 0, 0, null_mut(), null(), 0);
        result |= enif_set_option(null_mut(), 0);
        enif_set_pid_undefined(null_mut());
        result |= enif_sizeof_resource(null_mut()) as c_int;
        result |= enif_snprintf(null_mut(), 0, null());
        enif_system_info(null_mut(), 0);
        result |= enif_term_size(0) as c_int;
        result |= enif_term_to_binary(null_mut(), 0, null_mut());
        result |= enif_term_type(null_mut(), 0);
        result |= enif_time_offset(0) as c_int;
        result |= enif_whereis_pid(null_mut(), 0, null_mut());
        result |= enif_whereis_port(null_mut(), 0, null_mut());
    }
    assert_eq!(result, 0);
}

/// A stub that a call of a NIF runs records the runtime error of the call.
#[test]
fn records_a_runtime_error_for_the_call_of_a_nif() {
    let (node, mut readers) = Node::new(1).unwrap();
    let atoms = Arc::new(AtomTable::new());
    let mut env = Env::new(&atoms, None, &[]);
    let (value, ended) = process::within(&atoms, &node, &raw mut env, || {
        // SAFETY: the stub reads no argument and gives 0.
        unsafe { enif_is_map(std::ptr::null_mut(), 0) }
    });
    readers.remove(0);
    assert_eq!(value, 0);
    assert_eq!(
        ended.unsupported.as_deref(),
        Some("the NIF called enif_is_map, which PBeam does not implement yet")
    );
}

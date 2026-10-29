//! The kind of the thread of `enif_thread_type`.

use std::thread;

use crate::runtime::nif::abi::thread::{
    DIRTY_CPU, DIRTY_IO, NORMAL, UNDEFINED, enif_thread_type, set,
};

/// A thread that PBeam does not manage has no kind, as a thread of a NIF
/// library has no scheduler data in ERTS (`erl_nif.c:3588-3591`).
#[test]
fn gives_the_undefined_kind_of_a_thread_that_pbeam_does_not_manage() {
    assert_eq!(enif_thread_type(), UNDEFINED);
}

/// Each thread that PBeam manages has the kind that its starter set.
#[test]
fn gives_the_kind_that_the_thread_set() {
    for kind in [NORMAL, DIRTY_CPU, DIRTY_IO] {
        thread::spawn(move || {
            set(kind);
            assert_eq!(enif_thread_type(), kind);
        })
        .join()
        .unwrap();
    }
    assert_eq!(enif_thread_type(), UNDEFINED);
}

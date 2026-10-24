use super::*;

#[test]
fn gives_no_env_on_a_thread_that_runs_no_nif_call() {
    // A thread of the library itself runs no NIF call, so PBeam has no
    // atom table for the env.
    assert!(enif_alloc_env().is_null());
}

//! The load-check table of the `erl_tracer` NIF library
//! (`erts/emulator/nifs/common/erl_tracer_nif.c`), which
//! `erl_tracer:on_load/0` loads (`erl_tracer.erl:566-569`).
//!
//! PBeam has no binding of an `erl_tracer` NIF. A call of a function of the
//! table stops the run with the runtime error of the static library
//! (`Library::unbound_fails`). The load callback of the library returns 0
//! for each load info (`erl_tracer_nif.c:89-99`).

/// The functions of the ERTS `erl_tracer` NIF library, as `(name, arity)`
/// in the order of `nif_funcs` (`erl_tracer_nif.c:49-52`).
/// `erlang:load_nif/2` checks each of them in this order. The entries have
/// no flags field, so their flags are 0.
///
/// # Examples
///
/// ```rust,ignore
/// assert_eq!(FUNCTIONS[1], ("trace", 5));
/// ```
pub(super) const FUNCTIONS: [(&str, u32); 2] = [("enabled", 3), ("trace", 5)];

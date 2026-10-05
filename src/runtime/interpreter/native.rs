//! Native bindings: the NIFs that PBeam links statically, separate from the
//! BIF table.
//!
//! ERTS loads a NIF library with `erlang:load_nif/2`, which writes the call
//! of each NIF over the first instruction of its function
//! (`beam/erl_nif.c:4922-4928`, `5050-5098`). PBeam loads no NIF library. A
//! binding in this registry is active when its module loads, and the
//! `nif_start` instruction at the entry of the function runs it. Each OTP
//! module has one submodule with its bindings.

use super::*;

mod prim_tty;

/// Run the native binding of `module:name/arity`, with the arity of
/// `arguments`. Return `None` when PBeam has no binding for the MFA; the
/// function then runs its BEAM stub body.
///
/// # Examples
///
/// ```rust,ignore
/// let tty = Term::static_atom("stdout");
/// let iovec = Term::List(Vec::new());
/// assert!(matches!(call("prim_tty", "write_nif", &[tty, iovec]), Some(BifResult::Raise { .. })));
/// assert!(call("nif_error_probe", "nif_stub", &[]).is_none());
/// ```
pub(super) fn call(module: &str, name: &str, arguments: &[Term]) -> Option<BifResult> {
    match (module, name, arguments) {
        ("prim_tty", "write_nif", [tty, iovec]) => Some(prim_tty::write_nif(tty, iovec)),
        _ => None,
    }
}

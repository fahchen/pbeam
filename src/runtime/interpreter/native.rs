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
/// `arguments`, for the current process of `scheduler`. Return `None` when
/// PBeam has no binding for the MFA; the function then runs its BEAM stub
/// body.
///
/// # Examples
///
/// ```rust,ignore
/// let stdout = Term::Atom(scheduler.atoms.put("stdout")?);
/// // `false` when stdout is a file.
/// assert!(matches!(call(&mut scheduler, "prim_tty", "isatty", &[stdout]), Some(Ok(BifResult::Value(_)))));
/// assert!(call(&mut scheduler, "nif_error_probe", "nif_stub", &[]).is_none());
/// ```
///
/// # Errors
///
/// A binding gives a runtime error for a part that PBeam does not implement
/// yet, or when a new result atom does not fit in the atom table.
pub(super) fn call(
    scheduler: &mut Scheduler,
    module: &str,
    name: &str,
    arguments: &[Term],
) -> Option<Result<BifResult, BeamError>> {
    match (module, name, arguments) {
        ("prim_tty", "isatty", [tty]) => Some(prim_tty::isatty(&mut scheduler.atoms, tty)),
        ("prim_tty", "tty_create", [fd]) => Some(prim_tty::tty_create(scheduler, fd)),
        ("prim_tty", "tty_window_size", [tty]) => Some(prim_tty::tty_window_size(scheduler, tty)),
        ("prim_tty", "write_nif", [tty, iovec]) => {
            Some(prim_tty::write_nif(&scheduler.resources, tty, iovec))
        }
        _ => None,
    }
}

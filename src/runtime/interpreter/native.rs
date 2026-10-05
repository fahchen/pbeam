//! Native bindings: the NIFs that PBeam links statically, separate from the
//! BIF table.
//!
//! ERTS loads a NIF library with `erlang:load_nif/2`, which writes the call
//! of each NIF over the first instruction of its function
//! (`beam/erl_nif.c:4922-4928`, `5050-5098`). PBeam loads no NIF library. A
//! binding in this registry is active when its module loads, and the
//! `nif_start` instruction at the entry of the function runs it. Each OTP
//! module has one submodule that registers its bindings in `BINDINGS`, and
//! `LIBRARIES` registers each such list under its module.

use super::*;

mod prim_tty;

/// The native binding of one NIF of a module. ERTS finds a NIF of a library
/// by its exact name and arity (`beam/erl_nif.c:4845-4866`).
///
/// # Examples
///
/// ```rust,ignore
/// const WRITE_NIF: Binding = Binding {
///     name: "write_nif",
///     arity: 2,
///     run: |scheduler, arguments| write_nif(scheduler.state.borrow_mut::<Ttys>(), &arguments[0], &arguments[1]),
/// };
/// ```
struct Binding {
    /// The name of the NIF function.
    name: &'static str,
    /// The arity of the NIF function.
    arity: u32,
    /// Runs the NIF for the current process of the scheduler. The lookup
    /// gives exactly `arity` arguments.
    run: fn(&mut Scheduler, &[Term]) -> Result<BifResult, BeamError>,
}

/// The registered bindings of each module, sorted by the module name for a
/// binary search. The bindings of a module are sorted by name, then by
/// arity.
///
/// # Examples
///
/// ```rust,ignore
/// assert_eq!(LIBRARIES[0].0, "prim_tty");
/// ```
const LIBRARIES: &[(&str, &[Binding])] = &[("prim_tty", prim_tty::BINDINGS)];

/// Return the registered binding of exactly `module:name/arity`, or `None`
/// when PBeam has no binding for the MFA.
///
/// # Examples
///
/// ```rust,ignore
/// assert!(binding("prim_tty", "write_nif", 2).is_some());
/// assert!(binding("prim_tty", "write_nif", 1).is_none());
/// assert!(binding("nif_error_probe", "nif_stub", 0).is_none());
/// ```
fn binding(module: &str, name: &str, arity: u32) -> Option<&'static Binding> {
    let library = LIBRARIES.binary_search_by_key(&module, |&(found, _)| found);
    let bindings = LIBRARIES[library.ok()?].1;
    let index =
        bindings.binary_search_by_key(&(name, arity), |binding| (binding.name, binding.arity));
    Some(&bindings[index.ok()?])
}

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
    let binding = binding(module, name, u32::try_from(arguments.len()).ok()?)?;
    Some((binding.run)(scheduler, arguments))
}

#[cfg(test)]
mod tests {
    use super::{LIBRARIES, binding};

    /// `binding` finds a binding with binary searches, so the modules and the
    /// bindings of each module are strictly increasing: sorted, with no
    /// duplicate entry. The lookup matches the exact MFA.
    #[test]
    fn registry_is_sorted_and_matches_the_exact_mfa() {
        assert!(LIBRARIES.windows(2).all(|pair| pair[0].0 < pair[1].0));
        for (_, bindings) in LIBRARIES {
            assert!(
                bindings
                    .windows(2)
                    .all(|pair| (pair[0].name, pair[0].arity) < (pair[1].name, pair[1].arity))
            );
        }
        assert!(binding("prim_tty", "write_nif", 2).is_some());
        assert!(binding("prim_tty", "write_nif", 1).is_none());
        assert!(binding("prim_tty", "write", 2).is_none());
        assert!(binding("prim_file", "write_nif", 2).is_none());
    }
}

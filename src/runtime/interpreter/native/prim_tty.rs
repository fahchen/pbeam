//! Native bindings of the `prim_tty` NIFs (`erts/emulator/nifs/common/prim_tty_nif.c`).

use super::*;

/// The registered bindings of `prim_tty`, sorted by name, then by arity.
///
/// # Examples
///
/// ```rust,ignore
/// assert_eq!((BINDINGS[0].name, BINDINGS[0].arity), ("write_nif", 2));
/// ```
pub(super) const BINDINGS: &[Binding] = &[Binding {
    name: "write_nif",
    arity: 2,
    run: |arguments| write_nif(&arguments[0], &arguments[1]),
}];

/// Run `prim_tty:write_nif(TTY, IOVec)`. The ERTS NIF `tty_write_nif`
/// (`prim_tty_nif.c:354-427`) first gets the TTY resource from `TTY` and
/// raises `badarg` when `TTY` is not a TTY resource (`prim_tty_nif.c:363-364`).
/// Only then does it read `IOVec` and write it. Only `prim_tty:tty_create/1`
/// makes a TTY resource, and PBeam keeps no TTY resource in the node state.
/// Thus no PBeam term is a TTY resource: the binding raises `error:badarg`
/// for each argument, and it writes nothing.
///
/// # Examples
///
/// ```rust,ignore
/// let result = write_nif(&Term::static_atom("stdout"), &Term::static_atom("not_iodata"));
/// assert!(matches!(result, BifResult::Raise { class: ExceptionClass::Error, .. }));
/// ```
fn write_nif(_tty: &Term, _iovec: &Term) -> BifResult {
    BifResult::badarg()
}

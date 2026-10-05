//! Native bindings of the `prim_tty` NIFs (`erts/emulator/nifs/common/prim_tty_nif.c`).

use super::*;

/// Run `prim_tty:write_nif(TTY, IOVec)`. The ERTS NIF `tty_write_nif`
/// (`prim_tty_nif.c:354-427`) first gets the TTY resource from `TTY` and
/// raises `badarg` when `TTY` is not a TTY resource (`prim_tty_nif.c:363-364`).
/// Only then does it read `IOVec` and write it. Only `prim_tty:tty_create/1`
/// makes a TTY resource, and PBeam has no NIF resource term. Thus no PBeam
/// term is a TTY resource: the binding raises `error:badarg` for each
/// argument, and it writes nothing.
///
/// # Examples
///
/// ```rust,ignore
/// let result = write_nif(&Term::static_atom("stdout"), &Term::static_atom("not_iodata"));
/// assert!(matches!(result, BifResult::Raise { class: ExceptionClass::Error, .. }));
/// ```
pub(super) fn write_nif(_tty: &Term, _iovec: &Term) -> BifResult {
    BifResult::badarg()
}

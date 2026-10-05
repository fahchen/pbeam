//! Native bindings of the `prim_tty` NIFs (`erts/emulator/nifs/common/prim_tty_nif.c`).

use super::*;
use crate::runtime::host::terminal::{self, TerminalCheck};

/// Return the fd of the atom `stdin`, `stdout`, or `stderr`: 0, 1, or 2, as
/// ERTS `tty_get_fd` (`prim_tty_nif.c:228-239`) gives with `fileno` of the
/// C standard streams. Return `None` for each other term.
///
/// # Examples
///
/// ```rust,ignore
/// let stderr = Term::Atom(atoms.put("stderr")?);
/// assert_eq!(tty_get_fd(&stderr), Some(2));
/// assert_eq!(tty_get_fd(&Term::static_atom("ok")), None);
/// ```
fn tty_get_fd(term: &Term) -> Option<libc::c_int> {
    match term {
        Term::Atom(atom) => match atom.name() {
            "stdin" => Some(libc::STDIN_FILENO),
            "stdout" => Some(libc::STDOUT_FILENO),
            "stderr" => Some(libc::STDERR_FILENO),
            _ => None,
        },
        _ => None,
    }
}

/// Run `prim_tty:isatty(Fd)`, as the ERTS NIF `isatty_nif`
/// (`prim_tty_nif.c:257-285`) does on Unix. The atom `stdin`, `stdout`, or
/// `stderr` gives the result of `isatty_fd` for fd 0, 1, or 2. Each other
/// term raises `error:badarg`.
///
/// # Examples
///
/// ```rust,ignore
/// let stdout = Term::Atom(atoms.put("stdout")?);
/// // `true` when stdout is a terminal, `false` when it is a file.
/// assert!(matches!(isatty(&mut atoms, &stdout)?, BifResult::Value(Term::Atom(_))));
/// assert!(matches!(isatty(&mut atoms, &Term::Integer(1))?, BifResult::Raise { .. }));
/// ```
///
/// # Errors
///
/// The atom `ebadf` in a full atom table gives the error of
/// `AtomTable::put`.
pub(super) fn isatty(atoms: &mut AtomTable, fd: &Term) -> Result<BifResult, BeamError> {
    match tty_get_fd(fd) {
        Some(fd) => isatty_fd(atoms, fd),
        None => Ok(BifResult::badarg()),
    }
}

/// Return the `isatty_nif` result for a host fd (`prim_tty_nif.c:270-278`):
/// `true` for a terminal, `false` when `isatty` fails with `EINVAL` or
/// `ENOTTY`, and `ebadf` for each other errno.
///
/// # Examples
///
/// ```rust,ignore
/// let ebadf = Term::Atom(atoms.put("ebadf")?);
/// assert!(matches!(isatty_fd(&mut atoms, -1)?, BifResult::Value(term) if term == ebadf));
/// ```
///
/// # Errors
///
/// The atom `ebadf` in a full atom table gives the error of
/// `AtomTable::put`.
fn isatty_fd(atoms: &mut AtomTable, fd: libc::c_int) -> Result<BifResult, BeamError> {
    let result = match terminal::check(fd) {
        TerminalCheck::Terminal => Term::static_atom("true"),
        TerminalCheck::NotTerminal => Term::static_atom("false"),
        TerminalCheck::Error => Term::Atom(atoms.put("ebadf")?),
    };
    Ok(BifResult::Value(result))
}

/// Run `prim_tty:write_nif(TTY, IOVec)`. The ERTS NIF `tty_write_nif`
/// (`prim_tty_nif.c:354-427`) first gets the TTY resource from `TTY` and
/// raises `badarg` when `TTY` is not a TTY resource (`prim_tty_nif.c:363-364`).
/// Only then does it read `IOVec` and write it. Only `prim_tty:tty_create/1`
/// makes a TTY resource, and the PBeam resource store holds only compiled
/// regular expressions. Thus no PBeam term is a TTY resource: the binding
/// raises `error:badarg` for each argument, and it writes nothing.
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn gives_ebadf_for_an_fd_that_is_not_open() {
        // `isatty(-1)` fails with `EBADF`, which is not `EINVAL` or
        // `ENOTTY` (prim_tty_nif.c:272-277). No fd layout of a child
        // process reaches this result for fds 0, 1, and 2, because Rust and
        // ERTS open `/dev/null` on a closed standard fd.
        let mut atoms = AtomTable::new();
        let result = isatty_fd(&mut atoms, -1).unwrap();
        assert!(
            matches!(&result, BifResult::Value(Term::Atom(atom)) if atom.name() == "ebadf"),
            "{result:?}"
        );
    }
}

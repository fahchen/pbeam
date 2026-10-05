//! Native bindings of the `prim_tty` NIFs (`erts/emulator/nifs/common/prim_tty_nif.c`).

use super::*;
use crate::runtime::{
    host::{
        errno::errno_id,
        terminal::{self, TerminalCheck},
    },
    resource::{Resources, TtyResource},
};

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

/// Run `prim_tty:tty_create(Fd)`, as the ERTS NIF `tty_create_nif`
/// (`prim_tty_nif.c:988-1072`) does on Unix. The atom `stdout` or `stderr`
/// gives a new TTY resource with the output fd 1 or 2, and the result is
/// `{ok, Ref}`. The atom `stdin` (fd 0) and each other term raise
/// `error:badarg` (`prim_tty_nif.c:1003-1004`).
///
/// # Examples
///
/// ```rust,ignore
/// let stdout = Term::Atom(scheduler.atoms.put("stdout")?);
/// // {ok, Ref}
/// assert!(matches!(tty_create(&mut scheduler, &stdout)?, BifResult::Value(Term::Tuple(_))));
/// assert!(matches!(tty_create(&mut scheduler, &Term::Integer(1))?, BifResult::Raise { .. }));
/// ```
///
/// # Errors
///
/// A full reference counter gives the error of `Scheduler::make_ref`.
pub(super) fn tty_create(scheduler: &mut Scheduler, fd: &Term) -> Result<BifResult, BeamError> {
    let Some(ofd) = tty_get_fd(fd).filter(|&ofd| ofd != libc::STDIN_FILENO) else {
        return Ok(BifResult::badarg());
    };
    let reference = scheduler.make_ref()?;
    scheduler
        .resources
        .insert_tty(reference, TtyResource { ofd });
    Ok(BifResult::Value(Term::Tuple(vec![
        Term::static_atom("ok"),
        Term::Reference(reference),
    ])))
}

/// Run `prim_tty:tty_window_size(TTY)`, as the ERTS NIF
/// `tty_window_size_nif` (`prim_tty_nif.c:1171-1211`) does on Unix. The
/// result is `{ok, {Cols, Rows}}` from `terminal::window_size`, where -1 is
/// an unknown count, or `{error, enotsup}` when both counts are unknown. A
/// term that is not a TTY resource raises `error:badarg`.
///
/// # Examples
///
/// ```rust,ignore
/// // fd 0 is a 120 by 40 terminal: {ok, {120, 40}}
/// assert!(matches!(tty_window_size(&mut scheduler, &tty)?, BifResult::Value(Term::Tuple(_))));
/// assert!(matches!(tty_window_size(&mut scheduler, &Term::Integer(1))?, BifResult::Raise { .. }));
/// ```
///
/// # Errors
///
/// The atom `enotsup` in a full atom table gives the error of
/// `AtomTable::put`.
pub(super) fn tty_window_size(
    scheduler: &mut Scheduler,
    tty: &Term,
) -> Result<BifResult, BeamError> {
    let Some(tty) = tty_resource(&scheduler.resources, tty) else {
        return Ok(BifResult::badarg());
    };
    let result = match terminal::window_size(tty.ofd) {
        (-1, -1) => Term::Tuple(vec![
            Term::static_atom("error"),
            Term::Atom(scheduler.atoms.put("enotsup")?),
        ]),
        (columns, rows) => Term::Tuple(vec![
            Term::static_atom("ok"),
            Term::Tuple(vec![
                Term::Integer(columns.into()),
                Term::Integer(rows.into()),
            ]),
        ]),
    };
    Ok(BifResult::Value(result))
}

/// Run `prim_tty:setlocale(TTY)`, as the ERTS NIF `setlocale_nif`
/// (`prim_tty_nif.c:747-773`) does on Unix. It does not read its argument.
/// It sets the `LC_CTYPE` locale from the environment, and the result is
/// `true` when the codeset of that locale is UTF-8, else `false`.
///
/// # Examples
///
/// ```rust,ignore
/// // LC_ALL=C.UTF-8
/// assert!(matches!(setlocale(), BifResult::Value(Term::Atom(atom)) if atom.name() == "true"));
/// ```
pub(super) fn setlocale() -> BifResult {
    BifResult::Value(Term::static_atom(if terminal::setlocale_utf8() {
        "true"
    } else {
        "false"
    }))
}

/// Run `prim_tty:setupterm_nif()`, as the ERTS NIF `tty_setupterm_nif`
/// (`prim_tty_nif.c:774-791`) does with `HAVE_TERMCAP`. The result is `ok`
/// when `setupterm` reads the terminfo entry of `TERM`, else
/// `{error, {setupterm, Errno}}`.
///
/// # Examples
///
/// ```rust,ignore
/// // TERM=dumb
/// assert!(matches!(setupterm_nif(&mut atoms)?, BifResult::Value(Term::Atom(atom)) if atom.name() == "ok"));
/// // TERM=pbeam-unknown-term on macOS: {error, {setupterm, enoent}}
/// assert!(matches!(setupterm_nif(&mut atoms)?, BifResult::Value(Term::Tuple(_))));
/// ```
///
/// # Errors
///
/// A new atom in a full atom table gives the error of `AtomTable::put`.
pub(super) fn setupterm_nif(atoms: &mut AtomTable) -> Result<BifResult, BeamError> {
    let result = match terminal::setupterm() {
        Ok(()) => Term::static_atom("ok"),
        Err(errno) => errno_error(atoms, "setupterm", errno)?,
    };
    Ok(BifResult::Value(result))
}

/// Return `{error, {Function, Errno}}`, where `Errno` is the `erl_errno_id`
/// atom of `errno`, as `make_errno_error` (`prim_tty_nif.c:214-226`) does.
///
/// # Examples
///
/// ```rust,ignore
/// // {error, {setupterm, enoent}}
/// let error = errno_error(&mut atoms, "setupterm", libc::ENOENT)?;
/// ```
///
/// # Errors
///
/// A new atom in a full atom table gives the error of `AtomTable::put`.
fn errno_error(
    atoms: &mut AtomTable,
    function: &str,
    errno: libc::c_int,
) -> Result<Term, BeamError> {
    Ok(Term::Tuple(vec![
        Term::static_atom("error"),
        Term::Tuple(vec![
            Term::Atom(atoms.put(function)?),
            Term::Atom(atoms.put(&errno_id(errno))?),
        ]),
    ]))
}

/// Run `prim_tty:write_nif(TTY, IOVec)`. The ERTS NIF `tty_write_nif`
/// (`prim_tty_nif.c:354-427`) first gets the TTY resource from `TTY` and
/// raises `badarg` when `TTY` is not a TTY resource (`prim_tty_nif.c:363-364`).
/// Only then does it read `IOVec` and write it. The binding raises
/// `error:badarg` for each term that is not a TTY resource, and it writes
/// nothing.
///
/// # Examples
///
/// ```rust,ignore
/// let result = write_nif(&resources, &Term::static_atom("stdout"), &Term::static_atom("not_iodata"));
/// assert!(matches!(result, Ok(BifResult::Raise { class: ExceptionClass::Error, .. })));
/// ```
///
/// # Errors
///
/// A TTY resource gives a runtime error: PBeam has no host write yet (#116).
pub(super) fn write_nif(
    resources: &Resources,
    tty: &Term,
    _iovec: &Term,
) -> Result<BifResult, BeamError> {
    match tty_resource(resources, tty) {
        Some(_) => Err(error(
            "prim_tty:write_nif/2 host write is not implemented yet (#116)",
        )),
        None => Ok(BifResult::badarg()),
    }
}

/// Return the TTY resource of `term`, as `enif_get_resource` with the TTY
/// resource type does. Each term that is not a reference to a TTY resource,
/// also a `make_ref/0` reference and the reference of a `re_pattern`, gives
/// `None`.
///
/// # Examples
///
/// ```rust,ignore
/// assert!(tty_resource(&resources, &Term::Reference(tty)).is_some());
/// assert!(tty_resource(&resources, &Term::static_atom("stdout")).is_none());
/// ```
fn tty_resource<'a>(resources: &'a Resources, term: &Term) -> Option<&'a TtyResource> {
    match term {
        Term::Reference(reference) => resources.tty(*reference),
        _ => None,
    }
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

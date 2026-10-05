//! Native bindings of the `prim_tty` NIFs (`erts/emulator/nifs/common/prim_tty_nif.c`).

use super::*;
use crate::runtime::{
    host::{
        errno::errno_id,
        terminal::{self, TerminalCheck},
    },
    tty::{TtyResource, TtyState, Ttys},
};

/// The registered bindings of `prim_tty`, sorted by name, then by arity.
///
/// # Examples
///
/// ```rust,ignore
/// assert_eq!((BINDINGS[0].name, BINDINGS[0].arity), ("isatty", 1));
/// ```
pub(super) const BINDINGS: &[Binding] = &[
    Binding {
        name: "isatty",
        arity: 1,
        run: |scheduler, arguments| isatty(&mut scheduler.atoms, &arguments[0]),
    },
    Binding {
        name: "setlocale",
        arity: 1,
        run: |_, _| Ok(setlocale()),
    },
    Binding {
        name: "setupterm_nif",
        arity: 0,
        run: |scheduler, _| setupterm_nif(&mut scheduler.atoms),
    },
    Binding {
        name: "tty_create",
        arity: 1,
        run: |scheduler, arguments| tty_create(scheduler, &arguments[0]),
    },
    Binding {
        name: "tty_init",
        arity: 2,
        run: |scheduler, arguments| tty_init(scheduler, &arguments[0], &arguments[1]),
    },
    Binding {
        name: "tty_window_size",
        arity: 1,
        run: |scheduler, arguments| tty_window_size(scheduler, &arguments[0]),
    },
    Binding {
        name: "write_nif",
        arity: 2,
        run: |scheduler, arguments| {
            write_nif(
                scheduler.state.borrow_mut::<Ttys>(),
                &arguments[0],
                &arguments[1],
            )
        },
    },
];

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
fn isatty(atoms: &mut AtomTable, fd: &Term) -> Result<BifResult, BeamError> {
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
/// `{ok, Ref}`. The resource keeps the mode of the output fd as its first
/// mode, and its state is `disabled` when `tcgetattr` reads that mode, else
/// `unavailable` (`prim_tty_nif.c:1006-1010`). The atom `stdin` (fd 0) and
/// each other term raise `error:badarg` (`prim_tty_nif.c:1003-1004`).
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
fn tty_create(scheduler: &mut Scheduler, fd: &Term) -> Result<BifResult, BeamError> {
    let Some(ofd) = tty_get_fd(fd).filter(|&ofd| ofd != libc::STDIN_FILENO) else {
        return Ok(BifResult::badarg());
    };
    let (state, first_mode) = match terminal::get_mode(ofd) {
        Some(mode) => (TtyState::Disabled, mode),
        None => (TtyState::Unavailable, terminal::EMPTY_MODE),
    };
    let reference = scheduler.make_ref()?;
    scheduler.state.borrow_mut::<Ttys>().insert(
        reference,
        TtyResource {
            ofd,
            state,
            first_mode,
            set_mode: first_mode,
            owner: None,
        },
    );
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
fn tty_window_size(scheduler: &mut Scheduler, tty: &Term) -> Result<BifResult, BeamError> {
    let Some(tty) = tty_resource(scheduler.state.borrow_mut::<Ttys>(), tty) else {
        return Ok(BifResult::badarg());
    };
    let result = match terminal::window_size(tty.ofd) {
        (-1, -1) => enotsup(&mut scheduler.atoms)?,
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

/// Run `prim_tty:tty_init(TTY, Options)`, as the ERTS NIF `tty_init_nif`
/// (`prim_tty_nif.c:1074-1169`) does on Unix with `HAVE_TERMCAP`. It
/// raises `error:badarg` when `Options` is not a map, then when `TTY` is
/// not a TTY resource, then when `Options` has no `input` key
/// (`prim_tty_nif.c:1081-1089`). An `unavailable` resource gives
/// `{error, enotsup}` for the input `raw` and `ok` for each other input
/// (`prim_tty_nif.c:1091-1095`).
///
/// Else the input `raw` makes the state `enabled`, and each other input
/// makes it `disabled`. For `enabled`, or when the set mode is not the
/// first mode, the binding sets the output fd to the first mode, or to its
/// raw form for `enabled`. A failure of `tcsetattr` gives
/// `{error, {tcsetattr, Errno}}` (`prim_tty_nif.c:1099-1134`). Last, the
/// caller becomes the owner, and the resource monitors it
/// (`prim_tty_nif.c:1162-1165`).
///
/// # Examples
///
/// ```rust,ignore
/// let options = Term::Map(Map::from_pairs(vec![(Term::static_atom("input"), Term::static_atom("raw"))])?);
/// // stdout is a terminal: ok
/// assert!(matches!(tty_init(&mut scheduler, &tty, &options)?, BifResult::Value(Term::Atom(_))));
/// assert!(matches!(tty_init(&mut scheduler, &tty, &Term::Integer(1))?, BifResult::Raise { .. }));
/// ```
///
/// # Errors
///
/// A new atom in a full atom table gives the error of `AtomTable::put`.
fn tty_init(scheduler: &mut Scheduler, tty: &Term, options: &Term) -> Result<BifResult, BeamError> {
    let Term::Map(options) = options else {
        return Ok(BifResult::badarg());
    };
    let caller = scheduler.current_process_mut().id;
    let Term::Reference(reference) = tty else {
        return Ok(BifResult::badarg());
    };
    let ttys = scheduler.state.borrow_mut::<Ttys>();
    let Some(tty) = ttys.get_mut(*reference) else {
        return Ok(BifResult::badarg());
    };
    let Some(input) = options.get(&Term::static_atom("input")) else {
        return Ok(BifResult::badarg());
    };
    let raw = *input == Term::static_atom("raw");
    if tty.state == TtyState::Unavailable {
        let result = if raw {
            enotsup(&mut scheduler.atoms)?
        } else {
            Term::static_atom("ok")
        };
        return Ok(BifResult::Value(result));
    }
    tty.state = if raw {
        TtyState::Enabled
    } else {
        TtyState::Disabled
    };
    if raw || !terminal::same_mode(&tty.set_mode, &tty.first_mode) {
        tty.set_mode = if raw {
            terminal::raw_mode(&tty.first_mode)
        } else {
            tty.first_mode
        };
        if let Err(errno) = terminal::set_mode(tty.ofd, &tty.set_mode) {
            let error = errno_error(&mut scheduler.atoms, "tcsetattr", errno)?;
            return Ok(BifResult::Value(error));
        }
    }
    tty.owner = Some(caller);
    ttys.monitor(caller, *reference);
    Ok(BifResult::Value(Term::static_atom("ok")))
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
fn setlocale() -> BifResult {
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
fn setupterm_nif(atoms: &mut AtomTable) -> Result<BifResult, BeamError> {
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

/// Return `{error, enotsup}`, as `make_enotsup` (`prim_tty_nif.c:208-210`)
/// does.
///
/// # Examples
///
/// ```rust,ignore
/// // {error, enotsup}
/// let error = enotsup(&mut atoms)?;
/// ```
///
/// # Errors
///
/// The atom `enotsup` in a full atom table gives the error of
/// `AtomTable::put`.
fn enotsup(atoms: &mut AtomTable) -> Result<Term, BeamError> {
    Ok(Term::Tuple(vec![
        Term::static_atom("error"),
        Term::Atom(atoms.put("enotsup")?),
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
/// let result = write_nif(&ttys, &Term::static_atom("stdout"), &Term::static_atom("not_iodata"));
/// assert!(matches!(result, Ok(BifResult::Raise { class: ExceptionClass::Error, .. })));
/// ```
///
/// # Errors
///
/// A TTY resource gives a runtime error: PBeam has no host write yet (#116).
fn write_nif(ttys: &Ttys, tty: &Term, _iovec: &Term) -> Result<BifResult, BeamError> {
    match tty_resource(ttys, tty) {
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
/// assert!(tty_resource(&ttys, &Term::Reference(tty)).is_some());
/// assert!(tty_resource(&ttys, &Term::static_atom("stdout")).is_none());
/// ```
fn tty_resource<'a>(ttys: &'a Ttys, term: &Term) -> Option<&'a TtyResource> {
    match term {
        Term::Reference(reference) => ttys.get(*reference),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::runtime::process::tests::frame;
    use crate::term::Map;

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

    #[test]
    fn gives_the_tcsetattr_errno_and_keeps_no_owner() {
        // `tcsetattr` on an fd that is not open fails with `EBADF`, and
        // `tty_init/2` returns before it sets `self` and monitors the caller
        // (prim_tty_nif.c:1131-1133). No fd layout reaches this result: a
        // TTY resource is `disabled` only when `tcgetattr` of its output fd
        // succeeds, and `tcsetattr` of the same fd then succeeds.
        let mut scheduler = Scheduler::new(AtomTable::new());
        scheduler.start(frame(), vec![]).unwrap();
        scheduler.schedule_next();
        let reference = scheduler.make_ref().unwrap();
        scheduler.state.borrow_mut::<Ttys>().insert(
            reference,
            TtyResource {
                ofd: -1,
                state: TtyState::Disabled,
                first_mode: terminal::EMPTY_MODE,
                set_mode: terminal::EMPTY_MODE,
                owner: None,
            },
        );
        let options = Term::Map(
            Map::from_pairs(vec![(Term::static_atom("input"), Term::static_atom("raw"))]).unwrap(),
        );
        let result = tty_init(&mut scheduler, &Term::Reference(reference), &options).unwrap();
        assert_eq!(
            format!("{result:?}"),
            r#"Value(Tuple([Atom("error"), Tuple([Atom("tcsetattr"), Atom("ebadf")])]))"#
        );
        let tty = scheduler.state.borrow_mut::<Ttys>().get(reference).unwrap();
        assert_eq!((&tty.state, tty.owner), (&TtyState::Enabled, None));
    }
}

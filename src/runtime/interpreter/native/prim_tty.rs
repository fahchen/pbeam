//! Native bindings of the `prim_tty` NIFs (`erts/emulator/nifs/common/prim_tty_nif.c`).

use super::bif::{denial, iodata_bytes};
use super::*;
use crate::runtime::{
    capability::{CapabilityKind, Operation},
    host::{
        errno::errno_id,
        input,
        output::{IovecWrite, IovecWritten},
        terminal::{self, TerminalCheck},
    },
    signal::Signal,
    tty::{InputSelect, TtyResource, TtyState, Ttys},
};
use crate::term::Reference;
use std::ffi::CString;

/// The functions of the ERTS `prim_tty` NIF library, as `(name, arity)` in
/// the order of `nif_funcs` (`prim_tty_nif.c:153-174`). `erlang:load_nif/2`
/// checks each of them in this order. PBeam has a binding for only some of
/// them; `BINDINGS` lists those.
///
/// # Examples
///
/// ```rust,ignore
/// assert_eq!(FUNCTIONS[2], ("tty_init", 2));
/// assert_eq!(FUNCTIONS[19], ("tputs_nif", 2));
/// ```
pub(super) const FUNCTIONS: [(&str, u32); 20] = [
    ("isatty", 1),
    ("tty_create", 1),
    ("tty_init", 2),
    ("setlocale", 1),
    ("tty_is_open", 2),
    ("tty_select", 2),
    ("tty_window_size", 1),
    ("write_nif", 2),
    ("tty_encoding", 1),
    ("read_nif", 3),
    ("isprint", 1),
    ("wcwidth", 1),
    ("wcswidth", 1),
    ("sizeof_wchar", 0),
    ("setupterm_nif", 0),
    ("tigetnum_nif", 1),
    ("tigetflag_nif", 1),
    ("tinfo_nif", 0),
    ("tigetstr_nif", 1),
    ("tputs_nif", 2),
];

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
        job: Job::Normal,
        run: |scheduler, arguments| isatty(scheduler, &arguments[0]),
    },
    Binding {
        name: "read_nif",
        arity: 3,
        job: Job::IoBound,
        run: |scheduler, arguments| {
            read_nif(scheduler, &arguments[0], &arguments[1], &arguments[2])
        },
    },
    Binding {
        name: "setlocale",
        arity: 1,
        job: Job::Normal,
        run: |scheduler, _| setlocale(scheduler),
    },
    Binding {
        name: "setupterm_nif",
        arity: 0,
        job: Job::Normal,
        run: |scheduler, _| setupterm_nif(scheduler),
    },
    Binding {
        name: "tigetflag_nif",
        arity: 1,
        job: Job::Normal,
        run: |scheduler, arguments| tigetflag_nif(scheduler, &arguments[0]),
    },
    Binding {
        name: "tigetnum_nif",
        arity: 1,
        job: Job::Normal,
        run: |scheduler, arguments| tigetnum_nif(scheduler, &arguments[0]),
    },
    Binding {
        name: "tty_create",
        arity: 1,
        job: Job::Normal,
        run: |scheduler, arguments| tty_create(scheduler, &arguments[0]),
    },
    Binding {
        name: "tty_encoding",
        arity: 1,
        job: Job::Normal,
        run: |_, _| Ok(tty_encoding()),
    },
    Binding {
        name: "tty_init",
        arity: 2,
        job: Job::Normal,
        run: |scheduler, arguments| tty_init(scheduler, &arguments[0], &arguments[1]),
    },
    Binding {
        name: "tty_select",
        arity: 2,
        job: Job::Normal,
        run: |scheduler, arguments| tty_select(scheduler, &arguments[0], &arguments[1]),
    },
    Binding {
        name: "tty_window_size",
        arity: 1,
        job: Job::Normal,
        run: |scheduler, arguments| tty_window_size(scheduler, &arguments[0]),
    },
    Binding {
        name: "write_nif",
        arity: 2,
        job: Job::IoBound,
        run: |scheduler, arguments| write_nif(scheduler, &arguments[0], &arguments[1]),
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
/// assert_eq!(tty_get_fd(&stderr, &atoms), Some(2));
/// assert_eq!(tty_get_fd(&Term::static_atom("ok"), &atoms), None);
/// ```
fn tty_get_fd(term: &Term, atoms: &AtomTable) -> Option<libc::c_int> {
    match term {
        Term::Atom(atom) => match atoms.name_of(atom) {
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
/// term raises `error:badarg`. Then a caller without a `terminal`
/// capability with `query` gets `error:capability_denied`, and the binding
/// does not call `isatty`.
///
/// # Examples
///
/// ```rust,ignore
/// let stdout = Term::Atom(scheduler.atoms.put("stdout")?);
/// // `true` when stdout is a terminal, `false` when it is a file.
/// assert!(matches!(isatty(&mut scheduler, &stdout)?, BifResult::Value(Term::Atom(_))));
/// assert!(matches!(isatty(&mut scheduler, &Term::Integer(1))?, BifResult::Raise { .. }));
/// ```
///
/// # Errors
///
/// The atom `ebadf` or `capability_denied` in a full atom table gives the
/// error of `AtomTable::put`.
fn isatty(scheduler: &mut Scheduler, fd: &Term) -> Result<BifResult, BeamError> {
    let Some(fd) = tty_get_fd(fd, &scheduler.atoms) else {
        return Ok(BifResult::badarg());
    };
    let check = denial(scheduler, CapabilityKind::Terminal, Operation::Query, None);
    if let Some(denied) = check? {
        return Ok(denied);
    }
    Ok(isatty_fd(fd))
}

/// Return the `isatty_nif` result for a host fd (`prim_tty_nif.c:270-278`):
/// `true` for a terminal, `false` when `isatty` fails with `EINVAL` or
/// `ENOTTY`, and `ebadf` for each other errno.
///
/// # Examples
///
/// ```rust,ignore
/// let ebadf = Term::Atom(Atom::EBADF);
/// assert!(matches!(isatty_fd(-1), BifResult::Value(term) if term == ebadf));
/// ```
fn isatty_fd(fd: libc::c_int) -> BifResult {
    let result = match terminal::check(fd) {
        TerminalCheck::Terminal => Term::static_atom("true"),
        TerminalCheck::NotTerminal => Term::static_atom("false"),
        TerminalCheck::Error => Term::Atom(Atom::EBADF),
    };
    BifResult::Value(result)
}

/// Run `prim_tty:tty_create(Fd)`, as the ERTS NIF `tty_create_nif`
/// (`prim_tty_nif.c:988-1072`) does on Unix. The atom `stdout` or `stderr`
/// gives a new TTY resource with the output fd 1 or 2, and the result is
/// `{ok, Ref}`. The resource keeps the mode of the output fd as its first
/// mode, and its state is `disabled` when `tcgetattr` reads that mode, else
/// `unavailable` (`prim_tty_nif.c:1006-1010`). The atom `stdin` (fd 0) and
/// each other term raise `error:badarg` (`prim_tty_nif.c:1003-1004`). Then
/// a caller without a `terminal` capability with `query` gets
/// `error:capability_denied`, and the binding reads no mode and makes no
/// resource.
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
/// A full reference counter gives the error of `Scheduler::make_ref`. The
/// atom `capability_denied` in a full atom table gives the error of
/// `AtomTable::put`.
fn tty_create(scheduler: &mut Scheduler, fd: &Term) -> Result<BifResult, BeamError> {
    let Some(ofd) = tty_get_fd(fd, &scheduler.atoms).filter(|&ofd| ofd != libc::STDIN_FILENO)
    else {
        return Ok(BifResult::badarg());
    };
    let check = denial(scheduler, CapabilityKind::Terminal, Operation::Query, None);
    if let Some(denied) = check? {
        return Ok(denied);
    }
    let (state, first_mode) = match terminal::get_mode(ofd) {
        Some(mode) => (TtyState::Disabled, mode),
        None => (TtyState::Unavailable, terminal::EMPTY_MODE),
    };
    let reference = scheduler.make_ref()?;
    scheduler.resources.borrow_mut::<Ttys>().insert(
        reference,
        TtyResource {
            ofd,
            state,
            first_mode,
            set_mode: first_mode,
            owner: None,
            reader: None,
        },
    );
    Ok(BifResult::Value(Term::tuple(vec![
        Term::static_atom("ok"),
        Term::Reference(reference),
    ])))
}

/// Run `prim_tty:tty_window_size(TTY)`, as the ERTS NIF
/// `tty_window_size_nif` (`prim_tty_nif.c:1171-1211`) does on Unix. The
/// result is `{ok, {Cols, Rows}}` from `terminal::window_size`, where -1 is
/// an unknown count, or `{error, enotsup}` when both counts are unknown. A
/// term that is not a TTY resource raises `error:badarg`. Then a caller
/// without a `terminal` capability with `window_size` gets
/// `error:capability_denied`, and the binding reads no window size.
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
/// The atom `enotsup` or `capability_denied` in a full atom table gives the
/// error of `AtomTable::put`.
fn tty_window_size(scheduler: &mut Scheduler, tty: &Term) -> Result<BifResult, BeamError> {
    let Some(&TtyResource { ofd, .. }) =
        tty_resource(scheduler.resources.borrow_mut::<Ttys>(), tty)
    else {
        return Ok(BifResult::badarg());
    };
    let check = denial(
        scheduler,
        CapabilityKind::Terminal,
        Operation::WindowSize,
        None,
    );
    if let Some(denied) = check? {
        return Ok(denied);
    }
    let result = match terminal::window_size(ofd) {
        (-1, -1) => enotsup(&scheduler.atoms)?,
        (columns, rows) => Term::tuple(vec![
            Term::static_atom("ok"),
            Term::tuple(vec![
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
/// (`prim_tty_nif.c:1081-1089`). Then a caller without a `terminal`
/// capability with `raw_mode` gets `error:capability_denied`, for each
/// input mode, and the binding changes nothing. An `unavailable` resource gives
/// `{error, enotsup}` for the input `raw` and `ok` for each other input
/// (`prim_tty_nif.c:1091-1095`).
///
/// Else the input `raw` makes the state `enabled`, and each other input
/// makes it `disabled`. For `enabled`, or when the set mode is not the
/// first mode, the binding sets the output fd to the first mode, or to its
/// raw form for `enabled`. A failure of `tcsetattr` gives
/// `{error, {tcsetattr, Errno}}` (`prim_tty_nif.c:1099-1134`). Last, the
/// caller becomes the owner, and the resource monitors it
/// (`prim_tty_nif.c:1162-1165`; see [`monitor_caller`]).
///
/// # Examples
///
/// ```rust,ignore
/// let options = Term::Map(Map::from_pairs(vec![(Term::static_atom("input"), Term::static_atom("raw"))], &AtomTable::new())?);
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
    if scheduler
        .resources
        .borrow_mut::<Ttys>()
        .get(*reference)
        .is_none()
    {
        return Ok(BifResult::badarg());
    }
    let Some(input) = options.get(&Term::static_atom("input")) else {
        return Ok(BifResult::badarg());
    };
    let check = denial(
        scheduler,
        CapabilityKind::Terminal,
        Operation::RawMode,
        None,
    );
    if let Some(denied) = check? {
        return Ok(denied);
    }
    let ttys = scheduler.resources.borrow_mut::<Ttys>();
    let tty = ttys.get_mut(*reference).expect("the TTY resource exists");
    let raw = *input == Term::static_atom("raw");
    if tty.state == TtyState::Unavailable {
        let result = if raw {
            enotsup(&scheduler.atoms)?
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
            let error = errno_error(&scheduler.atoms, "tcsetattr", errno)?;
            return Ok(BifResult::Value(error));
        }
    }
    tty.owner = Some(caller);
    monitor_caller(scheduler, *reference);
    Ok(BifResult::Value(Term::static_atom("ok")))
}

/// Monitor the calling process for the TTY resource `tty`, as
/// `enif_monitor_process` (`erl_nif.c:3957-3989`) does: send a monitor
/// signal with no sender process to the caller. The caller adds the monitor
/// to the end of its target monitor list when it handles the signal.
///
/// # Examples
///
/// ```rust,ignore
/// monitor_caller(&mut scheduler, tty);
/// ```
fn monitor_caller(scheduler: &mut Scheduler, tty: Reference) {
    let caller = scheduler.current_process_mut().id;
    scheduler
        .send_signal(None, caller, Signal::TtyMonitor { tty })
        .expect("the caller is alive");
}

/// Run `prim_tty:tty_select(TTY, Ref)`, as the ERTS NIF `tty_select_nif`
/// (`prim_tty_nif.c:1213-1233`) does on Unix. A term that is not a TTY
/// resource raises `error:badarg` (`prim_tty_nif.c:1220-1221`). Then a
/// caller without a `terminal` capability with `read` gets
/// `error:capability_denied`, and the binding selects nothing. Else the
/// binding selects fd 0, the input fd of the resource, for read with `Ref`
/// as the message reference and the caller as the recipient
/// (`prim_tty_nif.c:1227`; see [`select_input`]). The caller becomes the
/// reader of the resource, the resource monitors it
/// (`prim_tty_nif.c:1229-1230`; see [`monitor_caller`]), and the result is
/// `ok`. When the reader exits, its monitor removes the select of fd 0
/// (`prim_tty_nif.c:1242-1244`), and when it is also the owner, it sets
/// fd 0 to the first mode of the resource (`prim_tty_nif.c:1238-1239`).
///
/// # Examples
///
/// ```rust,ignore
/// let reference = Term::Reference(scheduler.make_ref()?);
/// assert!(matches!(tty_select(&mut scheduler, &tty, &reference)?, BifResult::Value(Term::Atom(atom)) if atom.is_static("ok"));
/// assert!(matches!(tty_select(&mut scheduler, &Term::static_atom("ok"), &reference)?, BifResult::Raise { .. }));
/// ```
///
/// # Errors
///
/// The atom `capability_denied` in a full atom table gives the error of
/// `AtomTable::put`.
fn tty_select(
    scheduler: &mut Scheduler,
    tty: &Term,
    reference: &Term,
) -> Result<BifResult, BeamError> {
    let Term::Reference(tty) = tty else {
        return Ok(BifResult::badarg());
    };
    if scheduler.resources.borrow_mut::<Ttys>().get(*tty).is_none() {
        return Ok(BifResult::badarg());
    }
    if let Some(denied) = denial(scheduler, CapabilityKind::Terminal, Operation::Read, None)? {
        return Ok(denied);
    }
    let caller = scheduler.current_process_mut().id;
    let resource = scheduler.resources.borrow_mut::<Ttys>().get_mut(*tty);
    resource.expect("the TTY resource exists").reader = Some(caller);
    select_input(scheduler, *tty, reference);
    monitor_caller(scheduler, *tty);
    Ok(BifResult::Value(Term::static_atom("ok")))
}

/// Select fd 0 for read for the TTY resource `tty`, with the caller as the
/// recipient and `reference` as the message reference, as
/// `enif_select(env, tty->ifd, ERL_NIF_SELECT_READ, tty, NULL, Ref)`
/// does (`erl_check_io.c:1236-1465`). When fd 0 has input, the caller gets
/// `{select, TTY, Ref, ready_input}` (see `Scheduler::wait_events`).
///
/// # Examples
///
/// ```rust,ignore
/// select_input(&mut scheduler, tty, &Term::static_atom("undefined"));
/// ```
fn select_input(scheduler: &mut Scheduler, tty: Reference, reference: &Term) {
    let recipient = scheduler.current_process_mut().id;
    scheduler
        .resources
        .borrow_mut::<Ttys>()
        .select_input(InputSelect {
            tty,
            recipient,
            reference: reference.clone(),
        });
    let index = scheduler.index();
    scheduler.node().poll.select_stdin(Some(index));
    crate::runtime::scheduler::poll::NodePoll::start(scheduler.node());
}

/// The most bytes of one `read_nif/3` (`prim_tty_nif.c:449`).
const READ_LIMIT: u64 = 1024;

/// Run `prim_tty:read_nif(TTY, Ref, N)`, as the ERTS NIF `tty_read_nif`
/// (`prim_tty_nif.c:429-675`) does on Unix. A term that is not a TTY
/// resource, then an `N` that is not an integer from 0 to 2^64 - 1, raises
/// `error:badarg` (`prim_tty_nif.c:443-447`). PBeam holds an integer only
/// up to 2^63 - 1 (see the "Integer range" row of
/// `docs/erts-differences.md`), so an `N` from 2^63 to 2^64 - 1 does not
/// get to the binding.
///
/// Then a caller without a `terminal` capability with `read` gets
/// `error:capability_denied`, and the binding reads nothing.
///
/// Else the binding reads at most `N` bytes, and at most 1024, from fd 0
/// with one `read` (`prim_tty_nif.c:449`, `646-657`):
///
/// - A `read` that gives 0 bytes, at the end of file or for an `N` of 0,
///   gives `{error, closed}`.
/// - A `read` that fails with `EAGAIN` or `EINTR` gives `{ok, <<>>}`.
/// - A `read` that fails with each other errno gives
///   `{error, {read, Errno}}` with the `erl_errno_id` atom.
/// - Else the result is `{ok, Bytes}`.
///
/// For `{ok, _}`, the binding first selects fd 0 for read again with `Ref`
/// and the caller as the recipient (`prim_tty_nif.c:660`; see
/// [`select_input`]).
///
/// ERTS runs this NIF on a dirty IO scheduler (`prim_tty_nif.c:163`), so
/// the binding has the job `IoBound`. PBeam runs it on the scheduler
/// thread, so a `read` that blocks stops the processes of the scheduler
/// until it returns. The `prim_tty` reader calls `read_nif/3` only after
/// its select message (prim_tty.erl:467-476), and then fd 0 has input.
///
/// # Examples
///
/// ```rust,ignore
/// // stdin is a pipe that holds "hi\n".
/// // {ok, <<"hi\n">>}
/// assert!(matches!(read_nif(&mut scheduler, &tty, &reference, &Term::Integer(1024))?, BifResult::Value(Term::Tuple(_))));
/// assert!(matches!(read_nif(&mut scheduler, &tty, &reference, &Term::Integer(-1))?, BifResult::Raise { .. }));
/// ```
///
/// # Errors
///
/// A new errno atom or the atom `read` or `capability_denied` in a full
/// atom table gives the error of `AtomTable::put`.
fn read_nif(
    scheduler: &mut Scheduler,
    tty: &Term,
    reference: &Term,
    size: &Term,
) -> Result<BifResult, BeamError> {
    let Term::Reference(tty) = tty else {
        return Ok(BifResult::badarg());
    };
    if scheduler.resources.borrow_mut::<Ttys>().get(*tty).is_none() {
        return Ok(BifResult::badarg());
    }
    let Term::Integer(size) = size else {
        return Ok(BifResult::badarg());
    };
    let Ok(size) = u64::try_from(*size) else {
        return Ok(BifResult::badarg());
    };
    if let Some(denied) = denial(scheduler, CapabilityKind::Terminal, Operation::Read, None)? {
        return Ok(denied);
    }
    let size = usize::try_from(size.min(READ_LIMIT)).expect("1024 fits in usize");
    let bytes = match input::read(libc::STDIN_FILENO, size) {
        Ok(bytes) if bytes.is_empty() => {
            let closed = Term::tuple(vec![
                Term::static_atom("error"),
                Term::static_atom("closed"),
            ]);
            return Ok(BifResult::Value(closed));
        }
        Ok(bytes) => bytes,
        Err(libc::EAGAIN | libc::EINTR) => Vec::new(),
        Err(errno) => {
            let error = errno_error(&scheduler.atoms, "read", errno)?;
            return Ok(BifResult::Value(error));
        }
    };
    select_input(scheduler, *tty, reference);
    Ok(BifResult::Value(Term::tuple(vec![
        Term::static_atom("ok"),
        Term::binary(bytes),
    ])))
}

/// Run `prim_tty:tty_encoding(TTY)`, as the ERTS NIF `tty_encoding_nif`
/// (`prim_tty_nif.c:287-297`) does on Unix. Only the Windows code reads
/// `TTY`, so each term gives `utf8`.
///
/// # Examples
///
/// ```rust,ignore
/// assert!(matches!(tty_encoding(), BifResult::Value(Term::Atom(atom)) if atom.is_static("utf8"));
/// ```
fn tty_encoding() -> BifResult {
    BifResult::Value(Term::static_atom("utf8"))
}

/// Run `prim_tty:setlocale(TTY)`, as the ERTS NIF `setlocale_nif`
/// (`prim_tty_nif.c:747-773`) does on Unix. It does not read its argument.
/// It sets the `LC_CTYPE` locale from the environment, and the result is
/// `true` when the codeset of that locale is UTF-8, else `false`. A caller
/// without a `terminal` capability with `setup` gets
/// `error:capability_denied`, and the binding does not change the locale.
///
/// # Examples
///
/// ```rust,ignore
/// // LC_ALL=C.UTF-8
/// assert!(matches!(setlocale(&mut scheduler)?, BifResult::Value(Term::Atom(atom)) if atom.is_static("true"));
/// ```
///
/// # Errors
///
/// The atom `capability_denied` in a full atom table gives the error of
/// `AtomTable::put`.
fn setlocale(scheduler: &mut Scheduler) -> Result<BifResult, BeamError> {
    let check = denial(scheduler, CapabilityKind::Terminal, Operation::Setup, None);
    if let Some(denied) = check? {
        return Ok(denied);
    }
    Ok(BifResult::Value(Term::static_atom(
        if terminal::setlocale_utf8() {
            "true"
        } else {
            "false"
        },
    )))
}

/// Run `prim_tty:setupterm_nif()`, as the ERTS NIF `tty_setupterm_nif`
/// (`prim_tty_nif.c:774-791`) does with `HAVE_TERMCAP`. The result is `ok`
/// when `setupterm` reads the terminfo entry of `TERM`, else
/// `{error, {setupterm, Errno}}`. A caller without a `terminal` capability
/// with `setup` gets `error:capability_denied`, and the binding does not
/// call `setupterm`.
///
/// # Examples
///
/// ```rust,ignore
/// // TERM=dumb
/// assert!(matches!(setupterm_nif(&mut scheduler)?, BifResult::Value(Term::Atom(atom)) if atom.is_static("ok"));
/// // TERM=pbeam-unknown-term on macOS: {error, {setupterm, enoent}}
/// assert!(matches!(setupterm_nif(&mut scheduler)?, BifResult::Value(Term::Tuple(_))));
/// ```
///
/// # Errors
///
/// A new atom in a full atom table gives the error of `AtomTable::put`.
fn setupterm_nif(scheduler: &mut Scheduler) -> Result<BifResult, BeamError> {
    let check = denial(scheduler, CapabilityKind::Terminal, Operation::Setup, None);
    if let Some(denied) = check? {
        return Ok(denied);
    }
    let result = match terminal::setupterm() {
        Ok(()) => Term::static_atom("ok"),
        Err(errno) => errno_error(&scheduler.atoms, "setupterm", errno)?,
    };
    Ok(BifResult::Value(result))
}

/// Run `prim_tty:tigetnum_nif(Code)`, as the ERTS NIF `tty_tigetnum_nif`
/// (`prim_tty_nif.c:841-850`) does with `HAVE_TERMCAP`. The result is the
/// integer that `tgetnum` gives for the termcap code in the iodata `Code`,
/// such as `"co\0"`, from the terminfo entry of the last successful
/// `setupterm_nif/0`; -1 when the entry has no such capability. A term
/// that is not iodata raises `error:badarg` (see [`termcap_code`]). Then a
/// caller without a `terminal` capability with `query` gets
/// `error:capability_denied`, and the binding does not call `tgetnum`.
///
/// # Examples
///
/// ```rust,ignore
/// // TERM=vt100, after setupterm_nif()
/// let code = Term::binary(b"co\0".to_vec());
/// assert!(matches!(tigetnum_nif(&mut scheduler, &code)?, BifResult::Value(Term::Integer(80))));
/// assert!(matches!(tigetnum_nif(&mut scheduler, &Term::static_atom("co"))?, BifResult::Raise { .. }));
/// ```
///
/// # Errors
///
/// The atom `capability_denied` in a full atom table gives the error of
/// `AtomTable::put`.
fn tigetnum_nif(scheduler: &mut Scheduler, code: &Term) -> Result<BifResult, BeamError> {
    let Some(code) = termcap_code(code) else {
        return Ok(BifResult::badarg());
    };
    let check = denial(scheduler, CapabilityKind::Terminal, Operation::Query, None);
    if let Some(denied) = check? {
        return Ok(denied);
    }
    Ok(BifResult::Value(Term::Integer(
        terminal::termcap_number(&code).into(),
    )))
}

/// Run `prim_tty:tigetflag_nif(Code)`, as the ERTS NIF `tty_tigetflag_nif`
/// (`prim_tty_nif.c:852-863`) does with `HAVE_TERMCAP`. The result is
/// `true` when `tgetflag` finds the boolean termcap code in the iodata
/// `Code`, such as `"xn\0"`, in the terminfo entry of the last successful
/// `setupterm_nif/0`, else `false`. A term that is not iodata raises
/// `error:badarg` (see [`termcap_code`]). Then a caller without a
/// `terminal` capability with `query` gets `error:capability_denied`, and
/// the binding does not call `tgetflag`.
///
/// # Examples
///
/// ```rust,ignore
/// // TERM=vt100, after setupterm_nif()
/// let code = Term::binary(b"xn\0".to_vec());
/// assert!(matches!(tigetflag_nif(&mut scheduler, &code)?, BifResult::Value(Term::Atom(atom)) if atom.is_static("true"));
/// assert!(matches!(tigetflag_nif(&mut scheduler, &Term::static_atom("xn"))?, BifResult::Raise { .. }));
/// ```
///
/// # Errors
///
/// The atom `capability_denied` in a full atom table gives the error of
/// `AtomTable::put`.
fn tigetflag_nif(scheduler: &mut Scheduler, code: &Term) -> Result<BifResult, BeamError> {
    let Some(code) = termcap_code(code) else {
        return Ok(BifResult::badarg());
    };
    let check = denial(scheduler, CapabilityKind::Terminal, Operation::Query, None);
    if let Some(denied) = check? {
        return Ok(denied);
    }
    Ok(BifResult::Value(Term::static_atom(
        if terminal::termcap_flag(&code) {
            "true"
        } else {
            "false"
        },
    )))
}

/// Return the termcap code that `tgetnum` and `tgetflag` read from the
/// iodata `term`: its bytes up to the first NUL, as a C string. ERTS gives
/// the bytes of `enif_inspect_iolist_as_binary` to the C function
/// (`prim_tty_nif.c:844-846`, `855-857`), which stops at the first NUL.
/// `prim_tty` always adds a NUL (`prim_tty.erl:1405-1408`). For iodata with
/// no NUL, ERTS reads the memory after the bytes, and PBeam reads all the
/// bytes (see "NIF stubs" in `docs/erts-differences.md`). Return `None`
/// for a term that is not iodata, for which ERTS raises `error:badarg`.
///
/// # Examples
///
/// ```rust,ignore
/// let code = Term::list_from(vec![Term::binary(b"c".to_vec()), Term::Integer(b'o'.into()), Term::Integer(0)]);
/// assert_eq!(termcap_code(&code), Some(c"co".to_owned()));
/// assert_eq!(termcap_code(&Term::binary(b"co\0x".to_vec())), Some(c"co".to_owned()));
/// assert_eq!(termcap_code(&Term::static_atom("co")), None);
/// ```
fn termcap_code(term: &Term) -> Option<CString> {
    let mut bytes = iodata_bytes(term)?;
    bytes.truncate(
        bytes
            .iter()
            .position(|&byte| byte == 0)
            .unwrap_or(bytes.len()),
    );
    Some(CString::new(bytes).expect("the bytes before the first NUL hold no NUL"))
}

/// Return `{error, {Function, Errno}}`, where `Errno` is the `erl_errno_id`
/// atom of `errno`, as `make_errno_error` (`prim_tty_nif.c:214-226`) does.
///
/// # Examples
///
/// ```rust,ignore
/// // {error, {setupterm, enoent}}
/// let error = errno_error(&atoms, "setupterm", libc::ENOENT)?;
/// ```
///
/// # Errors
///
/// A new atom in a full atom table gives the error of `AtomTable::put`.
fn errno_error(atoms: &AtomTable, function: &str, errno: libc::c_int) -> Result<Term, BeamError> {
    Ok(Term::tuple(vec![
        Term::static_atom("error"),
        Term::tuple(vec![
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
/// let error = enotsup(&atoms)?;
/// ```
///
/// # Errors
///
/// The atom `enotsup` in a full atom table gives the error of
/// `AtomTable::put`.
fn enotsup(atoms: &AtomTable) -> Result<Term, BeamError> {
    Ok(Term::tuple(vec![
        Term::static_atom("error"),
        Term::Atom(atoms.put("enotsup")?),
    ]))
}

/// Return `{error, Errno}`, where `Errno` is the `erl_errno_id` atom of
/// `errno`, as `make_error` with `make_errno` (`prim_tty_nif.c:204-220`)
/// does.
///
/// # Examples
///
/// ```rust,ignore
/// // {error, epipe}
/// let error = error_errno(&atoms, libc::EPIPE)?;
/// ```
///
/// # Errors
///
/// A new atom in a full atom table gives the error of `AtomTable::put`.
fn error_errno(atoms: &AtomTable, errno: libc::c_int) -> Result<Term, BeamError> {
    Ok(Term::tuple(vec![
        Term::static_atom("error"),
        Term::Atom(atoms.put(&errno_id(errno))?),
    ]))
}

/// The most elements of one iovec group, as `MAXIOV` (`prim_tty_nif.c:71-77`):
/// `IOV_MAX` when the host defines it, else `UIO_MAXIOV`. Both are 1024 on
/// macOS and Linux. The `libc` crate has no Linux `IOV_MAX`, so Linux uses
/// `UIO_MAXIOV`, which has the same value as the glibc `IOV_MAX`.
///
/// # Examples
///
/// ```rust,ignore
/// assert_eq!(MAXIOV, 1024);
/// ```
#[cfg(target_os = "macos")]
const MAXIOV: usize = libc::IOV_MAX as usize;
#[cfg(not(target_os = "macos"))]
const MAXIOV: usize = libc::UIO_MAXIOV as usize;

/// Run `prim_tty:write_nif(TTY, IOVec)`, as the ERTS NIF `tty_write_nif`
/// (`prim_tty_nif.c:354-427`) does on Unix. ERTS runs this NIF on a dirty
/// IO scheduler (`prim_tty_nif.c:161`), so the binding returns the host
/// work of the call for a dirty IO thread, and the caller switches out
/// until the work completes. [`dirty_write_result`] then gives the result
/// of the call.
///
/// A term that is not a TTY resource gives no write and raises
/// `error:badarg`, and the binding does not read `IOVec`
/// (`prim_tty_nif.c:363-364`). Else the binding reads
/// `IOVec` in groups of `MAXIOV` elements with the checks of
/// `enif_inspect_iovec` (`erl_nif.c:4092-4152`), and the work writes each
/// group to the output fd with `host::output::write_all` before the next
/// group. Each binary that is not empty gives one `writev` entry. A term
/// that is not a list, a group with an element that is not a binary, or
/// the group that holds the last element of an improper list, whatever its
/// tail, raises `error:badarg`, after the work writes the earlier groups
/// (`examine_iovec_term`, `erl_nif.c:4092-4150`;
/// `prim_tty_nif.c:366-368`). A failed write gives `{error, Errno}` with the
/// `erl_errno_id` atom (`prim_tty_nif.c:403-406`). Else the result is `ok`,
/// also for `[]`, which writes nothing.
///
/// After the TTY check and before it reads `IOVec`, the binding checks the
/// capabilities of the caller: without a `terminal` capability with
/// `write`, it raises `error:capability_denied` at once, with no dirty IO
/// call and no write.
///
/// # Examples
///
/// ```rust,ignore
/// let iovec = Term::list_from(vec![Term::binary(b"Hello, ".to_vec()), Term::binary(b"world!\n".to_vec())]);
/// // Writes "Hello, world!\n" to the output fd of the TTY.
/// assert!(matches!(write_nif(&mut scheduler, &tty, &iovec)?, BifResult::Dirty(_)));
/// // [<<"h">> | <<"i">>] writes nothing, then raises badarg.
/// let improper = Term::list_with_tail(vec![Term::binary(b"h".to_vec())], Term::binary(b"i".to_vec()));
/// assert!(matches!(write_nif(&mut scheduler, &tty, &improper)?, BifResult::Dirty(_)));
/// ```
///
/// # Errors
///
/// The atom `capability_denied` in a full atom table gives the error of
/// `AtomTable::put`.
fn write_nif(scheduler: &mut Scheduler, tty: &Term, iovec: &Term) -> Result<BifResult, BeamError> {
    let mut write = IovecWrite {
        fd: -1,
        groups: Vec::new(),
        bad_group: true,
    };
    let Some(&TtyResource { ofd, .. }) =
        tty_resource(scheduler.resources.borrow_mut::<Ttys>(), tty)
    else {
        return Ok(BifResult::Dirty(Work::Write(write)));
    };
    if let Some(denied) = denial(scheduler, CapabilityKind::Terminal, Operation::Write, None)? {
        return Ok(denied);
    }
    write.fd = ofd;
    let Some((elements, tail)) = iovec.list_cells() else {
        return Ok(BifResult::Dirty(Work::Write(write)));
    };
    let improper = tail.is_some();
    let elements: Vec<&Term> = elements.collect();
    let mut groups = elements.chunks(MAXIOV).peekable();
    while let Some(group) = groups.next() {
        // The group of the last element reads the tail after it.
        if improper && groups.peek().is_none() {
            return Ok(BifResult::Dirty(Work::Write(write)));
        }
        let mut buffers = Vec::with_capacity(group.len());
        for &element in group {
            match element {
                Term::Binary(bytes) if bytes.bit_size() == 0 => {}
                Term::Binary(bytes) if let Some(bytes) = bytes.bytes() => {
                    buffers.push(bytes.to_vec())
                }
                _ => return Ok(BifResult::Dirty(Work::Write(write))),
            }
        }
        write.groups.push(buffers);
    }
    write.bad_group = false;
    Ok(BifResult::Dirty(Work::Write(write)))
}

/// Return the result of a `prim_tty:write_nif/2` call from the end of its
/// host work: `ok`, `{error, Errno}` with the `erl_errno_id` atom
/// (`make_error` with `make_errno`, `prim_tty_nif.c:204-220`, `403-406`), or
/// `error:badarg` (`prim_tty_nif.c:363-368`).
///
/// # Examples
///
/// ```rust,ignore
/// assert!(matches!(dirty_write_result(&atoms, IovecWritten::All)?, BifResult::Value(_)));
/// // {error, epipe}
/// assert!(matches!(dirty_write_result(&atoms, IovecWritten::Failed(libc::EPIPE))?, BifResult::Value(Term::Tuple(_))));
/// assert!(matches!(dirty_write_result(&atoms, IovecWritten::BadGroup)?, BifResult::Raise { .. }));
/// ```
///
/// # Errors
///
/// A new errno atom in a full atom table gives the error of
/// `AtomTable::put`.
pub(in crate::runtime::interpreter) fn dirty_write_result(
    atoms: &AtomTable,
    written: IovecWritten,
) -> Result<BifResult, BeamError> {
    Ok(match written {
        IovecWritten::All => BifResult::Value(Term::static_atom("ok")),
        IovecWritten::Failed(errno) => BifResult::Value(error_errno(atoms, errno)?),
        IovecWritten::BadGroup => BifResult::badarg(),
    })
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
#[path = "../../../../tests/runtime/interpreter/native/prim_tty.rs"]
mod tests;

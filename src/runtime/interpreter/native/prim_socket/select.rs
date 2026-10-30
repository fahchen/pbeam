//! The bindings of `prim_socket:nif_connect/1,3`, `nif_listen/2`,
//! `nif_accept/2`, and `nif_cancel/3` (`prim_socket_nif.c:6027-6340`,
//! `13895-14100`), with the Unix I/O backend `unix_essio`
//! (`unix_socket_syncio.c:2008-2172`, `2379-2878`, `4967-5132`).
//!
//! A connect or an accept that would block makes a one-shot select on the
//! node poll thread (see `NodePoll::select`), as `esock_select_write` and
//! `esock_select_read` do. When the socket is ready, the thread sends
//! `{'$socket', Socket, select, Ref}` to the caller.

use std::os::fd::AsRawFd;

use super::socket::{
    closed, errno_error, invalid, send_aborts, send_registry, sockaddr, socket_argument,
    socket_message,
};
use super::supports::atom;
use super::{Arc, BeamError, BifResult, ExceptionClass, Scheduler, Term, error};
use crate::runtime::port::inet::errno_name;
use crate::runtime::resources::lock;
use crate::runtime::scheduler::poll::{Direction, NodePoll};
use crate::runtime::signal::Signal;
use crate::runtime::sockets::{
    ACCEPTING, CLOSED, CLOSING, CONNECTED, CONNECTING, IoCounters, LISTENING, OtpOptions, SELECTED,
    SocketState, Sockets,
};
use crate::term::{ProcessId, Reference};

/// Return the raise of `error:{invalid, state}` of `esock_raise_invalid`.
fn invalid_state(scheduler: &mut Scheduler) -> Result<BifResult, BeamError> {
    let state = atom(scheduler, "state")?;
    Ok(BifResult::Raise {
        class: ExceptionClass::Error,
        reason: Term::tuple(vec![Term::static_atom("invalid"), state]),
    })
}

/// Return `{error, Reason}` for the atom `reason`.
fn error_atom(scheduler: &mut Scheduler, reason: &str) -> Result<BifResult, BeamError> {
    let reason = atom(scheduler, reason)?;
    Ok(BifResult::Value(Term::tuple(vec![
        Term::static_atom("error"),
        reason,
    ])))
}

/// Return the atom `name` as the result.
fn value(scheduler: &mut Scheduler, name: &str) -> Result<BifResult, BeamError> {
    Ok(BifResult::Value(atom(scheduler, name)?))
}

/// Make the select of `fd` in `direction` for `pid`, with the message
/// `{'$socket', Socket, select, Ref}`, as `esock_select_read` and
/// `esock_select_write` do (`prim_socket_nif.c:17707-17760`). The caller
/// holds the lock of the socket, so that a close cannot run between its
/// check and the select and leave a closed fd in the poll table.
pub(super) fn select(
    scheduler: &mut Scheduler,
    socket: Reference,
    fd: i32,
    direction: Direction,
    pid: ProcessId,
    reference: Term,
) {
    // The load callback made the atoms (`prim_socket_nif.c:18842-18857`).
    let message = socket_message(scheduler, socket, "select", reference).expect("made at load");
    scheduler.node().poll.select(fd, direction, pid, message);
    NodePoll::start(scheduler.node());
}

/// Return the fd of the host socket of an open socket.
pub(super) fn fd(socket: &SocketState) -> i32 {
    socket
        .socket
        .as_ref()
        .expect("an open socket has a host socket")
        .as_raw_fd()
}

/// Run `prim_socket:nif_connect(Socket, Ref, SockAddr)` and
/// `nif_connect(Socket)` (`address` is `None`), as `nif_connect`
/// (`prim_socket_nif.c:6027-6175`) and `essio_connect`
/// (`unix_socket_syncio.c:2008-2168`) do. A `Ref` that is not a reference
/// gives `badarg`. A list of addresses raises `error:notsup`: the pinned
/// build has no `HAVE_SCTP` (`prim_socket_nif.c:6058-6112`). A
/// socket with a connect of another process gives `{error, already}`, or
/// raises `{invalid, state}` without an address. A connect without an
/// address and without a connect in progress raises `{invalid, state}`.
/// The connector ends its connect with `SO_ERROR` (`verify_is_connected`,
/// `unix_socket_syncio.c:2325-2370`). An `EINPROGRESS` makes the write
/// select and gives `select`.
///
/// # Errors
///
/// An address that `esock_decode_sockaddr` rejects stops the run (see the
/// row "NIF stubs"). A new atom in a full atom table gives the error of
/// `AtomTable::put`.
pub(super) fn connect(
    scheduler: &mut Scheduler,
    socket: &Term,
    reference: Option<(&Term, &Term)>,
) -> Result<BifResult, BeamError> {
    let Some((key, socket)) = socket_argument(scheduler, socket) else {
        return Ok(BifResult::badarg());
    };
    let address = match reference {
        None => None,
        Some((reference, address)) => {
            if !matches!(reference, Term::Reference(_)) {
                return Ok(BifResult::badarg());
            }
            if matches!(address, Term::Cons(_) | Term::Nil) {
                return Ok(BifResult::Raise {
                    class: ExceptionClass::Error,
                    reason: atom(scheduler, "notsup")?,
                });
            }
            let Some(decoded) = sockaddr(scheduler, address)? else {
                // ERTS sets `{invalid, sockaddr}` and then connects with the
                // address that it did not decode (`prim_socket_nif.c:6122-6143`),
                // so its result depends on stack memory.
                return Err(error(
                    "prim_socket:nif_connect/3: an address that does not decode is not supported"
                        .to_owned(),
                ));
            };
            Some((reference.clone(), decoded))
        }
    };
    let caller = scheduler.current_process().id;
    let mut state = lock(&socket);
    if state.write & (CLOSING | CLOSED) != 0 {
        return Ok(closed());
    }
    if let Some((connector, _)) = &state.connector {
        if *connector != caller {
            drop(state);
            if address.is_some() {
                return error_atom(scheduler, "already");
            }
            return invalid_state(scheduler);
        }
        state.connector = None;
        state.write &= !CONNECTING;
        let host = state
            .socket
            .as_ref()
            .expect("an open socket has a host socket");
        let failure = match host.take_error() {
            Ok(None) => None,
            Ok(Some(failure)) | Err(failure) => Some(failure),
        };
        if let Some(failure) = failure {
            drop(state);
            return errno_error(scheduler, &failure);
        }
        state.write |= CONNECTED;
        return Ok(BifResult::Value(Term::static_atom("ok")));
    }
    let Some((reference, address)) = address else {
        drop(state);
        return invalid_state(scheduler);
    };
    let host = state
        .socket
        .as_ref()
        .expect("an open socket has a host socket");
    match host.connect(&address) {
        Ok(()) => {
            state.write |= CONNECTED;
            Ok(BifResult::Value(Term::static_atom("ok")))
        }
        Err(failure) if failure.raw_os_error() == Some(libc::EINPROGRESS) => {
            let fd = fd(&state);
            state.connector = Some((caller, reference.clone()));
            state.write |= CONNECTING | SELECTED;
            select(scheduler, key, fd, Direction::Write, caller, reference);
            drop(state);
            monitor(scheduler, caller, key);
            value(scheduler, "select")
        }
        Err(failure) => {
            drop(state);
            errno_error(scheduler, &failure)
        }
    }
}

/// Make the socket `socket` monitor `pid`, as `MONP` does.
pub(super) fn monitor(scheduler: &mut Scheduler, pid: ProcessId, socket: Reference) {
    scheduler
        .send_signal(None, pid, Signal::SocketMonitor { socket })
        .expect("the caller is alive");
}

/// Run `prim_socket:nif_listen(Socket, Backlog)`, as `nif_listen` and
/// `esock_listen` do (`prim_socket_nif.c:6190-6276`): `badarg` for a term
/// that is not a socket or a `Backlog` that is not an integer,
/// `{invalid, {integer_range, Backlog}}` for an integer outside the `int`
/// range, `{error, closed}`, `{error, Errno}` of `listen(2)`, or `ok`,
/// which adds the read state `listening`.
///
/// # Errors
///
/// A new atom in a full atom table gives the error of `AtomTable::put`.
pub(super) fn listen(
    scheduler: &mut Scheduler,
    socket: &Term,
    backlog: &Term,
) -> Result<BifResult, BeamError> {
    let Some((_, socket)) = socket_argument(scheduler, socket) else {
        return Ok(BifResult::badarg());
    };
    let value = match backlog {
        Term::Integer(value) => i32::try_from(*value).ok(),
        Term::BigInt(_) => None,
        _ => return Ok(BifResult::badarg()),
    };
    let Some(backlog) = value else {
        let range = Term::tuple(vec![atom(scheduler, "integer_range")?, backlog.clone()]);
        return Ok(invalid(range));
    };
    let mut state = lock(&socket);
    if state.read & (CLOSING | CLOSED) != 0 {
        return Ok(closed());
    }
    let host = state
        .socket
        .as_ref()
        .expect("an open socket has a host socket");
    if let Err(failure) = host.listen(backlog) {
        drop(state);
        return errno_error(scheduler, &failure);
    }
    state.read |= LISTENING;
    Ok(BifResult::Value(Term::static_atom("ok")))
}

/// Run `prim_socket:nif_accept(LSocket, Ref)`, as `nif_accept`
/// (`prim_socket_nif.c:6291-6360`) and `essio_accept`
/// (`unix_socket_syncio.c:2379-2878`) do: `badarg` for a term that is not a
/// socket, `{error, closed}`, `{ok, Socket}` for an accepted connection,
/// `select` after `EAGAIN` with the read select, or `{error, Errno}`. A
/// call of the current acceptor that would block again cancels the old
/// select: an active select sends `{'$socket', LSocket, abort, {OldRef,
/// cancelled}}` and makes a new one; a select whose message went out gives
/// `select_sent` (`unix_socket_syncio.c:2643-2737`).
///
/// A call while another process is the current acceptor puts the caller
/// into the acceptor queue and gives `select`; a second call of a waiting
/// process raises `{invalid, state}` (`essio_accept_accepting_other`,
/// `unix_socket_syncio.c:2746-2758`). An accept of the current acceptor
/// makes the next waiting acceptor current with a read select, and an
/// accept error sends `abort` with the errno to each waiting acceptor
/// (`unix_socket_syncio.c:2596-2633`, `2703-2733`).
///
/// # Errors
///
/// A new atom in a full atom table gives the error of `AtomTable::put`.
pub(super) fn accept(
    scheduler: &mut Scheduler,
    listen: &Term,
    reference: &Term,
) -> Result<BifResult, BeamError> {
    let Some((key, socket)) = socket_argument(scheduler, listen) else {
        return Ok(BifResult::badarg());
    };
    let caller = scheduler.current_process().id;
    let mut state = lock(&socket);
    if state.read & (CLOSING | CLOSED) != 0 {
        return Ok(closed());
    }
    let current = match &state.acceptor {
        None => None,
        Some((acceptor, old)) if *acceptor == caller => Some(old.clone()),
        // Another process waits in the queue with a monitor and no select;
        // a second call of a waiting process raises `{invalid, state}`
        // (`essio_accept_accepting_other`, `unix_socket_syncio.c:2746-2758`).
        Some(_) => {
            if state
                .acceptors
                .iter()
                .any(|(waiting, _)| *waiting == caller)
            {
                drop(state);
                return invalid_state(scheduler);
            }
            state.acceptors.push_back((caller, reference.clone()));
            drop(state);
            monitor(scheduler, caller, key);
            return value(scheduler, "select");
        }
    };
    let host = state
        .socket
        .as_ref()
        .expect("an open socket has a host socket");
    let failure = match host.accept() {
        Ok((accepted, _)) => {
            // The next waiting acceptor becomes current
            // (`unix_socket_syncio.c:2596-2633`).
            if current.is_some()
                && let Some((next, next_reference, fd)) = state.activate_next_acceptor()
            {
                select(scheduler, key, fd, Direction::Read, next, next_reference);
            }
            let inherited = (
                state.domain,
                state.kind,
                state.protocol,
                state.registered,
                state.options,
            );
            drop(state);
            return accepted_socket(scheduler, accepted, inherited, caller);
        }
        Err(failure) => failure,
    };
    if !matches!(failure.raw_os_error(), Some(libc::EAGAIN)) {
        let mut waiting = Vec::new();
        if current.is_some() {
            // The current acceptor goes, and each waiting acceptor gets an
            // abort with the errno; `ACCEPTING` stays
            // (`unix_socket_syncio.c:2703-2733`).
            state.acceptor = None;
            waiting.extend(state.acceptors.drain(..));
        }
        drop(state);
        let reason = atom(scheduler, &errno_name(&failure))?;
        send_aborts(scheduler, key, waiting, &reason)?;
        return errno_error(scheduler, &failure);
    }
    let fd = fd(&state);
    if current.is_some() {
        if !scheduler.node().poll.cancel_select(fd, Direction::Read) {
            return value(scheduler, "select_sent");
        }
    } else {
        state.read |= ACCEPTING | SELECTED;
    }
    state.acceptor = Some((caller, reference.clone()));
    select(
        scheduler,
        key,
        fd,
        Direction::Read,
        caller,
        reference.clone(),
    );
    drop(state);
    if let Some(old) = current {
        let cancelled = atom(scheduler, "cancelled")?;
        let info = Term::tuple(vec![old, cancelled]);
        let abort = socket_message(scheduler, key, "abort", info)?;
        scheduler.send_message(caller, caller, abort);
    } else {
        monitor(scheduler, caller, key);
    }
    value(scheduler, "select")
}

/// Keep the socket `accepted` of a listen socket with the domain, type,
/// protocol, registry flag, and `otp` options `inherited`, owned by `owner`, as
/// `essio_accept_accepted` does (`unix_socket_syncio.c:2818-2878`), and
/// give `{ok, Socket}`.
fn accepted_socket(
    scheduler: &mut Scheduler,
    accepted: socket2::Socket,
    inherited: (i32, i32, i32, bool, OtpOptions),
    owner: ProcessId,
) -> Result<BifResult, BeamError> {
    let (domain, kind, protocol, registered, options) = inherited;
    accepted
        .set_nonblocking(true)
        .map_err(|failure| error(format!("prim_socket:nif_accept/2: {failure}")))?;
    let reference = scheduler.make_magic_ref()?;
    let sockets = scheduler.node().resources.kind::<Sockets>();
    let registry = {
        let mut sockets = lock(&sockets);
        sockets.counters.count(domain, kind, protocol, 1);
        sockets.insert(
            reference,
            SocketState {
                socket: Some(accepted),
                domain,
                kind,
                protocol,
                owner: Some(owner),
                closer: None,
                read: 0,
                write: CONNECTED,
                registered,
                // `alloc_descriptor` sets `meta.ref` to `undefined` (prim_socket_nif.c:17074).
                meta: Term::static_atom("undefined"),
                connector: None,
                acceptor: None,
                acceptors: std::collections::VecDeque::new(),
                reader: None,
                writer: None,
                counters: IoCounters::default(),
                options,
                select_read: false,
                recv_count_done: 0,
                sendfile: None,
                sendfile_counters: None,
            },
        );
        sockets.registry
    };
    monitor(scheduler, owner, reference);
    if let (true, Some(registry)) = (registered, registry) {
        // The load callback made the atoms (`prim_socket_nif.c:18842-18857`).
        send_registry(scheduler, registry, "add", reference).expect("made at load");
    }
    Ok(BifResult::Value(Term::tuple(vec![
        Term::static_atom("ok"),
        Term::Reference(reference),
    ])))
}

/// Run `prim_socket:nif_cancel(Socket, Op, Ref)`, as `nif_cancel` and
/// `esock_cancel` do (`prim_socket_nif.c:13895-14100`): `badarg` for a term
/// that is not a socket, an `Op` that is not an atom, or a `Ref` that is
/// not a reference. `connect` runs `essio_cancel_connect` and `accept` runs
/// `essio_cancel_accept` (`unix_socket_syncio.c:4967-5132`): `ok` for an
/// active select, `select_sent` for a select whose message went out, and
/// `{error, not_found}` or `not_found` for another request. The cancel of
/// the current acceptor makes the next waiting acceptor current, and the
/// cancel of a waiting acceptor removes it from the queue and gives `ok`. Another `Op`
/// gives `{error, closed}` for a closed socket and `badarg` for an open
/// one.
///
/// # Errors
///
/// An `Op` of a read or a write (`recv`, `send`, and the others of
/// `esock_cancel`) stops the run: PBeam has no reader or writer yet. A new
/// atom in a full atom table gives the error of `AtomTable::put`.
pub(super) fn cancel(
    scheduler: &mut Scheduler,
    socket: &Term,
    operation: &Term,
    reference: &Term,
) -> Result<BifResult, BeamError> {
    let Some((key, socket)) = socket_argument(scheduler, socket) else {
        return Ok(BifResult::badarg());
    };
    let (Term::Atom(operation), Term::Reference(_)) = (operation, reference) else {
        return Ok(BifResult::badarg());
    };
    let caller = scheduler.current_process().id;
    let name = scheduler.atoms.name_of(operation).to_owned();
    let poll = Arc::clone(&scheduler.node().poll);
    let mut state = lock(&socket);
    // The `connect` cancel gives `{error, not_found}`; the others give
    // `not_found` (`unix_socket_syncio.c:4967-5132`, `5144-5355`). The
    // reader and writer cancels keep the states.
    let (direction, open, requestor, not_found_error, bit) = match name.as_str() {
        "connect" => (
            Direction::Write,
            state.write,
            &mut state.connector,
            true,
            CONNECTING,
        ),
        "accept" => (
            Direction::Read,
            state.read,
            &mut state.acceptor,
            false,
            ACCEPTING,
        ),
        // Each send and receive operation cancels the current writer or
        // reader (prim_socket_nif.c:13955-14078).
        "send" | "sendto" | "sendmsg" | "sendmmsg" | "sendv" | "sendfile" => {
            (Direction::Write, state.write, &mut state.writer, false, 0)
        }
        "recv" | "recvfrom" | "recvmsg" | "recvmmsg" => {
            (Direction::Read, state.read, &mut state.reader, false, 0)
        }
        _ if state.read & (CLOSING | CLOSED) != 0 => return Ok(closed()),
        _ => return Ok(BifResult::badarg()),
    };
    if open & (CLOSING | CLOSED) != 0 {
        return Ok(closed());
    }
    // A waiting acceptor leaves the queue (`essio_cancel_accept_waiting`,
    // `unix_socket_syncio.c:5054-5066`, `5119-5132`).
    if bit == ACCEPTING
        && let Some((current, _)) = requestor
        && *current != caller
    {
        let position = state
            .acceptors
            .iter()
            .position(|(waiting, old)| *waiting == caller && old == reference);
        if let Some(position) = position {
            state.acceptors.remove(position);
        }
        drop(state);
        return value(
            scheduler,
            if position.is_some() {
                "ok"
            } else {
                "not_found"
            },
        );
    }
    let found = requestor
        .as_ref()
        .is_some_and(|(pid, old)| *pid == caller && old == reference);
    if !found {
        drop(state);
        return if not_found_error {
            error_atom(scheduler, "not_found")
        } else {
            value(scheduler, "not_found")
        };
    }
    *requestor = None;
    if direction == Direction::Write {
        state.write &= !bit;
    } else {
        state.read &= !bit;
    }
    let active = poll.cancel_select(fd(&state), direction);
    // The next waiting acceptor becomes current
    // (`essio_cancel_accept_current`, `unix_socket_syncio.c:5082-5113`).
    let activated = if bit == ACCEPTING {
        state.activate_next_acceptor()
    } else {
        None
    };
    drop(state);
    if let Some((next, next_reference, fd)) = activated {
        select(scheduler, key, fd, Direction::Read, next, next_reference);
    }
    value(scheduler, if active { "ok" } else { "select_sent" })
}

#[cfg(test)]
#[path = "../../../../../tests/runtime/interpreter/native/prim_socket/select.rs"]
mod tests;

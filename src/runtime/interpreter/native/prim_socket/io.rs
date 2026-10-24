//! The bindings of `prim_socket:nif_send/4` and `nif_recv/4`
//! (`prim_socket_nif.c:6440-6509`, `7056-7124`), with the Unix I/O backend
//! `unix_essio` (`unix_socket_syncio.c:2996-3036`, `3796-3878`,
//! `6809-7196`, `9869-10520`).
//!
//! A send or a receive that would block makes a one-shot select on the
//! node poll thread, as `esock_select_write` and `esock_select_read` do, and
//! the caller becomes the current writer or reader. PBeam has no queue of
//! waiting writers or readers: a call of another process while a current
//! writer or reader waits stops the run (see the row "NIF stubs").

use std::io::{self, Read};
use std::sync::MutexGuard;

use super::select::{fd, monitor, select};
use super::socket::{closed, errno_error, invalid, socket_argument};
use super::supports::atom;
use super::*;
use crate::runtime::resources::lock;
use crate::runtime::scheduler::poll::Direction;
use crate::runtime::sockets::{CLOSED, CLOSING, SELECTED, SocketState};
use crate::term::{ProcessId, Reference};

/// The default buffer size of a receive with length 0,
/// `ESOCK_RECV_BUFFER_SIZE_DEFAULT` (`prim_socket_nif.c:505-510`). The
/// option `{otp, rcvbuf}` that changes it is not supported yet, and the
/// buffer count `rNum` keeps its default 0 (`prim_socket_nif.c:17061-17064`).
const RECV_BUFFER_SIZE: usize = 8 * 1024;

/// Return `{error, {invalid, state}}` of `esock_make_error_invalid`.
fn invalid_state(scheduler: &mut Scheduler) -> Result<BifResult, BeamError> {
    let state = atom(scheduler, "state")?;
    Ok(BifResult::Value(Term::tuple(vec![
        Term::static_atom("error"),
        Term::tuple(vec![Term::static_atom("invalid"), state]),
    ])))
}

/// Return `{invalid, {integer_range, Value}}` of
/// `esock_make_error_integer_range` (`socket_util.c:3320-3339`).
fn integer_range(scheduler: &mut Scheduler, value: &Term) -> Result<BifResult, BeamError> {
    let range = atom(scheduler, "integer_range")?;
    Ok(invalid(Term::tuple(vec![range, value.clone()])))
}

/// Return the `int` of `term` as `enif_get_int` gives it, `Err(true)` for
/// another integer, and `Err(false)` for a term that is not an integer.
fn int(term: &Term) -> Result<i32, bool> {
    match term {
        Term::Integer(value) => i32::try_from(*value).map_err(|_| true),
        Term::BigInt(_) => Err(true),
        _ => Err(false),
    }
}

/// Make `caller` the current requestor `requestor` with `reference`, and
/// monitor it when it is new, as `recv_init_current_reader` and
/// `send_check_retry` do (`unix_socket_syncio.c:7131-7149`, `10419-10450`).
fn make_current(
    scheduler: &mut Scheduler,
    requestor: &mut Option<(ProcessId, Term)>,
    caller: ProcessId,
    reference: &Term,
    key: Reference,
) {
    let new = requestor.is_none();
    *requestor = Some((caller, reference.clone()));
    if new {
        monitor(scheduler, caller, key);
    }
}

/// Run `prim_socket:nif_send(Socket, Data, Flags, Ref)`, as `nif_send`
/// (`prim_socket_nif.c:6440-6509`), `essio_send`
/// (`unix_socket_syncio.c:2996-3036`), and `send_check_result`
/// (`unix_socket_syncio.c:6868-7196`) do: `badarg` for a term that is not
/// a socket, a `Ref` that is not a reference, or `Data` that is not a
/// binary; `{invalid, {integer_range, Flags}}` for an integer outside the
/// `int` range; `{error, closed}`; `{error, {invalid, state}}` during a
/// connect; `ok` when the host takes each byte; `{select, Written}` or
/// `select` after a partial write or `EAGAIN`, with the write select; and
/// `{error, Errno}`.
///
/// # Errors
///
/// A call of another process while a writer waits stops the run: PBeam
/// has no writer queue. A new atom in a full atom table gives the error of
/// `AtomTable::put`.
pub(super) fn send(
    scheduler: &mut Scheduler,
    socket: &Term,
    data: &Term,
    flags: &Term,
    reference: &Term,
) -> Result<BifResult, BeamError> {
    let Some((key, socket)) = socket_argument(scheduler, socket) else {
        return Ok(BifResult::badarg());
    };
    let (Term::Reference(_), Some(bytes)) = (reference, data.binary_bytes()) else {
        return Ok(BifResult::badarg());
    };
    let flags = match int(flags) {
        Ok(flags) => flags,
        Err(true) => return integer_range(scheduler, flags),
        Err(false) => return Ok(BifResult::badarg()),
    };
    let caller = scheduler.current_process().id;
    let mut state = lock(&socket);
    if state.write & (CLOSING | CLOSED) != 0 {
        return Ok(closed());
    }
    if state.connector.is_some() {
        drop(state);
        return invalid_state(scheduler);
    }
    if state.writer.as_ref().is_some_and(|(pid, _)| *pid != caller) {
        return Err(error(
            "prim_socket:nif_send/4: a second writer process is not supported".to_owned(),
        ));
    }
    state.counters.write_tries += 1;
    let host = state
        .socket
        .as_ref()
        .expect("an open socket has a host socket");
    match host.send_with_flags(bytes, flags) {
        Ok(written) if written < bytes.len() => Ok(send_retry(
            scheduler,
            state,
            Some(written),
            key,
            caller,
            reference,
        )),
        Ok(written) => {
            let counters = &mut state.counters;
            counters.write_pkg += 1;
            counters.write_byte += written as u64;
            counters.write_pkg_max_count += written as u64;
            counters.write_pkg_max = counters.write_pkg_max.max(counters.write_pkg_max_count);
            counters.write_pkg_max_count = 0;
            state.writer = None;
            Ok(BifResult::Value(Term::static_atom("ok")))
        }
        Err(failure) if matches!(failure.raw_os_error(), Some(libc::EAGAIN | libc::EINTR)) => {
            Ok(send_retry(scheduler, state, None, key, caller, reference))
        }
        Err(failure) => {
            state.counters.write_fails += 1;
            if failure.raw_os_error() != Some(libc::EINVAL) {
                state.writer = None;
            }
            drop(state);
            errno_error(scheduler, &failure)
        }
    }
}

/// Make the write select after a partial write of `written` bytes or no
/// write, as `send_check_retry` does for a stream socket
/// (`unix_socket_syncio.c:7077-7196`), and return `{select, Written}` or
/// `select`.
fn send_retry(
    scheduler: &mut Scheduler,
    mut state: MutexGuard<'_, SocketState>,
    written: Option<usize>,
    key: Reference,
    caller: ProcessId,
    reference: &Term,
) -> BifResult {
    if let Some(written) = written {
        state.counters.write_pkg_max_count += written as u64;
    }
    let mut writer = state.writer.take();
    make_current(scheduler, &mut writer, caller, reference, key);
    state.writer = writer;
    select(
        scheduler,
        key,
        fd(&state),
        Direction::Write,
        caller,
        reference.clone(),
    );
    state.counters.write_waits += 1;
    state.write |= SELECTED;
    BifResult::Value(match written {
        Some(written) => Term::tuple(vec![
            Term::static_atom("select"),
            Term::Integer(written as i64),
        ]),
        None => Term::static_atom("select"),
    })
}

/// Run `prim_socket:nif_recv(Socket, Length, Flags, Ref)`, as `nif_recv`
/// (`prim_socket_nif.c:7056-7124`) and `essio_recv`
/// (`unix_socket_syncio.c:3796-3878`, `9869-10410`) do: `badarg` for a term
/// that is not a socket, a `Ref` that is neither a reference nor `zero`,
/// or a `Length` or `Flags` that is not an integer;
/// `{invalid, {integer_range, Value}}` for a `Length` outside the `uint64`
/// range or `Flags` outside the `int` range; `{error, closed}`;
/// `{error, {invalid, state}}` with a current acceptor; `{ok, Data}`;
/// `{error, closed}` at the end of a stream; `select` or `{select, Data}`
/// with the read select, or `timeout` or `{timeout, Data}` for the `Ref`
/// `zero`, when fewer bytes than `Length` came; and `{error, Errno}`.
///
/// # Errors
///
/// A `Length` above the `ssize_t` range stops the run: ERTS then gives a
/// term made of the integer itself (`prim_socket_nif.c:7092-7094`). A call
/// of another process while a reader waits stops the run: PBeam has no
/// reader queue. `Flags` other than 0 stop the run: PBeam reads with
/// `read(2)`. A new atom in a full atom table gives the error of
/// `AtomTable::put`.
pub(super) fn recv(
    scheduler: &mut Scheduler,
    socket: &Term,
    length: &Term,
    flags: &Term,
    reference: &Term,
) -> Result<BifResult, BeamError> {
    let Some((key, socket)) = socket_argument(scheduler, socket) else {
        return Ok(BifResult::badarg());
    };
    let zero = match reference {
        Term::Reference(_) => false,
        Term::Atom(name) if scheduler.atoms.name_of(name) == "zero" => true,
        _ => return Ok(BifResult::badarg()),
    };
    let size = match length {
        Term::Integer(value) => Some(u64::try_from(*value).ok()),
        Term::BigInt(value) => Some(u64::try_from(value.as_ref()).ok()),
        _ => None,
    };
    let (size, flags) = match (size, int(flags)) {
        (None, _) | (_, Err(false)) => return Ok(BifResult::badarg()),
        (Some(None), _) => return integer_range(scheduler, length),
        (_, Err(true)) => return integer_range(scheduler, flags),
        (Some(Some(size)), Ok(flags)) => (size, flags),
    };
    if i64::try_from(size).is_err() {
        return Err(error(format!(
            "prim_socket:nif_recv/4: the length {size} above the ssize_t range is not supported"
        )));
    }
    let caller = scheduler.current_process().id;
    let mut state = lock(&socket);
    if state.read & (CLOSING | CLOSED) != 0 {
        return Ok(closed());
    }
    if state.acceptor.is_some() {
        drop(state);
        return invalid_state(scheduler);
    }
    if state.reader.as_ref().is_some_and(|(pid, _)| *pid != caller) {
        return Err(error(
            "prim_socket:nif_recv/4: a second reader process is not supported".to_owned(),
        ));
    }
    if flags != 0 {
        return Err(error(format!(
            "prim_socket:nif_recv/4: the flags {flags} are not supported"
        )));
    }
    let buffer_size = if size == 0 {
        RECV_BUFFER_SIZE
    } else {
        usize::try_from(size).expect("a length in the ssize_t range")
    };
    let mut buffer = vec![0; buffer_size];
    state.counters.read_tries += 1;
    let stream = state.kind == libc::SOCK_STREAM;
    let mut host = state
        .socket
        .as_ref()
        .expect("an open socket has a host socket");
    let read = match host.read(&mut buffer) {
        Ok(0) if stream => {
            state.counters.read_fails += 1;
            state.reader = None;
            return Ok(closed());
        }
        Ok(read) => read,
        Err(failure) => return recv_fail(scheduler, state, &failure, zero, key, caller, reference),
    };
    buffer.truncate(read);
    let length = read as u64;
    let counters = &mut state.counters;
    counters.read_byte += length;
    counters.read_pkg_max_count += length;
    if read == buffer_size && size == 0 && stream {
        // `recv_check_full_maybe_done` counts the bytes once, and
        // `recv_check_full_done` counts them again
        // (`unix_socket_syncio.c:10050-10137`).
        counters.read_byte += length;
        counters.read_pkg_max_count += length;
    }
    counters.read_pkg_max = counters.read_pkg_max.max(counters.read_pkg_max_count);
    counters.read_pkg_max_count = 0;
    let data = Term::binary(buffer);
    if read < buffer_size && stream && size > 0 {
        // `recv_check_partial` (`unix_socket_syncio.c:10343-10382`).
        if !zero {
            let data = Term::tuple(vec![Term::static_atom("select"), data]);
            return Ok(recv_select(scheduler, state, key, caller, reference, data));
        }
        state.counters.read_pkg += 1;
        state.reader = None;
        return Ok(BifResult::Value(Term::tuple(vec![
            Term::static_atom("timeout"),
            data,
        ])));
    }
    state.counters.read_pkg += 1;
    state.reader = None;
    Ok(BifResult::Value(Term::tuple(vec![
        Term::static_atom("ok"),
        data,
    ])))
}

/// Handle the failed read `failure`, as `recv_check_fail`
/// (`unix_socket_syncio.c:10145-10266`) does: `EAGAIN` gives `timeout` for
/// the `Ref` `zero` and makes the read select for another `Ref`; another
/// errno gives `{error, Errno}` and releases the current reader.
fn recv_fail(
    scheduler: &mut Scheduler,
    mut state: MutexGuard<'_, SocketState>,
    failure: &io::Error,
    zero: bool,
    key: Reference,
    caller: ProcessId,
    reference: &Term,
) -> Result<BifResult, BeamError> {
    if failure.raw_os_error() != Some(libc::EAGAIN) {
        state.counters.read_fails += 1;
        state.reader = None;
        drop(state);
        return errno_error(scheduler, failure);
    }
    if zero {
        state.reader = None;
        return Ok(BifResult::Value(Term::static_atom("timeout")));
    }
    let select = Term::static_atom("select");
    Ok(recv_select(
        scheduler, state, key, caller, reference, select,
    ))
}

/// Make the read select and the current reader, and return `result`, as
/// `recv_check_select` does (`unix_socket_syncio.c:10275-10310`).
fn recv_select(
    scheduler: &mut Scheduler,
    mut state: MutexGuard<'_, SocketState>,
    key: Reference,
    caller: ProcessId,
    reference: &Term,
    result: Term,
) -> BifResult {
    select(
        scheduler,
        key,
        fd(&state),
        Direction::Read,
        caller,
        reference.clone(),
    );
    let mut reader = state.reader.take();
    make_current(scheduler, &mut reader, caller, reference, key);
    state.reader = reader;
    state.read |= SELECTED;
    BifResult::Value(result)
}

#[cfg(test)]
#[path = "../../../../../tests/runtime/interpreter/native/prim_socket/io.rs"]
mod tests;

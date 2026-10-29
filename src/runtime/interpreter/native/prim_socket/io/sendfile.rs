//! The bindings of `prim_socket:nif_sendfile/1,4,5` (`prim_socket_nif.c:6914-7034`),
//! with the Unix I/O backend `essio_sendfile_start`, `essio_sendfile_cont`,
//! and `essio_sendfile_deferred_close` (`unix_socket_syncio.c:3531-3785`,
//! `9156-9497`).
//!
//! A sendfile gets a copy of the host file of a `prim_file` resource, as
//! the dynamic resource call `prim_file_nif_dyncall_dup` does
//! (`prim_file_nif.c:426-460`), and keeps it until the file is sent, the
//! send fails, or `nif_sendfile/1` closes it.

use std::fs::File;
use std::sync::MutexGuard;

use socket2::Socket;

use super::super::socket::count;
use super::*;
use crate::runtime::files::{FileData, Files};
use crate::runtime::port::inet::errno_name;
use crate::runtime::sockets::SendfileCounters;

/// The most bytes of one `sendfile(2)` call, `0x20000000`
/// (`unix_socket_syncio.c:9172`).
const CHUNK: u64 = 0x2000_0000;

/// The end of [`send_loop`], as the result of `essio_sendfile`
/// (`unix_socket_syncio.c:9156-9345`), with the bytes that the call sent.
#[derive(Debug, PartialEq, Eq)]
enum Sent {
    /// Each byte of the count, or the bytes up to the end of the file, is
    /// sent (0).
    Done(u64),
    /// The host gave `EAGAIN` (1).
    Select(u64),
    /// The host gave the errno (-1).
    Failed(i32),
}

/// Run the loop of `essio_sendfile` (`unix_socket_syncio.c:9171-9344`):
/// call `send` with the offset and a chunk of at most [`CHUNK`] bytes
/// until `count` bytes are sent or, for a `count` of 0, the end of the
/// file, and add each call to `counters`. `send` gives the bytes sent and
/// the errno of the call. `EINTR` calls `send` again.
fn send_loop(
    counters: &mut SendfileCounters,
    mut offset: i64,
    mut count: u64,
    mut send: impl FnMut(i64, usize) -> (u64, Option<i32>),
) -> Sent {
    let mut package = 0;
    loop {
        let chunk = if count > 0 && count < CHUNK {
            count
        } else {
            CHUNK
        };
        let (sent, errno) = send(offset, chunk as usize);
        offset += sent as i64;
        counters.calls += 1;
        if sent != 0 {
            package += sent;
            counters.pkg += 1;
            counters.byte += sent;
            counters.pkg_max = counters.pkg_max.max(package);
            counters.max_count += sent;
            counters.max = counters.max.max(counters.max_count);
        }
        if count > 0 {
            count -= sent;
            if count == 0 {
                return Sent::Done(package);
            }
        }
        match errno {
            Some(libc::EAGAIN) => return Sent::Select(package),
            Some(libc::EINTR) => {}
            Some(errno) => return Sent::Failed(errno),
            None if sent == 0 => return Sent::Done(package),
            None => {}
        }
    }
}

/// Run one `sendfile(2)` of at most `count` bytes of `file` at `offset` to
/// `socket`, as the Linux branch of `essio_sendfile` does
/// (`unix_socket_syncio.c:9187-9210`): the bytes sent are the change of
/// the offset.
#[cfg(target_os = "linux")]
fn host_sendfile(socket: &Socket, file: &File, offset: i64, count: usize) -> (u64, Option<i32>) {
    let mut moved = offset;
    let result = nix::sys::sendfile::sendfile(socket, file, Some(&mut moved), count);
    (
        (moved - offset) as u64,
        result.err().map(|errno| errno as i32),
    )
}

/// Run one `sendfile(2)` of at most `count` bytes of `file` at `offset` to
/// `socket`, as the macOS branch of `essio_sendfile` does
/// (`unix_socket_syncio.c:9211-9251`): an errno other than `EAGAIN` and
/// `EINTR` sends no byte.
#[cfg(target_vendor = "apple")]
fn host_sendfile(socket: &Socket, file: &File, offset: i64, count: usize) -> (u64, Option<i32>) {
    let (result, sent) =
        nix::sys::sendfile::sendfile(file, socket, offset, Some(count as i64), None, None);
    match result.err().map(|errno| errno as i32) {
        Some(errno) if errno != libc::EAGAIN && errno != libc::EINTR => (0, Some(errno)),
        errno => (sent as u64, errno),
    }
}

/// Return the `int64` of `term`, as `enif_get_int64` gives it.
fn int64(term: &Term) -> Option<i64> {
    match term {
        Term::Integer(value) => Some(*value),
        _ => None,
    }
}

/// Return the `uint64` of `term`, as `enif_get_uint64` gives it.
fn uint64(term: &Term) -> Option<u64> {
    match term {
        Term::Integer(value) => u64::try_from(*value).ok(),
        Term::BigInt(value) => num_traits::ToPrimitive::to_u64(&**value),
        _ => None,
    }
}

/// Run `prim_socket:nif_sendfile(Socket, Ref, Offset, Count, FileRef)` to
/// start a sendfile, or `nif_sendfile(Socket, Ref, Offset, Count)` to
/// continue it, as `nif_sendfile` does (`prim_socket_nif.c:6948-7022`):
/// `badarg` for a term that is not a socket, a `Ref` that is not a
/// reference, an `Offset` or a `Count` that is not an integer, or a
/// `FileRef` that is not a reference; `{invalid, {integer_range, V}}` for
/// an `Offset` outside the `int64` range or a `Count` outside the `uint64`
/// range; then [`start`] or [`cont`].
///
/// # Errors
///
/// The errors of [`start`].
pub(in crate::runtime::interpreter::native::prim_socket) fn sendfile(
    scheduler: &mut Scheduler,
    socket: &Term,
    reference: &Term,
    offset: &Term,
    count: &Term,
    file: Option<&Term>,
) -> Result<BifResult, BeamError> {
    let Some((key, socket)) = socket_argument(scheduler, socket) else {
        return Ok(BifResult::badarg());
    };
    if !matches!(reference, Term::Reference(_)) {
        return Ok(BifResult::badarg());
    }
    let (start_offset, size) = match (int64(offset), uint64(count)) {
        (Some(start_offset), Some(size)) => (start_offset, size),
        (start_offset, _) => {
            let integer = |term: &Term| matches!(term, Term::Integer(_) | Term::BigInt(_));
            if !integer(offset) || !integer(count) {
                return Ok(BifResult::badarg());
            }
            return integer_range(
                scheduler,
                if start_offset.is_none() {
                    offset
                } else {
                    count
                },
            );
        }
    };
    let caller = scheduler.current_process().id;
    let request = (key, caller, reference);
    match file {
        Some(Term::Reference(file)) => {
            let data = lock(&scheduler.node().resources.kind::<Files>()).get(*file);
            let state = lock(&socket);
            start(scheduler, state, data, start_offset, size, request)
        }
        Some(_) => Ok(BifResult::badarg()),
        None => cont(scheduler, lock(&socket), start_offset, size, request),
    }
}

/// Start a sendfile of the file resource `data` (`None` for a reference
/// that is not a file resource), as `essio_sendfile_start` does
/// (`unix_socket_syncio.c:3531-3687`): `{error, closed}`;
/// `{error, {invalid, state}}` during a connect or a sendfile;
/// `{error, {invalid, efile}}` without a file resource; `{error, Errno}`
/// for a file that is not open or a failed send; then the results of the
/// send.
///
/// # Errors
///
/// A call of another process while a writer waits stops the run: PBeam
/// has no writer queue. A new atom in a full atom table gives the error of
/// `AtomTable::put`.
fn start(
    scheduler: &mut Scheduler,
    mut state: MutexGuard<'_, SocketState>,
    data: Option<Arc<FileData>>,
    offset: i64,
    count: u64,
    request: (Reference, ProcessId, &Term),
) -> Result<BifResult, BeamError> {
    let (_, caller, reference) = request;
    if let Some(result) = write_entry(scheduler, &state, "nif_sendfile/5", caller)? {
        return Ok(result);
    }
    if state.sendfile.is_some() {
        return invalid_state(scheduler);
    }
    let Some(data) = data else {
        let efile = Term::tuple(vec![
            Term::static_atom("invalid"),
            atom(scheduler, "efile")?,
        ]);
        return Ok(failure(state, efile));
    };
    // `efile_dup_handle` copies the fd of an idle resource, and a resource
    // in another state gives `EINVAL` (`prim_file_nif.c:426-460`).
    let copy = match data.acquire() {
        Some(file) => {
            let copy = file.try_clone();
            data.release(file);
            copy
        }
        None => Err(io::Error::from_raw_os_error(libc::EINVAL)),
    };
    let copy = match copy {
        Ok(copy) => copy,
        Err(error) => return errno_failure(scheduler, state, &error),
    };
    state.sendfile = Some(copy);
    let counters = state.sendfile_counters.get_or_insert_default();
    counters.tries += 1;
    counters.max_count = 0;
    let sent = send(&mut state, offset, count);
    if matches!(sent, Sent::Select(_)) && state.writer.is_none() {
        let mut writer = None;
        make_current(scheduler, &mut writer, caller, reference, request.0);
        state.writer = writer;
    }
    sent_result(scheduler, state, sent, request)
}

/// Continue the sendfile of the caller, as `essio_sendfile_cont` does
/// (`unix_socket_syncio.c:3695-3761`): `{error, closed}`;
/// `{error, {invalid, state}}` during a connect; the exception
/// `error:{invalid, state}` when the caller has no sendfile in progress;
/// then the results of the send.
///
/// # Errors
///
/// A new atom in a full atom table gives the error of `AtomTable::put`.
fn cont(
    scheduler: &mut Scheduler,
    mut state: MutexGuard<'_, SocketState>,
    offset: i64,
    count: u64,
    request: (Reference, ProcessId, &Term),
) -> Result<BifResult, BeamError> {
    let (_, caller, reference) = request;
    if state.write & (CLOSING | CLOSED) != 0 {
        return Ok(closed());
    }
    if state.connector.is_some() {
        return invalid_state(scheduler);
    }
    let writer = state.writer.as_ref().map(|(pid, _)| *pid);
    if writer != Some(caller) || state.sendfile.is_none() {
        let reason = Term::tuple(vec![
            Term::static_atom("invalid"),
            atom(scheduler, "state")?,
        ]);
        return Ok(BifResult::Raise {
            class: ExceptionClass::Error,
            reason,
        });
    }
    let sent = send(&mut state, offset, count);
    if matches!(sent, Sent::Select(_)) {
        state.writer = Some((caller, reference.clone()));
    }
    sent_result(scheduler, state, sent, request)
}

/// Send the file of the sendfile in progress with [`send_loop`].
fn send(state: &mut SocketState, offset: i64, count: u64) -> Sent {
    let socket = state
        .socket
        .as_ref()
        .expect("an open socket has a host socket");
    let file = state.sendfile.as_ref().expect("a sendfile has a file");
    let counters = state.sendfile_counters.get_or_insert_default();
    send_loop(counters, offset, count, |offset, chunk| {
        host_sendfile(socket, file, offset, chunk)
    })
}

/// Return the result of `sent`, as `essio_sendfile_ok`,
/// `essio_sendfile_select`, and `essio_sendfile_errno` give it
/// (`unix_socket_syncio.c:9348-9497`): `{ok, Sent}` closes the file and
/// ends the writer; `{select, Sent}` makes the write select; and
/// `{error, Errno}` closes the file.
///
/// # Errors
///
/// A new atom in a full atom table gives the error of `AtomTable::put`.
fn sent_result(
    scheduler: &mut Scheduler,
    mut state: MutexGuard<'_, SocketState>,
    sent: Sent,
    (key, caller, reference): (Reference, ProcessId, &Term),
) -> Result<BifResult, BeamError> {
    match sent {
        Sent::Done(sent) => {
            state.writer = None;
            state.counters.write_pkg_max_count = 0;
            state.sendfile = None;
            Ok(BifResult::Value(Term::tuple(vec![
                Term::static_atom("ok"),
                count(sent),
            ])))
        }
        Sent::Select(sent) => {
            select(
                scheduler,
                key,
                fd(&state),
                Direction::Write,
                caller,
                reference.clone(),
            );
            state.sendfile_counters.get_or_insert_default().waits += 1;
            state.write |= SELECTED;
            Ok(BifResult::Value(Term::tuple(vec![
                Term::static_atom("select"),
                count(sent),
            ])))
        }
        Sent::Failed(errno) => {
            state.sendfile = None;
            errno_failure(scheduler, state, &io::Error::from_raw_os_error(errno))
        }
    }
}

/// Return `{error, Errno}` of `essio_sendfile_errno`
/// (`unix_socket_syncio.c:9348-9357`), with the changes of [`failure`].
///
/// # Errors
///
/// A new atom in a full atom table gives the error of `AtomTable::put`.
fn errno_failure(
    scheduler: &mut Scheduler,
    state: MutexGuard<'_, SocketState>,
    error: &io::Error,
) -> Result<BifResult, BeamError> {
    let reason = atom(scheduler, &errno_name(error))?;
    Ok(failure(state, reason))
}

/// Return `{error, Reason}` of `essio_sendfile_error`
/// (`unix_socket_syncio.c:9360-9400`): the counter `sendfile_fails` grows
/// and the writer ends.
fn failure(mut state: MutexGuard<'_, SocketState>, reason: Term) -> BifResult {
    state.sendfile_counters.get_or_insert_default().fails += 1;
    state.writer = None;
    BifResult::Value(Term::tuple(vec![Term::static_atom("error"), reason]))
}

/// Run `prim_socket:nif_sendfile(Socket)`, as `essio_sendfile_deferred_close`
/// does (`unix_socket_syncio.c:3769-3785`): `badarg` for a term that is
/// not a socket, `{error, {invalid, state}}` without a sendfile file, and
/// `ok` after the file closes.
///
/// # Errors
///
/// A new atom in a full atom table gives the error of `AtomTable::put`.
pub(in crate::runtime::interpreter::native::prim_socket) fn deferred_close(
    scheduler: &mut Scheduler,
    socket: &Term,
) -> Result<BifResult, BeamError> {
    let Some((_, socket)) = socket_argument(scheduler, socket) else {
        return Ok(BifResult::badarg());
    };
    if lock(&socket).sendfile.take().is_none() {
        return invalid_state(scheduler);
    }
    Ok(BifResult::Value(Term::static_atom("ok")))
}

#[cfg(test)]
#[path = "../../../../../../tests/runtime/interpreter/native/prim_socket/io/sendfile.rs"]
mod tests;

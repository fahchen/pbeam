//! The bindings of `prim_socket:nif_send/4`, `nif_sendto/5`, `nif_sendv/3`,
//! `nif_recv/4`, and `nif_recvfrom/4` (`prim_socket_nif.c:6440-6600`,
//! `6777-6822`, `7056-7231`), with the Unix I/O backend `unix_essio`
//! (`unix_socket_syncio.c:2996-3091`, `3361-3524`, `3796-3977`, `6809-7196`,
//! `9869-10520`).
//!
//! A send or a receive that would block makes a one-shot select on the
//! node poll thread, as `esock_select_write` and `esock_select_read` do, and
//! the caller becomes the current writer or reader. PBeam has no queue of
//! waiting writers or readers: a call of another process while a current
//! writer or reader waits stops the run (see the row "NIF stubs").

use std::io::{self, IoSlice, IoSliceMut};
use std::os::fd::AsRawFd;
use std::sync::{Arc, Mutex, MutexGuard};

use nix::sys::socket::{MsgFlags, SockaddrStorage};
use socket2::SockAddr;

use super::select::{fd, monitor, select};
use super::socket::{
    closed, decode_cmsgs, encode_cmsgs, errno_error, field, invalid, sockaddr, socket_argument,
    source_term,
};
use super::supports::{HOST, MSG_FLAGS, atom};
use super::*;
use crate::runtime::resources::lock;
use crate::runtime::scheduler::poll::Direction;
use crate::runtime::sockets::{CLOSED, CLOSING, SELECTED, SocketState};
use crate::term::{ProcessId, Reference};

pub(super) mod sendfile;

/// The most elements of the I/O vector of one `nif_sendv/3` call,
/// `iov_max`, which is `sysconf(_SC_IOV_MAX)` (`prim_socket_nif.c:18981-18990`):
/// 1024 on Linux and macOS.
const IOV_MAX: usize = 1024;

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
    write(
        scheduler,
        "nif_send/4",
        socket,
        data,
        None,
        flags,
        reference,
    )
}

/// Run `prim_socket:nif_sendto(Socket, Data, Dest, Flags, Ref)`, as
/// `nif_sendto` (`prim_socket_nif.c:6527-6600`) and `essio_sendto`
/// (`unix_socket_syncio.c:3042-3091`) do: the results of [`send`], and
/// `{invalid, sockaddr}` for a `Dest` that does not decode, which comes
/// after the check of `Flags` and before the check of the socket state.
///
/// # Errors
///
/// The errors of [`send`], and a native `sockaddr` stops the run (see
/// `socket::sockaddr`).
pub(super) fn sendto(
    scheduler: &mut Scheduler,
    socket: &Term,
    data: &Term,
    destination: &Term,
    flags: &Term,
    reference: &Term,
) -> Result<BifResult, BeamError> {
    let destination = Some(destination);
    write(
        scheduler,
        "nif_sendto/5",
        socket,
        data,
        destination,
        flags,
        reference,
    )
}

/// Run the send of `nif_send/4` or, with a `destination`, of
/// `nif_sendto/5`, which calls `sendto(2)` (`unix_socket_syncio.c:3079-3087`).
fn write(
    scheduler: &mut Scheduler,
    name: &str,
    socket: &Term,
    data: &Term,
    destination: Option<&Term>,
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
    let destination = match destination {
        Some(address) => match sockaddr(scheduler, address)? {
            Some(address) => Some(address),
            None => return Ok(invalid(atom(scheduler, "sockaddr")?)),
        },
        None => None,
    };
    let caller = scheduler.current_process().id;
    let mut state = lock(&socket);
    if let Some(result) = write_entry(scheduler, &state, name, caller)? {
        return Ok(result);
    }
    state.counters.write_tries += 1;
    let host = state
        .socket
        .as_ref()
        .expect("an open socket has a host socket");
    let sent = match &destination {
        Some(address) => host.send_to_with_flags(bytes, address, flags),
        None => host.send_with_flags(bytes, flags),
    };
    let request = (key, caller, reference);
    send_result(scheduler, state, sent, bytes.len(), false, request)
}

/// Check the socket state before a write, as `essio_send` does
/// (`unix_socket_syncio.c:3005-3021`): `{error, closed}`, and
/// `{error, {invalid, state}}` during a connect.
///
/// # Errors
///
/// A call of another process while a writer waits stops the run: PBeam
/// has no writer queue.
fn write_entry(
    scheduler: &mut Scheduler,
    state: &SocketState,
    name: &str,
    caller: ProcessId,
) -> Result<Option<BifResult>, BeamError> {
    if let Some(result) = write_open(scheduler, state)? {
        return Ok(Some(result));
    }
    if state.writer.as_ref().is_some_and(|(pid, _)| *pid != caller) {
        return Err(error(format!(
            "prim_socket:{name}: a second writer process is not supported"
        )));
    }
    Ok(None)
}

/// Check that a write can start, as `essio_send` and
/// `essio_sendfile_cont` do (`unix_socket_syncio.c:3005-3013`,
/// `3713-3720`): `{error, closed}`, and `{error, {invalid, state}}` during
/// a connect.
///
/// # Errors
///
/// A new atom in a full atom table gives the error of `AtomTable::put`.
fn write_open(
    scheduler: &mut Scheduler,
    state: &SocketState,
) -> Result<Option<BifResult>, BeamError> {
    if state.write & (CLOSING | CLOSED) != 0 {
        return Ok(Some(closed()));
    }
    if state.connector.is_some() {
        return invalid_state(scheduler).map(Some);
    }
    Ok(None)
}

/// Run `prim_socket:nif_sendv(Socket, IOV, Ref)`, as `nif_sendv`
/// (`prim_socket_nif.c:6777-6822`) and `essio_sendv`
/// (`unix_socket_syncio.c:3361-3524`) do: `badarg` for a term that is not a
/// socket or a `Ref` that is not a reference; `{error, closed}`;
/// `{error, {invalid, state}}` during a connect; `{invalid, iov}` for an
/// `IOV` that `enif_inspect_iovec` does not take (`erl_nif.c:4092-4152`),
/// or for more than `iov_max` elements on a socket that is not a stream
/// socket; then one `writev(2)` of the first `iov_max` elements
/// (`unix_socket_syncio.c:254`, `3509`) with the results of [`send`], and
/// `{iov, Written}` without a select when the host takes each byte and more
/// elements follow (`unix_socket_syncio.c:6920-6931`, `7151-7155`).
///
/// # Errors
///
/// The errors of [`send`].
pub(super) fn sendv(
    scheduler: &mut Scheduler,
    socket: &Term,
    iov: &Term,
    reference: &Term,
) -> Result<BifResult, BeamError> {
    let Some((key, socket)) = socket_argument(scheduler, socket) else {
        return Ok(BifResult::badarg());
    };
    let Term::Reference(_) = reference else {
        return Ok(BifResult::badarg());
    };
    let caller = scheduler.current_process().id;
    let mut state = lock(&socket);
    if let Some(result) = write_entry(scheduler, &state, "nif_sendv/3", caller)? {
        return Ok(result);
    }
    let Some((buffers, data_in_tail)) = iovec(iov) else {
        drop(state);
        return Ok(invalid(atom(scheduler, "iov")?));
    };
    if data_in_tail && state.kind != libc::SOCK_STREAM {
        drop(state);
        return Ok(invalid(atom(scheduler, "iov")?));
    }
    let size = buffers.iter().map(|buffer| buffer.len()).sum();
    let slices: Vec<IoSlice<'_>> = buffers.iter().map(|buffer| IoSlice::new(buffer)).collect();
    state.counters.write_tries += 1;
    let host = state
        .socket
        .as_ref()
        .expect("an open socket has a host socket");
    let sent = nix::sys::uio::writev(host, &slices).map_err(io::Error::from);
    let request = (key, caller, reference);
    send_result(scheduler, state, sent, size, data_in_tail, request)
}

/// Return the bytes of the binaries that are not empty in the first
/// `iov_max` elements of `iov` and whether more elements follow, as
/// `examine_iovec_term` reads them (`erl_nif.c:4092-4152`): an empty binary
/// gives no `writev(2)` entry, so `[<<>>]` gives no entry. Return `None`
/// for a term that is not a list, an element that is not a binary, or an
/// improper tail. A tail after
/// `iov_max` elements counts as data even when it holds only empty
/// binaries: `essio_sendv` sets `dataInTail` before it skips them
/// (`unix_socket_syncio.c:3414`, `3451-3461`).
fn iovec(iov: &Term) -> Option<(Vec<&[u8]>, bool)> {
    let mut elements = iov.list_iter()?;
    let mut buffers = Vec::new();
    for element in elements.by_ref().take(IOV_MAX) {
        let bytes = element.binary_bytes()?;
        if !bytes.is_empty() {
            buffers.push(bytes);
        }
    }
    let tail = elements.rest();
    match tail {
        Term::Nil => Some((buffers, false)),
        Term::Cons(_) => Some((buffers, true)),
        _ => None,
    }
}

/// Handle the result `sent` of a send of `size` bytes, as
/// `send_check_result` does (`unix_socket_syncio.c:6868-6945`): a partial
/// write or `EAGAIN` makes the write select; each byte written with
/// `data_in_tail` gives `{iov, Written}`; each byte written gives `ok`; and
/// another errno gives `{error, Errno}`.
fn send_result(
    scheduler: &mut Scheduler,
    mut state: MutexGuard<'_, SocketState>,
    sent: io::Result<usize>,
    size: usize,
    data_in_tail: bool,
    (key, caller, reference): (Reference, ProcessId, &Term),
) -> Result<BifResult, BeamError> {
    match sent {
        Ok(written) if written < size => Ok(send_retry(
            scheduler,
            state,
            Some(written),
            key,
            caller,
            reference,
        )),
        Ok(written) if data_in_tail => {
            // `send_check_retry` with the `sendRef` `iov`
            // (`unix_socket_syncio.c:7091-7155`).
            state.counters.write_pkg_max_count += written as u64;
            let iov = atom(scheduler, "iov")?;
            let mut writer = state.writer.take();
            make_current(scheduler, &mut writer, caller, &iov, key);
            state.writer = writer;
            Ok(BifResult::Value(Term::tuple(vec![
                iov,
                Term::Integer(written as i64),
            ])))
        }
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

/// Run `prim_socket:nif_sendmsg(Socket, Msg, Flags, Ref, IOV)`, as
/// `nif_sendmsg` (`prim_socket_nif.c:6619-6675`) and `essio_sendmsg`
/// (`unix_socket_syncio.c:3099-3355`) do: `badarg` for a term that is not a
/// socket, a `Ref` that is not a reference, or a `Msg` that is not a map;
/// `{invalid, {integer_range, Flags}}` for an integer outside the `int`
/// range; `{error, closed}`; `{error, {invalid, state}}` during a connect;
/// `{invalid, addr}` for an `addr` of `Msg` that does not decode;
/// `{invalid, iov}` as [`sendv`] gives it; `{invalid, ctrl}` for a `ctrl` of
/// `Msg` that does not decode into the `wCtrlSz` bytes of the option
/// `{otp, sndctrlbuf}`; then one `sendmsg(2)` with the results of
/// [`sendv`]. The `iov` of `Msg` is not read: `IOV` has it
/// (`unix_socket_syncio.c:3095-3096`).
///
/// # Errors
///
/// The errors of [`send`], a native `sockaddr` (see `socket::sockaddr`),
/// and a control message `value` that PBeam does not decode stop the run.
pub(super) fn sendmsg(
    scheduler: &mut Scheduler,
    socket: &Term,
    message: &Term,
    flags: &Term,
    reference: &Term,
    iov: &Term,
) -> Result<BifResult, BeamError> {
    let Some((key, socket)) = socket_argument(scheduler, socket) else {
        return Ok(BifResult::badarg());
    };
    let (Term::Reference(_), Term::Map(_)) = (reference, message) else {
        return Ok(BifResult::badarg());
    };
    let flags = match int(flags) {
        Ok(flags) => flags,
        Err(true) => return integer_range(scheduler, flags),
        Err(false) => return Ok(BifResult::badarg()),
    };
    let caller = scheduler.current_process().id;
    let mut state = lock(&socket);
    if let Some(result) = write_entry(scheduler, &state, "nif_sendmsg/5", caller)? {
        return Ok(result);
    }
    let address = match message_address(scheduler, message)? {
        Ok(address) => address,
        Err(result) => return Ok(result),
    };
    let buffers = match iovec(iov) {
        Some((buffers, data_in_tail)) if !data_in_tail || state.kind == libc::SOCK_STREAM => {
            Some((buffers, data_in_tail))
        }
        _ => None,
    };
    let Some((buffers, data_in_tail)) = buffers else {
        drop(state);
        return Ok(invalid(atom(scheduler, "iov")?));
    };
    let control = match message_control(scheduler, message, state.options.send_ctrl)? {
        Ok(control) => control,
        Err(result) => return Ok(result),
    };
    let size = buffers.iter().map(|buffer| buffer.len()).sum();
    state.counters.write_tries += 1;
    let sent = send_message(
        &state,
        address.as_ref(),
        &buffers,
        control.as_deref(),
        flags,
    );
    let request = (key, caller, reference);
    send_result(scheduler, state, sent, size, data_in_tail, request)
}

/// Return the address of the `addr` of the message `message`, `None`
/// without `addr`, or `{invalid, addr}` for an `addr` that does not decode,
/// as `essio_sendmsg` reads it (`unix_socket_syncio.c:3148-3172`).
///
/// # Errors
///
/// A native `sockaddr` stops the run (see `socket::sockaddr`).
fn message_address(
    scheduler: &mut Scheduler,
    message: &Term,
) -> Result<Result<Option<SockAddr>, BifResult>, BeamError> {
    let Some(address) = field(&scheduler.atoms, message, "addr") else {
        return Ok(Ok(None));
    };
    match sockaddr(scheduler, address)? {
        Some(address) => Ok(Ok(Some(address))),
        None => Ok(Err(invalid(atom(scheduler, "addr")?))),
    }
}

/// Return the control buffer of the `ctrl` of the message `message` in
/// `capacity` bytes, `None` without `ctrl`, or `{invalid, ctrl}` for a
/// `ctrl` that does not decode, as `essio_sendmsg` reads it
/// (`unix_socket_syncio.c:3299-3326`).
///
/// # Errors
///
/// A control message `value` that PBeam does not decode stops the run.
fn message_control(
    scheduler: &mut Scheduler,
    message: &Term,
    capacity: usize,
) -> Result<Result<Option<Vec<u8>>, BifResult>, BeamError> {
    let Some(ctrl) = field(&scheduler.atoms, message, "ctrl") else {
        return Ok(Ok(None));
    };
    match decode_cmsgs(scheduler, ctrl, capacity)? {
        Some(control) => Ok(Ok(Some(control))),
        None => Ok(Err(invalid(atom(scheduler, "ctrl")?))),
    }
}

/// Send one message of the binaries `buffers` to `address` with the
/// control buffer `control` and `flags`, as the `sendmsg(2)` of
/// `essio_sendmsg` does (`unix_socket_syncio.c:3338`).
fn send_message(
    state: &SocketState,
    address: Option<&SockAddr>,
    buffers: &[&[u8]],
    control: Option<&[u8]>,
    flags: i32,
) -> io::Result<usize> {
    let slices: Vec<IoSlice<'_>> = buffers.iter().map(|buffer| IoSlice::new(buffer)).collect();
    let mut header = socket2::MsgHdr::new().with_buffers(&slices);
    if let Some(address) = address {
        header = header.with_addr(address);
    }
    if let Some(control) = control {
        header = header.with_control(control);
    }
    let host = state
        .socket
        .as_ref()
        .expect("an open socket has a host socket");
    host.sendmsg(&header, flags)
}

/// The most messages of one `nif_sendmmsg/4` or `nif_recvmmsg/6` call,
/// `ESOCK_MMSG_MAX` (`unix_socket_syncio.c:263`, `4137-4138`, `4376-4377`).
#[cfg(target_os = "linux")]
const MMSG_MAX: usize = 1024;

/// Run `prim_socket:nif_sendmmsg(Socket, Msgs, Flags, Ref)`, as
/// `nif_sendmmsg` (`prim_socket_nif.c:6691-6760`) does: `badarg` for a term
/// that is not a socket, a `Ref` that is not a reference, `Msgs` that is
/// not a list, or `Flags` that is not an integer;
/// `{invalid, {integer_range, Flags}}` for an integer outside the `int`
/// range; `badarg` for `Msgs` that is not a proper list; then the send of
/// [`send_messages`].
///
/// # Errors
///
/// The errors of [`send_messages`].
pub(super) fn sendmmsg(
    scheduler: &mut Scheduler,
    socket: &Term,
    messages: &Term,
    flags: &Term,
    reference: &Term,
) -> Result<BifResult, BeamError> {
    let Some((key, socket)) = socket_argument(scheduler, socket) else {
        return Ok(BifResult::badarg());
    };
    if !matches!(reference, Term::Reference(_)) || !messages.is_list() {
        return Ok(BifResult::badarg());
    }
    let flags = match int(flags) {
        Ok(flags) => flags,
        Err(true) => return integer_range(scheduler, flags),
        Err(false) => return Ok(BifResult::badarg()),
    };
    // `enif_get_list_length` takes only a proper list
    // (`prim_socket_nif.c:6726-6730`).
    let Some(messages) = messages.proper_list() else {
        return Ok(BifResult::badarg());
    };
    let messages: Vec<&Term> = messages.collect();
    send_messages(scheduler, (key, &socket), &messages, flags, reference)
}

/// Raise `error:notsup` after the checks of `nif_sendmmsg`, as the stub of
/// `essio_sendmmsg` does on a host without `sendmmsg(2)`
/// (`unix_socket_syncio.c:4526-4537`).
#[cfg(not(target_os = "linux"))]
fn send_messages(
    _scheduler: &mut Scheduler,
    _socket: (Reference, &Mutex<SocketState>),
    _messages: &[&Term],
    _flags: i32,
    _reference: &Term,
) -> Result<BifResult, BeamError> {
    Ok(notsup())
}

/// Send `messages` as `essio_sendmmsg` does (`unix_socket_syncio.c:4320-4524`):
/// `{error, closed}`; `{error, {invalid, state}}` during a connect; `badarg`
/// for an element that is not a map; `ok` for no message; then for each of
/// the first `ESOCK_MMSG_MAX` messages in turn `{invalid, addr}` for an
/// `addr` that does not decode, `badarg` for an `iov` that is missing or
/// that `enif_inspect_iovec` does not take, and `{invalid, ctrl}` for a
/// `ctrl` that does not decode. The elements of an `iov` after the first
/// `iov_max` are not sent. Then `ok` when the host takes each byte of each
/// message that it sends, `{ok, [{Index, Written}]}` for the messages that
/// it takes in part, and the results of [`send`] when it sends no message.
/// A send counts only as a write try, and it does not release the current
/// writer.
///
/// PBeam sends each message with the safe `sendmsg` of socket2: the
/// `sendmmsg` of nix sends one list of typed control messages with each
/// message. A nonblocking `sendmmsg(2)` sends the messages in turn until
/// the first error or the first message that it sends in part, and it gives
/// the error only when it sends no message (`sendmmsg(2)`), and so does
/// this loop. So only the last message that it sends can be partial. Linux
/// sends at most `MAX_RW_COUNT` bytes, below 2 GiB, in one message, so the
/// `(int)` of `Written` (`unix_socket_syncio.c:4498-4499`) does not wrap.
///
/// # Errors
///
/// The errors of [`send`], a native `sockaddr` (see `socket::sockaddr`),
/// and a control message `value` that PBeam does not decode stop the run.
#[cfg(target_os = "linux")]
fn send_messages(
    scheduler: &mut Scheduler,
    (key, socket): (Reference, &Mutex<SocketState>),
    messages: &[&Term],
    flags: i32,
    reference: &Term,
) -> Result<BifResult, BeamError> {
    let caller = scheduler.current_process().id;
    let mut state = lock(socket);
    if let Some(result) = write_entry(scheduler, &state, "nif_sendmmsg/4", caller)? {
        return Ok(result);
    }
    if !messages
        .iter()
        .all(|message| matches!(message, Term::Map(_)))
    {
        return Ok(BifResult::badarg());
    }
    if messages.is_empty() {
        return Ok(BifResult::Value(Term::static_atom("ok")));
    }
    let mut parts = Vec::new();
    for &message in messages.iter().take(MMSG_MAX) {
        let address = match message_address(scheduler, message)? {
            Ok(address) => address,
            Err(result) => return Ok(result),
        };
        let iov = field(&scheduler.atoms, message, "iov");
        let Some((buffers, _)) = iov.and_then(iovec) else {
            return Ok(BifResult::badarg());
        };
        let control = match message_control(scheduler, message, state.options.send_ctrl)? {
            Ok(control) => control,
            Err(result) => return Ok(result),
        };
        parts.push((address, buffers, control));
    }
    state.counters.write_tries += 1;
    let mut written = Vec::new();
    for (address, buffers, control) in &parts {
        match send_message(&state, address.as_ref(), buffers, control.as_deref(), flags) {
            // `sendmmsg(2)` stops after a message that it sends in part.
            Ok(sent) => {
                written.push(sent);
                if sent != buffers.iter().map(|buffer| buffer.len()).sum::<usize>() {
                    break;
                }
            }
            Err(failure) if written.is_empty() => {
                let request = (key, caller, reference);
                return send_result(scheduler, state, Err(failure), 0, false, request);
            }
            Err(_) => break,
        }
    }
    // Each message whose `msg_len` is not the length of its I/O vector
    // (`unix_socket_syncio.c:4489-4501`).
    let partial: Vec<Term> = written
        .iter()
        .zip(&parts)
        .enumerate()
        .filter(|(_, (sent, (_, buffers, _)))| {
            **sent != buffers.iter().map(|buffer| buffer.len()).sum::<usize>()
        })
        .map(|(index, (sent, _))| {
            Term::tuple(vec![
                Term::Integer(index as i64),
                Term::Integer(*sent as i64),
            ])
        })
        .collect();
    if partial.is_empty() {
        return Ok(BifResult::Value(Term::static_atom("ok")));
    }
    Ok(BifResult::Value(Term::tuple(vec![
        Term::static_atom("ok"),
        Term::list_from(partial),
    ])))
}

/// Return the `uint64` of `term` as `enif_get_uint64` gives it, `Err(true)`
/// for another integer, and `Err(false)` for a term that is not an integer.
fn uint64(term: &Term) -> Result<u64, bool> {
    match term {
        Term::Integer(value) => u64::try_from(*value).map_err(|_| true),
        Term::BigInt(value) => u64::try_from(value.as_ref()).map_err(|_| true),
        _ => Err(false),
    }
}

/// Run `prim_socket:nif_recvmmsg(Socket, VLen, BufSz, CtrlSz, Flags, Ref)`,
/// as `nif_recvmmsg` does (`prim_socket_nif.c:7447-7540`): `badarg` for a
/// term that is not a socket, a `Ref` that is neither a reference nor
/// `zero`, or a `VLen`, `BufSz`, `CtrlSz`, or `Flags` that is not an
/// integer; `{invalid, {integer_range, Value}}` for the first of them
/// outside the `uint64` or `int` range, and for a `VLen` of 0 or above the
/// `unsigned int` range; then the receive of [`receive_messages`].
///
/// # Errors
///
/// A `BufSz` or `CtrlSz` above the `ssize_t` range stops the run: ERTS then
/// gives a term made of the integer itself (`prim_socket_nif.c:7502-7508`).
/// The errors of [`receive_messages`].
pub(super) fn recvmmsg(
    scheduler: &mut Scheduler,
    socket: &Term,
    [count, buffer_size, control_size, flags, reference]: [&Term; 5],
) -> Result<BifResult, BeamError> {
    let Some((key, socket)) = socket_argument(scheduler, socket) else {
        return Ok(BifResult::badarg());
    };
    let zero = match reference {
        Term::Reference(_) => false,
        Term::Atom(name) if scheduler.atoms.name_of(name) == "zero" => true,
        _ => return Ok(BifResult::badarg()),
    };
    let lengths = [count, buffer_size, control_size];
    let (values, flags_value) = (lengths.map(uint64), int(flags));
    if values.contains(&Err(false)) || flags_value == Err(false) {
        return Ok(BifResult::badarg());
    }
    if let Some(index) = values.iter().position(Result::is_err) {
        return integer_range(scheduler, lengths[index]);
    }
    let Ok(flags) = flags_value else {
        return integer_range(scheduler, flags);
    };
    let [vlen, size, control] = values.map(Result::unwrap_or_default);
    let Some(vlen) = u32::try_from(vlen).ok().filter(|&vlen| vlen != 0) else {
        return integer_range(scheduler, count);
    };
    if let Some(length) = [size, control]
        .into_iter()
        .find(|&length| i64::try_from(length).is_err())
    {
        return Err(error(format!(
            "prim_socket:nif_recvmmsg/6: the length {length} above the ssize_t range is not supported"
        )));
    }
    let caller = scheduler.current_process().id;
    let request = ReadRequest {
        key,
        socket,
        zero,
        size,
        flags,
        caller,
    };
    receive_messages(scheduler, request, vlen, control, reference)
}

/// Raise `error:notsup` after the checks of `nif_recvmmsg`, as the stub of
/// `essio_recvmmsg` does on a host without `recvmmsg(2)`
/// (`unix_socket_syncio.c:4297-4309`).
#[cfg(not(target_os = "linux"))]
fn receive_messages(
    _scheduler: &mut Scheduler,
    _request: ReadRequest,
    _count: u32,
    _control_size: u64,
    _reference: &Term,
) -> Result<BifResult, BeamError> {
    Ok(notsup())
}

/// Receive at most `count` messages, as `essio_recvmmsg` does
/// (`unix_socket_syncio.c:4111-4295`): the state checks of [`recv`]; then
/// at most `ESOCK_MMSG_MAX` messages, each into a buffer of `BufSz` bytes,
/// or `rBufSz`, and a control buffer of `CtrlSz` bytes, or `rCtrlSz`, which
/// give `{ok, [Msg]}` or `{select_read, [Msg]}` with each `Msg` of
/// [`recvmsg`], counted as one read try; and the results of [`recvfrom`]
/// after a failed first read. The end of a stream gives a message without
/// data, as a read of length 0 does, and does not stop the receive.
///
/// PBeam receives each message with the safe `recvmsg` of nix: the
/// `recvmmsg` of nix gives the control messages only as typed values, not
/// as the bytes that `encode_cmsgs` reads. A nonblocking `recvmmsg(2)`
/// receives the messages in turn until the first error, which it gives only
/// when it receives no message (`recvmmsg(2)`), and so does this loop.
///
/// # Errors
///
/// A socket for which [`flagged_source_known`] is `false` and a control
/// message that PBeam does not encode stop the run.
#[cfg(target_os = "linux")]
fn receive_messages(
    scheduler: &mut Scheduler,
    request: ReadRequest,
    count: u32,
    control_size: u64,
    reference: &Term,
) -> Result<BifResult, BeamError> {
    let name = "nif_recvmmsg/6";
    let ReadRequest {
        key,
        socket,
        zero,
        size,
        flags,
        caller,
    } = request;
    let mut state = lock(&socket);
    if let Some(result) = read_entry(scheduler, &state, name, caller)? {
        return Ok(result);
    }
    if !flagged_source_known(&state, flags) {
        return Err(unknown_source(name, &state, flags));
    }
    let size = buffer_size(&state, size);
    let control_size = control_buffer_size(&state, control_size);
    state.counters.read_tries += 1;
    let mut received = Vec::new();
    for _ in 0..(count as usize).min(MMSG_MAX) {
        let mut buffer = vec![0; size];
        let mut control = vec![0; control_size];
        match recv_message(&state, &mut buffer, &mut control, flags) {
            Ok((read, from, message_flags)) => {
                // With `MSG_TRUNC`, the length can pass the buffer size
                // (`unix_socket_syncio.c:4219-4224`).
                buffer.truncate(read);
                received.push((buffer, control, from, message_flags));
            }
            // A failed read ends the buffers of `rNum` (`recv_check_fail`,
            // unix_socket_syncio.c:10153).
            Err(failure) if received.is_empty() => {
                state.recv_count_done = 0;
                return recv_fail(
                    scheduler,
                    state,
                    &failure.into(),
                    zero,
                    key,
                    caller,
                    reference,
                );
            }
            Err(_) => break,
        }
    }
    let counters = &mut state.counters;
    counters.read_pkg += received.len() as u64;
    for (buffer, ..) in &received {
        counters.read_byte += buffer.len() as u64;
        counters.read_pkg_max = counters.read_pkg_max.max(buffer.len() as u64);
    }
    let mut messages = Vec::new();
    for (buffer, control, from, message_flags) in received {
        messages.push(message_term(
            scheduler,
            buffer,
            &control,
            from,
            message_flags,
        )?);
    }
    let messages = Term::list_from(messages);
    // With `selectRead`, `{select_read, Msgs}` and the next read select
    // (`unix_socket_syncio.c:4262-4268`).
    if state.select_read && !zero {
        let result = Term::tuple(vec![atom(scheduler, "select_read")?, messages]);
        return Ok(recv_select(
            scheduler, state, key, caller, reference, result,
        ));
    }
    state.reader = None;
    drop(state);
    Ok(BifResult::Value(Term::tuple(vec![
        Term::static_atom("ok"),
        messages,
    ])))
}

/// Return the exception `error:notsup` of `enif_raise_exception`.
#[cfg(not(target_os = "linux"))]
fn notsup() -> BifResult {
    BifResult::Raise {
        class: ExceptionClass::Error,
        reason: Term::static_atom("notsup"),
    }
}

/// The checked arguments of `nif_recv/4` and `nif_recvfrom/4`.
struct ReadRequest {
    key: Reference,
    socket: Arc<Mutex<SocketState>>,
    zero: bool,
    size: u64,
    flags: i32,
    caller: ProcessId,
}

/// Check the arguments of `nif_recv/4` or `nif_recvfrom/4`, as `nif_recv`
/// (`prim_socket_nif.c:7056-7124`) and `nif_recvfrom`
/// (`prim_socket_nif.c:7153-7195`) do, and return the request or the
/// result of a failed check.
///
/// # Errors
///
/// A `Length` above the `ssize_t` range stops the run: ERTS then gives a
/// term made of the integer itself (`prim_socket_nif.c:7092-7094`,
/// `7193-7195`).
fn read_request(
    scheduler: &mut Scheduler,
    name: &str,
    socket: &Term,
    length: &Term,
    flags: &Term,
    reference: &Term,
) -> Result<Result<ReadRequest, BifResult>, BeamError> {
    let Some((key, socket)) = socket_argument(scheduler, socket) else {
        return Ok(Err(BifResult::badarg()));
    };
    let zero = match reference {
        Term::Reference(_) => false,
        Term::Atom(name) if scheduler.atoms.name_of(name) == "zero" => true,
        _ => return Ok(Err(BifResult::badarg())),
    };
    let size = match length {
        Term::Integer(value) => Some(u64::try_from(*value).ok()),
        Term::BigInt(value) => Some(u64::try_from(value.as_ref()).ok()),
        _ => None,
    };
    let (size, flags) = match (size, int(flags)) {
        (None, _) | (_, Err(false)) => return Ok(Err(BifResult::badarg())),
        (Some(None), _) => return integer_range(scheduler, length).map(Err),
        (_, Err(true)) => return integer_range(scheduler, flags).map(Err),
        (Some(Some(size)), Ok(flags)) => (size, flags),
    };
    if i64::try_from(size).is_err() {
        return Err(error(format!(
            "prim_socket:{name}: the length {size} above the ssize_t range is not supported"
        )));
    }
    let caller = scheduler.current_process().id;
    Ok(Ok(ReadRequest {
        key,
        socket,
        zero,
        size,
        flags,
        caller,
    }))
}

/// Check the socket state before a read, as `recv_check_entry` does
/// (`unix_socket_syncio.c:9869-9931`): `{error, closed}`, and
/// `{error, {invalid, state}}` with a current acceptor.
///
/// # Errors
///
/// A call of another process while a reader waits stops the run: PBeam
/// has no reader queue.
fn read_entry(
    scheduler: &mut Scheduler,
    state: &SocketState,
    name: &str,
    caller: ProcessId,
) -> Result<Option<BifResult>, BeamError> {
    if state.read & (CLOSING | CLOSED) != 0 {
        return Ok(Some(closed()));
    }
    if state.acceptor.is_some() {
        return invalid_state(scheduler).map(Some);
    }
    if state.reader.as_ref().is_some_and(|(pid, _)| *pid != caller) {
        return Err(error(format!(
            "prim_socket:{name}: a second reader process is not supported"
        )));
    }
    Ok(None)
}

/// Return the buffer size of a read of `size` bytes from `state`: `rBufSz`
/// of the option `{otp, rcvbuf}` for 0 (`unix_socket_syncio.c:3900`).
fn buffer_size(state: &SocketState, size: u64) -> usize {
    if size == 0 {
        state.options.recv_buffer
    } else {
        usize::try_from(size).expect("a length in the ssize_t range")
    }
}

/// Run `prim_socket:nif_recv(Socket, Length, Flags, Ref)`, as `nif_recv`
/// (`prim_socket_nif.c:7056-7124`) and `essio_recv`
/// (`unix_socket_syncio.c:3796-3878`, `9869-10410`) do: `badarg` for a term
/// that is not a socket, a `Ref` that is neither a reference nor `zero`,
/// or a `Length` or `Flags` that is not an integer;
/// `{invalid, {integer_range, Value}}` for a `Length` outside the `uint64`
/// range or `Flags` outside the `int` range; `{error, closed}`;
/// `{error, {invalid, state}}` with a current acceptor; `{ok, Data}`;
/// `{more, Data}` while the buffers of `{otp, rcvbuf}` `{N, Size}` come;
/// `{select_read, Data}` with `{otp, select_read}` and a reference;
/// `{error, closed}` at the end of a stream; `select` or `{select, Data}`
/// with the read select, or `timeout` or `{timeout, Data}` for the `Ref`
/// `zero`, when fewer bytes than `Length` came; and `{error, Errno}`.
///
/// # Errors
///
/// A `Length` above the `ssize_t` range stops the run: ERTS then gives a
/// term made of the integer itself (`prim_socket_nif.c:7092-7094`). A call
/// of another process while a reader waits stops the run: PBeam has no
/// reader queue. A new atom in a full atom table gives the error of
/// `AtomTable::put`.
pub(super) fn recv(
    scheduler: &mut Scheduler,
    socket: &Term,
    length: &Term,
    flags: &Term,
    reference: &Term,
) -> Result<BifResult, BeamError> {
    let request = match read_request(scheduler, "nif_recv/4", socket, length, flags, reference)? {
        Ok(request) => request,
        Err(result) => return Ok(result),
    };
    let ReadRequest {
        key,
        socket,
        zero,
        size,
        flags,
        caller,
    } = request;
    let mut state = lock(&socket);
    if let Some(result) = read_entry(scheduler, &state, "nif_recv/4", caller)? {
        return Ok(result);
    }
    let buffer_size = buffer_size(&state, size);
    let mut buffer = vec![0; buffer_size];
    state.counters.read_tries += 1;
    let stream = state.kind == libc::SOCK_STREAM;
    let host = state
        .socket
        .as_ref()
        .expect("an open socket has a host socket");
    // `essio_recv` calls `recv(2)` with the flags (unix_socket_syncio.c:3835).
    let received = nix::sys::socket::recv(
        host.as_raw_fd(),
        &mut buffer,
        nix::sys::socket::MsgFlags::from_bits_retain(flags),
    );
    let read = match received.map_err(io::Error::from) {
        Ok(0) if stream => return Ok(end_of_stream(state)),
        Ok(read) => read,
        // A failed read ends the buffers of `rNum` (`recv_check_fail`,
        // unix_socket_syncio.c:10153).
        Err(failure) => {
            state.recv_count_done = 0;
            return recv_fail(scheduler, state, &failure, zero, key, caller, reference);
        }
    };
    buffer.truncate(read);
    let length = read as u64;
    state.counters.read_byte += length;
    state.counters.read_pkg_max_count += length;
    if read == buffer_size && size == 0 && stream {
        // `recv_check_full_maybe_done` counts the bytes once, gives
        // `{more, Bin}` until `rNum` buffers came, and else
        // `recv_check_full_done` counts them again
        // (`unix_socket_syncio.c:10050-10137`).
        state.recv_count_done += 1;
        if state.recv_count_done < state.options.recv_count {
            let mut reader = state.reader.take();
            make_current(scheduler, &mut reader, caller, reference, key);
            state.reader = reader;
            return Ok(BifResult::Value(Term::tuple(vec![
                Term::static_atom("more"),
                Term::binary(buffer),
            ])));
        }
        state.counters.read_byte += length;
        state.counters.read_pkg_max_count += length;
    }
    state.recv_count_done = 0;
    let counters = &mut state.counters;
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
    // With `selectRead`, a receive with a reference gives
    // `{select_read, Bin}` and starts the next read select
    // (`unix_socket_syncio.c:10122-10125`, `10397-10400`).
    if state.select_read && !zero {
        let data = Term::tuple(vec![atom(scheduler, "select_read")?, data]);
        return Ok(recv_select(scheduler, state, key, caller, reference, data));
    }
    state.reader = None;
    Ok(BifResult::Value(Term::tuple(vec![
        Term::static_atom("ok"),
        data,
    ])))
}

/// Run `prim_socket:nif_recvfrom(Socket, Length, Flags, Ref)`, as
/// `nif_recvfrom` (`prim_socket_nif.c:7153-7231`) and `essio_recvfrom`
/// (`unix_socket_syncio.c:3888-3977`) do: the argument errors and the state
/// checks of [`recv`]; `{ok, {SockAddr, Data}}` with one message, which
/// the buffer of `Length` bytes truncates; `{error, closed}` at the end of
/// a stream; `select` with the read select, or `timeout` for the `Ref`
/// `zero`, after `EAGAIN`; `{select_read, {SockAddr, Data}}` with the
/// option `select_read` and a reference; and `{error, Errno}`.
///
/// `Flags` other than 0 go to `recvmsg(2)` (see [`recv_flagged`]).
///
/// # Errors
///
/// The errors of [`recv`], and `Flags` other than 0 on a socket for which
/// [`flagged_source_known`] is `false` stop the run.
pub(super) fn recvfrom(
    scheduler: &mut Scheduler,
    socket: &Term,
    length: &Term,
    flags: &Term,
    reference: &Term,
) -> Result<BifResult, BeamError> {
    let name = "nif_recvfrom/4";
    let request = match read_request(scheduler, name, socket, length, flags, reference)? {
        Ok(request) => request,
        Err(result) => return Ok(result),
    };
    let ReadRequest {
        key,
        socket,
        zero,
        size,
        flags,
        caller,
    } = request;
    let mut state = lock(&socket);
    if let Some(result) = read_entry(scheduler, &state, name, caller)? {
        return Ok(result);
    }
    let mut buffer = vec![0; self::buffer_size(&state, size)];
    let received = if flags == 0 {
        state.counters.read_tries += 1;
        let host = state
            .socket
            .as_ref()
            .expect("an open socket has a host socket");
        nix::sys::socket::recvfrom::<SockaddrStorage>(host.as_raw_fd(), &mut buffer)
    } else {
        if !flagged_source_known(&state, flags) {
            return Err(unknown_source(name, &state, flags));
        }
        state.counters.read_tries += 1;
        recv_flagged(&state, &mut buffer, flags)
    };
    let stream = state.kind == libc::SOCK_STREAM;
    let (read, from) = match received.map_err(io::Error::from) {
        Ok((0, _)) if stream => return Ok(end_of_stream(state)),
        Ok(received) => received,
        // A failed read ends the buffers of `rNum` (`recv_check_fail`,
        // unix_socket_syncio.c:10153).
        Err(failure) => {
            state.recv_count_done = 0;
            return recv_fail(scheduler, state, &failure, zero, key, caller, reference);
        }
    };
    // A read ends the buffers of `rNum` (unix_socket_syncio.c:3948).
    state.recv_count_done = 0;
    buffer.truncate(read);
    let length = read as u64;
    let counters = &mut state.counters;
    counters.read_pkg += 1;
    counters.read_byte += length;
    counters.read_pkg_max = counters.read_pkg_max.max(length);
    let from = source_term(scheduler, from.as_ref())?;
    let result = Term::tuple(vec![from, Term::binary(buffer)]);
    // With `selectRead`, `{select_read, {From, Bin}}` and the next read
    // select (`unix_socket_syncio.c:3967-3970`).
    if state.select_read && !zero {
        let result = Term::tuple(vec![atom(scheduler, "select_read")?, result]);
        return Ok(recv_select(
            scheduler, state, key, caller, reference, result,
        ));
    }
    state.reader = None;
    drop(state);
    Ok(BifResult::Value(Term::tuple(vec![
        Term::static_atom("ok"),
        result,
    ])))
}

/// Return the `uint64` length of `nif_recvmsg/5`, as `nif_recvmsg` reads
/// it (`prim_socket_nif.c:7305-7330`): `Err(badarg)` for a term that is
/// not an integer and `Err({invalid, {integer_range, Value}})` for another
/// integer outside the `uint64` range.
///
/// # Errors
///
/// A length above the `ssize_t` range stops the run: ERTS then gives a
/// term made of the integer itself (`prim_socket_nif.c:7323-7330`).
fn message_length(
    scheduler: &mut Scheduler,
    length: &Term,
) -> Result<Result<u64, BifResult>, BeamError> {
    let size = match length {
        Term::Integer(value) => u64::try_from(*value).ok(),
        Term::BigInt(value) => u64::try_from(value.as_ref()).ok(),
        _ => return Ok(Err(BifResult::badarg())),
    };
    let Some(size) = size else {
        return integer_range(scheduler, length).map(Err);
    };
    if i64::try_from(size).is_err() {
        return Err(error(format!(
            "prim_socket:nif_recvmsg/5: the length {size} above the ssize_t range is not supported"
        )));
    }
    Ok(Ok(size))
}

/// Run `prim_socket:nif_recvmsg(Socket, BufSz, CtrlSz, Flags, Ref)`, as
/// `nif_recvmsg` (`prim_socket_nif.c:7265-7429`) and `essio_recvmsg`
/// (`unix_socket_syncio.c:3987-4105`) do: `badarg` for a term that is not a
/// socket, a `Ref` that is neither a reference nor `zero`, or a `BufSz`,
/// `CtrlSz`, or `Flags` that is not an integer, checked in this order with
/// `{invalid, {integer_range, Value}}` for an integer outside the range;
/// the state checks of [`recv`]; then one `recvmsg(2)` into a buffer of
/// `BufSz` bytes, or `rBufSz`, and a control buffer of `CtrlSz` bytes, or
/// `rCtrlSz`, which gives `{ok, Msg}` or `{select_read, Msg}` with
/// `#{iov => [Data], ctrl => Cmsgs, flags => Flags}` and the source
/// `addr` that the host gives (`encode_msg`,
/// `unix_socket_syncio.c:7579-7713`); and the results of [`recvfrom`] at
/// the end of a stream and after a failed read.
///
/// # Errors
///
/// A length above the `ssize_t` range, a socket for which
/// [`flagged_source_known`] is `false`, and a control message that PBeam
/// does not encode stop the run.
pub(super) fn recvmsg(
    scheduler: &mut Scheduler,
    socket: &Term,
    buffer_size: &Term,
    control_size: &Term,
    flags: &Term,
    reference: &Term,
) -> Result<BifResult, BeamError> {
    let name = "nif_recvmsg/5";
    let Some((key, socket)) = socket_argument(scheduler, socket) else {
        return Ok(BifResult::badarg());
    };
    let zero = match reference {
        Term::Reference(_) => false,
        Term::Atom(name) if scheduler.atoms.name_of(name) == "zero" => true,
        _ => return Ok(BifResult::badarg()),
    };
    let size = match message_length(scheduler, buffer_size)? {
        Ok(size) => size,
        Err(result) => return Ok(result),
    };
    let control_size = match message_length(scheduler, control_size)? {
        Ok(size) => size,
        Err(result) => return Ok(result),
    };
    let flags = match int(flags) {
        Ok(flags) => flags,
        Err(true) => return integer_range(scheduler, flags),
        Err(false) => return Ok(BifResult::badarg()),
    };
    let caller = scheduler.current_process().id;
    let mut state = lock(&socket);
    if let Some(result) = read_entry(scheduler, &state, name, caller)? {
        return Ok(result);
    }
    if !flagged_source_known(&state, flags) {
        return Err(unknown_source(name, &state, flags));
    }
    let mut buffer = vec![0; self::buffer_size(&state, size)];
    let mut control = vec![0; control_buffer_size(&state, control_size)];
    state.counters.read_tries += 1;
    let received = recv_message(&state, &mut buffer, &mut control, flags);
    let stream = state.kind == libc::SOCK_STREAM;
    let (read, from, message_flags) = match received.map_err(io::Error::from) {
        Ok((0, ..)) if stream => return Ok(end_of_stream(state)),
        Ok(received) => received,
        Err(failure) => {
            state.recv_count_done = 0;
            return recv_fail(scheduler, state, &failure, zero, key, caller, reference);
        }
    };
    // A read ends the buffers of `rNum` (unix_socket_syncio.c:4082).
    state.recv_count_done = 0;
    buffer.truncate(read);
    let length = read as u64;
    let counters = &mut state.counters;
    counters.read_pkg += 1;
    counters.read_byte += length;
    counters.read_pkg_max = counters.read_pkg_max.max(length);
    let message = message_term(scheduler, buffer, &control, from, message_flags)?;
    // With `selectRead`, `{select_read, Msg}` and the next read select
    // (`unix_socket_syncio.c:4095-4098`).
    if state.select_read && !zero {
        let result = Term::tuple(vec![atom(scheduler, "select_read")?, message]);
        return Ok(recv_select(
            scheduler, state, key, caller, reference, result,
        ));
    }
    state.reader = None;
    drop(state);
    Ok(BifResult::Value(Term::tuple(vec![
        Term::static_atom("ok"),
        message,
    ])))
}

/// Return the message `#{iov => [Data], ctrl => Cmsgs, flags => Flags}`
/// of the data `buffer`, the control buffer `control`, and the
/// `msg_flags` `message_flags`, with the source address `from` as `addr`
/// when the host gives one, as `encode_msg` makes it
/// (`unix_socket_syncio.c:7579-7713`).
///
/// # Errors
///
/// A control message that PBeam does not encode stops the run.
fn message_term(
    scheduler: &mut Scheduler,
    buffer: Vec<u8>,
    control: &[u8],
    from: Option<SockaddrStorage>,
    message_flags: i32,
) -> Result<Term, BeamError> {
    let ctrl = encode_cmsgs(scheduler, control)?;
    let mut entries = vec![
        (
            atom(scheduler, "iov")?,
            Term::list_from([Term::binary(buffer)]),
        ),
        (atom(scheduler, "ctrl")?, ctrl),
        (
            atom(scheduler, "flags")?,
            message_flags_term(scheduler, message_flags)?,
        ),
    ];
    if let Some(from) = from {
        entries.push((
            atom(scheduler, "addr")?,
            source_term(scheduler, Some(&from))?,
        ));
    }
    Ok(Term::Map(
        crate::term::Map::from_pairs(entries, &scheduler.atoms)
            .expect("atom keys fit each map form"),
    ))
}

/// Return the control buffer size of a read with `size` bytes from
/// `state`: `rCtrlSz` of the option `{otp, rcvctrlbuf}` for 0
/// (`unix_socket_syncio.c:3999`).
fn control_buffer_size(state: &SocketState, size: u64) -> usize {
    if size == 0 {
        state.options.recv_ctrl
    } else {
        usize::try_from(size).expect("a length in the ssize_t range")
    }
}

/// Return the error of a receive with `flags` on a socket for which
/// [`flagged_source_known`] is `false`.
fn unknown_source(name: &str, state: &SocketState, flags: i32) -> BeamError {
    error(format!(
        "prim_socket:{name}: the flags {flags} on a socket of the domain {} are not supported: see \"NIF stubs\" in docs/erts-differences.md",
        state.domain
    ))
}

/// Receive one message into `buffer` and `control` with `flags`, as the
/// `recvmsg(2)` of `essio_recvmsg` does (`unix_socket_syncio.c:4033-4048`),
/// and return the length, the source address, and the `msg_flags`. The TCP
/// stream socket gives no address, which `encode_msg` leaves out
/// (`unix_socket_syncio.c:7599-7606`), and another socket gives the address
/// of the sender (see [`flagged_source_known`]).
fn recv_message(
    state: &SocketState,
    buffer: &mut [u8],
    control: &mut [u8],
    flags: i32,
) -> nix::Result<(usize, Option<SockaddrStorage>, i32)> {
    let host = state
        .socket
        .as_ref()
        .expect("an open socket has a host socket");
    let mut slices = [IoSliceMut::new(buffer)];
    let flags = MsgFlags::from_bits_retain(flags);
    let control = Some(control);
    if state.kind == libc::SOCK_STREAM {
        let received =
            nix::sys::socket::recvmsg::<()>(host.as_raw_fd(), &mut slices, control, flags)?;
        return Ok((received.bytes, None, received.flags.bits()));
    }
    let received = nix::sys::socket::recvmsg::<SockaddrStorage>(
        host.as_raw_fd(),
        &mut slices,
        control,
        flags,
    )?;
    Ok((received.bytes, received.address, received.flags.bits()))
}

/// Return the list of the `msg_flags` `flags`, as `esock_encode_msg_flags`
/// makes it (`prim_socket_nif.c:14225-14261`): `[]` for 0, else the name of
/// each flag of the host in the order of `esock_msg_flags`, and then the
/// other bits as one integer.
fn message_flags_term(scheduler: &mut Scheduler, mut flags: i32) -> Result<Term, BeamError> {
    let mut names = Vec::new();
    for (name, values) in MSG_FLAGS {
        let flag = values[HOST];
        if flags != 0 && flag != 0 && flags & flag == flag {
            flags &= !flag;
            names.push(atom(scheduler, name)?);
        }
    }
    if flags != 0 {
        names.push(Term::Integer(flags.into()));
    }
    Ok(Term::list_from(names))
}

/// Return whether the kernel gives the source address of a receive with
/// `flags` other than 0 on the socket of `state` each time, or never: always
/// on an inet or inet6 socket that is not a stream socket, and never on an
/// inet or inet6 TCP stream socket (the protocol 0 is TCP on macOS, which
/// has no `SO_PROTOCOL`). The safe `recvmsg` of nix reads an address that it
/// does not initialize when the kernel gives one of length 0, as on a local
/// socket of an unbound peer, on another stream protocol such as MPTCP, or
/// with `MSG_ERRQUEUE` on Linux, so these receives are not supported (row
/// "NIF stubs"). Another stream protocol such as SCTP gives an address.
fn flagged_source_known(state: &SocketState, flags: i32) -> bool {
    let tcp = state.kind != libc::SOCK_STREAM || matches!(state.protocol, 0 | libc::IPPROTO_TCP);
    matches!(state.domain, libc::AF_INET | libc::AF_INET6) && tcp && !errqueue(flags)
}

/// Return whether `flags` hold the flag `MSG_ERRQUEUE` of Linux.
#[cfg(target_os = "linux")]
fn errqueue(flags: i32) -> bool {
    flags & libc::MSG_ERRQUEUE != 0
}

/// Return `false`: the host has no flag `MSG_ERRQUEUE`.
#[cfg(not(target_os = "linux"))]
fn errqueue(_flags: i32) -> bool {
    false
}

/// Receive one message with `flags` other than 0 into `buffer`, as the
/// `recvfrom(2)` of `essio_recvfrom` does (`unix_socket_syncio.c:3925-3927`),
/// with `recvmsg(2)`: the TCP stream socket gives no address, and another socket
/// gives the address of the sender (see [`flagged_source_known`]).
fn recv_flagged(
    state: &SocketState,
    buffer: &mut [u8],
    flags: i32,
) -> nix::Result<(usize, Option<SockaddrStorage>)> {
    let host = state
        .socket
        .as_ref()
        .expect("an open socket has a host socket");
    let mut slices = [IoSliceMut::new(buffer)];
    let flags = MsgFlags::from_bits_retain(flags);
    if state.kind == libc::SOCK_STREAM {
        let received = nix::sys::socket::recvmsg::<()>(host.as_raw_fd(), &mut slices, None, flags)?;
        return Ok((received.bytes, None));
    }
    let received =
        nix::sys::socket::recvmsg::<SockaddrStorage>(host.as_raw_fd(), &mut slices, None, flags)?;
    Ok((received.bytes, received.address))
}

/// Count the failed read at the end of a stream, release the current
/// reader, and return `{error, closed}`, as `recv_check_result` does
/// (`unix_socket_syncio.c:9949-9969`).
fn end_of_stream(mut state: MutexGuard<'_, SocketState>) -> BifResult {
    state.counters.read_fails += 1;
    state.reader = None;
    closed()
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

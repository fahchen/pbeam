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

use super::select::{fd, monitor, select};
use super::socket::{closed, errno_error, invalid, sockaddr, socket_argument, source_term};
use super::supports::atom;
use super::{BeamError, BifResult, Scheduler, Term, error};
use crate::runtime::resources::lock;
use crate::runtime::scheduler::poll::Direction;
use crate::runtime::sockets::{CLOSED, CLOSING, SELECTED, SocketState};
use crate::term::{ProcessId, Reference};

/// A byte count of one socket call fits in `u64`.
fn to_u64(count: usize) -> u64 {
    u64::try_from(count).expect("a byte count fits in u64")
}

/// A byte count of one socket call fits in `i64`.
fn to_i64(count: usize) -> i64 {
    i64::try_from(count).expect("a byte count fits in i64")
}

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
    send_result(
        scheduler,
        state,
        sent,
        bytes.len(),
        IovTail::Absent,
        request,
    )
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
    if state.write & (CLOSING | CLOSED) != 0 {
        return Ok(Some(closed()));
    }
    if state.connector.is_some() {
        return invalid_state(scheduler).map(Some);
    }
    if state.writer.as_ref().is_some_and(|(pid, _)| *pid != caller) {
        return Err(error(format!(
            "prim_socket:{name}: a second writer process is not supported"
        )));
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
    send_result(
        scheduler,
        state,
        sent,
        size,
        IovTail::from(data_in_tail),
        request,
    )
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

/// Whether the iovec of a send has data after its first `iov_max` elements.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum IovTail {
    /// The iovec ends within `iov_max` elements.
    Absent,
    /// A tail follows: the send reports the written bytes as `{iov, Written}`.
    Present,
}

impl From<bool> for IovTail {
    fn from(present: bool) -> Self {
        if present { Self::Present } else { Self::Absent }
    }
}

/// The `Ref` argument of a receive.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum RecvRef {
    /// The atom `zero`: the receive does not select.
    Zero,
    /// A reference: the receive selects after `EAGAIN`.
    Reference,
}

impl From<bool> for RecvRef {
    fn from(zero: bool) -> Self {
        if zero { Self::Zero } else { Self::Reference }
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
    tail: IovTail,
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
        Ok(written) if tail == IovTail::Present => {
            // `send_check_retry` with the `sendRef` `iov`
            // (`unix_socket_syncio.c:7091-7155`).
            state.counters.write_pkg_max_count += to_u64(written);
            let iov = atom(scheduler, "iov")?;
            let mut writer = state.writer.take();
            make_current(scheduler, &mut writer, caller, &iov, key);
            state.writer = writer;
            Ok(BifResult::Value(Term::tuple(vec![
                iov,
                Term::Integer(to_i64(written)),
            ])))
        }
        Ok(written) => {
            let counters = &mut state.counters;
            counters.write_pkg += 1;
            counters.write_byte += to_u64(written);
            counters.write_pkg_max_count += to_u64(written);
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
        state.counters.write_pkg_max_count += to_u64(written);
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
            Term::Integer(to_i64(written)),
        ]),
        None => Term::static_atom("select"),
    })
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
/// of the option `{otp, rcvbuf}` for 0 (`unix_socket_syncio.c:3900`). The
/// buffer count `rNum` stays 0: PBeam does not set it
/// (`prim_socket_nif.c:17061-17064`).
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
        Err(failure) => {
            return recv_fail(
                scheduler,
                state,
                &failure,
                RecvRef::from(zero),
                key,
                caller,
                reference,
            );
        }
    };
    buffer.truncate(read);
    let length = to_u64(read);
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

/// Run `prim_socket:nif_recvfrom(Socket, Length, Flags, Ref)`, as
/// `nif_recvfrom` (`prim_socket_nif.c:7153-7231`) and `essio_recvfrom`
/// (`unix_socket_syncio.c:3888-3977`) do: the argument errors and the state
/// checks of [`recv`]; `{ok, {SockAddr, Data}}` with one message, which
/// the buffer of `Length` bytes truncates; `{error, closed}` at the end of
/// a stream; `select` with the read select, or `timeout` for the `Ref`
/// `zero`, after `EAGAIN`; and `{error, Errno}`. The option `select_read`
/// that gives `{select_read, ...}` is not supported, so it keeps its
/// default `false` (`prim_socket_nif.c:17070`).
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
    let mut buffer = vec![0; buffer_size(&state, size)];
    let received = if flags == 0 {
        state.counters.read_tries += 1;
        let host = state
            .socket
            .as_ref()
            .expect("an open socket has a host socket");
        nix::sys::socket::recvfrom::<SockaddrStorage>(host.as_raw_fd(), &mut buffer)
    } else {
        if !flagged_source_known(&state, flags) {
            return Err(error(format!(
                "prim_socket:{name}: the flags {flags} on a socket of the domain {} are not supported: see \"NIF stubs\" in docs/erts-differences.md",
                state.domain
            )));
        }
        state.counters.read_tries += 1;
        recv_flagged(&state, &mut buffer, flags)
    };
    let stream = state.kind == libc::SOCK_STREAM;
    let (read, from) = match received.map_err(io::Error::from) {
        Ok((0, _)) if stream => return Ok(end_of_stream(state)),
        Ok(received) => received,
        Err(failure) => {
            return recv_fail(
                scheduler,
                state,
                &failure,
                RecvRef::from(zero),
                key,
                caller,
                reference,
            );
        }
    };
    buffer.truncate(read);
    let length = to_u64(read);
    let counters = &mut state.counters;
    counters.read_pkg += 1;
    counters.read_byte += length;
    counters.read_pkg_max = counters.read_pkg_max.max(length);
    state.reader = None;
    drop(state);
    let from = source_term(scheduler, from.as_ref())?;
    Ok(BifResult::Value(Term::tuple(vec![
        Term::static_atom("ok"),
        Term::tuple(vec![from, Term::binary(buffer)]),
    ])))
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
    recv_ref: RecvRef,
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
    if recv_ref == RecvRef::Zero {
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

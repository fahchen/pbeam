//! The bindings of `prim_socket:nif_info/0,1`, `nif_open/4`, `nif_bind/2`,
//! `nif_sockname/1`, `nif_close/1`, and `nif_finalize_close/1`
//! (`prim_socket_nif.c`), with the Unix I/O backend `unix_essio`
//! (`unix_socket_syncio.c`).
//!
//! A socket resource is a reference to a `SocketState` in the `Sockets`
//! value of the node resources, so a socket does not pin its process to
//! one scheduler. PBeam runs no select, so `nif_close/1` always returns
//! `ok` (`unix_socket_syncio.c:4574-4593`, `4627-4633`).

use std::io;
use std::net::{Ipv4Addr, Ipv6Addr, SocketAddrV4, SocketAddrV6};
use std::os::unix::ffi::OsStrExt;
use std::sync::{Arc, Mutex};

use socket2::{Domain, Protocol, SockAddr, Socket, Type};

use super::supports::atom;
use super::*;
use crate::runtime::port::inet::errno_name;
use crate::runtime::resources::lock;
use crate::runtime::sockets::{BOUND, CLOSED, CLOSING, SocketState, Sockets};
use crate::term::{ProcessId, Reference};

/// The size of `sun_path` of `struct sockaddr_un` on the host.
#[cfg(target_vendor = "apple")]
const SUN_PATH: usize = 104;
/// The size of `sun_path` of `struct sockaddr_un` on the host.
#[cfg(not(target_vendor = "apple"))]
const SUN_PATH: usize = 108;

/// The keys of the socket counters of `esock_socket_info_counters`
/// (`prim_socket_nif.c:4920-4990`). PBeam sends and receives nothing on a
/// socket yet, so each counter is 0.
const SOCKET_COUNTERS: [&str; 16] = [
    "read_byte",
    "read_fails",
    "read_pkg",
    "read_pkg_max",
    "read_tries",
    "read_waits",
    "write_byte",
    "write_fails",
    "write_pkg",
    "write_pkg_max",
    "write_tries",
    "write_waits",
    "acc_success",
    "acc_fails",
    "acc_tries",
    "acc_waits",
];

/// Return the map of `pairs`, whose keys are atoms.
fn map(scheduler: &Scheduler, pairs: Vec<(Term, Term)>) -> Term {
    // ponytail: atom keys fit each map form, so `from_pairs` cannot fail.
    Term::Map(Map::from_pairs(pairs, &scheduler.atoms).expect("atom keys fit each map form"))
}

/// Return the pairs of `names` with their values as atoms keys.
fn pairs(
    scheduler: &mut Scheduler,
    entries: &[(&str, Term)],
) -> Result<Vec<(Term, Term)>, BeamError> {
    entries
        .iter()
        .map(|(name, value)| Ok((atom(scheduler, name)?, value.clone())))
        .collect()
}

/// Return the atom `true` or `false`.
fn boolean(value: bool) -> Term {
    Term::static_atom(if value { "true" } else { "false" })
}

/// Return the unsigned integer term of a counter.
fn count(value: u64) -> Term {
    Term::Integer(i64::try_from(value).unwrap_or(i64::MAX))
}

/// Run `prim_socket:nif_info()`, as `esock_global_info` does
/// (`prim_socket_nif.c:4592-4672`). The debug flags are off, `eei` and
/// `use_registry` are on, and `iow` is off, as the defaults of
/// `prim_socket:on_load/1` set them (`prim_socket.erl:137-182`).
/// `iov_max` is the `IOV_MAX` of Linux and macOS, 1024. The I/O backend is
/// `unix_essio` without SCTP (`unix_socket_syncio.c:1341-1395`).
///
/// # Errors
///
/// A new atom in a full atom table gives the error of `AtomTable::put`.
pub(super) fn info_0(scheduler: &mut Scheduler) -> Result<BifResult, BeamError> {
    let counters = lock(&scheduler.node().resources.kind::<Sockets>()).counters;
    let counters = pairs(
        scheduler,
        &[
            ("num_cnt_bits", Term::Integer(64)),
            ("num_sockets", count(counters.sockets)),
            ("num_tdgrams", count(counters.dgrams)),
            ("num_tstreams", count(counters.streams)),
            ("num_tseqpkgs", count(counters.seqpackets)),
            ("num_dlocal", Term::Integer(0)),
            ("num_dinet", count(counters.inet)),
            ("num_dinet6", count(counters.inet6)),
            ("num_pip", count(counters.ip)),
            ("num_ptcp", count(counters.tcp)),
            ("num_pudp", count(counters.udp)),
            ("num_psctp", count(counters.sctp)),
        ],
    )?;
    let counters = map(scheduler, counters);
    let name = atom(scheduler, "unix_essio")?;
    let backend = pairs(
        scheduler,
        &[("name", name), ("sctp", Term::static_atom("undefined"))],
    )?;
    let backend = map(scheduler, backend);
    let info = pairs(
        scheduler,
        &[
            ("debug", boolean(false)),
            ("socket_debug", boolean(false)),
            ("eei", boolean(true)),
            ("use_registry", boolean(true)),
            ("iow", boolean(false)),
            ("counters", counters),
            ("iov_max", Term::Integer(1024)),
            ("io_backend", backend),
        ],
    )?;
    Ok(BifResult::Value(map(scheduler, info)))
}

/// Return the socket of the resource term `socket`, or `None` for a term
/// that is not a socket resource, which raises `error:badarg`.
fn socket_argument(
    scheduler: &Scheduler,
    socket: &Term,
) -> Option<(Reference, Arc<Mutex<SocketState>>)> {
    let Term::Reference(reference) = socket else {
        return None;
    };
    let state = lock(&scheduler.node().resources.kind::<Sockets>()).get(*reference)?;
    Some((*reference, state))
}

/// Return the list of the atoms of the state bits `state`, as
/// `esock_socket_info_state` does (`prim_socket_nif.c:4815-4904`).
fn states(scheduler: &mut Scheduler, state: u32) -> Result<Term, BeamError> {
    let mut names = Vec::new();
    for (bit, name) in [(BOUND, "bound"), (CLOSING, "closing"), (CLOSED, "closed")] {
        if state & bit != 0 {
            names.push(atom(scheduler, name)?);
        }
    }
    Ok(Term::list_from(names))
}

/// Return the domain term of `domain`, as `esock_encode_domain` does
/// (`socket_util.c:2229-2259`).
fn domain_term(scheduler: &mut Scheduler, domain: i32) -> Result<Term, BeamError> {
    match domain {
        libc::AF_INET => atom(scheduler, "inet"),
        libc::AF_INET6 => atom(scheduler, "inet6"),
        libc::AF_UNIX => atom(scheduler, "local"),
        libc::AF_UNSPEC => atom(scheduler, "unspec"),
        other => Ok(Term::Integer(other.into())),
    }
}

/// Run `prim_socket:nif_info(Socket)`, as `esock_socket_info` does
/// (`prim_socket_nif.c:4689-4743`). PBeam has no reader, writer, or
/// acceptor queue yet, so each of these numbers is 0.
///
/// # Errors
///
/// A new atom in a full atom table gives the error of `AtomTable::put`.
pub(super) fn info_1(scheduler: &mut Scheduler, socket: &Term) -> Result<BifResult, BeamError> {
    let Some((_, socket)) = socket_argument(scheduler, socket) else {
        return Ok(BifResult::badarg());
    };
    let socket = lock(&socket);
    let domain = domain_term(scheduler, socket.domain)?;
    let kind = match socket.kind {
        libc::SOCK_STREAM => atom(scheduler, "stream")?,
        libc::SOCK_DGRAM => atom(scheduler, "dgram")?,
        libc::SOCK_RAW => atom(scheduler, "raw")?,
        libc::SOCK_SEQPACKET => atom(scheduler, "seqpacket")?,
        libc::SOCK_RDM => atom(scheduler, "rdm")?,
        other => Term::Integer(other.into()),
    };
    let owner = socket
        .owner
        .map_or(Term::static_atom("undefined"), Term::Pid);
    let read = states(scheduler, socket.read)?;
    let write = states(scheduler, socket.write)?;
    let counters = SOCKET_COUNTERS
        .iter()
        .map(|name| (*name, Term::Integer(0)))
        .collect::<Vec<_>>();
    let counters = pairs(scheduler, &counters)?;
    let counters = map(scheduler, counters);
    let normal = atom(scheduler, "normal")?;
    let info = pairs(
        scheduler,
        &[
            ("domain", domain),
            ("type", kind),
            ("protocol", Term::Integer(socket.protocol.into())),
            ("owner", owner),
            ("rstates", read),
            ("wstates", write),
            ("ctype", normal),
            ("counters", counters),
            ("num_readers", Term::Integer(0)),
            ("num_writers", Term::Integer(0)),
            ("num_acceptors", Term::Integer(0)),
        ],
    )?;
    Ok(BifResult::Value(map(scheduler, info)))
}

/// Return the `int` of a term as `enif_get_int` gives it.
fn int(term: &Term) -> Option<i32> {
    match term {
        Term::Integer(value) => i32::try_from(*value).ok(),
        _ => None,
    }
}

/// Return the `unsigned int` of a term as `enif_get_uint` gives it.
fn uint(term: &Term) -> Option<u32> {
    match term {
        Term::Integer(value) => u32::try_from(*value).ok(),
        _ => None,
    }
}

/// Return the name of the atom `term`, or `None` for another term.
fn atom_name<'a>(scheduler: &'a Scheduler, term: &Term) -> Option<&'a str> {
    match term {
        Term::Atom(atom) => Some(scheduler.atoms.name_of(atom)),
        _ => None,
    }
}

/// Return the result `{invalid, Reason}` of `esock_make_invalid`
/// (`socket_util.c:3336-3340`).
fn invalid(reason: Term) -> BifResult {
    BifResult::Value(Term::tuple(vec![Term::static_atom("invalid"), reason]))
}

/// Return `{error, Errno}` of a failed socket call, as
/// `esock_make_error_errno` does.
fn errno_error(scheduler: &mut Scheduler, failure: &io::Error) -> Result<BifResult, BeamError> {
    let name = atom(scheduler, &errno_name(failure))?;
    Ok(BifResult::Value(Term::tuple(vec![
        Term::static_atom("error"),
        name,
    ])))
}

/// Return `{error, closed}` of `esock_make_error_closed`.
fn closed() -> BifResult {
    BifResult::Value(Term::tuple(vec![
        Term::static_atom("error"),
        Term::static_atom("closed"),
    ]))
}

/// Run `prim_socket:nif_open(Domain, Type, Protocol, Opts)`, as `nif_open`
/// (`prim_socket_nif.c:5733-5820`) and `essio_open_plain`
/// (`unix_socket_syncio.c:1689-1798`) do. An integer `Protocol` outside
/// the `int` range gives `{invalid, {integer_range, Protocol}}`. ERTS
/// raises `error:badarg` for a `Protocol` that is not an integer or `Opts`
/// that is not a map; `nif_open/4` is not exported, and
/// `prim_socket:open/4` always gives it an integer and a map
/// (`prim_socket.erl:432-457`), so PBeam has no such branch. A `Domain` or `Type` that `esock_decode_domain` or
/// `esock_decode_type` rejects gives `{invalid, domain}` or
/// `{invalid, type}` (`socket_util.c:2180-2214`, `2368-2405`). A failed
/// `socket(2)` gives `{error, Errno}`. Protocol 0 takes the protocol of
/// `SO_PROTOCOL` on Linux; macOS has no `SO_PROTOCOL`, so it stays 0
/// (`prim_socket_nif.c:5840-5847`). The socket registry gets
/// `{'$socket', add, {'$socket', Ref}}` when the option `use_registry` is
/// not `false` (`unix_socket_syncio.c:1795`, `prim_socket_nif.c:17551-17558`).
///
/// # Errors
///
/// In a run with a policy, an open by a runtime process stops the run: no
/// capability permits a network effect yet, as for an inet port. A new atom
/// in a full atom table gives the error of `AtomTable::put`.
pub(super) fn open_4(
    scheduler: &mut Scheduler,
    arguments: &[Term],
) -> Result<BifResult, BeamError> {
    let [domain, kind, protocol, options] = arguments else {
        unreachable!("nif_open/4 has four arguments")
    };
    let Some(protocol) = int(protocol) else {
        let range = Term::tuple(vec![atom(scheduler, "integer_range")?, protocol.clone()]);
        return Ok(invalid(range));
    };
    let domain = match atom_name(scheduler, domain) {
        Some("inet") => libc::AF_INET,
        Some("inet6") => libc::AF_INET6,
        Some("local") => libc::AF_UNIX,
        Some("unspec") => libc::AF_UNSPEC,
        _ => match int(domain) {
            Some(domain) => domain,
            None => return Ok(invalid(atom(scheduler, "domain")?)),
        },
    };
    let kind = match atom_name(scheduler, kind) {
        Some("stream") => libc::SOCK_STREAM,
        Some("dgram") => libc::SOCK_DGRAM,
        Some("raw") => libc::SOCK_RAW,
        Some("seqpacket") => libc::SOCK_SEQPACKET,
        _ => match int(kind) {
            Some(kind) => kind,
            None => return Ok(invalid(Term::static_atom("type"))),
        },
    };
    if !scheduler.skips_checks() && !scheduler.current_process().security.is_boot() {
        return Err(error(
            "unsupported prim_socket:nif_open/4 in a run with a policy: no capability permits a network effect: see \"NIF stubs\" in docs/erts-differences.md",
        ));
    }
    let socket = match Socket::new(
        Domain::from(domain),
        Type::from(kind),
        Some(Protocol::from(protocol)),
    ) {
        Ok(socket) => socket,
        Err(failure) => return errno_error(scheduler, &failure),
    };
    let protocol = which_protocol(&socket, protocol);
    socket
        .set_nonblocking(true)
        .map_err(|failure| error(format!("prim_socket:nif_open/4: {failure}")))?;
    let registered = field(scheduler, options, "use_registry")
        .is_none_or(|value| *value != Term::static_atom("false"));
    let owner = scheduler.current_process().id;
    let reference = scheduler.make_magic_ref()?;
    let sockets = scheduler.node().resources.kind::<Sockets>();
    let registry = {
        let mut sockets = lock(&sockets);
        sockets.counters.count(domain, kind, protocol, 1);
        sockets.insert(
            reference,
            SocketState {
                socket: Some(socket),
                domain,
                kind,
                protocol,
                owner: Some(owner),
                closer: None,
                read: 0,
                write: 0,
                registered,
            },
        );
        sockets.registry
    };
    if registered {
        send_registry(scheduler, registry, "add", reference)?;
    }
    Ok(BifResult::Value(Term::tuple(vec![
        Term::static_atom("ok"),
        Term::Reference(reference),
    ])))
}

/// Return the protocol of a socket that `nif_open/4` opened with
/// `protocol`, as `esock_open_which_protocol` gives it
/// (`prim_socket_nif.c:5840-5847`).
#[cfg(any(target_os = "linux", target_os = "android"))]
fn which_protocol(socket: &Socket, protocol: i32) -> i32 {
    if protocol != 0 {
        return protocol;
    }
    socket.protocol().ok().flatten().map_or(protocol, i32::from)
}

/// Return the protocol of a socket that `nif_open/4` opened with
/// `protocol`. The host has no `SO_PROTOCOL`, so the protocol stays as
/// given (`prim_socket_nif.c:5840-5847`).
#[cfg(not(any(target_os = "linux", target_os = "android")))]
fn which_protocol(_socket: &Socket, protocol: i32) -> i32 {
    protocol
}

/// Send `{'$socket', Tag, {'$socket', Ref}}` from the caller to the socket
/// registry, as `mk_reg_msg` and `esock_send_msg` do
/// (`prim_socket_nif.c:17300-17338`, `17551-17558`).
fn send_registry(
    scheduler: &mut Scheduler,
    registry: Option<ProcessId>,
    tag: &str,
    reference: Reference,
) -> Result<(), BeamError> {
    let Some(registry) = registry else {
        return Ok(());
    };
    let socket_tag = atom(scheduler, "$socket")?;
    let socket = Term::tuple(vec![socket_tag.clone(), Term::Reference(reference)]);
    let message = Term::tuple(vec![socket_tag, Term::static_atom(tag), socket]);
    let caller = scheduler.current_process().id;
    scheduler.send_message(caller, registry, message);
    Ok(())
}

/// Return the address of a `sockaddr` map, as `esock_decode_sockaddr`
/// does (`socket_util.c:484-548`): `inet` (`socket_util.c:1118-1166`,
/// `1782-1843`), `inet6` (`socket_util.c:1239-1310`, `1896-1967`), and
/// `local` (`socket_util.c:1394-1440`). `None` gives
/// `{invalid, sockaddr}`. The port is an `int` that `htons` truncates to 16
/// bits. `prim_socket:bind/2` gives only a map with a family, and it adds
/// each other key of the family (`prim_socket.erl:1109-1155`).
///
/// # Errors
///
/// The family `unspec` or a native family number stops the run: PBeam does
/// not decode a native `sockaddr` yet.
fn sockaddr(scheduler: &Scheduler, address: &Term) -> Result<Option<SockAddr>, BeamError> {
    let get = |name: &str| field(scheduler, address, name);
    let family = get("family");
    let port = || get("port").and_then(int).map(|port| port as u16);
    match family.and_then(|family| atom_name(scheduler, family)) {
        Some("inet") => {
            let address = get("addr").and_then(|address| in_addr(scheduler, address));
            Ok(port()
                .zip(address)
                .map(|(port, address)| SockAddr::from(SocketAddrV4::new(address, port))))
        }
        Some("inet6") => {
            let flowinfo = get("flowinfo").and_then(uint);
            let scope_id = get("scope_id").and_then(uint);
            let address = get("addr").and_then(|address| in6_addr(scheduler, address));
            Ok(match (port(), flowinfo, scope_id, address) {
                (Some(port), Some(flowinfo), Some(scope_id), Some(address)) => Some(
                    SockAddr::from(SocketAddrV6::new(address, port, flowinfo, scope_id)),
                ),
                _ => None,
            })
        }
        Some("local") => {
            let path = get("path").and_then(Term::binary_bytes).unwrap_or_default();
            let terminator = usize::from(!(cfg!(target_os = "linux") && path.first() == Some(&0)));
            if path.len() + terminator > SUN_PATH {
                return Ok(None);
            }
            Ok(SockAddr::unix(std::ffi::OsStr::from_bytes(path)).ok())
        }
        Some("unspec") => Err(native_family()),
        None if matches!(family, Some(Term::Integer(_) | Term::BigInt(_))) => Err(native_family()),
        _ => Ok(None),
    }
}

/// Return the value of the atom key `name` of the map `map`, or `None`
/// for a map without the key or a term that is not a map.
fn field<'a>(scheduler: &Scheduler, map: &'a Term, name: &str) -> Option<&'a Term> {
    let key = Term::Atom(scheduler.atoms.get(name)?);
    match map {
        Term::Map(map) => map.get(&key),
        _ => None,
    }
}

/// Return the runtime error of a native `sockaddr`.
fn native_family() -> BeamError {
    error(
        "unsupported prim_socket sockaddr of the family unspec or of a native family number: see \"NIF stubs\" in docs/erts-differences.md",
    )
}

/// Return the IPv4 address of `addr`, as `esock_decode_in_addr` does
/// (`socket_util.c:1782-1843`).
fn in_addr(scheduler: &Scheduler, address: &Term) -> Option<Ipv4Addr> {
    match atom_name(scheduler, address) {
        Some("loopback") => return Some(Ipv4Addr::LOCALHOST),
        Some("any") => return Some(Ipv4Addr::UNSPECIFIED),
        Some("broadcast") => return Some(Ipv4Addr::BROADCAST),
        Some(_) => return None,
        None => {}
    }
    let [a, b, c, d] = address.tuple_elements()? else {
        return None;
    };
    let byte = |term: &Term| int(term).and_then(|value| u8::try_from(value).ok());
    Some(Ipv4Addr::new(byte(a)?, byte(b)?, byte(c)?, byte(d)?))
}

/// Return the IPv6 address of `addr`, as `esock_decode_in6_addr` does
/// (`socket_util.c:1896-1967`).
fn in6_addr(scheduler: &Scheduler, address: &Term) -> Option<Ipv6Addr> {
    match atom_name(scheduler, address) {
        Some("loopback") => return Some(Ipv6Addr::LOCALHOST),
        Some("any") => return Some(Ipv6Addr::UNSPECIFIED),
        Some(_) => return None,
        None => {}
    }
    let elements = address.tuple_elements()?;
    let segments: [u16; 8] = elements
        .iter()
        .map(|term| int(term).and_then(|value| u16::try_from(value).ok()))
        .collect::<Option<Vec<_>>>()?
        .try_into()
        .ok()?;
    Some(Ipv6Addr::from(segments))
}

/// Run `prim_socket:nif_bind(Socket, Addr)`, as `nif_bind`
/// (`prim_socket_nif.c:5872-5917`) and `essio_bind`
/// (`unix_socket_syncio.c:1914-1938`) do: `badarg` for a term that is not
/// a socket, `{invalid, sockaddr}` for an address that does not decode,
/// `{error, closed}` for a closing socket, `{error, Errno}` of `bind(2)`,
/// and `ok`, which adds the read state `bound`.
///
/// # Errors
///
/// A native `sockaddr` stops the run (see [`sockaddr`]). A new atom in a
/// full atom table gives the error of `AtomTable::put`.
pub(super) fn bind_2(
    scheduler: &mut Scheduler,
    socket: &Term,
    address: &Term,
) -> Result<BifResult, BeamError> {
    let Some((_, socket)) = socket_argument(scheduler, socket) else {
        return Ok(BifResult::badarg());
    };
    let Some(address) = sockaddr(scheduler, address)? else {
        return Ok(invalid(atom(scheduler, "sockaddr")?));
    };
    let mut socket = lock(&socket);
    if socket.read & (CLOSING | CLOSED) != 0 {
        return Ok(closed());
    }
    let host = socket
        .socket
        .as_ref()
        .expect("an open socket has a host socket");
    if let Err(failure) = host.bind(&address) {
        return errno_error(scheduler, &failure);
    }
    socket.read |= BOUND;
    Ok(BifResult::Value(Term::static_atom("ok")))
}

/// Return the `sockaddr` map of `address`, as `esock_encode_sockaddr`
/// does (`socket_util.c:726-800`, `1182-1218`, `1345-1374`, `1458-1510`).
fn sockaddr_term(scheduler: &mut Scheduler, address: &SockAddr) -> Result<Term, BeamError> {
    let family = domain_term(scheduler, i32::from(address.family()))?;
    let entries = match address.as_socket() {
        Some(std::net::SocketAddr::V4(address)) => {
            let [a, b, c, d] = address.ip().octets().map(|byte| Term::Integer(byte.into()));
            vec![
                ("family", family),
                ("port", Term::Integer(address.port().into())),
                ("addr", Term::tuple(vec![a, b, c, d])),
            ]
        }
        Some(std::net::SocketAddr::V6(address)) => {
            let segments = address
                .ip()
                .segments()
                .map(|segment| Term::Integer(segment.into()));
            vec![
                ("family", family),
                ("port", Term::Integer(address.port().into())),
                ("addr", Term::tuple(segments.to_vec())),
                ("flowinfo", Term::Integer(address.flowinfo().into())),
                ("scope_id", Term::Integer(address.scope_id().into())),
            ]
        }
        None => vec![
            ("family", family),
            ("path", Term::binary(local_path(address))),
        ],
    };
    let entries = pairs(scheduler, &entries)?;
    Ok(map(scheduler, entries))
}

/// Return the path bytes of a local address, as
/// `esock_encode_sockaddr_un` gives them (`socket_util.c:1458-1510`): the
/// bytes before the first NUL, and on Linux each byte of an abstract name
/// with its leading NUL.
fn local_path(address: &SockAddr) -> Vec<u8> {
    #[cfg(any(target_os = "linux", target_os = "android"))]
    if let Some(name) = address.as_abstract_namespace() {
        return [&[0], name].concat();
    }
    address
        .as_pathname()
        .map(|path| path.as_os_str().as_encoded_bytes().to_vec())
        .unwrap_or_default()
}

/// Run `prim_socket:nif_sockname(Socket)`, as `nif_sockname` and
/// `esock_sockname` do (`prim_socket_nif.c:13468-13545`): `badarg` for a
/// term that is not a socket, `{error, closed}` for a closing socket,
/// `{error, Errno}` of `getsockname(2)`, and `{ok, SockAddr}`.
///
/// # Errors
///
/// A new atom in a full atom table gives the error of `AtomTable::put`.
pub(super) fn sockname(scheduler: &mut Scheduler, socket: &Term) -> Result<BifResult, BeamError> {
    let Some((_, socket)) = socket_argument(scheduler, socket) else {
        return Ok(BifResult::badarg());
    };
    let socket = lock(&socket);
    if socket.read & (CLOSING | CLOSED) != 0 {
        return Ok(closed());
    }
    let host = socket
        .socket
        .as_ref()
        .expect("an open socket has a host socket");
    match host.local_addr() {
        Ok(address) => {
            let address = sockaddr_term(scheduler, &address)?;
            Ok(BifResult::Value(Term::tuple(vec![
                Term::static_atom("ok"),
                address,
            ])))
        }
        Err(failure) => errno_error(scheduler, &failure),
    }
}

/// Run `prim_socket:nif_close(Socket)`, as `essio_close` does
/// (`unix_socket_syncio.c:4544-4594`): `badarg` for a term that is not a
/// socket, `{error, closed}` for a closing socket, and `ok`, which keeps the
/// caller as the closer and adds the state `closing`. PBeam runs no select,
/// so `do_stop` schedules no stop and the result is `ok`.
pub(super) fn close(scheduler: &mut Scheduler, socket: &Term) -> BifResult {
    let Some((_, socket)) = socket_argument(scheduler, socket) else {
        return BifResult::badarg();
    };
    let mut socket = lock(&socket);
    if socket.read & (CLOSING | CLOSED) != 0 {
        return closed();
    }
    socket.closer = Some(scheduler.current_process().id);
    socket.read |= CLOSING;
    socket.write |= CLOSING;
    BifResult::Value(Term::static_atom("ok"))
}

/// Run `prim_socket:nif_finalize_close(Socket)`, as `essio_fin_close` and
/// `esock_close_socket` do (`unix_socket_syncio.c:4772-4853`,
/// `prim_socket_nif.c:7648-7705`): `badarg` for a term that is not a
/// socket, `{error, closed}` for a closed socket, `error:{invalid, state}`
/// before `nif_close/1` or from a process that is not the closer, and
/// `ok`, which closes the host socket, adds the state `closed`, clears the
/// owner, counts the socket out, and sends `{'$socket', del, Socket}` to the
/// registry.
///
/// # Errors
///
/// A new atom in a full atom table gives the error of `AtomTable::put`.
pub(super) fn finalize_close(
    scheduler: &mut Scheduler,
    socket: &Term,
) -> Result<BifResult, BeamError> {
    let Some((reference, socket)) = socket_argument(scheduler, socket) else {
        return Ok(BifResult::badarg());
    };
    let caller = scheduler.current_process().id;
    let (registered, counted) = {
        let mut socket = lock(&socket);
        if socket.read & CLOSED != 0 {
            return Ok(closed());
        }
        if socket.read & CLOSING == 0 || socket.closer != Some(caller) {
            let state = Term::tuple(vec![
                Term::static_atom("invalid"),
                atom(scheduler, "state")?,
            ]);
            return Ok(BifResult::Raise {
                class: ExceptionClass::Error,
                reason: state,
            });
        }
        socket.closer = None;
        socket.owner = None;
        socket.socket = None;
        socket.read |= CLOSED;
        socket.write |= CLOSED;
        (
            socket.registered,
            (socket.domain, socket.kind, socket.protocol),
        )
    };
    let sockets = scheduler.node().resources.kind::<Sockets>();
    let registry = {
        let mut sockets = lock(&sockets);
        let (domain, kind, protocol) = counted;
        sockets.counters.count(domain, kind, protocol, -1);
        sockets.registry
    };
    if registered {
        send_registry(scheduler, registry, "del", reference)?;
    }
    Ok(BifResult::Value(Term::static_atom("ok")))
}

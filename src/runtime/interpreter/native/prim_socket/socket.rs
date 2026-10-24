//! The bindings of `prim_socket:nif_info/0,1`, `nif_open/4`, `nif_bind/2`,
//! `nif_sockname/1`, `nif_close/1`, and `nif_finalize_close/1`, and of
//! `nif_setopt/5` and `nif_getopt/3,5` (see `options`)
//! (`prim_socket_nif.c`), with the Unix I/O backend `unix_essio`
//! (`unix_socket_syncio.c`).
//!
//! A socket resource is a reference to a `SocketState` in the `Sockets`
//! value of the node resources, so a socket does not pin its process to
//! one scheduler. The selects of a connect and an accept are in the
//! sibling module `select`.

use std::io;
use std::net::{Ipv4Addr, Ipv6Addr, Shutdown, SocketAddrV4, SocketAddrV6};
use std::os::unix::ffi::OsStrExt;
use std::sync::{Arc, Mutex};

use nix::sys::socket::{SockaddrStorage, UnixAddr};
use socket2::{Domain, Protocol, SockAddr, Socket, Type};

mod options;

use super::supports::atom;
use super::*;
use crate::runtime::port::inet::errno_name;
use crate::runtime::resources::lock;
use crate::runtime::signal::Signal;
use crate::runtime::sockets::{
    ACCEPTING, BOUND, CLOSED, CLOSING, CONNECTED, CONNECTING, IoCounters, LISTENING, SELECTED,
    SocketState, Sockets,
};
use crate::term::{AtomTable, ProcessId, Reference};
pub(super) use options::{getopt_3, getopt_5, setopt_5};

/// The size of `sun_path` of `struct sockaddr_un` on the host.
#[cfg(target_vendor = "apple")]
const SUN_PATH: usize = 104;
/// The size of `sun_path` of `struct sockaddr_un` on the host.
#[cfg(not(target_vendor = "apple"))]
const SUN_PATH: usize = 108;

/// Return the socket counters of `esock_socket_info_counters`
/// (`prim_socket_nif.c:4920-4990`) in its key order. PBeam does not count
/// the accepts yet, so each `acc_` counter is 0.
fn socket_counters(counters: &IoCounters) -> [(&'static str, Term); 16] {
    [
        ("read_byte", count(counters.read_byte)),
        ("read_fails", count(counters.read_fails)),
        ("read_pkg", count(counters.read_pkg)),
        ("read_pkg_max", count(counters.read_pkg_max)),
        ("read_tries", count(counters.read_tries)),
        ("read_waits", Term::Integer(0)),
        ("write_byte", count(counters.write_byte)),
        ("write_fails", count(counters.write_fails)),
        ("write_pkg", count(counters.write_pkg)),
        ("write_pkg_max", count(counters.write_pkg_max)),
        ("write_tries", count(counters.write_tries)),
        ("write_waits", count(counters.write_waits)),
        ("acc_success", Term::Integer(0)),
        ("acc_fails", Term::Integer(0)),
        ("acc_tries", Term::Integer(0)),
        ("acc_waits", Term::Integer(0)),
    ]
}

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
    let counters = [
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
    ];
    let counters = pairs(scheduler, &counters)?;
    let counters = map(scheduler, counters);
    let name = atom(scheduler, "unix_essio")?;
    let backend = [("name", name), ("sctp", Term::static_atom("undefined"))];
    let backend = pairs(scheduler, &backend)?;
    let backend = map(scheduler, backend);
    let info = [
        ("debug", boolean(false)),
        ("socket_debug", boolean(false)),
        ("eei", boolean(true)),
        ("use_registry", boolean(true)),
        ("iow", boolean(false)),
        ("counters", counters),
        ("iov_max", Term::Integer(1024)),
        ("io_backend", backend),
    ];
    let info = pairs(scheduler, &info)?;
    Ok(BifResult::Value(map(scheduler, info)))
}

/// Return the socket of the resource term `socket`, or `None` for a term
/// that is not a socket resource, which raises `error:badarg`.
pub(super) fn socket_argument(
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
    let bits = [
        (BOUND, "bound"),
        (LISTENING, "listening"),
        (ACCEPTING, "accepting"),
        (CONNECTING, "connecting"),
        (CONNECTED, "connected"),
        (SELECTED, "selected"),
        (CLOSING, "closing"),
        (CLOSED, "closed"),
    ];
    for (bit, name) in bits {
        if state & bit != 0 {
            names.push(atom(scheduler, name)?);
        }
    }
    Ok(Term::list_from(names))
}

/// Return the domain term of `domain`, as `esock_encode_domain` does
/// (`socket_util.c:2229-2259`).
fn domain_term(scheduler: &mut Scheduler, domain: i32) -> Result<Term, BeamError> {
    let names = [
        (libc::AF_INET, "inet"),
        (libc::AF_INET6, "inet6"),
        (libc::AF_UNIX, "local"),
        (libc::AF_UNSPEC, "unspec"),
    ];
    named(scheduler, &names, domain)
}

/// Return the type term of `kind`, as `esock_encode_type` does
/// (`socket_util.c:2420-2452`).
fn type_term(scheduler: &mut Scheduler, kind: i32) -> Result<Term, BeamError> {
    let names = [
        (libc::SOCK_STREAM, "stream"),
        (libc::SOCK_DGRAM, "dgram"),
        (libc::SOCK_RAW, "raw"),
        (libc::SOCK_SEQPACKET, "seqpacket"),
        (libc::SOCK_RDM, "rdm"),
    ];
    named(scheduler, &names, kind)
}

/// Return the atom of the name of `value` in `names`, or the integer
/// `value` for a value without a name.
fn named(scheduler: &mut Scheduler, names: &[(i32, &str)], value: i32) -> Result<Term, BeamError> {
    match names.iter().find(|(number, _)| *number == value) {
        Some((_, name)) => atom(scheduler, name),
        None => Ok(Term::Integer(value.into())),
    }
}

/// Run `prim_socket:nif_info(Socket)`, as `esock_socket_info` does
/// (`prim_socket_nif.c:4689-4743`). PBeam has no reader, writer, or
/// acceptor queue, so `num_readers` and `num_writers` count only the
/// current reader and writer (`socket_info_reqs`,
/// `prim_socket_nif.c:5185-5196`). `num_acceptors` is 0.
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
    let kind = type_term(scheduler, socket.kind)?;
    let owner = socket
        .owner
        .map_or(Term::static_atom("undefined"), Term::Pid);
    let read = states(scheduler, socket.read)?;
    let write = states(scheduler, socket.write)?;
    let counters = pairs(scheduler, &socket_counters(&socket.counters))?;
    let counters = map(scheduler, counters);
    let normal = atom(scheduler, "normal")?;
    let info = [
        ("domain", domain),
        ("type", kind),
        ("protocol", Term::Integer(socket.protocol.into())),
        ("owner", owner),
        ("rstates", read),
        ("wstates", write),
        ("ctype", normal),
        ("counters", counters),
        ("num_readers", Term::Integer(socket.reader.is_some().into())),
        ("num_writers", Term::Integer(socket.writer.is_some().into())),
        ("num_acceptors", Term::Integer(0)),
    ];
    let info = pairs(scheduler, &info)?;
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
pub(super) fn invalid(reason: Term) -> BifResult {
    BifResult::Value(Term::tuple(vec![Term::static_atom("invalid"), reason]))
}

/// Return `{error, Errno}` of a failed socket call, as
/// `esock_make_error_errno` does.
pub(super) fn errno_error(
    scheduler: &mut Scheduler,
    failure: &io::Error,
) -> Result<BifResult, BeamError> {
    let name = atom(scheduler, &errno_name(failure))?;
    Ok(BifResult::Value(Term::tuple(vec![
        Term::static_atom("error"),
        name,
    ])))
}

/// Return `{error, closed}` of `esock_make_error_closed`.
pub(super) fn closed() -> BifResult {
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
/// On Linux, the option `netns` stops the run with a runtime error.
///
/// # Errors
///
/// In a run with a policy, an open by a runtime process stops the run: no
/// capability permits a network effect yet, as for an inet port. A new atom
/// in a full atom table gives the error of `AtomTable::put`.
pub(super) fn open_4(
    scheduler: &mut Scheduler,
    domain: &Term,
    kind: &Term,
    protocol: &Term,
    options: &Term,
) -> Result<BifResult, BeamError> {
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
    // ERTS on Linux enters the network namespace of the option `netns`
    // before `socket(2)` (`unix_socket_syncio.c:1729-1745`); PBeam does not
    // implement it. The macOS build has no `HAVE_SETNS` and ignores it.
    #[cfg(target_os = "linux")]
    if field(&scheduler.atoms, options, "netns").is_some() {
        return Err(error(
            "prim_socket:nif_open/4 option netns is not supported: see \"NIF stubs\" in docs/erts-differences.md",
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
    let registered = field(&scheduler.atoms, options, "use_registry")
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
                meta: Term::static_atom("undefined"),
                connector: None,
                acceptor: None,
                reader: None,
                writer: None,
                counters: IoCounters::default(),
            },
        );
        sockets.registry
    };
    if let (true, Some(registry)) = (registered, registry) {
        send_registry(scheduler, registry, "add", reference)?;
    }
    // The socket monitors its owner (`unix_socket_syncio.c:1785-1788`).
    scheduler
        .send_signal(None, owner, Signal::SocketMonitor { socket: reference })
        .expect("the caller is alive");
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
/// Return `{'$socket', {'$socket', Ref}, Tag, Info}` of the socket `socket`,
/// as `esock_mk_socket_msg` makes it (`prim_socket_nif.c:17640-17680`).
pub(super) fn socket_message(
    scheduler: &mut Scheduler,
    socket: Reference,
    tag: &str,
    info: Term,
) -> Result<Term, BeamError> {
    let socket_tag = atom(scheduler, "$socket")?;
    let socket = Term::tuple(vec![socket_tag.clone(), Term::Reference(socket)]);
    let tag = atom(scheduler, tag)?;
    Ok(Term::tuple(vec![socket_tag, socket, tag, info]))
}

pub(super) fn send_registry(
    scheduler: &mut Scheduler,
    registry: ProcessId,
    tag: &str,
    reference: Reference,
) -> Result<(), BeamError> {
    let socket_tag = atom(scheduler, "$socket")?;
    let socket = Term::tuple(vec![socket_tag.clone(), Term::Reference(reference)]);
    let tag = atom(scheduler, tag)?;
    let message = Term::tuple(vec![socket_tag, tag, socket]);
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
pub(super) fn sockaddr(
    scheduler: &Scheduler,
    address: &Term,
) -> Result<Option<SockAddr>, BeamError> {
    let get = |name: &str| field(&scheduler.atoms, address, name);
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
fn field<'a>(atoms: &AtomTable, map: &'a Term, name: &str) -> Option<&'a Term> {
    let key = Term::Atom(atoms.get(name)?);
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

/// Return the `sockaddr` map of the source address `from` of a received
/// message, as `esock_encode_sockaddr` does (`socket_util.c:726-800`).
/// `None`, the address of length 0 that `recvfrom(2)` gives on a stream
/// socket, has the family `AF_UNSPEC` of the zeroed address and gives
/// `#{family => unspec, addr => <<>>}` (`socket_util.c:777-787`,
/// `2830-2859`).
pub(super) fn source_term(
    scheduler: &mut Scheduler,
    from: Option<&SockaddrStorage>,
) -> Result<Term, BeamError> {
    let Some(from) = from else {
        let family = domain_term(scheduler, libc::AF_UNSPEC)?;
        let addr = Term::binary(Vec::new());
        let entries = pairs(scheduler, &[("family", family), ("addr", addr)])?;
        return Ok(map(scheduler, entries));
    };
    if let Some(address) = from.as_unix_addr() {
        let family = domain_term(scheduler, libc::AF_UNIX)?;
        let path = Term::binary(unix_path(address));
        let entries = pairs(scheduler, &[("family", family), ("path", path)])?;
        return Ok(map(scheduler, entries));
    }
    let address = from
        .as_sockaddr_in()
        .map(|address| SockAddr::from(SocketAddrV4::from(*address)))
        .or_else(|| {
            from.as_sockaddr_in6()
                .map(|address| SockAddr::from(SocketAddrV6::from(*address)))
        })
        .expect("a socket of the family inet, inet6, or local");
    sockaddr_term(scheduler, &address)
}

/// Return the path bytes of the received local address `address`, as
/// `esock_encode_sockaddr_un` gives them (`socket_util.c:1458-1510`): the
/// bytes before the first NUL, and on Linux each byte of an abstract name
/// with its leading NUL.
pub(super) fn unix_path(address: &UnixAddr) -> Vec<u8> {
    #[cfg(any(target_os = "linux", target_os = "android"))]
    if let Some(name) = address.as_abstract() {
        return [&[0], name].concat();
    }
    let path = address
        .path()
        .map(|path| path.as_os_str().as_bytes())
        .unwrap_or_default();
    path.split(|byte| *byte == 0)
        .next()
        .unwrap_or_default()
        .to_vec()
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
    // `getsockname(2)` of an open socket fails only for `ENOBUFS`; PBeam
    // stops the run there, where ERTS gives `{error, enobufs}`.
    let address = host
        .local_addr()
        .map_err(|failure| error(format!("prim_socket:nif_sockname/1: {failure}")))?;
    let address = sockaddr_term(scheduler, &address)?;
    Ok(BifResult::Value(Term::tuple(vec![
        Term::static_atom("ok"),
        address,
    ])))
}

/// Run `prim_socket:nif_peername(Socket)`, as `nif_peername` and
/// `esock_peername` do (`prim_socket_nif.c:13626-13704`): `badarg` for a
/// term that is not a socket, `{error, closed}` for a closing socket,
/// `{error, Errno}` of `getpeername(2)`, and `{ok, SockAddr}`.
///
/// # Errors
///
/// A new atom in a full atom table gives the error of `AtomTable::put`.
pub(super) fn peername(scheduler: &mut Scheduler, socket: &Term) -> Result<BifResult, BeamError> {
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
    let address = match host.peer_addr() {
        Ok(address) => address,
        Err(failure) => return errno_error(scheduler, &failure),
    };
    let address = sockaddr_term(scheduler, &address)?;
    Ok(BifResult::Value(Term::tuple(vec![
        Term::static_atom("ok"),
        address,
    ])))
}

/// Run `prim_socket:nif_shutdown(Socket, How)`, as `nif_shutdown`
/// (`prim_socket_nif.c:7722-7761`), `ehow2how`
/// (`prim_socket_nif.c:17232-17265`), and `esock_shutdown`
/// (`prim_socket_nif.c:7767-7779`) do: `badarg` for a term that is not a
/// socket, the exception `{invalid, {how, How}}` for a `How` other than
/// `read`, `write`, and `read_write`, `{error, closed}` for a closing
/// socket, `{error, Errno}` of `shutdown(2)`, and `ok`.
///
/// # Errors
///
/// A new atom in a full atom table gives the error of `AtomTable::put`.
pub(super) fn shutdown(
    scheduler: &mut Scheduler,
    socket: &Term,
    how: &Term,
) -> Result<BifResult, BeamError> {
    let Some((_, socket)) = socket_argument(scheduler, socket) else {
        return Ok(BifResult::badarg());
    };
    let direction = match atom_name(scheduler, how) {
        Some("read") => Shutdown::Read,
        Some("write") => Shutdown::Write,
        Some("read_write") => Shutdown::Both,
        _ => {
            let reason = Term::tuple(vec![atom(scheduler, "how")?, how.clone()]);
            return Ok(BifResult::Raise {
                class: ExceptionClass::Error,
                reason: Term::tuple(vec![Term::static_atom("invalid"), reason]),
            });
        }
    };
    let socket = lock(&socket);
    if socket.read & (CLOSING | CLOSED) != 0 {
        return Ok(closed());
    }
    let host = socket
        .socket
        .as_ref()
        .expect("an open socket has a host socket");
    match host.shutdown(direction) {
        Ok(()) => Ok(BifResult::Value(Term::static_atom("ok"))),
        Err(failure) => errno_error(scheduler, &failure),
    }
}

/// Run `prim_socket:nif_close(Socket)`, as `essio_close` does
/// (`unix_socket_syncio.c:4544-4594`): `badarg` for a term that is not a
/// socket, `{error, closed}` for a closing socket, and `ok`, which keeps the
/// caller as the closer and adds the state `closing`. `do_stop` cancels the
/// selects at once (`ERL_NIF_SELECT_STOP_CALLED`, as the pinned ERTS gives
/// for a socket of the poll thread), so the result is `ok`, and each
/// requestor whose select was active gets `{'$socket', Socket, abort,
/// {Ref, closed}}` (`unix_socket_syncio.c:4600-4730`).
///
/// # Errors
///
/// A new atom in a full atom table gives the error of `AtomTable::put`.
pub(super) fn close(scheduler: &mut Scheduler, socket: &Term) -> Result<BifResult, BeamError> {
    let Some((reference, socket)) = socket_argument(scheduler, socket) else {
        return Ok(BifResult::badarg());
    };
    let mut socket = lock(&socket);
    if socket.read & (CLOSING | CLOSED) != 0 {
        return Ok(closed());
    }
    let caller = scheduler.current_process().id;
    socket.closer = Some(caller);
    socket.read |= CLOSING;
    socket.write |= CLOSING;
    let owner = socket.owner;
    let aborted = socket.stop(&scheduler.node().poll);
    drop(socket);
    // A closer that is not the owner gets a monitor
    // (`unix_socket_syncio.c:4560-4566`).
    if owner != Some(caller) {
        scheduler
            .send_signal(None, caller, Signal::SocketMonitor { socket: reference })
            .expect("the caller is alive");
    }
    for (requestor, operation) in aborted {
        let info = Term::tuple(vec![operation, Term::static_atom("closed")]);
        let abort = socket_message(scheduler, reference, "abort", info)?;
        scheduler.send_message(caller, requestor, abort);
    }
    Ok(BifResult::Value(Term::static_atom("ok")))
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
    if let (true, Some(registry)) = (registered, registry) {
        send_registry(scheduler, registry, "del", reference)?;
    }
    Ok(BifResult::Value(Term::static_atom("ok")))
}

#[cfg(test)]
#[path = "../../../../../tests/runtime/interpreter/native/prim_socket/socket.rs"]
mod tests;

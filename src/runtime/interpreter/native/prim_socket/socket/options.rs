//! The bindings of `prim_socket:nif_setopt/5` and `nif_getopt/3,5`
//! (`prim_socket_nif.c:7801-7867`, `11504-11585`), with the handlers of
//! the Unix I/O backend: `esock_setopt`, `esock_setopt_otp`,
//! `esock_getopt`, and `esock_getopt_otp` (`prim_socket_nif.c:19063-19065`).
//!
//! PBeam sets and gets a host option through the safe `nix` and `socket2`
//! APIs, which have one typed call for each option. An option without a
//! typed call, and a native value or value spec, go through [`raw_set`]
//! and [`raw_get`], the two owner-approved `unsafe` calls of `setsockopt`
//! and `getsockopt` (#1209).

use nix::errno::Errno;
use nix::sys::socket::{getsockopt, setsockopt, sockopt};

use std::os::fd::AsRawFd;
use std::ptr;

use super::{
    Arc, BeamError, BifResult, CLOSED, CLOSING, ExceptionClass, Ipv4Addr, Mutex, OtpOptions,
    Reference, Scheduler, Signal, Socket, SocketState, Sockets, Term, atom, atom_name, closed,
    decode_domain, domain_term, errno_error, error, field, in_addr, int, invalid, io, lock, map,
    named, pairs, send_registry, socket_argument, type_term, uint,
};
use crate::runtime::interpreter::native::prim_socket::supports::option_name;

/// The number `ESOCK_OPT_OTP_DEBUG` (`prim_socket_nif.c:920`).
const OTP_DEBUG: i32 = 1001;
/// The number `ESOCK_OPT_OTP_IOW` (`prim_socket_nif.c:921`).
const OTP_IOW: i32 = 1002;
/// The number `ESOCK_OPT_OTP_CTRL_PROC` (`prim_socket_nif.c:922`).
const OTP_CTRL_PROC: i32 = 1003;
/// The number `ESOCK_OPT_OTP_RCVBUF` (`prim_socket_nif.c:923`).
const OTP_RCVBUF: i32 = 1004;
/// The number `ESOCK_OPT_OTP_RCVCTRLBUF` (`prim_socket_nif.c:925`).
const OTP_RCVCTRLBUF: i32 = 1006;
/// The number `ESOCK_OPT_OTP_SNDCTRLBUF` (`prim_socket_nif.c:926`).
const OTP_SNDCTRLBUF: i32 = 1007;
/// The number `ESOCK_OPT_OTP_FD` (`prim_socket_nif.c:927`).
const OTP_FD: i32 = 1008;
/// The number `ESOCK_OPT_OTP_META` (`prim_socket_nif.c:928`).
const OTP_META: i32 = 1009;
/// The number `ESOCK_OPT_OTP_USE_REGISTRY` (`prim_socket_nif.c:929`).
const OTP_USE_REGISTRY: i32 = 1010;
/// The number `ESOCK_OPT_OTP_SELECT_READ` (`prim_socket_nif.c:930`).
const OTP_SELECT_READ: i32 = 1011;
/// The number `ESOCK_OPT_OTP_DOMAIN` (`prim_socket_nif.c:932`).
const OTP_DOMAIN: i32 = 1999;

/// The value of a host option that [`get_host`] reads.
enum Got {
    /// The value of `esock_getopt_bool_opt` (`prim_socket_nif.c:13008-13028`).
    Bool(bool),
    /// The value of `esock_getopt_int_opt` (`prim_socket_nif.c:13033-13048`).
    Int(i64),
    /// The type of `esock_getopt_sock_type` (`prim_socket_nif.c:12542-12563`).
    Type(i32),
    /// The value of `esock_getopt_linger` (`prim_socket_nif.c:12482-12530`).
    Linger(libc::linger),
    /// The address of `esock_getopt_multicast_if`
    /// (`prim_socket_nif.c:12638-12669`).
    Addr(Ipv4Addr),
    /// The `int` of `esock_getopt_tos` (`prim_socket_nif.c:12679-12697`).
    Tos(i32),
    /// The bytes of `esock_getopt_str_opt` (`prim_socket_nif.c:13422-13451`).
    Bytes(Vec<u8>),
    /// The `int` of `esock_getopt_ip_mtu_discover` and
    /// `esock_getopt_ipv6_mtu_discover` (`prim_socket_nif.c:12608-12629`,
    /// `12709-12728`).
    Pmtu(i32),
    /// The domain of `esock_getopt_sock_domain`
    /// (`prim_socket_nif.c:12456-12476`).
    #[cfg(target_os = "linux")]
    Domain(i32),
    /// The protocol of `esock_getopt_sock_protocol`
    /// (`prim_socket_nif.c:12569-12600`).
    #[cfg(target_os = "linux")]
    Protocol(i32),
}

/// The value form of a host option that [`set_host`] writes.
#[derive(Clone, Copy)]
enum Form {
    /// `esock_setopt_bool_opt` (`prim_socket_nif.c:11218-11234`).
    Bool,
    /// `esock_setopt_int_opt` (`prim_socket_nif.c:11241-11259`).
    Int,
    /// `esock_setopt_linger` (`prim_socket_nif.c:8551-8597`).
    Linger,
    /// `esock_setopt_uint_opt` (`prim_socket_nif.c:11263-11288`).
    #[cfg(target_os = "linux")]
    Uint,
    /// `esock_setopt_hops` (`prim_socket_nif.c:8941-8954`).
    Hops,
    /// `esock_setopt_multicast_if` (`prim_socket_nif.c:8728-8746`).
    Addr,
    /// `esock_setopt_in_update_membership` (`prim_socket_nif.c:8788-8834`).
    Membership,
    /// `esock_setopt_tos` (`prim_socket_nif.c:8757-8775`).
    Tos,
    /// `esock_setopt_str_opt` with `IFNAMSIZ` (`prim_socket_nif.c:8538-8545`,
    /// `11297-11338`).
    Str,
    /// `esock_setopt_ip_mtu_discover` and `esock_setopt_ipv6_mtu_discover`
    /// (`prim_socket_nif.c:8704-8717`, `8921-8934`).
    Pmtu,
    /// `esock_setopt_in_update_source` (`prim_socket_nif.c:8851-8878`).
    Source,
    /// `esock_setopt_addrform` (`prim_socket_nif.c:8887-8909`).
    Addrform,
    /// `esock_setopt_tcp_congestion` (`prim_socket_nif.c:9018-9027`).
    #[cfg(target_os = "linux")]
    Congestion,
    /// `esock_setopt_in6_update_membership` (`prim_socket_nif.c:8961-9005`).
    #[cfg(target_os = "linux")]
    Membership6,
    /// `esock_setopt_msfilter` (`prim_socket_nif.c:8602-8695`).
    #[cfg(target_os = "linux")]
    Msfilter,
}

/// A decoded value of a host option that [`set_host`] writes.
enum Value {
    /// The C `int` of [`Form::Bool`] and [`Form::Int`].
    Int(i32),
    /// The `struct linger` of [`Form::Linger`].
    Linger(libc::linger),
    /// The `struct in_addr` of [`Form::Addr`].
    Addr(Ipv4Addr),
    /// The `struct ip_mreq` of [`Form::Membership`]: the group and the
    /// interface.
    Membership(Ipv4Addr, Ipv4Addr),
    /// The string of [`Form::Str`] and [`Form::Congestion`], without a NUL
    /// byte.
    Bytes(Vec<u8>),
    /// The `struct ip_mreq_source` of [`Form::Source`].
    Source(libc::ip_mreq_source),
    /// The `struct ipv6_mreq` of [`Form::Membership6`]: the group and the
    /// interface index.
    #[cfg(target_os = "linux")]
    Membership6(Ipv6Addr, u32),
}

/// Return `{invalid, {integer_range, Term}}` of
/// `esock_make_error_integer_range` (`socket_util.c:3320-3324`) for an
/// integer `term` outside the `int` range, and `error:badarg` for each
/// other term (`prim_socket_nif.c:7835-7842`, `7862-7865`).
fn range_or_badarg(scheduler: &mut Scheduler, term: &Term) -> Result<BifResult, BeamError> {
    if term.is_integer() {
        let range = Term::tuple(vec![atom(scheduler, "integer_range")?, term.clone()]);
        Ok(invalid(range))
    } else {
        Ok(BifResult::badarg())
    }
}

/// Return the level number of `level`, as `esock_decode_level` gives it
/// (`socket_util.c:3037-3046`): the atom `socket` is `SOL_SOCKET`.
fn level_number(scheduler: &Scheduler, level: &Term) -> Option<i32> {
    match atom_name(scheduler, level) {
        Some("socket") => Some(libc::SOL_SOCKET),
        _ => int(level),
    }
}

/// Return the runtime error of an option or a form that PBeam does not
/// have yet.
fn unsupported(what: &str) -> BeamError {
    error(format!(
        "unsupported prim_socket {what}: see \"NIF stubs\" in docs/erts-differences.md"
    ))
}

/// Return the result of a host call: `ok` or `{ok, Value}`, or
/// `{error, Errno}` (`prim_socket_nif.c:11384-11395`).
fn host_result(
    scheduler: &mut Scheduler,
    result: io::Result<Option<Term>>,
) -> Result<BifResult, BeamError> {
    match result {
        Ok(None) => Ok(BifResult::Value(Term::static_atom("ok"))),
        Ok(Some(value)) => Ok(BifResult::Value(Term::tuple(vec![
            Term::static_atom("ok"),
            value,
        ]))),
        Err(failure) => errno_error(scheduler, &failure),
    }
}

/// Run `prim_socket:nif_setopt(Socket, Level, Opt, Value, NativeValue)`,
/// as `nif_setopt` does (`prim_socket_nif.c:7801-7867`).
///
/// # Errors
///
/// An option without a handler in PBeam, or a native value, stops the run.
/// A new atom in a full atom table gives the error of `AtomTable::put`.
pub(in crate::runtime::interpreter::native::prim_socket) fn setopt_5(
    scheduler: &mut Scheduler,
    arguments: &[Term],
) -> Result<BifResult, BeamError> {
    let [socket, level, opt, value, native] = [
        &arguments[0],
        &arguments[1],
        &arguments[2],
        &arguments[3],
        &arguments[4],
    ];
    let (Some((reference, state)), Some(native)) =
        (socket_argument(scheduler, socket), int(native))
    else {
        return Ok(BifResult::badarg());
    };
    let Some(opt) = int(opt) else {
        return range_or_badarg(scheduler, opt);
    };
    if let (Some("otp"), 0) = (atom_name(scheduler, level), native) {
        return setopt_otp(scheduler, reference, &state, opt, value);
    }
    let Some(level) = level_number(scheduler, level) else {
        return range_or_badarg(scheduler, level);
    };
    let socket = lock(&state);
    if socket.write & (CLOSING | CLOSED) != 0 {
        return Ok(closed());
    }
    let host = socket
        .socket
        .as_ref()
        .expect("an open socket has a host socket");
    if native != 0 {
        return setopt_native(scheduler, host, level, opt, value);
    }
    // `prim_socket:setopt/3` gives a value form only with the numbers of
    // `nif_supports(options)` (prim_socket.erl:1273-1280), and the native
    // forms return above.
    let name = option_name(level, opt).expect("prim_socket gives a host option");
    // Each host option without a set form has no set handler in ERTS:
    // `{socket, type}`, and on Linux `{ip, mtu}`, `{ip, pktoptions}`,
    // `{socket, domain}`, and `{socket, protocol}` (`prim_socket_nif.c:3272-3274`,
    // `3377-3379`, `3481-3488`, `3580-3586`, `3654-3660`, `8512-8514`).
    let Some(form) = set_form(name) else {
        return Ok(invalid(atom(scheduler, "socket_option")?));
    };
    let decoded = match form {
        Form::Bool => decode_bool(scheduler, value).map(|flag| Value::Int(flag.into())),
        Form::Int => int(value).map(Value::Int),
        Form::Linger => decode_linger(scheduler, value).map(Value::Linger),
        // The `unsigned int` reaches the host with the same bits.
        #[cfg(target_os = "linux")]
        Form::Uint => value
            .bigint()
            .and_then(|value| u32::try_from(value.as_ref()).ok())
            .map(|value| Value::Int(value.cast_signed())),
        Form::Hops => decode_hops(scheduler, value).map(Value::Int),
        Form::Addr => in_addr(scheduler, value).map(Value::Addr),
        Form::Membership => decode_membership(scheduler, value),
        Form::Tos => decode_tos(scheduler, value).map(Value::Int),
        Form::Str => decode_str(value, libc::IFNAMSIZ).map(Value::Bytes),
        Form::Pmtu => decode_pmtu(scheduler, value).map(Value::Int),
        Form::Source => decode_source(scheduler, value).map(Value::Source),
        Form::Addrform => decode_domain(scheduler, value).map(Value::Int),
        #[cfg(target_os = "linux")]
        Form::Congestion => decode_str(value, CONGESTION_NAME_MAX + 1).map(Value::Bytes),
        #[cfg(target_os = "linux")]
        Form::Membership6 => decode_membership6(scheduler, value),
        #[cfg(target_os = "linux")]
        Form::Msfilter => decode_msfilter(scheduler, value).map(Value::Bytes),
    };
    let Some(decoded) = decoded else {
        return Ok(invalid(atom(scheduler, "value")?));
    };
    let result = set_host(host, name, (level, opt), decoded).map(|()| None);
    host_result(scheduler, result)
}

/// Set the option `opt` at the level `level` of `host` to a native value,
/// as `esock_setopt_native` does (`prim_socket_nif.c:8444-8456`): the
/// bytes of iodata (`GET_BIN` is `enif_inspect_iolist_as_binary`,
/// `socket_int.h:880-881`), or the C `int` of an integer or a boolean.
fn setopt_native(
    scheduler: &mut Scheduler,
    host: &Socket,
    level: i32,
    opt: i32,
    value: &Term,
) -> Result<BifResult, BeamError> {
    let iodata = crate::runtime::interpreter::bif::iodata_bytes(value);
    let bytes = match (iodata, int(value), decode_bool(scheduler, value)) {
        (Some(bytes), _, _) => bytes,
        (None, Some(integer), _) => integer.to_ne_bytes().to_vec(),
        (None, None, Some(flag)) => i32::from(flag).to_ne_bytes().to_vec(),
        // `prim_socket:setopt_common/4` does not rewrite the error of
        // `esock_make_error_invalid` (`prim_socket_nif.c:8455`).
        (None, None, None) => return error_invalid(scheduler, "value"),
    };
    let result = keep_tos_and_priority(host, opt, || raw_set(host, level, opt, bytes.as_slice()));
    host_result(scheduler, result.map(|()| None))
}

/// Return the form of the host option `name` that PBeam sets, or `None`.
fn set_form(name: (&str, &str)) -> Option<Form> {
    match name {
        (
            "socket",
            "broadcast" | "keepalive" | "reuseaddr" | "reuseport" | "oobinline" | "dontroute"
            | "timestamp" | "acceptconn",
        )
        | ("tcp", "nodelay")
        | ("ip", "multicast_loop" | "recvtos" | "hdrincl")
        | ("ipv6", "v6only" | "recvtclass" | "multicast_loop") => Some(Form::Bool),
        #[cfg(any(target_os = "linux", target_os = "macos"))]
        ("ip", "pktinfo") => Some(Form::Bool),
        #[cfg(target_os = "macos")]
        ("ip", "recvif" | "recvdstaddr") => Some(Form::Bool),
        #[cfg(target_os = "linux")]
        ("ip", "recvttl") | ("tcp", "cork") | ("socket", "passcred") => Some(Form::Bool),
        #[cfg(target_os = "linux")]
        ("socket", "timestampns")
        | ("ip", "freebind" | "transparent" | "recverr" | "multicast_all" | "recvorigdstaddr")
        | ("ipv6", "recverr" | "recvhoplimit" | "recvpktinfo") => Some(Form::Bool),
        ("socket", "rcvbuf" | "sndbuf" | "priority")
        | ("tcp", "keepcnt" | "keepintvl" | "maxseg")
        | ("ip", "ttl" | "multicast_ttl")
        | ("ipv6", "tclass" | "multicast_if") => Some(Form::Int),
        #[cfg(target_os = "linux")]
        ("tcp", "keepidle") => Some(Form::Int),
        #[cfg(target_os = "linux")]
        ("tcp", "user_timeout") => Some(Form::Uint),
        ("ipv6", "multicast_hops" | "unicast_hops") => Some(Form::Hops),
        ("ip", "multicast_if") => Some(Form::Addr),
        ("ip", "add_membership" | "drop_membership") => Some(Form::Membership),
        ("socket", "linger") => Some(Form::Linger),
        ("ip", "tos") => Some(Form::Tos),
        #[cfg(target_os = "linux")]
        ("tcp", "congestion") => Some(Form::Congestion),
        #[cfg(target_os = "linux")]
        ("ipv6", "add_membership" | "drop_membership") => Some(Form::Membership6),
        #[cfg(target_os = "linux")]
        ("ip", "msfilter") => Some(Form::Msfilter),
        _ => raw_form(name),
    }
}

/// Return the form of the host option `name` that PBeam sets and gets with
/// [`raw_set`] and [`raw_get`], or `None` (`prim_socket_nif.c:3231-3236`,
/// `3263-3264`, `3357-3358`, `3397-3398`, `3447-3448`, `3507-3741`,
/// `3781-3782`, `3798-3991`, `4299-4301`, `4325-4327`). The host options
/// differ between Linux and macOS, so a name of the other host never
/// reaches the match.
fn raw_form(name: (&str, &str)) -> Option<Form> {
    match name {
        ("socket", "debug" | "rcvlowat" | "sndlowat" | "peek_off")
        | ("ip", "minttl" | "router_alert")
        | ("ipv6", "mtu" | "router_alert") => Some(Form::Int),
        ("ip", "recvopts" | "retopts" | "recvttl" | "nodefrag")
        | ("ipv6", "authhdr" | "dstopts" | "flowinfo" | "hoplimit" | "hopopts" | "rthdr")
        | ("tcp", "nopush")
        | ("udp", "cork") => Some(Form::Bool),
        ("socket", "bindtodevice") => Some(Form::Str),
        ("ip" | "ipv6", "mtu_discover") => Some(Form::Pmtu),
        (
            "ip",
            "add_source_membership" | "drop_source_membership" | "block_source" | "unblock_source",
        ) => Some(Form::Source),
        ("ipv6", "addrform") => Some(Form::Addrform),
        _ => None,
    }
}

/// The values of `IP_PMTUDISC_*` and `IPV6_PMTUDISC_*` in the glibc
/// header `bits/in.h`, which the NIF decodes and encodes with the atoms
/// `dont`, `want`, `do`, and `probe` (`prim_socket_nif.c:16364-16388`,
/// `16428-16463`, `16481-16509`, `16528-16570`). Only Linux has the
/// options `{ip | ipv6, mtu_discover}`.
const PMTUDISC: [(i32, &str); 4] = [(0, "dont"), (1, "want"), (2, "do"), (3, "probe")];

/// Return the `int` of `decode_ip_pmtudisc` and `decode_ipv6_pmtudisc`
/// (`prim_socket_nif.c:16364-16388`, `16428-16463`): the value of a
/// [`PMTUDISC`] atom, or an integer.
fn decode_pmtu(scheduler: &Scheduler, value: &Term) -> Option<i32> {
    let name = atom_name(scheduler, value);
    if name.is_some() {
        return named_value(&PMTUDISC, name);
    }
    int(value)
}

/// Return the `struct ip_mreq_source` of `esock_setopt_in_update_source`
/// (`prim_socket_nif.c:8860-8872`): the map `#{multiaddr := Addr,
/// interface := Addr, sourceaddr := Addr}`, with the addresses of
/// `esock_decode_in_addr`.
fn decode_source(scheduler: &Scheduler, value: &Term) -> Option<libc::ip_mreq_source> {
    let multiaddr = field(&scheduler.atoms, value, "multiaddr")?;
    let interface = field(&scheduler.atoms, value, "interface")?;
    let sourceaddr = field(&scheduler.atoms, value, "sourceaddr")?;
    let address = |term: &Term| {
        in_addr(scheduler, term).map(|address| libc::in_addr {
            s_addr: u32::from_ne_bytes(address.octets()),
        })
    };
    Some(libc::ip_mreq_source {
        imr_multiaddr: address(multiaddr)?,
        imr_interface: address(interface)?,
        imr_sourceaddr: address(sourceaddr)?,
    })
}

/// Return the hop count of `decode_hops` (`prim_socket_nif.c:16395-16410`):
/// an integer from 0 to 255, or -1 for the atom `default`.
fn decode_hops(scheduler: &Scheduler, value: &Term) -> Option<i32> {
    match int(value) {
        Some(hops) => (0..=255).contains(&hops).then_some(hops),
        None => (atom_name(scheduler, value) == Some("default")).then_some(-1),
    }
}

/// Return the `struct ip_mreq` of `esock_setopt_in_update_membership`
/// (`prim_socket_nif.c:8797-8830`): the map `#{multiaddr := Addr,
/// interface := Addr}`, with the addresses of `esock_decode_in_addr`.
fn decode_membership(scheduler: &Scheduler, value: &Term) -> Option<Value> {
    let multiaddr = field(&scheduler.atoms, value, "multiaddr")?;
    let interface = field(&scheduler.atoms, value, "interface")?;
    Some(Value::Membership(
        in_addr(scheduler, multiaddr)?,
        in_addr(scheduler, interface)?,
    ))
}

/// Return the `struct ipv6_mreq` of `esock_setopt_in6_update_membership`
/// (`prim_socket_nif.c:8970-8998`): the map `#{multiaddr := Addr,
/// interface := Index}`, with the address of `esock_decode_in6_addr` and
/// an `unsigned int` index.
#[cfg(target_os = "linux")]
fn decode_membership6(scheduler: &Scheduler, value: &Term) -> Option<Value> {
    let multiaddr = field(&scheduler.atoms, value, "multiaddr")?;
    let interface = field(&scheduler.atoms, value, "interface")?;
    Some(Value::Membership6(
        in6_addr(scheduler, multiaddr)?,
        uint(interface)?,
    ))
}

/// Return the `struct ip_msfilter` of `esock_setopt_msfilter`
/// (`prim_socket_nif.c:8617-8670`): no bytes for the atom `null`, or the
/// map `#{multiaddr := Addr, interface := Addr, mode := include | exclude,
/// slist := [Addr]}` in `IP_MSFILTER_SIZE(length(Slist))` bytes, with the
/// addresses of `esock_decode_in_addr` and the modes `MCAST_INCLUDE` and
/// `MCAST_EXCLUDE` of `decode_msfilter_mode` (`prim_socket_nif.c:8676-8693`).
#[cfg(target_os = "linux")]
fn decode_msfilter(scheduler: &Scheduler, value: &Term) -> Option<Vec<u8>> {
    if atom_name(scheduler, value) == Some("null") {
        return Some(Vec::new());
    }
    let multiaddr = field(&scheduler.atoms, value, "multiaddr")?;
    let interface = field(&scheduler.atoms, value, "interface")?;
    let mode = match atom_name(scheduler, field(&scheduler.atoms, value, "mode")?) {
        Some("include") => libc::MCAST_INCLUDE,
        Some("exclude") => libc::MCAST_EXCLUDE,
        _ => return None,
    };
    let sources = field(&scheduler.atoms, value, "slist")?
        .proper_list()?
        .map(|source| in_addr(scheduler, source))
        .collect::<Option<Vec<_>>>()?;
    let mut bytes = Vec::new();
    bytes.extend(in_addr(scheduler, multiaddr)?.octets());
    bytes.extend(in_addr(scheduler, interface)?.octets());
    bytes.extend(mode.to_ne_bytes());
    // `imsf_numsrc` is a `uint32_t`, so a longer list wraps as in C.
    let [b0, b1, b2, b3, ..] = sources.len().to_le_bytes();
    bytes.extend(u32::from_le_bytes([b0, b1, b2, b3]).to_ne_bytes());
    bytes.extend(sources.iter().flat_map(Ipv4Addr::octets));
    Some(bytes)
}

/// Return the four bytes at `at` of `bytes`.
#[cfg(target_os = "linux")]
fn word(bytes: &[u8], at: usize) -> [u8; 4] {
    [bytes[at], bytes[at + 1], bytes[at + 2], bytes[at + 3]]
}

/// Return the list of `esock_getopt_pktoptions` for the control messages in
/// `buffer` (`prim_socket_nif.c:13269-13405`): a map `#{level, type, data,
/// value}` for each message, without `value` when the data is shorter than
/// the value, up to the first header whose data does not fit, such as the
/// zero bytes after the last message. The Linux kernel gives only
/// `IP_PKTINFO`, `IP_TTL`, and `IP_TOS` for `IP_PKTOPTIONS`
/// (`net/ipv4/ip_sockglue.c`), with the encoders of the table `cmsgLevelIP`
/// (`prim_socket_nif.c:14176-14215`, `14335-14387`, `14453-14490`,
/// `15674-15694`). The level is an integer (`socket_util.c:3054-3060`).
///
/// # Errors
///
/// Another message stops the run. A new atom in a full atom table gives
/// the error of `AtomTable::put`.
#[cfg(target_os = "linux")]
fn pktoptions(scheduler: &mut Scheduler, buffer: &[u8]) -> Result<Term, BeamError> {
    const HEADER: usize = std::mem::size_of::<libc::cmsghdr>();
    // glibc `CMSG_ALIGN` aligns to a `size_t`, the type of `cmsg_len`.
    const SIZE: usize = std::mem::size_of::<usize>();
    let address = |bytes: &[u8]| {
        Term::tuple(
            bytes
                .iter()
                .map(|byte| Term::Integer((*byte).into()))
                .collect(),
        )
    };
    let mut messages = Vec::new();
    let mut start = 0;
    // ERTS also reads a header past the end of the buffer; PBeam stops.
    while let Some(header) = buffer.get(start..start + HEADER) {
        let mut length = [0; SIZE];
        length.copy_from_slice(&header[..SIZE]);
        let length = usize::from_ne_bytes(length);
        let level = i32::from_ne_bytes(word(header, SIZE));
        let kind = i32::from_ne_bytes(word(header, SIZE + 4));
        // A length below the header size wraps the data size, as in C.
        let Some(data) = buffer.get(start + HEADER..start.wrapping_add(length)) else {
            break;
        };
        let (name, value) = match (level, kind) {
            (libc::IPPROTO_IP, libc::IP_TOS) => {
                let value = match data.first() {
                    Some(tos) => Some(tos_term(scheduler, (*tos).into())?),
                    None => None,
                };
                ("tos", value)
            }
            (libc::IPPROTO_IP, libc::IP_TTL) => {
                let value = (data.len() >= 4).then(|| i32::from_ne_bytes(word(data, 0)));
                ("ttl", value.map(|ttl| Term::Integer(ttl.into())))
            }
            (libc::IPPROTO_IP, libc::IP_PKTINFO) => {
                let value = match data.get(..12) {
                    Some(info) => {
                        let index = u32::from_ne_bytes(word(info, 0));
                        let entries = [
                            ("ifindex", Term::Integer(index.into())),
                            ("spec_dst", address(&info[4..8])),
                            ("addr", address(&info[8..12])),
                        ];
                        let entries = pairs(scheduler, &entries)?;
                        Some(map(scheduler, entries))
                    }
                    None => None,
                };
                ("pktinfo", value)
            }
            _ => {
                return Err(unsupported(&format!("control message {{{level}, {kind}}}")));
            }
        };
        let mut entries = vec![
            ("level", Term::Integer(level.into())),
            ("type", atom(scheduler, name)?),
            ("data", Term::binary(data.to_vec())),
        ];
        entries.extend(value.map(|value| ("value", value)));
        let entries = pairs(scheduler, &entries)?;
        messages.push(map(scheduler, entries));
        start += length.next_multiple_of(SIZE);
    }
    Ok(Term::list_from(messages))
}

/// Return the `struct linger` of `esock_setopt_linger`
/// (`prim_socket_nif.c:8568-8587`): the map `#{onoff := boolean(), linger
/// := non_neg_integer()}`, or the atom `abort` for `{1, 0}`.
fn decode_linger(scheduler: &Scheduler, value: &Term) -> Option<libc::linger> {
    let onoff = field(&scheduler.atoms, value, "onoff");
    let linger = field(&scheduler.atoms, value, "linger");
    let (Some(onoff), Some(linger)) = (onoff, linger) else {
        return (atom_name(scheduler, value) == Some("abort")).then_some(libc::linger {
            l_onoff: 1,
            l_linger: 0,
        });
    };
    Some(libc::linger {
        l_onoff: decode_bool(scheduler, onoff)?.into(),
        l_linger: int(linger).filter(|linger| *linger >= 0)?,
    })
}

/// The number `ESOCK_OPT_TCP_CONGESTION_NAME_MAX`, which is 256
/// (`prim_socket_nif.c:490-495`): only the kernel header
/// `linux/tcp.h` defines `TCP_CA_NAME_MAX`, and the NIF includes the
/// glibc header `netinet/tcp.h`, so a name of 17 bytes reaches the
/// kernel and gives its `ENOENT` on the pinned ERTS.
#[cfg(target_os = "linux")]
const CONGESTION_NAME_MAX: usize = 256;

/// The TOS values of `ESOCK_TOS_*` (`prim_socket_nif.c:470-474`).
/// `decode_ip_tos` does not accept the first, `default`
/// (`prim_socket_nif.c:16113-16125`).
const TOS: [(i32, &str); 5] = [
    (0, "default"),
    (8, "lowdelay"),
    (4, "throughput"),
    (2, "reliability"),
    (1, "mincost"),
];

/// The precedence values of `ESOCK_TOS_PREC_*` (`prim_socket_nif.c:479-486`).
/// Each value is a DSCP class selector, so only `routine` fits the 3 bits
/// of the precedence in `decode_iptos_tos_value` and `encode_iptos_tos`
/// (`prim_socket_nif.c:16340-16341`, `16662-16701`).
const PRECEDENCES: [(i32, &str); 8] = [
    (56, "netcontrol"),
    (48, "internetcontrol"),
    (40, "critical_ecp"),
    (32, "flashoverride"),
    (24, "flash"),
    (16, "immediate"),
    (8, "priority"),
    (0, "routine"),
];

/// The DSCP values of `ESOCK_DSCP_*` (`prim_socket_nif.c:438-462`).
const DSCPS: [(i32, &str); 24] = [
    (0, "cs0"),
    (8, "cs1"),
    (16, "cs2"),
    (24, "cs3"),
    (32, "cs4"),
    (40, "cs5"),
    (48, "cs6"),
    (56, "cs7"),
    (10, "af11"),
    (12, "af12"),
    (14, "af13"),
    (18, "af21"),
    (20, "af22"),
    (22, "af23"),
    (26, "af31"),
    (28, "af32"),
    (30, "af33"),
    (34, "af41"),
    (36, "af42"),
    (38, "af43"),
    (46, "ef"),
    (44, "voice_admit"),
    (1, "le"),
    (45, "nqb"),
];

/// Return the value of the atom `name` in `table`, or `None`.
fn named_value(table: &[(i32, &str)], name: Option<&str>) -> Option<i32> {
    let name = name?;
    table
        .iter()
        .find(|(_, entry)| *entry == name)
        .map(|(value, _)| *value)
}

/// Return the `int` of `decode_ip_tos` (`prim_socket_nif.c:16087-16219`):
/// an integer, a TOS atom shifted by 1, a DSCP atom shifted by 2, or the
/// map of `decode_iptos_tos_value`.
pub(super) fn decode_tos(scheduler: &Scheduler, value: &Term) -> Option<i32> {
    if matches!(value, Term::Integer(_) | Term::BigInt(_) | Term::Float(_)) {
        return int(value);
    }
    let name = atom_name(scheduler, value);
    if name.is_some() {
        return named_value(&TOS[1..], name)
            .map(|tos| tos << 1)
            .or_else(|| named_value(&DSCPS, name).map(|dscp| dscp << 2));
    }
    decode_tos_map(scheduler, value)
}

/// Return the `int` of `decode_iptos_tos_value`
/// (`prim_socket_nif.c:16222-16344`): the map `#{precedence := Prec, tos
/// := Tos}`, where each number is an `unsigned int` that the NIF cuts to
/// an `unsigned char`.
fn decode_tos_map(scheduler: &Scheduler, value: &Term) -> Option<i32> {
    let decode = |term: &Term, table: &[(i32, &str)]| match uint(term) {
        Some(number) => Some(i32::from(number.to_le_bytes()[0])),
        None => named_value(table, atom_name(scheduler, term)),
    };
    let precedence = decode(field(&scheduler.atoms, value, "precedence")?, &PRECEDENCES)?;
    let tos = decode(field(&scheduler.atoms, value, "tos")?, &TOS)?;
    Some(((precedence << 5) & 0xe0) | ((tos << 1) & 0x1e))
}

/// Return the map of `encode_ip_tos` (`prim_socket_nif.c:16637-16816`):
/// `#{native => Value, tos => #{precedence => Prec, tos => Tos}, dscp =>
/// Dscp}`, with an atom for each known value and an integer for each other
/// value.
pub(super) fn tos_term(scheduler: &mut Scheduler, value: i32) -> Result<Term, BeamError> {
    let mut name = |table: &[(i32, &str)], native: i32| -> Result<Term, BeamError> {
        match table.iter().find(|(entry, _)| *entry == native) {
            Some((_, name)) => atom(scheduler, name),
            None => Ok(Term::Integer(native.into())),
        }
    };
    let precedence = name(&PRECEDENCES, (value & 0xe0) >> 5)?;
    let tos = name(&TOS, (value & 0x1e) >> 1)?;
    let dscp = name(&DSCPS, (value & 0xfc) >> 2)?;
    let tos = pairs(scheduler, &[("precedence", precedence), ("tos", tos)])?;
    let tos = map(scheduler, tos);
    let entries = [
        ("native", Term::Integer(value.into())),
        ("tos", tos),
        ("dscp", dscp),
    ];
    let entries = pairs(scheduler, &entries)?;
    Ok(map(scheduler, entries))
}

/// Return the bytes of `esock_setopt_str_opt` with the buffer size `max`
/// (`prim_socket_nif.c:11297-11338`): a Latin-1 string that `GET_STR` puts
/// with its NUL byte in `max` bytes, or the bytes of a binary before its
/// first NUL byte, at most `max - 1`.
fn decode_str(value: &Term, max: usize) -> Option<Vec<u8>> {
    if let Some(elements) = value.proper_list() {
        let bytes = elements
            .map(|element| match element {
                Term::Integer(byte) => u8::try_from(*byte).ok(),
                _ => None,
            })
            .collect::<Option<Vec<u8>>>()?;
        return (bytes.len() < max).then_some(bytes);
    }
    let bytes = value.binary_bytes()?;
    Some(
        bytes
            .iter()
            .take(max - 1)
            .take_while(|byte| **byte != 0)
            .copied()
            .collect(),
    )
}

/// Return whether ERTS has no get handler for the host option `name` that
/// PBeam sets (`prim_socket_nif.c:3496-3541`, `3571-3577`, `3781-3833`,
/// `12234-12236`).
fn set_form_only(name: (&str, &str)) -> bool {
    match name {
        ("ip", "add_membership" | "drop_membership") => true,
        #[cfg(target_os = "linux")]
        ("ip", "msfilter") => true,
        _ if matches!(raw_form(name), Some(Form::Source | Form::Addrform)) => true,
        #[cfg(target_os = "linux")]
        ("ipv6", "add_membership" | "drop_membership") => true,
        _ => false,
    }
}

/// Set the host option `name` with the host level and number `level` and
/// `opt` of `host` to `value`, as
/// `esock_setopt_level_opt` and `socket_setopt` do
/// (`prim_socket_nif.c:11384-11395`, `11422-11481`). Each option of the
/// form [`Form::Int`] has an unsigned type in `nix` or `socket2`, and a
/// negative value reaches the host as the same C `int` as on ERTS.
fn set_host(
    host: &Socket,
    name: (&str, &str),
    (level, opt): (i32, i32),
    value: Value,
) -> io::Result<()> {
    let value = match value {
        Value::Linger(linger) => {
            return keep_tos_and_priority(host, opt, || {
                setsockopt(host, sockopt::Linger, &linger).map_err(io::Error::from)
            });
        }
        Value::Addr(address) => {
            return keep_tos_and_priority(host, opt, || host.set_multicast_if_v4(&address));
        }
        Value::Membership(group, interface) => {
            return keep_tos_and_priority(host, opt, || match name {
                ("ip", "add_membership") => host.join_multicast_v4(&group, &interface),
                _ => host.leave_multicast_v4(&group, &interface),
            });
        }
        Value::Bytes(bytes) => {
            return keep_tos_and_priority(host, opt, || match name {
                #[cfg(target_os = "linux")]
                ("tcp", "congestion") => host.set_tcp_congestion(&bytes),
                _ => raw_set(host, level, opt, bytes.as_slice()),
            });
        }
        Value::Source(source) => {
            return keep_tos_and_priority(host, opt, || raw_set(host, level, opt, &source));
        }
        #[cfg(target_os = "linux")]
        Value::Membership6(group, interface) => {
            return keep_tos_and_priority(host, opt, || match name {
                ("ipv6", "add_membership") => host.join_multicast_v6(&group, interface),
                _ => host.leave_multicast_v6(&group, interface),
            });
        }
        Value::Int(value) => value,
    };
    let flag = value != 0;
    // The safe APIs pass these values on as a C `int` (`nix` `SetUsize`
    // casts with `as c_int`, `socket2::set_ttl_v4` casts the `u32`), so the
    // bit-identical casts below give the host the `int` that ERTS gives.
    let count = value.cast_unsigned();
    keep_tos_and_priority(host, opt, || {
        let result = match name {
            ("socket", "broadcast") => setsockopt(host, sockopt::Broadcast, &flag),
            ("socket", "keepalive") => setsockopt(host, sockopt::KeepAlive, &flag),
            ("socket", "reuseaddr") => setsockopt(host, sockopt::ReuseAddr, &flag),
            ("socket", "reuseport") => setsockopt(host, sockopt::ReusePort, &flag),
            ("socket", "oobinline") => setsockopt(host, sockopt::OobInline, &flag),
            ("socket", "dontroute") => setsockopt(host, sockopt::DontRoute, &flag),
            ("socket", "rcvbuf") => setsockopt(host, sockopt::RcvBuf, &sign_extended(value)),
            ("socket", "sndbuf") => setsockopt(host, sockopt::SndBuf, &sign_extended(value)),
            ("tcp", "nodelay") => setsockopt(host, sockopt::TcpNoDelay, &flag),
            ("tcp", "keepcnt") => setsockopt(host, sockopt::TcpKeepCount, &count),
            ("tcp", "keepintvl") => setsockopt(host, sockopt::TcpKeepInterval, &count),
            ("ipv6", "v6only") => setsockopt(host, sockopt::Ipv6V6Only, &flag),
            ("socket", "timestamp") => setsockopt(host, sockopt::ReceiveTimestamp, &flag),
            #[cfg(any(target_os = "linux", target_os = "macos"))]
            ("ip", "pktinfo") => setsockopt(host, sockopt::Ipv4PacketInfo, &flag),
            #[cfg(target_os = "macos")]
            ("ip", "recvif") => setsockopt(host, sockopt::Ipv4RecvIf, &flag),
            #[cfg(target_os = "macos")]
            ("ip", "recvdstaddr") => setsockopt(host, sockopt::Ipv4RecvDstAddr, &flag),
            #[cfg(target_os = "linux")]
            ("ip", "recvttl") => setsockopt(host, sockopt::Ipv4RecvTtl, &flag),
            #[cfg(target_os = "linux")]
            ("socket", "passcred") => setsockopt(host, sockopt::PassCred, &flag),
            #[cfg(target_os = "linux")]
            ("socket", "priority") => setsockopt(host, sockopt::Priority, &value),
            #[cfg(target_os = "linux")]
            ("tcp", "keepidle") => setsockopt(host, sockopt::TcpKeepIdle, &count),
            #[cfg(target_os = "linux")]
            ("tcp", "user_timeout") => setsockopt(host, sockopt::TcpUserTimeout, &count),
            #[cfg(target_os = "linux")]
            ("socket", "timestampns") => setsockopt(host, sockopt::ReceiveTimestampns, &flag),
            #[cfg(target_os = "linux")]
            ("ip", "freebind") => setsockopt(host, sockopt::IpFreebind, &flag),
            #[cfg(target_os = "linux")]
            ("ip", "transparent") => setsockopt(host, sockopt::IpTransparent, &flag),
            #[cfg(target_os = "linux")]
            ("ip", "recverr") => setsockopt(host, sockopt::Ipv4RecvErr, &flag),
            #[cfg(target_os = "linux")]
            ("ip", "recvorigdstaddr") => setsockopt(host, sockopt::Ipv4OrigDstAddr, &flag),
            #[cfg(target_os = "linux")]
            ("ipv6", "recverr") => setsockopt(host, sockopt::Ipv6RecvErr, &flag),
            #[cfg(target_os = "linux")]
            ("ipv6", "recvhoplimit") => setsockopt(host, sockopt::Ipv6RecvHopLimit, &flag),
            #[cfg(target_os = "linux")]
            ("ipv6", "recvpktinfo") => setsockopt(host, sockopt::Ipv6RecvPacketInfo, &flag),
            _ => return set_socket2(host, name, (level, opt), value),
        };
        result.map_err(io::Error::from)
    })
}

/// Set the host option `name` of `host` to the C `int` `value` with the
/// typed calls of `socket2`, which pass each `u32` and `bool` on as a C
/// `int`, or with [`raw_set`] for an option without a typed call.
fn set_socket2(
    host: &Socket,
    name: (&str, &str),
    (level, opt): (i32, i32),
    value: i32,
) -> io::Result<()> {
    let flag = value != 0;
    let count = value.cast_unsigned();
    match name {
        ("ip", "multicast_ttl") => host.set_multicast_ttl_v4(count),
        ("ip", "multicast_loop") => host.set_multicast_loop_v4(flag),
        ("ip", "recvtos") => host.set_recv_tos_v4(flag),
        ("ip", "hdrincl") => host.set_header_included_v4(flag),
        ("ipv6", "tclass") => host.set_tclass_v6(count),
        ("ipv6", "recvtclass") => host.set_recv_tclass_v6(flag),
        ("ipv6", "multicast_loop") => host.set_multicast_loop_v6(flag),
        ("ipv6", "multicast_if") => host.set_multicast_if_v6(count),
        ("ipv6", "multicast_hops") => host.set_multicast_hops_v6(count),
        ("ipv6", "unicast_hops") => host.set_unicast_hops_v6(count),
        ("tcp", "maxseg") => host.set_tcp_mss(count),
        ("ip", "tos") => host.set_tos_v4(count),
        #[cfg(target_os = "linux")]
        ("tcp", "cork") => host.set_tcp_cork(flag),
        #[cfg(target_os = "linux")]
        ("ip", "multicast_all") => host.set_multicast_all_v4(flag),
        ("ip", "ttl") => host.set_ttl_v4(count),
        _ => raw_set(host, level, opt, &value),
    }
}

/// The `socklen_t` of a byte size, which wraps as the C cast of a `size_t`
/// does.
fn socklen(size: usize) -> libc::socklen_t {
    let [b0, b1, b2, b3, ..] = size.to_le_bytes();
    libc::socklen_t::from_le_bytes([b0, b1, b2, b3])
}

/// The `size_t` of a C `int`, which keeps the sign bits as the C
/// conversion does.
fn sign_extended(value: i32) -> usize {
    isize::try_from(value)
        .expect("an isize holds an i32 on every supported target")
        .cast_unsigned()
}

/// The `int` of a socket type, which `nix` gives as a `repr(i32)` enum.
#[expect(
    clippy::as_conversions,
    reason = "nix::sys::socket::SockType is a repr(i32) enum and has no From or TryFrom to i32"
)]
fn sock_type_code(kind: nix::sys::socket::SockType) -> i32 {
    kind as i32
}

/// Set the option `opt` at the level `level` of `host` to the bytes of
/// `value`, as `socket_setopt` passes a value and its size to
/// `setsockopt` (`prim_socket_nif.c:11422-11481`). The size is a
/// `socklen_t`, so a larger size wraps as in C.
fn raw_set<T: ?Sized>(host: &Socket, level: i32, opt: i32, value: &T) -> io::Result<()> {
    let size = socklen(std::mem::size_of_val(value));
    // SAFETY: `value` is a live shared reference, so the `size` bytes at its
    // address stay valid for reads during the call, and `setsockopt` reads
    // at most `size` bytes. `host` owns the descriptor during the call.
    // The owner approved this call for #1209 (AGENTS.md).
    let result = unsafe {
        libc::setsockopt(
            host.as_raw_fd(),
            level,
            opt,
            ptr::from_ref(value).cast(),
            size,
        )
    };
    Errno::result(result).map(drop).map_err(io::Error::from)
}

/// Read the option `opt` at the level `level` of `host` into `buffer`, as
/// `sock_getopt` does, and return the bytes that the host gives
/// (`prim_socket_nif.c:13143-13166`, `13187-13211`). `None` gives the host
/// a null value and a null size, as `esock_getopt_size_opt` does for the
/// size 0 (`prim_socket_nif.c:13136-13141`). The size is a `socklen_t`, so
/// a larger buffer size wraps as in C.
fn raw_get(host: &Socket, level: i32, opt: i32, buffer: Option<Vec<u8>>) -> io::Result<Vec<u8>> {
    let mut buffer = buffer;
    let mut size = socklen(buffer.as_ref().map_or(0, Vec::len));
    let (value, size_ptr) = match buffer.as_mut() {
        Some(buffer) => (buffer.as_mut_ptr().cast(), ptr::from_mut(&mut size)),
        None => (ptr::null_mut(), ptr::null_mut()),
    };
    // SAFETY: `value` is null or the start of `buffer`, which is valid for
    // writes of `size` bytes and lives past the call. `size_ptr` is null or
    // points to `size`, which lives past the call. `getsockopt` writes at
    // most `size` bytes and then the new size, and it gives `EFAULT` for a
    // null pointer that it must use. `host` owns the descriptor during the
    // call. The owner approved this call for #1209 (AGENTS.md).
    let result = unsafe { libc::getsockopt(host.as_raw_fd(), level, opt, value, size_ptr) };
    Errno::result(result).map_err(io::Error::from)?;
    let mut bytes = buffer.unwrap_or_default();
    bytes.truncate(usize::try_from(size).expect("a socklen_t fits in usize"));
    Ok(bytes)
}

/// Read the C `int` of the option `opt` at the level `level` of `host`, as
/// `esock_getopt_int` does (`prim_socket_nif.c:13080-13098`): the host
/// writes at most 4 bytes over a zero `int`.
fn raw_int(host: &Socket, level: i32, opt: i32) -> io::Result<i32> {
    let bytes = raw_get(host, level, opt, Some(vec![0; 4]))?;
    let mut value = [0; 4];
    value[..bytes.len()].copy_from_slice(&bytes);
    Ok(i32::from_ne_bytes(value))
}

/// Run `set` on `host`, as `socket_setopt` does on Linux
/// (`prim_socket_nif.c:11428-11472`): read `SO_PRIORITY` and `IP_TOS`
/// first, and after a successful set, set `IP_TOS` and then `SO_PRIORITY`
/// back to the values that it could read. A failed reset of `SO_PRIORITY`
/// with `EPERM` is not an error. ERTS compares only the option number
/// `opt`, at each level: it resets nothing for the number `SO_PRIORITY`,
/// and it does not reset `IP_TOS` for the number `IP_TOS`.
#[cfg(target_os = "linux")]
fn keep_tos_and_priority(
    host: &Socket,
    opt: i32,
    set: impl FnOnce() -> io::Result<()>,
) -> io::Result<()> {
    let priority = getsockopt(host, sockopt::Priority);
    let tos = getsockopt(host, sockopt::Ipv4Tos);
    set()?;
    if opt == libc::SO_PRIORITY {
        return Ok(());
    }
    if opt != libc::IP_TOS {
        tos.map_or(Ok(()), |tos| setsockopt(host, sockopt::Ipv4Tos, &tos))?;
    }
    priority.map_or(Ok(()), |priority| {
        allow_eperm(setsockopt(host, sockopt::Priority, &priority))
    })?;
    Ok(())
}

/// Return `result`, but with `Ok` for the error `EPERM`: a failed reset of
/// `SO_PRIORITY` with `EPERM` is not an error
/// (`prim_socket_nif.c:11460-11466`).
#[cfg(target_os = "linux")]
fn allow_eperm(result: nix::Result<()>) -> nix::Result<()> {
    match result {
        Err(nix::errno::Errno::EPERM) => Ok(()),
        result => result,
    }
}

/// Run `set` on `host`: macOS has no `SO_PRIORITY`, so `socket_setopt`
/// only sets the option (`prim_socket_nif.c:11474-11478`).
#[cfg(not(target_os = "linux"))]
fn keep_tos_and_priority(
    _host: &Socket,
    _opt: i32,
    set: impl FnOnce() -> io::Result<()>,
) -> io::Result<()> {
    set()
}

/// Run `prim_socket:nif_getopt(Socket, Level, Opt)`, as `nif_getopt` does
/// (`prim_socket_nif.c:11504-11585`).
///
/// # Errors
///
/// An option without a handler in PBeam stops the run. A new atom in a
/// full atom table gives the error of `AtomTable::put`.
pub(in crate::runtime::interpreter::native::prim_socket) fn getopt_3(
    scheduler: &mut Scheduler,
    arguments: &[Term],
) -> Result<BifResult, BeamError> {
    getopt(scheduler, &arguments[0], &arguments[1], &arguments[2], None)
}

/// Run `prim_socket:nif_getopt(Socket, Level, Opt, Kind, Value)`, as
/// `nif_getopt` does (`prim_socket_nif.c:11504-11585`). The handlers of
/// PBeam read `Value` only for the kind `native`
/// (`prim_socket_nif.c:11558-11565`, `13018`, `13042`).
///
/// # Errors
///
/// An option without a handler in PBeam stops the run. A new atom in a
/// full atom table gives the error of `AtomTable::put`.
pub(in crate::runtime::interpreter::native::prim_socket) fn getopt_5(
    scheduler: &mut Scheduler,
    arguments: &[Term],
) -> Result<BifResult, BeamError> {
    getopt(
        scheduler,
        &arguments[0],
        &arguments[1],
        &arguments[2],
        Some((&arguments[3], &arguments[4])),
    )
}

/// Run `nif_getopt` with the kind and the value of `nif_getopt/5`, or
/// `None`.
fn getopt(
    scheduler: &mut Scheduler,
    socket: &Term,
    level: &Term,
    opt: &Term,
    kind: Option<(&Term, &Term)>,
) -> Result<BifResult, BeamError> {
    let Some((_, state)) = socket_argument(scheduler, socket) else {
        return Ok(BifResult::badarg());
    };
    let Some(opt) = int(opt) else {
        return range_or_badarg(scheduler, opt);
    };
    if let (Some("otp"), None) = (atom_name(scheduler, level), kind) {
        return getopt_otp(scheduler, &state, opt);
    }
    let Some(level) = level_number(scheduler, level) else {
        return range_or_badarg(scheduler, level);
    };
    let spec = kind
        .and_then(|(kind, spec)| (atom_name(scheduler, kind) == Some("native")).then_some(spec));
    let socket = lock(&state);
    if socket.read & (CLOSING | CLOSED) != 0 {
        return Ok(closed());
    }
    let host = socket
        .socket
        .as_ref()
        .expect("an open socket has a host socket");
    if let Some(spec) = spec {
        return getopt_native(scheduler, host, level, opt, spec);
    }
    let Some(name) = option_name(level, opt) else {
        return Ok(invalid(atom(scheduler, "socket_option")?));
    };
    if set_form_only(name) {
        return Ok(invalid(atom(scheduler, "socket_option")?));
    }
    // `esock_getopt_pktoptions` reads into a buffer of `rCtrlSz` bytes, cut
    // to a `socklen_t` as in C (`prim_socket_nif.c:13285-13295`).
    #[cfg(target_os = "linux")]
    if name == ("ip", "pktoptions") {
        let buffer = vec![
            0;
            usize::try_from(socklen(socket.options.recv_ctrl))
                .expect("a socklen_t fits in usize")
        ];
        let value = match raw_get(host, level, opt, Some(buffer)) {
            Ok(bytes) => Ok(Some(pktoptions(scheduler, &bytes)?)),
            Err(failure) => Err(failure),
        };
        return host_result(scheduler, value);
    }
    let got = get_host(host, name, (level, opt)).expect("each host option has a get handler");
    let value = match got {
        Ok(Got::Bool(value)) => Ok(Some(Term::static_atom(if value {
            "true"
        } else {
            "false"
        }))),
        Ok(Got::Int(value)) => Ok(Some(Term::Integer(value))),
        Ok(Got::Type(kind)) => type_term(scheduler, kind).map(Some).map(Ok)?,
        Ok(Got::Linger(linger)) => {
            let entries = [
                ("onoff", Term::from(linger.l_onoff != 0)),
                ("linger", Term::Integer(linger.l_linger.into())),
            ];
            let pairs = pairs(scheduler, &entries)?;
            Ok(Some(map(scheduler, pairs)))
        }
        Ok(Got::Addr(address)) => {
            let octets = address.octets().map(|byte| Term::Integer(byte.into()));
            Ok(Some(Term::tuple(octets.to_vec())))
        }
        Ok(Got::Tos(value)) => Ok(Some(tos_term(scheduler, value)?)),
        Ok(Got::Pmtu(value)) => Ok(Some(named(scheduler, &PMTUDISC, value)?)),
        Ok(Got::Bytes(bytes)) => Ok(Some(Term::list_from(
            bytes.into_iter().map(|byte| Term::Integer(byte.into())),
        ))),
        #[cfg(target_os = "linux")]
        Ok(Got::Domain(domain)) => Ok(Some(domain_term(scheduler, domain)?)),
        // `esock_getopt_sock_protocol` gives `default` for the protocol 0
        // of a local socket (`prim_socket_nif.c:12585-12594`).
        #[cfg(target_os = "linux")]
        Ok(Got::Protocol(0)) if socket.domain == libc::AF_UNIX => {
            Ok(Some(atom(scheduler, "default")?))
        }
        #[cfg(target_os = "linux")]
        Ok(Got::Protocol(protocol)) => Ok(Some(Term::Integer(protocol.into()))),
        Err(failure) => Err(failure),
    };
    host_result(scheduler, value)
}

/// Read the option `opt` at the level `level` of `host` with the value spec
/// `spec`, as `esock_getopt_native` does (`prim_socket_nif.c:12151-12183`):
/// a binary of a size, an `integer`, a `boolean`, or a binary of the bytes
/// of a binary spec. The size 0 gives `ok` (`prim_socket_nif.c:13136-13141`).
fn getopt_native(
    scheduler: &mut Scheduler,
    host: &Socket,
    level: i32,
    opt: i32,
    spec: &Term,
) -> Result<BifResult, BeamError> {
    let binary = |bytes: Vec<u8>| Some(Term::binary(bytes));
    let value = match (int(spec), atom_name(scheduler, spec), spec.binary_bytes()) {
        (Some(0), _, _) => raw_get(host, level, opt, None).map(|_| None),
        // `valueSz` is a `socklen_t`, so a negative size wraps as in C.
        (Some(size), _, _) => raw_get(
            host,
            level,
            opt,
            Some(vec![
                0;
                usize::try_from(size.cast_unsigned())
                    .expect("a u32 fits in usize")
            ]),
        )
        .map(binary),
        (None, Some("integer"), _) => {
            raw_int(host, level, opt).map(|value| Some(Term::Integer(value.into())))
        }
        (None, Some("boolean"), _) => {
            raw_int(host, level, opt).map(|value| Some(Term::from(value != 0)))
        }
        // `esock_getopt_bin_opt` gives `{error, {invalid, data_size}}` for
        // a size that a `socklen_t` cannot hold
        // (`prim_socket_nif.c:13187-13189`).
        (None, _, Some(bytes)) if libc::socklen_t::try_from(bytes.len()).is_err() => {
            return error_invalid(scheduler, "data_size");
        }
        (None, _, Some(bytes)) => raw_get(host, level, opt, Some(bytes.to_vec())).map(binary),
        (None, _, None) => return Ok(invalid(atom(scheduler, "value")?)),
    };
    host_result(scheduler, value)
}

/// Read the host option `name` with the host level and number `level_opt`
/// of `host`, or `None` for an option without a handler in PBeam.
fn get_host(host: &Socket, name: (&str, &str), level_opt: (i32, i32)) -> Option<io::Result<Got>> {
    let result = match name {
        ("socket", "broadcast") => getsockopt(host, sockopt::Broadcast).map(Got::Bool),
        ("socket", "keepalive") => getsockopt(host, sockopt::KeepAlive).map(Got::Bool),
        ("socket", "reuseaddr") => getsockopt(host, sockopt::ReuseAddr).map(Got::Bool),
        ("socket", "reuseport") => getsockopt(host, sockopt::ReusePort).map(Got::Bool),
        ("socket", "oobinline") => getsockopt(host, sockopt::OobInline).map(Got::Bool),
        ("socket", "dontroute") => getsockopt(host, sockopt::DontRoute).map(Got::Bool),
        ("socket", "acceptconn") => getsockopt(host, sockopt::AcceptConn).map(Got::Bool),
        ("socket", "linger") => getsockopt(host, sockopt::Linger).map(Got::Linger),
        #[cfg(target_os = "linux")]
        ("socket", "priority") => {
            getsockopt(host, sockopt::Priority).map(|value| Got::Int(value.into()))
        }
        ("socket", "rcvbuf") => getsockopt(host, sockopt::RcvBuf)
            .map(|value| Got::Int(i64::try_from(value).expect("a buffer size fits in i64"))),
        ("socket", "sndbuf") => getsockopt(host, sockopt::SndBuf)
            .map(|value| Got::Int(i64::try_from(value).expect("a buffer size fits in i64"))),
        ("socket", "type") => {
            getsockopt(host, sockopt::SockType).map(|kind| Got::Type(sock_type_code(kind)))
        }
        ("tcp", "nodelay") => getsockopt(host, sockopt::TcpNoDelay).map(Got::Bool),
        ("tcp", "keepcnt") => {
            getsockopt(host, sockopt::TcpKeepCount).map(|value| Got::Int(value.into()))
        }
        ("tcp", "keepintvl") => {
            getsockopt(host, sockopt::TcpKeepInterval).map(|value| Got::Int(value.into()))
        }
        ("ipv6", "v6only") => getsockopt(host, sockopt::Ipv6V6Only).map(Got::Bool),
        ("socket", "timestamp") => getsockopt(host, sockopt::ReceiveTimestamp).map(Got::Bool),
        #[cfg(any(target_os = "linux", target_os = "macos"))]
        ("ip", "pktinfo") => getsockopt(host, sockopt::Ipv4PacketInfo).map(Got::Bool),
        #[cfg(target_os = "macos")]
        ("ip", "recvif") => getsockopt(host, sockopt::Ipv4RecvIf).map(Got::Bool),
        #[cfg(target_os = "macos")]
        ("ip", "recvdstaddr") => getsockopt(host, sockopt::Ipv4RecvDstAddr).map(Got::Bool),
        #[cfg(target_os = "linux")]
        ("ip", "recvttl") => getsockopt(host, sockopt::Ipv4RecvTtl).map(Got::Bool),
        #[cfg(target_os = "linux")]
        ("socket", "passcred") => getsockopt(host, sockopt::PassCred).map(Got::Bool),
        #[cfg(target_os = "linux")]
        ("tcp", "keepidle") => {
            getsockopt(host, sockopt::TcpKeepIdle).map(|value| Got::Int(value.cast_signed().into()))
        }
        #[cfg(target_os = "linux")]
        ("tcp", "user_timeout") => {
            getsockopt(host, sockopt::TcpUserTimeout).map(|value| Got::Int(value.into()))
        }
        #[cfg(target_os = "linux")]
        ("socket", "timestampns") => getsockopt(host, sockopt::ReceiveTimestampns).map(Got::Bool),
        #[cfg(target_os = "linux")]
        ("ip", "freebind") => getsockopt(host, sockopt::IpFreebind).map(Got::Bool),
        #[cfg(target_os = "linux")]
        ("ip", "transparent") => getsockopt(host, sockopt::IpTransparent).map(Got::Bool),
        #[cfg(target_os = "linux")]
        ("ip", "recverr") => getsockopt(host, sockopt::Ipv4RecvErr).map(Got::Bool),
        #[cfg(target_os = "linux")]
        ("ip", "recvorigdstaddr") => getsockopt(host, sockopt::Ipv4OrigDstAddr).map(Got::Bool),
        #[cfg(target_os = "linux")]
        ("ip", "mtu") => getsockopt(host, sockopt::IpMtu).map(|value| Got::Int(value.into())),
        #[cfg(target_os = "linux")]
        ("ipv6", "recverr") => getsockopt(host, sockopt::Ipv6RecvErr).map(Got::Bool),
        #[cfg(target_os = "linux")]
        ("ipv6", "recvhoplimit") => getsockopt(host, sockopt::Ipv6RecvHopLimit).map(Got::Bool),
        #[cfg(target_os = "linux")]
        ("ipv6", "recvpktinfo") => getsockopt(host, sockopt::Ipv6RecvPacketInfo).map(Got::Bool),
        _ => return get_socket2(host, name, level_opt),
    };
    Some(result.map_err(io::Error::from))
}

/// Read the host option `name` of `host` with the typed calls of
/// `socket2`, which read a C `int`, or with [`get_raw`] for an option
/// without a typed call. The casts `as i32` give back the bits of the
/// `int`.
fn get_socket2(
    host: &Socket,
    name: (&str, &str),
    level_opt: (i32, i32),
) -> Option<io::Result<Got>> {
    let int = |value: io::Result<u32>| value.map(|value| Got::Int(value.cast_signed().into()));
    Some(match name {
        ("ip", "ttl") => int(host.ttl_v4()),
        ("ip", "multicast_ttl") => int(host.multicast_ttl_v4()),
        ("ip", "multicast_loop") => host.multicast_loop_v4().map(Got::Bool),
        ("ip", "multicast_if") => host.multicast_if_v4().map(Got::Addr),
        ("ip", "recvtos") => host.recv_tos_v4().map(Got::Bool),
        ("ip", "hdrincl") => host.header_included_v4().map(Got::Bool),
        ("ipv6", "tclass") => int(host.tclass_v6()),
        ("ipv6", "recvtclass") => host.recv_tclass_v6().map(Got::Bool),
        ("ipv6", "multicast_loop") => host.multicast_loop_v6().map(Got::Bool),
        ("ipv6", "multicast_if") => int(host.multicast_if_v6()),
        ("ipv6", "multicast_hops") => int(host.multicast_hops_v6()),
        ("ipv6", "unicast_hops") => int(host.unicast_hops_v6()),
        ("tcp", "maxseg") => int(host.tcp_mss()),
        ("ip", "tos") => host.tos_v4().map(|value| Got::Tos(value.cast_signed())),
        #[cfg(target_os = "linux")]
        ("tcp", "cork") => host.tcp_cork().map(Got::Bool),
        #[cfg(target_os = "linux")]
        ("ip", "multicast_all") => host.multicast_all_v4().map(Got::Bool),
        // `esock_getopt_str_opt` removes one NUL byte at the end
        // (`prim_socket_nif.c:12741-12752`, `13442-13446`).
        #[cfg(target_os = "linux")]
        ("tcp", "congestion") => host.tcp_congestion().map(|mut name| {
            if name.last() == Some(&0) {
                name.pop();
            }
            Got::Bytes(name)
        }),
        #[cfg(target_os = "linux")]
        ("socket", "domain") => host.domain().map(|domain| Got::Domain(domain.into())),
        #[cfg(target_os = "linux")]
        ("socket", "protocol") => host
            .protocol()
            .map(|protocol| Got::Protocol(protocol.map_or(0, i32::from))),
        _ => return get_raw(host, name, level_opt),
    })
}

/// Read the host option `name` of `host` with [`raw_get`] in the form of
/// [`raw_form`], or `None` for an option without a handler in PBeam. ERTS
/// gives each byte of `{socket, bindtodevice}` that the host gives, in a
/// buffer of `IFNAMSIZ + 1` bytes (`prim_socket_nif.c:12259-12268`,
/// `13430-13446`).
fn get_raw(host: &Socket, name: (&str, &str), (level, opt): (i32, i32)) -> Option<io::Result<Got>> {
    Some(match raw_form(name)? {
        Form::Bool => raw_int(host, level, opt).map(|value| Got::Bool(value != 0)),
        Form::Pmtu => raw_int(host, level, opt).map(Got::Pmtu),
        Form::Str => {
            let buffer = vec![0; libc::IFNAMSIZ + 1];
            raw_get(host, level, opt, Some(buffer)).map(Got::Bytes)
        }
        // `Form::Int`: `getopt` answers the set-only forms `Source` and
        // `Addrform` before.
        _ => raw_int(host, level, opt).map(|value| Got::Int(value.into())),
    })
}

/// Return the raise of `error:{invalid, {otp_socket_option, Opt}}` of
/// `esock_raise_invalid` for an `otp` option number without a handler
/// (`prim_socket_nif.c:7942-7957`, `11692-11706`): `{otp, fd}` and
/// `{otp, domain}` have no setter.
fn invalid_otp_option(scheduler: &mut Scheduler, opt: i32) -> Result<BifResult, BeamError> {
    let what = Term::tuple(vec![
        atom(scheduler, "otp_socket_option")?,
        Term::Integer(opt.into()),
    ]);
    Ok(BifResult::Raise {
        class: ExceptionClass::Error,
        reason: Term::tuple(vec![Term::static_atom("invalid"), what]),
    })
}

/// Return the boolean of `esock_decode_bool` (`socket_util.c:3004-3014`).
fn decode_bool(scheduler: &Scheduler, value: &Term) -> Option<bool> {
    match atom_name(scheduler, value) {
        Some("true") => Some(true),
        Some("false") => Some(false),
        _ => None,
    }
}

/// Return the buffer size of `esock_decode_bufsz` (`socket_util.c:2887-2906`):
/// an `unsigned long` above 0, or `default` for the atom `default`.
fn decode_bufsz(scheduler: &Scheduler, value: &Term, default: usize) -> Option<usize> {
    if atom_name(scheduler, value) == Some("default") {
        return Some(default);
    }
    let size = u64::try_from(value.bigint()?.as_ref()).ok()?;
    usize::try_from(size).ok().filter(|size| *size != 0)
}

/// Return the integer term of a size, as `MKUL` makes it.
fn size_term(size: usize) -> Term {
    Term::from_bigint(size.into())
}

/// Return `{error, {invalid, Reason}}` of `esock_make_error_invalid`
/// (`socket_util.c:3303-3308`).
fn error_invalid(scheduler: &mut Scheduler, reason: &str) -> Result<BifResult, BeamError> {
    let reason = Term::tuple(vec![Term::static_atom("invalid"), atom(scheduler, reason)?]);
    Ok(BifResult::Value(Term::tuple(vec![
        Term::static_atom("error"),
        reason,
    ])))
}

/// Run `esock_setopt_otp` (`prim_socket_nif.c:7873-8405`).
///
/// # Errors
///
/// `{otp, debug}` and `{otp, select_read}` with `true`, and `{otp, rcvbuf}`
/// with a buffer count, stop the run. A new atom in a full atom table
/// gives the error of `AtomTable::put`.
fn setopt_otp(
    scheduler: &mut Scheduler,
    reference: Reference,
    state: &Arc<Mutex<SocketState>>,
    opt: i32,
    value: &Term,
) -> Result<BifResult, BeamError> {
    let mut socket = lock(state);
    // Each handler checks the read or the write state first.
    let bits = match opt {
        OTP_SELECT_READ | OTP_RCVBUF | OTP_RCVCTRLBUF => socket.read,
        OTP_DEBUG | OTP_IOW | OTP_CTRL_PROC | OTP_SNDCTRLBUF | OTP_META | OTP_USE_REGISTRY => {
            socket.write
        }
        _ => return invalid_otp_option(scheduler, opt),
    };
    if bits & (CLOSING | CLOSED) != 0 {
        return Ok(closed());
    }
    let ok = Ok(BifResult::Value(Term::static_atom("ok")));
    let invalid_value = |scheduler: &mut Scheduler| Ok(invalid(atom(scheduler, "value")?));
    match opt {
        OTP_DEBUG | OTP_SELECT_READ | OTP_IOW => {
            let Some(flag) = decode_bool(scheduler, value) else {
                return invalid_value(scheduler);
            };
            match opt {
                OTP_IOW => socket.options.iow = flag,
                OTP_SELECT_READ => socket.select_read = flag,
                // `dbg` turns on the debug output of `SSDBG`.
                _ if flag => return Err(unsupported("option value {otp, debug} true")),
                _ => {}
            }
            ok
        }
        OTP_CTRL_PROC => {
            if socket.owner != Some(scheduler.current_process().id) {
                return error_invalid(scheduler, "not_owner");
            }
            let Term::Pid(owner) = value else {
                return invalid_value(scheduler);
            };
            let owner = *owner;
            socket.owner = Some(owner);
            drop(socket);
            // The old monitor stays: `Sockets::monitor_down` acts only for
            // the current owner. The signal to a new owner that is gone
            // already gives the down of `essio_down_ctrl`
            // (`prim_socket_nif.c:8100-8130`).
            let _ = scheduler.send_signal(None, owner, Signal::SocketMonitor { socket: reference });
            ok
        }
        OTP_RCVBUF => {
            let default = OtpOptions::default().recv_buffer;
            // A size sets the buffer count `rNum` to 0, and `{N, Size}` with
            // an `unsigned int` N above 0 sets it to N
            // (`prim_socket_nif.c:8193-8212`, `8220-8222`).
            let parsed = match decode_bufsz(scheduler, value, default) {
                Some(size) => Some((0, size)),
                None => value
                    .tuple_elements()
                    .filter(|elements| elements.len() == 2)
                    .and_then(|elements| {
                        let count = uint(&elements[0]).filter(|count| *count != 0)?;
                        Some((count, decode_bufsz(scheduler, &elements[1], default)?))
                    }),
            };
            let Some((count, size)) = parsed else {
                return invalid_value(scheduler);
            };
            socket.options.recv_count = count;
            // The `ssize_t` check of `esock_setopt_otp_rcvbuf` converts the
            // size and back, so it refuses no size
            // (`prim_socket_nif.c:8215-8218`).
            socket.options.recv_buffer = size;
            ok
        }
        OTP_RCVCTRLBUF | OTP_SNDCTRLBUF => {
            let default = OtpOptions::default();
            let (slot, default) = match opt {
                OTP_RCVCTRLBUF => (&mut socket.options.recv_ctrl, default.recv_ctrl),
                _ => (&mut socket.options.send_ctrl, default.send_ctrl),
            };
            let Some(size) = decode_bufsz(scheduler, value, default) else {
                return invalid_value(scheduler);
            };
            *slot = size;
            ok
        }
        OTP_META => {
            if socket.owner != Some(scheduler.current_process().id) {
                return error_invalid(scheduler, "not_owner");
            }
            socket.meta = value.clone();
            ok
        }
        // OTP_USE_REGISTRY, the last option of the match above. Only `true`
        // is valid: the option only turns the registry on.
        _ => {
            if *value != Term::static_atom("true") {
                return invalid_value(scheduler);
            }
            let newly = !socket.registered;
            socket.registered = true;
            drop(socket);
            let registry = lock(&scheduler.node().resources.kind::<Sockets>()).registry;
            if let (true, Some(registry)) = (newly, registry) {
                send_registry(scheduler, registry, "add", reference)?;
            }
            ok
        }
    }
}

/// Run `esock_getopt_otp` (`prim_socket_nif.c:11593-12010`).
///
/// # Errors
///
/// A new atom in a full atom table gives the error of `AtomTable::put`.
fn getopt_otp(
    scheduler: &mut Scheduler,
    state: &Arc<Mutex<SocketState>>,
    opt: i32,
) -> Result<BifResult, BeamError> {
    let socket = lock(state);
    // Each handler but the one of `use_registry` checks the read or the
    // write state first.
    let bits = match opt {
        OTP_USE_REGISTRY => 0,
        OTP_SNDCTRLBUF | OTP_META => socket.write,
        OTP_DEBUG | OTP_IOW | OTP_CTRL_PROC | OTP_SELECT_READ | OTP_RCVBUF | OTP_RCVCTRLBUF
        | OTP_FD | OTP_DOMAIN => socket.read,
        _ => return invalid_otp_option(scheduler, opt),
    };
    if bits & (CLOSING | CLOSED) != 0 {
        return Ok(closed());
    }
    let options = socket.options;
    let value = match opt {
        OTP_USE_REGISTRY => Term::from(socket.registered),
        // PBeam does not set `dbg` (see `setopt_otp`).
        OTP_DEBUG => Term::static_atom("false"),
        OTP_SELECT_READ => Term::from(socket.select_read),
        OTP_IOW => Term::from(options.iow),
        OTP_CTRL_PROC => Term::Pid(socket.owner.expect("an open socket has an owner")),
        // `{N, Size}` while `rNum` is set, with `MKI` of the `unsigned int`
        // N, which wraps to an `int` (`prim_socket_nif.c:11838-11844`).
        OTP_RCVBUF if options.recv_count != 0 => Term::tuple(vec![
            Term::Integer((options.recv_count.cast_signed()).into()),
            size_term(options.recv_buffer),
        ]),
        OTP_RCVBUF => size_term(options.recv_buffer),
        OTP_RCVCTRLBUF => size_term(options.recv_ctrl),
        OTP_SNDCTRLBUF => size_term(options.send_ctrl),
        OTP_FD => Term::Integer(
            socket
                .socket
                .as_ref()
                .expect("an open socket has a host socket")
                .as_raw_fd()
                .into(),
        ),
        OTP_META => socket.meta.clone(),
        // OTP_DOMAIN, the last option of the match above.
        _ => domain_term(scheduler, socket.domain)?,
    };
    Ok(BifResult::Value(Term::tuple(vec![
        Term::static_atom("ok"),
        value,
    ])))
}

#[cfg(test)]
#[path = "../../../../../../tests/runtime/interpreter/native/prim_socket/socket/options.rs"]
mod tests;

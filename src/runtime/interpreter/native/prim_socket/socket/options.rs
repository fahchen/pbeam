//! The bindings of `prim_socket:nif_setopt/5` and `nif_getopt/3,5`
//! (`prim_socket_nif.c:7801-7867`, `11504-11585`), with the handlers of
//! the Unix I/O backend: `esock_setopt`, `esock_setopt_otp`,
//! `esock_getopt`, and `esock_getopt_otp` (`prim_socket_nif.c:19063-19065`).
//!
//! PBeam sets and gets a host option through the safe `nix` and `socket2`
//! APIs, which have one typed call for each option. So PBeam has a handler
//! only for the options of [`get_host`] and [`set_host`]. Each other option
//! of the host and a native value or value spec stop the run with a
//! runtime error (row "NIF stubs" of `docs/erts-differences.md`).

use nix::sys::socket::{getsockopt, setsockopt, sockopt};

use std::os::fd::AsRawFd;

use super::*;
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
    #[cfg(target_os = "linux")]
    Bytes(Vec<u8>),
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
    #[cfg(target_os = "linux")]
    Str,
    /// `esock_setopt_tcp_congestion` (`prim_socket_nif.c:9018-9027`).
    #[cfg(target_os = "linux")]
    Congestion,
    /// `esock_setopt_in6_update_membership` (`prim_socket_nif.c:8961-9005`).
    #[cfg(target_os = "linux")]
    Membership6,
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
    #[cfg(target_os = "linux")]
    Bytes(Vec<u8>),
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
    if native != 0 {
        return Err(unsupported("native option value"));
    }
    let socket = lock(&state);
    if socket.write & (CLOSING | CLOSED) != 0 {
        return Ok(closed());
    }
    // `prim_socket:setopt/3` gives a value form only with the numbers of
    // `nif_supports(options)` (prim_socket.erl:1273-1280), and the native
    // forms stop above.
    let name = option_name(level, opt).expect("prim_socket gives a host option");
    let Some(form) = set_form(name) else {
        return match get_form_only(name) {
            true => Ok(invalid(atom(scheduler, "socket_option")?)),
            false => Err(unsupported(&format!("option {name:?}"))),
        };
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
            .map(|value| Value::Int(value as i32)),
        Form::Hops => decode_hops(scheduler, value).map(Value::Int),
        Form::Addr => in_addr(scheduler, value).map(Value::Addr),
        Form::Membership => decode_membership(scheduler, value),
        Form::Tos => decode_tos(scheduler, value).map(Value::Int),
        #[cfg(target_os = "linux")]
        Form::Str => decode_str(value, libc::IFNAMSIZ).map(Value::Bytes),
        #[cfg(target_os = "linux")]
        Form::Congestion => decode_str(value, CONGESTION_NAME_MAX + 1).map(Value::Bytes),
        #[cfg(target_os = "linux")]
        Form::Membership6 => decode_membership6(scheduler, value),
    };
    let Some(decoded) = decoded else {
        return Ok(invalid(atom(scheduler, "value")?));
    };
    let host = socket
        .socket
        .as_ref()
        .expect("an open socket has a host socket");
    let result = set_host(host, name, opt, decoded).map(|()| None);
    host_result(scheduler, result)
}

/// Return the form of the host option `name` that PBeam sets, or `None`.
fn set_form(name: (&str, &str)) -> Option<Form> {
    match name {
        (
            "socket",
            "broadcast" | "keepalive" | "reuseaddr" | "reuseport" | "oobinline" | "dontroute",
        )
        | ("socket", "timestamp")
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
        ("socket", "bindtodevice") => Some(Form::Str),
        #[cfg(target_os = "linux")]
        ("tcp", "congestion") => Some(Form::Congestion),
        #[cfg(target_os = "linux")]
        ("ipv6", "add_membership" | "drop_membership") => Some(Form::Membership6),
        _ => None,
    }
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
fn decode_tos(scheduler: &Scheduler, value: &Term) -> Option<i32> {
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
        Some(number) => Some(i32::from(number as u8)),
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
fn tos_term(scheduler: &mut Scheduler, value: i32) -> Result<Term, BeamError> {
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
#[cfg(target_os = "linux")]
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

/// Return whether ERTS has no set handler for the host option `name` that
/// PBeam gets (`prim_socket_nif.c:3272-3274`, `3377-3379`, `3481-3488`,
/// `3580-3586`, `8512-8514`).
fn get_form_only(name: (&str, &str)) -> bool {
    match name {
        ("socket", "type") => true,
        #[cfg(target_os = "linux")]
        ("ip", "mtu") | ("socket", "domain" | "protocol") => true,
        _ => false,
    }
}

/// Return whether ERTS has no get handler for the host option `name` that
/// PBeam sets (`prim_socket_nif.c:3496-3532`, `3806-3833`, `12234-12236`).
fn set_form_only(name: (&str, &str)) -> bool {
    match name {
        ("ip", "add_membership" | "drop_membership") => true,
        #[cfg(target_os = "linux")]
        ("ipv6", "add_membership" | "drop_membership") => true,
        _ => false,
    }
}

/// Set the host option `name` with the host number `opt` of `host` to
/// `value`, as
/// `esock_setopt_level_opt` and `socket_setopt` do
/// (`prim_socket_nif.c:11384-11395`, `11422-11481`). Each option of the
/// form [`Form::Int`] has an unsigned type in `nix` or `socket2`, and a
/// negative value reaches the host as the same C `int` as on ERTS.
fn set_host(host: &Socket, name: (&str, &str), opt: i32, value: Value) -> io::Result<()> {
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
        #[cfg(target_os = "linux")]
        Value::Bytes(bytes) => {
            return keep_tos_and_priority(host, opt, || match name {
                ("tcp", "congestion") => host.set_tcp_congestion(&bytes),
                _ => host.bind_device(Some(bytes.as_slice())),
            });
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
    let count = value as u32;
    keep_tos_and_priority(host, opt, || {
        let result = match name {
            ("socket", "broadcast") => setsockopt(host, sockopt::Broadcast, &flag),
            ("socket", "keepalive") => setsockopt(host, sockopt::KeepAlive, &flag),
            ("socket", "reuseaddr") => setsockopt(host, sockopt::ReuseAddr, &flag),
            ("socket", "reuseport") => setsockopt(host, sockopt::ReusePort, &flag),
            ("socket", "oobinline") => setsockopt(host, sockopt::OobInline, &flag),
            ("socket", "dontroute") => setsockopt(host, sockopt::DontRoute, &flag),
            ("socket", "rcvbuf") => setsockopt(host, sockopt::RcvBuf, &(value as usize)),
            ("socket", "sndbuf") => setsockopt(host, sockopt::SndBuf, &(value as usize)),
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
            _ => return set_socket2(host, name, value),
        };
        result.map_err(io::Error::from)
    })
}

/// Set the host option `name` of `host` to the C `int` `value` with the
/// typed calls of `socket2`, which pass each `u32` and `bool` on as a C
/// `int`.
fn set_socket2(host: &Socket, name: (&str, &str), value: i32) -> io::Result<()> {
    let flag = value != 0;
    let count = value as u32;
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
        // ("ip", "ttl"), the last option of the form `Int` of `set_form`.
        _ => host.set_ttl_v4(count),
    }
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
/// PBeam do not read `Value` (`prim_socket_nif.c:13018`, `13042`).
///
/// # Errors
///
/// The kind `native` and an option without a handler in PBeam stop the
/// run. A new atom in a full atom table gives the error of
/// `AtomTable::put`.
pub(in crate::runtime::interpreter::native::prim_socket) fn getopt_5(
    scheduler: &mut Scheduler,
    arguments: &[Term],
) -> Result<BifResult, BeamError> {
    getopt(
        scheduler,
        &arguments[0],
        &arguments[1],
        &arguments[2],
        Some(&arguments[3]),
    )
}

/// Run `nif_getopt` with the kind `kind` of `nif_getopt/5`, or `None`.
fn getopt(
    scheduler: &mut Scheduler,
    socket: &Term,
    level: &Term,
    opt: &Term,
    kind: Option<&Term>,
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
    if kind.is_some_and(|kind| atom_name(scheduler, kind) == Some("native")) {
        return Err(unsupported("native value spec"));
    }
    let socket = lock(&state);
    if socket.read & (CLOSING | CLOSED) != 0 {
        return Ok(closed());
    }
    let Some(name) = option_name(level, opt) else {
        return Ok(invalid(atom(scheduler, "socket_option")?));
    };
    let host = socket
        .socket
        .as_ref()
        .expect("an open socket has a host socket");
    if set_form_only(name) {
        return Ok(invalid(atom(scheduler, "socket_option")?));
    }
    let Some(got) = get_host(host, name) else {
        return Err(unsupported(&format!("option {name:?}")));
    };
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
                ("onoff", boolean(linger.l_onoff != 0)),
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
        #[cfg(target_os = "linux")]
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

/// Read the host option `name` of `host`, or `None` for an option without
/// a handler in PBeam.
fn get_host(host: &Socket, name: (&str, &str)) -> Option<io::Result<Got>> {
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
        ("socket", "rcvbuf") => {
            getsockopt(host, sockopt::RcvBuf).map(|value| Got::Int(value as i64))
        }
        ("socket", "sndbuf") => {
            getsockopt(host, sockopt::SndBuf).map(|value| Got::Int(value as i64))
        }
        ("socket", "type") => {
            getsockopt(host, sockopt::SockType).map(|kind| Got::Type(kind as i32))
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
            getsockopt(host, sockopt::TcpKeepIdle).map(|value| Got::Int((value as i32).into()))
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
        _ => return get_socket2(host, name),
    };
    Some(result.map_err(io::Error::from))
}

/// Read the host option `name` of `host` with the typed calls of
/// `socket2`, which read a C `int`, or `None` for an option without a
/// handler in PBeam. The casts `as i32` give back the bits of the `int`.
fn get_socket2(host: &Socket, name: (&str, &str)) -> Option<io::Result<Got>> {
    let int = |value: io::Result<u32>| value.map(|value| Got::Int((value as i32).into()));
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
        ("ip", "tos") => host.tos_v4().map(|value| Got::Tos(value as i32)),
        #[cfg(target_os = "linux")]
        ("tcp", "cork") => host.tcp_cork().map(Got::Bool),
        #[cfg(target_os = "linux")]
        ("ip", "multicast_all") => host.multicast_all_v4().map(Got::Bool),
        // ERTS gives each byte that the host gives, with the NUL byte
        // (`prim_socket_nif.c:12259-12268`, `13436-13446`); `socket2`
        // removes the last byte, which is the NUL byte of the name.
        #[cfg(target_os = "linux")]
        ("socket", "bindtodevice") => host.device().map(|device| {
            Got::Bytes(device.map_or_else(Vec::new, |name| [name, vec![0]].concat()))
        }),
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
        _ => return None,
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

/// Return `{error, {invalid, not_owner}}` of `esock_make_error_invalid`
/// (`prim_socket_nif.c:8075-8079`).
fn not_owner(scheduler: &mut Scheduler) -> Result<BifResult, BeamError> {
    let reason = Term::tuple(vec![
        Term::static_atom("invalid"),
        atom(scheduler, "not_owner")?,
    ]);
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
            if opt == OTP_IOW {
                socket.options.iow = flag;
            } else if flag {
                // `dbg` turns on the debug output of `SSDBG`, and
                // `selectRead` makes a read select after each receive
                // (`unix_socket_syncio.c:3967-3970`).
                return Err(unsupported(&format!("option value {{otp, {opt}}} true")));
            }
            ok
        }
        OTP_CTRL_PROC => {
            if socket.owner != Some(scheduler.current_process().id) {
                return not_owner(scheduler);
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
            let Some(size) = decode_bufsz(scheduler, value, default) else {
                // `{N, Size}` sets the buffer count `rNum`
                // (`prim_socket_nif.c:8198-8212`), which PBeam does not have.
                let count = value
                    .tuple_elements()
                    .filter(|elements| elements.len() == 2)
                    .filter(|elements| uint(&elements[0]).is_some_and(|count| count != 0))
                    .filter(|elements| decode_bufsz(scheduler, &elements[1], default).is_some());
                return match count {
                    Some(_) => Err(unsupported("option value {otp, rcvbuf} {N, Size}")),
                    None => invalid_value(scheduler),
                };
            };
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
                return not_owner(scheduler);
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
        OTP_USE_REGISTRY => boolean(socket.registered),
        // PBeam sets neither `dbg` nor `selectRead` (see `setopt_otp`).
        OTP_DEBUG | OTP_SELECT_READ => Term::static_atom("false"),
        OTP_IOW => boolean(options.iow),
        OTP_CTRL_PROC => Term::Pid(socket.owner.expect("an open socket has an owner")),
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

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

use super::*;
use crate::runtime::interpreter::native::prim_socket::supports::option_name;

/// The number `ESOCK_OPT_OTP_META` (`prim_socket_nif.c:928`).
const OTP_META: i32 = 1009;
/// The number `ESOCK_OPT_OTP_USE_REGISTRY` (`prim_socket_nif.c:929`).
const OTP_USE_REGISTRY: i32 = 1010;
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
}

/// The value form of a host option that [`set_host`] writes.
#[derive(Clone, Copy)]
enum Form {
    /// `esock_setopt_bool_opt` (`prim_socket_nif.c:11218-11234`).
    Bool,
    /// `esock_setopt_int_opt` (`prim_socket_nif.c:11241-11259`).
    Int,
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
    let [socket, level, opt, value, native] = arguments else {
        unreachable!("the binding has arity 5");
    };
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
        Form::Bool => match atom_name(scheduler, value) {
            Some("true") => Some(1),
            Some("false") => Some(0),
            _ => None,
        },
        Form::Int => int(value),
    };
    let Some(decoded) = decoded else {
        return Ok(invalid(atom(scheduler, "value")?));
    };
    let host = socket
        .socket
        .as_ref()
        .expect("an open socket has a host socket");
    let result = set_host(host, name, decoded).map(|()| None);
    host_result(scheduler, result)
}

/// Return the form of the host option `name` that PBeam sets, or `None`.
fn set_form(name: (&str, &str)) -> Option<Form> {
    match name {
        (
            "socket",
            "broadcast" | "keepalive" | "reuseaddr" | "reuseport" | "oobinline" | "dontroute",
        )
        | ("tcp", "nodelay")
        | ("ipv6", "v6only") => Some(Form::Bool),
        ("socket", "rcvbuf" | "sndbuf") | ("tcp", "keepcnt" | "keepintvl") | ("ip", "ttl") => {
            Some(Form::Int)
        }
        _ => None,
    }
}

/// Return whether ERTS has no set handler for the host option `name` that
/// PBeam gets (`prim_socket_nif.c:3481-3488`, `8512-8514`).
fn get_form_only(name: (&str, &str)) -> bool {
    name == ("socket", "type")
}

/// Set the host option `name` of `host` to `value`, as
/// `esock_setopt_level_opt` and `socket_setopt` do
/// (`prim_socket_nif.c:11384-11395`, `11422-11481`). Each option of the
/// form [`Form::Int`] has an unsigned type in `nix` or `socket2`, and a
/// negative value reaches the host as the same C `int` as on ERTS.
fn set_host(host: &Socket, name: (&str, &str), value: i32) -> io::Result<()> {
    let flag = value != 0;
    // The safe APIs pass these values on as a C `int` (`nix` `SetUsize`
    // casts with `as c_int`, `socket2::set_ttl_v4` casts the `u32`), so the
    // bit-identical casts below give the host the `int` that ERTS gives.
    let count = value as u32;
    keep_tos_and_priority(host, || {
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
            // ("ip", "ttl"), the last option of `set_form`.
            _ => return host.set_ttl_v4(count),
        };
        result.map_err(io::Error::from)
    })
}

/// Run `set` on `host`, as `socket_setopt` does on Linux
/// (`prim_socket_nif.c:11428-11472`): read `SO_PRIORITY` and `IP_TOS`
/// first, and after a successful set, set `IP_TOS` and then `SO_PRIORITY`
/// back to the values that it could read. A failed reset of `SO_PRIORITY`
/// with `EPERM` is not an error. PBeam sets neither of these two options
/// yet, so it always resets both.
#[cfg(target_os = "linux")]
fn keep_tos_and_priority(host: &Socket, set: impl FnOnce() -> io::Result<()>) -> io::Result<()> {
    let priority = getsockopt(host, sockopt::Priority);
    let tos = getsockopt(host, sockopt::Ipv4Tos);
    set()?;
    if let Ok(tos) = tos {
        setsockopt(host, sockopt::Ipv4Tos, &tos)?;
    }
    if let Ok(priority) = priority {
        setsockopt(host, sockopt::Priority, &priority).or_else(|failure| match failure {
            nix::errno::Errno::EPERM => Ok(()),
            failure => Err(failure),
        })?;
    }
    Ok(())
}

/// Run `set` on `host`: macOS has no `SO_PRIORITY`, so `socket_setopt`
/// only sets the option (`prim_socket_nif.c:11474-11478`).
#[cfg(not(target_os = "linux"))]
fn keep_tos_and_priority(_host: &Socket, set: impl FnOnce() -> io::Result<()>) -> io::Result<()> {
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
        ("ip", "ttl") => return Some(host.ttl_v4().map(|value| Got::Int(value.into()))),
        _ => return None,
    };
    Some(result.map_err(io::Error::from))
}

/// Run `esock_setopt_otp` for the options `meta` and `use_registry`
/// (`prim_socket_nif.c:7873-7961`, `8328-8405`).
///
/// # Errors
///
/// Each other option of the level `otp` stops the run. A new atom in a full
/// atom table gives the error of `AtomTable::put`.
fn setopt_otp(
    scheduler: &mut Scheduler,
    reference: Reference,
    state: &Arc<Mutex<SocketState>>,
    opt: i32,
    value: &Term,
) -> Result<BifResult, BeamError> {
    let mut socket = lock(state);
    if !matches!(opt, OTP_META | OTP_USE_REGISTRY) {
        return Err(unsupported(&format!("option {{otp, {opt}}}")));
    }
    if socket.write & (CLOSING | CLOSED) != 0 {
        return Ok(closed());
    }
    if opt == OTP_META {
        // `esock_make_error_invalid` gives `{error, {invalid, not_owner}}`.
        if socket.owner != Some(scheduler.current_process().id) {
            let reason = Term::tuple(vec![
                Term::static_atom("invalid"),
                atom(scheduler, "not_owner")?,
            ]);
            return Ok(BifResult::Value(Term::tuple(vec![
                Term::static_atom("error"),
                reason,
            ])));
        }
        socket.meta = value.clone();
        return Ok(BifResult::Value(Term::static_atom("ok")));
    }
    // Only `true` is valid: the option only turns the registry on.
    if *value != Term::static_atom("true") {
        return Ok(invalid(atom(scheduler, "value")?));
    }
    if !socket.registered {
        socket.registered = true;
        drop(socket);
        let registry = lock(&scheduler.node().resources.kind::<Sockets>()).registry;
        if let Some(registry) = registry {
            send_registry(scheduler, registry, "add", reference)?;
        }
    }
    Ok(BifResult::Value(Term::static_atom("ok")))
}

/// Run `esock_getopt_otp` for the options `domain`, `meta`, and
/// `use_registry` (`prim_socket_nif.c:11593-11710`, `11946-12005`).
///
/// # Errors
///
/// Each other option of the level `otp` stops the run. A new atom in a full
/// atom table gives the error of `AtomTable::put`.
fn getopt_otp(
    scheduler: &mut Scheduler,
    state: &Arc<Mutex<SocketState>>,
    opt: i32,
) -> Result<BifResult, BeamError> {
    let socket = lock(state);
    let value = match opt {
        OTP_USE_REGISTRY => Term::static_atom(if socket.registered { "true" } else { "false" }),
        OTP_META if socket.write & (CLOSING | CLOSED) != 0 => return Ok(closed()),
        OTP_DOMAIN if socket.read & (CLOSING | CLOSED) != 0 => return Ok(closed()),
        OTP_META => socket.meta.clone(),
        OTP_DOMAIN => domain_term(scheduler, socket.domain)?,
        _ => return Err(unsupported(&format!("option {{otp, {opt}}}"))),
    };
    Ok(BifResult::Value(Term::tuple(vec![
        Term::static_atom("ok"),
        value,
    ])))
}

#[cfg(test)]
#[path = "../../../../../../tests/runtime/interpreter/native/prim_socket/socket/options.rs"]
mod tests;

//! The load-check table of the `net` NIF library
//! (`erts/emulator/nifs/common/prim_net_nif.c`), which the module
//! `prim_net` loads with `erlang:load_nif("net", Extra)`
//! (`prim_net.erl:367-372`). ERTS registers the library under the module
//! `prim_net` (`prim_net_nif.c:5148`).
//!
//! PBeam binds `nif_info/0`, `nif_command/1`, `nif_gethostname/0`,
//! `nif_getnameinfo/2`, `nif_getaddrinfo/3`, `nif_getifaddrs/1`,
//! `nif_getservbyname/2`, `nif_getservbyport/2`, `nif_if_name2index/1`,
//! `nif_if_index2name/1`, `nif_if_names/0`, and the four Windows functions,
//! which raise `notsup` on Unix. The binding covers each function of the
//! table. The load callback of the library returns 0
//! for each load info: it reads only the optional `debug` flag of a map
//! (`prim_net_nif.c:5103-5107`, `5116-5146`). ERTS gives the veto 1 only
//! when it cannot make its resource type (`prim_net_nif.c:5135-5145`);
//! PBeam makes no resource type, so it has no such veto.

use std::ffi::{CString, OsString, c_int};
use std::os::unix::ffi::OsStrExt;

use super::prim_file::dirty;
use super::*;
use crate::runtime::host::{errno::errno_id, netdb};

mod ifaddrs;

/// The functions of the ERTS `net` NIF library, as `(name, arity)` in the
/// order of `net_funcs` (`prim_net_nif.c:4997-5022`). `erlang:load_nif/2`
/// checks each of them in this order. The flags of `nif_getifaddrs/1`,
/// `nif_get_adapters_addresses/1`, `nif_get_if_entry/1`,
/// `nif_get_interface_info/1`, `nif_get_ip_address_table/1`,
/// `nif_getservbyname/2`, and `nif_getservbyport/2` are
/// `ERL_NIF_DIRTY_JOB_IO_BOUND` (`prim_net_nif.c:5010-5016`), and the flags
/// of each other function are 0.
///
/// # Examples
///
/// ```rust,ignore
/// assert_eq!(FUNCTIONS[0], ("nif_info", 0));
/// assert_eq!(FUNCTIONS[14], ("nif_if_names", 0));
/// ```
pub(super) const FUNCTIONS: [(&str, u32); 15] = [
    ("nif_info", 0),
    ("nif_command", 1),
    ("nif_gethostname", 0),
    ("nif_getnameinfo", 2),
    ("nif_getaddrinfo", 3),
    ("nif_getifaddrs", 1),
    ("nif_get_adapters_addresses", 1),
    ("nif_get_if_entry", 1),
    ("nif_get_interface_info", 1),
    ("nif_get_ip_address_table", 1),
    ("nif_getservbyname", 2),
    ("nif_getservbyport", 2),
    ("nif_if_name2index", 1),
    ("nif_if_index2name", 1),
    ("nif_if_names", 0),
];

/// The bindings of the `net` NIF library, sorted by name.
///
/// # Examples
///
/// ```rust,ignore
/// assert_eq!((BINDINGS[0].name, BINDINGS[0].arity), ("nif_command", 1));
/// ```
pub(super) const BINDINGS: &[Binding] = &[
    Binding {
        name: "nif_command",
        arity: 1,
        job: Job::Normal,
        run: |scheduler, arguments| command(&scheduler.atoms, &arguments[0]),
    },
    Binding {
        name: "nif_get_adapters_addresses",
        arity: 1,
        job: Job::IoBound,
        run: |scheduler, _| notsup(&scheduler.atoms),
    },
    Binding {
        name: "nif_get_if_entry",
        arity: 1,
        job: Job::IoBound,
        run: |scheduler, _| notsup(&scheduler.atoms),
    },
    Binding {
        name: "nif_get_interface_info",
        arity: 1,
        job: Job::IoBound,
        run: |scheduler, _| notsup(&scheduler.atoms),
    },
    Binding {
        name: "nif_get_ip_address_table",
        arity: 1,
        job: Job::IoBound,
        run: |scheduler, _| notsup(&scheduler.atoms),
    },
    Binding {
        name: "nif_getaddrinfo",
        arity: 3,
        job: Job::Normal,
        run: |scheduler, arguments| getaddrinfo(&scheduler.atoms, &arguments[0], &arguments[1]),
    },
    Binding {
        name: "nif_gethostname",
        arity: 0,
        job: Job::Normal,
        run: |_, _| Ok(BifResult::Value(gethostname(nix::unistd::gethostname()))),
    },
    Binding {
        name: "nif_getifaddrs",
        arity: 1,
        job: Job::IoBound,
        run: |scheduler, arguments| getifaddrs(&scheduler.atoms, &arguments[0]),
    },
    Binding {
        name: "nif_getnameinfo",
        arity: 2,
        job: Job::Normal,
        run: |scheduler, arguments| getnameinfo(&scheduler.atoms, &arguments[0], &arguments[1]),
    },
    Binding {
        name: "nif_getservbyname",
        arity: 2,
        job: Job::IoBound,
        run: |_, arguments| getservbyname(&arguments[0], &arguments[1]),
    },
    Binding {
        name: "nif_getservbyport",
        arity: 2,
        job: Job::IoBound,
        run: |_, arguments| getservbyport(&arguments[0], &arguments[1]),
    },
    Binding {
        name: "nif_if_index2name",
        arity: 1,
        job: Job::Normal,
        run: |scheduler, arguments| if_index2name(&scheduler.atoms, &arguments[0]),
    },
    Binding {
        name: "nif_if_name2index",
        arity: 1,
        job: Job::Normal,
        run: |scheduler, arguments| if_name2index(&scheduler.atoms, &arguments[0]),
    },
    Binding {
        name: "nif_if_names",
        arity: 0,
        job: Job::Normal,
        run: |scheduler, _| if_names(&scheduler.atoms, nix::net::if_::if_nameindex()),
    },
    Binding {
        name: "nif_info",
        arity: 0,
        job: Job::Normal,
        run: |scheduler, _| info(&scheduler.atoms),
    },
];

/// Run `nif_info/0` (`prim_net_nif.c:848-870`): the map of the `debug`
/// flag and of `netns`. The debug flag of PBeam is always `false`: [`command`]
/// does not set it, and the load info `#{debug => true}` is not checked
/// (row "NIF stubs" of `docs/erts-differences.md`). `netns` is `false` on each host: the
/// pinned OTP build defines `HAVE_SETNS`, but no OTP file defines the
/// `HAVE_NETNS` that `nif_info` reads (`prim_net_nif.c:853-857`).
///
/// # Examples
///
/// ```rust,ignore
/// assert!(matches!(info(&atoms)?, BifResult::Value(Term::Map(_))));
/// ```
fn info(atoms: &AtomTable) -> Result<BifResult, BeamError> {
    let pairs = vec![
        (Term::Atom(atoms.put("debug")?), Term::static_atom("false")),
        (Term::Atom(atoms.put("netns")?), Term::static_atom("false")),
    ];
    let map = Map::from_pairs(pairs, atoms).expect("atom keys fit each map form");
    Ok(BifResult::Value(Term::Map(map)))
}

/// Run `nif_command/1` (`prim_net_nif.c:886-956`) on the map `argument`.
///
/// A term that is not a map, or a map without the key `command` or `data`,
/// raises `badarg`. A command other than `debug` raises
/// `{invalid, {command, Command}}`, and data that is not a boolean raises
/// `{invalid, {data, Data}}` (`esock_raise_invalid`). The data `false`
/// gives `ok`. The data `true` stops the run with a runtime error: ERTS
/// then writes a debug trace of each later call, and PBeam has no such
/// trace (row "NIF stubs" of `docs/erts-differences.md`).
///
/// # Examples
///
/// ```rust,ignore
/// let command = Term::Atom(atoms.put("x")?);
/// assert!(matches!(command(&atoms, &command)?, BifResult::Error { .. }));
/// ```
fn command(atoms: &AtomTable, argument: &Term) -> Result<BifResult, BeamError> {
    let Term::Map(map) = argument else {
        return Ok(BifResult::badarg());
    };
    let command = map.get(&Term::Atom(atoms.put("command")?));
    let data = map.get(&Term::Atom(atoms.put("data")?));
    let (Some(command), Some(data)) = (command, data) else {
        return Ok(BifResult::badarg());
    };
    let (key, value) = if *command != Term::Atom(atoms.put("debug")?) {
        ("command", command)
    } else if *data == Term::static_atom("false") {
        return Ok(BifResult::Value(Term::static_atom("ok")));
    } else if *data == Term::static_atom("true") {
        return Err(error("prim_net:nif_command/1 debug trace is not supported"));
    } else {
        ("data", data)
    };
    let detail = Term::tuple(vec![Term::Atom(atoms.put(key)?), value.clone()]);
    Ok(BifResult::Raise {
        class: ExceptionClass::Error,
        reason: Term::tuple(vec![Term::Atom(atoms.put("invalid")?), detail]),
    })
}

/// Give the result of `nif_gethostname/0` (`prim_net_nif.c:967-1021`) for
/// the result of `gethostname`: `{ok, Name}` with the bytes of the name as
/// a list. `enet_gethostname` switches on the return value of
/// `gethostname`, which is -1 on an error, not an `errno`, so each error
/// gives `{error, -1}` (`prim_net_nif.c:994-1017`).
///
/// # Examples
///
/// ```rust,ignore
/// let name = gethostname(Ok("h".into()));
/// assert_eq!(name, Term::tuple(vec![Term::static_atom("ok"), Term::list_from([Term::Integer(104)])]));
/// ```
fn gethostname(result: nix::Result<OsString>) -> Term {
    match result {
        Ok(name) => {
            let bytes = name.as_bytes().iter();
            let name = Term::list_from(bytes.map(|&byte| Term::Integer(byte.into())));
            Term::tuple(vec![Term::static_atom("ok"), name])
        }
        Err(_) => Term::tuple(vec![Term::static_atom("error"), Term::Integer(-1)]),
    }
}

/// The `<netdb.h>` values that `prim_net_nif.c` switches on, as ERTS sees
/// them: glibc with `-D_GNU_SOURCE` (erts/configure.ac:543) on Linux, and
/// the macOS header without `_POSIX_C_SOURCE` on macOS.
#[cfg(target_os = "linux")]
mod eai {
    use std::ffi::c_int;
    pub(super) const BADFLAGS: c_int = -1;
    pub(super) const NONAME: c_int = -2;
    pub(super) const AGAIN: c_int = -3;
    pub(super) const FAIL: c_int = -4;
    pub(super) const NODATA: c_int = -5;
    pub(super) const FAMILY: c_int = -6;
    pub(super) const SOCKTYPE: c_int = -7;
    pub(super) const SERVICE: c_int = -8;
    pub(super) const ADDRFAMILY: c_int = -9;
    pub(super) const MEMORY: c_int = -10;
    pub(super) const SYSTEM: c_int = -11;
    pub(super) const OVERFLOW: c_int = -12;
    /// `NI_IDN` of glibc, which `decode_nameinfo_flags_list` accepts
    /// (`prim_net_nif.c:4617-4620`).
    pub(super) const NI_IDN: Option<c_int> = Some(32);
}

/// The `<netdb.h>` values of macOS (see the Linux module).
#[cfg(not(target_os = "linux"))]
mod eai {
    use std::ffi::c_int;
    pub(super) const ADDRFAMILY: c_int = 1;
    pub(super) const AGAIN: c_int = 2;
    pub(super) const BADFLAGS: c_int = 3;
    pub(super) const FAIL: c_int = 4;
    pub(super) const FAMILY: c_int = 5;
    pub(super) const MEMORY: c_int = 6;
    pub(super) const NODATA: c_int = 7;
    pub(super) const NONAME: c_int = 8;
    pub(super) const SERVICE: c_int = 9;
    pub(super) const SOCKTYPE: c_int = 10;
    pub(super) const SYSTEM: c_int = 11;
    pub(super) const OVERFLOW: c_int = 14;
    /// macOS has no `NI_IDN`, so the flag `idn` is not valid.
    pub(super) const NI_IDN: Option<c_int> = None;
}

/// The size of `sun_path` of a `sockaddr_un`.
#[cfg(target_os = "linux")]
const SUN_PATH_LEN: usize = 108;
/// The size of `sun_path` of a `sockaddr_un` on macOS.
#[cfg(not(target_os = "linux"))]
const SUN_PATH_LEN: usize = 104;

/// The size of a `sockaddr_storage`, the `ESockAddress` of ERTS
/// (`socket_int.h:154-156`).
const STORAGE_LEN: usize = 128;

/// Give `{error, Reason}`.
fn failure(atoms: &AtomTable, reason: &str) -> Result<BifResult, BeamError> {
    let reason = Term::Atom(atoms.put(reason)?);
    Ok(BifResult::Value(Term::tuple(vec![
        Term::static_atom("error"),
        reason,
    ])))
}

/// Give `{ok, Value}`.
fn success(value: Term) -> Result<BifResult, BeamError> {
    Ok(BifResult::Value(Term::tuple(vec![
        Term::static_atom("ok"),
        value,
    ])))
}

/// Give the string `bytes` as a list of bytes, as `enif_make_string` with
/// `ERL_NIF_LATIN1` does.
fn latin1(bytes: &[u8]) -> Term {
    Term::list_from(bytes.iter().map(|&byte| Term::Integer(byte.into())))
}

/// Read `term` as `enif_get_string` with `ERL_NIF_LATIN1` does: a proper
/// list of bytes, with at most `limit` elements when `limit` is given. The
/// C string ends at the first NUL byte.
fn c_string(term: &Term, limit: Option<usize>) -> Option<CString> {
    let elements = term.proper_list()?;
    let bytes: Vec<u8> = elements
        .map(|element| match element {
            Term::Integer(value) => u8::try_from(*value).ok(),
            _ => None,
        })
        .collect::<Option<_>>()?;
    if limit.is_some_and(|limit| bytes.len() > limit) {
        return None;
    }
    let end = bytes
        .iter()
        .position(|&byte| byte == 0)
        .unwrap_or(bytes.len());
    CString::new(&bytes[..end]).ok()
}

/// Read `term` as `enif_get_int`.
fn int(term: &Term) -> Option<i32> {
    match term {
        Term::Integer(value) => i32::try_from(*value).ok(),
        _ => None,
    }
}

/// Read `term` as `enif_get_uint`.
fn uint(term: &Term) -> Option<u32> {
    match term {
        Term::Integer(value) => u32::try_from(*value).ok(),
        _ => None,
    }
}

/// The first bytes of a socket address of the family `family` and the
/// size `length`: `sa_len` and the `sa_family_t` of one byte on macOS, and
/// the `sa_family_t` of two bytes on Linux. Each C cast truncates.
#[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
fn header(length: usize, family: c_int) -> Vec<u8> {
    #[cfg(target_os = "linux")]
    {
        let _ = length;
        (family as u16).to_ne_bytes().to_vec()
    }
    #[cfg(not(target_os = "linux"))]
    {
        vec![length as u8, family as u8]
    }
}

/// The family of the socket address `bytes`, which has at least the bytes
/// of [`header`].
fn family_of(bytes: &[u8]) -> c_int {
    #[cfg(target_os = "linux")]
    {
        c_int::from(u16::from_ne_bytes([bytes[0], bytes[1]]))
    }
    #[cfg(not(target_os = "linux"))]
    {
        c_int::from(bytes[1])
    }
}

/// Decode the socket address map `term` to the bytes of its C form, as
/// `esock_decode_sockaddr` does (`socket_util.c:484-548`). `None` is a
/// term that it does not decode.
fn sockaddr(atoms: &AtomTable, term: &Term) -> Result<Option<Vec<u8>>, BeamError> {
    let Term::Map(map) = term else {
        return Ok(None);
    };
    let atom = |name: &str| atoms.put(name).map(Term::Atom);
    let get = |name: &str| atom(name).map(|key| map.get(&key).cloned());
    let Some(family) = get("family")? else {
        return Ok(None);
    };
    Ok(if family == atom("inet")? {
        inet(get("port")?, get("addr")?, atoms)?
    } else if family == atom("inet6")? {
        let fields = [
            get("port")?,
            get("flowinfo")?,
            get("scope_id")?,
            get("addr")?,
        ];
        inet6(fields, atoms)?
    } else if family == atom("local")? {
        get("path")?.and_then(|path| local(&path))
    } else if family == atom("unspec")? {
        get("addr")?.and_then(|address| native(libc::AF_UNSPEC, &address))
    } else if let Some(family) = int(&family) {
        get("addr")?.and_then(|address| native(family, &address))
    } else {
        None
    })
}

/// Decode an IPv4 socket address, as `esock_decode_sockaddr_in` does
/// (`socket_util.c:1118-1166`): `htons` truncates the port to 16 bits.
#[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
fn inet(
    port: Option<Term>,
    address: Option<Term>,
    atoms: &AtomTable,
) -> Result<Option<Vec<u8>>, BeamError> {
    let Some(port) = port.as_ref().and_then(int) else {
        return Ok(None);
    };
    let Some(address) = address else {
        return Ok(None);
    };
    let Some(address) = in_addr(&address, atoms)? else {
        return Ok(None);
    };
    let mut bytes = header(16, libc::AF_INET);
    bytes.extend_from_slice(&(port as u16).to_be_bytes());
    bytes.extend_from_slice(&address);
    bytes.extend_from_slice(&[0; 8]);
    Ok(Some(bytes))
}

/// Decode an IPv4 address, as `esock_decode_in_addr` does
/// (`socket_util.c:1781-1843`).
fn in_addr(term: &Term, atoms: &AtomTable) -> Result<Option<[u8; 4]>, BeamError> {
    if let Term::Atom(atom) = term {
        let name = Some(atoms.name_of(atom));
        return Ok(match name {
            Some("loopback") => Some([127, 0, 0, 1]),
            Some("any") => Some([0; 4]),
            Some("broadcast") => Some([255; 4]),
            _ => None,
        });
    }
    Ok(match term.tuple_elements() {
        Some(elements) if elements.len() == 4 => {
            let bytes: Option<Vec<u8>> = elements
                .iter()
                .map(|element| int(element).and_then(|value| u8::try_from(value).ok()))
                .collect();
            bytes.map(|bytes| [bytes[0], bytes[1], bytes[2], bytes[3]])
        }
        _ => None,
    })
}

/// Decode an IPv6 socket address from its port, flow info, scope id, and
/// address, as `esock_decode_sockaddr_in6` does (`socket_util.c:1239-1325`).
/// ERTS sets `sin6_len` to the size of a `sockaddr_in`, 16, not 28
/// (`socket_util.c:1252-1254`).
#[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
fn inet6(fields: [Option<Term>; 4], atoms: &AtomTable) -> Result<Option<Vec<u8>>, BeamError> {
    let [port, flowinfo, scope_id, address] = fields;
    let port = port.as_ref().and_then(int);
    let flowinfo = flowinfo.as_ref().and_then(uint);
    let scope_id = scope_id.as_ref().and_then(uint);
    let (Some(port), Some(flowinfo), Some(scope_id), Some(address)) =
        (port, flowinfo, scope_id, address)
    else {
        return Ok(None);
    };
    let Some(address) = in6_addr(&address, atoms)? else {
        return Ok(None);
    };
    let mut bytes = header(16, libc::AF_INET6);
    bytes.extend_from_slice(&(port as u16).to_be_bytes());
    bytes.extend_from_slice(&flowinfo.to_ne_bytes());
    bytes.extend_from_slice(&address);
    bytes.extend_from_slice(&scope_id.to_ne_bytes());
    Ok(Some(bytes))
}

/// Decode an IPv6 address, as `esock_decode_in6_addr` does
/// (`socket_util.c:1896-1967`).
fn in6_addr(term: &Term, atoms: &AtomTable) -> Result<Option<[u8; 16]>, BeamError> {
    if let Term::Atom(atom) = term {
        let name = Some(atoms.name_of(atom));
        let mut loopback = [0; 16];
        loopback[15] = 1;
        return Ok(match name {
            Some("loopback") => Some(loopback),
            Some("any") => Some([0; 16]),
            _ => None,
        });
    }
    Ok(match term.tuple_elements() {
        Some(elements) if elements.len() == 8 => {
            let mut bytes = [0; 16];
            for (index, element) in elements.iter().enumerate() {
                let Some(value) = int(element).and_then(|value| u16::try_from(value).ok()) else {
                    return Ok(None);
                };
                bytes[index * 2..index * 2 + 2].copy_from_slice(&value.to_be_bytes());
            }
            Some(bytes)
        }
        _ => None,
    })
}

/// Decode a Unix domain socket address from its path, as
/// `esock_decode_sockaddr_un` does (`socket_util.c:1394-1440`). On Linux a
/// path that starts with a NUL byte needs no NUL at its end.
fn local(path: &Term) -> Option<Vec<u8>> {
    let Term::Binary(path) = path else {
        return None;
    };
    let path = path.bytes()?;
    let terminator = usize::from(!(cfg!(target_os = "linux") && path.first() == Some(&0)));
    if path.len() + terminator > SUN_PATH_LEN {
        return None;
    }
    let mut bytes = header(2 + path.len(), libc::AF_LOCAL);
    bytes.extend_from_slice(path);
    Some(bytes)
}

/// Decode a socket address of the family `family` from its binary
/// address, as `esock_decode_sockaddr_native` does
/// (`socket_util.c:2788-2821`).
fn native(family: c_int, address: &Term) -> Option<Vec<u8>> {
    let Term::Binary(address) = address else {
        return None;
    };
    let address = address.bytes()?;
    if address.len() > STORAGE_LEN - 2 {
        return None;
    }
    let mut bytes = header(2 + address.len(), family);
    bytes.extend_from_slice(address);
    Some(bytes)
}

/// Decode the flags of `nif_getnameinfo/2`, as `decode_nameinfo_flags`
/// does (`prim_net_nif.c:4551-4659`): `undefined` or a list of flags. The
/// walk stops at the first tail that is not a cons cell, so an improper
/// tail is not checked.
fn nameinfo_flags(atoms: &AtomTable, term: &Term) -> Result<Option<c_int>, BeamError> {
    if let Term::Atom(_) = term {
        return Ok((*term == Term::Atom(atoms.put("undefined")?)).then_some(0));
    }
    let Some((elements, _)) = term.list_elements() else {
        return Ok(None);
    };
    let mut flags = 0;
    for element in &elements {
        let name = match element {
            Term::Atom(atom) => Some(atoms.name_of(atom)),
            _ => None,
        };
        flags |= match name {
            Some("namereqd") => libc::NI_NAMEREQD,
            Some("dgram") => libc::NI_DGRAM,
            Some("nofqdn") => libc::NI_NOFQDN,
            Some("numerichost") => libc::NI_NUMERICHOST,
            Some("numericserv") => libc::NI_NUMERICSERV,
            Some("idn") if eai::NI_IDN.is_some() => eai::NI_IDN.unwrap_or(0),
            _ => return Ok(None),
        };
    }
    Ok(Some(flags))
}

/// Run `nif_getnameinfo/2` (`prim_net_nif.c:1036-1177`): a socket address
/// that does not decode gives `{error, einval}`, and then flags that do
/// not decode raise `badarg`.
fn getnameinfo(atoms: &AtomTable, address: &Term, flags: &Term) -> Result<BifResult, BeamError> {
    let Some(address) = sockaddr(atoms, address)? else {
        return failure(atoms, "einval");
    };
    let Some(flags) = nameinfo_flags(atoms, flags)? else {
        return Ok(BifResult::badarg());
    };
    match netdb::getnameinfo(&address, flags) {
        Ok((host, service)) => {
            let pairs = vec![
                (Term::Atom(atoms.put("host")?), latin1(&host)),
                (Term::Atom(atoms.put("service")?), latin1(&service)),
            ];
            success(Term::Map(
                Map::from_pairs(pairs, atoms).expect("atom keys fit each map form"),
            ))
        }
        Err((code, errno)) => failure(atoms, &nameinfo_error(code, errno)),
    }
}

/// The reason of the error `code` of `getnameinfo`, with the `errno` of an
/// `EAI_SYSTEM` (`prim_net_nif.c:1108-1174`).
fn nameinfo_error(code: c_int, errno: c_int) -> String {
    match code {
        eai::AGAIN => "eagain".into(),
        eai::BADFLAGS => "ebadflags".into(),
        eai::FAIL => "efail".into(),
        eai::FAMILY => "efamily".into(),
        eai::MEMORY => "emem".into(),
        eai::NONAME => "enoname".into(),
        eai::OVERFLOW => "eoverflow".into(),
        eai::SYSTEM => errno_id(errno),
        _ => "einval".into(),
    }
}

/// Read a host or a service of `nif_getaddrinfo/3`, as
/// `decode_addrinfo_string` does (`prim_net_nif.c:4667-4691`): `undefined`
/// gives NULL, and each other atom does not decode.
fn addrinfo_string(atoms: &AtomTable, term: &Term) -> Result<Option<Option<CString>>, BeamError> {
    if let Term::Atom(_) = term {
        return Ok((*term == Term::Atom(atoms.put("undefined")?)).then_some(None));
    }
    Ok(c_string(term, None).map(Some))
}

/// Run `nif_getaddrinfo/3` (`prim_net_nif.c:1194-1362`). The hints are
/// ignored (`prim_net_nif.c:1212`, `1227-1230`). A host or a service that
/// does not decode, or two NULL names, raise `badarg`.
fn getaddrinfo(atoms: &AtomTable, host: &Term, service: &Term) -> Result<BifResult, BeamError> {
    let Some(host) = addrinfo_string(atoms, host)? else {
        return Ok(BifResult::badarg());
    };
    let Some(service) = addrinfo_string(atoms, service)? else {
        return Ok(BifResult::badarg());
    };
    if host.is_none() && service.is_none() {
        return Ok(BifResult::badarg());
    }
    match netdb::getaddrinfo(host.as_deref(), service.as_deref()) {
        Ok(entries) => {
            let infos = entries
                .iter()
                .map(|entry| address_info(atoms, entry))
                .collect::<Result<Vec<_>, _>>()?;
            success(Term::list_from(infos))
        }
        Err(code) => failure(atoms, addrinfo_error(code)),
    }
}

/// The reason of the error `code` of `getaddrinfo`
/// (`prim_net_nif.c:1280-1359`).
fn addrinfo_error(code: c_int) -> &'static str {
    match code {
        eai::ADDRFAMILY => "eaddrfamily",
        eai::AGAIN => "eagain",
        eai::BADFLAGS => "ebadflags",
        eai::FAIL => "efail",
        eai::FAMILY => "efamily",
        eai::MEMORY => "emem",
        eai::NODATA => "enodata",
        eai::NONAME => "enoname",
        eai::SERVICE => "eservice",
        eai::SOCKTYPE => "esocktype",
        eai::SYSTEM => "esystem",
        _ => "einval",
    }
}

/// Encode an entry of `getaddrinfo`, as `encode_address_info` does
/// (`prim_net_nif.c:4766-4842`): the map of `family`, `socktype`,
/// `protocol`, and `addr`.
fn address_info(atoms: &AtomTable, entry: &netdb::AddressInfo) -> Result<Term, BeamError> {
    let pairs = vec![
        (
            Term::Atom(atoms.put("family")?),
            domain(atoms, entry.family)?,
        ),
        (
            Term::Atom(atoms.put("socktype")?),
            socktype(atoms, entry.socktype)?,
        ),
        (
            Term::Atom(atoms.put("protocol")?),
            Term::Integer(entry.protocol.into()),
        ),
        (
            Term::Atom(atoms.put("addr")?),
            encode_sockaddr(atoms, &entry.address)?,
        ),
    ];
    Ok(Term::Map(
        Map::from_pairs(pairs, atoms).expect("atom keys fit each map form"),
    ))
}

/// Encode a family, as `esock_encode_domain` does
/// (`socket_util.c:2229-2259`).
fn domain(atoms: &AtomTable, family: c_int) -> Result<Term, BeamError> {
    let name = match family {
        libc::AF_INET => "inet",
        libc::AF_INET6 => "inet6",
        libc::AF_LOCAL => "local",
        libc::AF_UNSPEC => "unspec",
        _ => return Ok(Term::Integer(family.into())),
    };
    Ok(Term::Atom(atoms.put(name)?))
}

/// Encode a socket type, as `encode_address_info_type` and
/// `esock_encode_type` do (`prim_net_nif.c:4809-4821`,
/// `socket_util.c:2420-2452`): 0 gives `any`.
fn socktype(atoms: &AtomTable, socktype: c_int) -> Result<Term, BeamError> {
    let name = match socktype {
        0 => "any",
        libc::SOCK_STREAM => "stream",
        libc::SOCK_DGRAM => "dgram",
        libc::SOCK_RAW => "raw",
        libc::SOCK_SEQPACKET => "seqpacket",
        libc::SOCK_RDM => "rdm",
        _ => return Ok(Term::Integer(socktype.into())),
    };
    Ok(Term::Atom(atoms.put(name)?))
}

/// Encode an IPv4 or an IPv6 socket address of full size, as
/// `esock_encode_sockaddr_in` and `esock_encode_sockaddr_in6` do
/// (`socket_util.c:1182-1218`, `1345-1374`). `MKI` makes the flow info
/// and the scope id signed. Each other address stops the run with a
/// runtime error: `getaddrinfo` with NULL hints gives only these two
/// families.
#[allow(clippy::cast_possible_wrap)]
fn encode_sockaddr(atoms: &AtomTable, bytes: &[u8]) -> Result<Term, BeamError> {
    let family = if bytes.len() >= 2 {
        family_of(bytes)
    } else {
        -1
    };
    let key = |name: &str| atoms.put(name).map(Term::Atom);
    let word =
        |at: usize| u32::from_ne_bytes([bytes[at], bytes[at + 1], bytes[at + 2], bytes[at + 3]]);
    let pairs = match family {
        libc::AF_INET if bytes.len() >= 8 => {
            let address = bytes[4..8]
                .iter()
                .map(|&byte| Term::Integer(byte.into()))
                .collect();
            vec![
                (key("family")?, key("inet")?),
                (
                    key("port")?,
                    Term::Integer(u16::from_be_bytes([bytes[2], bytes[3]]).into()),
                ),
                (key("addr")?, Term::tuple(address)),
            ]
        }
        libc::AF_INET6 if bytes.len() >= 28 => {
            let address = bytes[8..24]
                .chunks(2)
                .map(|pair| Term::Integer(u16::from_be_bytes([pair[0], pair[1]]).into()))
                .collect();
            vec![
                (key("family")?, key("inet6")?),
                (
                    key("port")?,
                    Term::Integer(u16::from_be_bytes([bytes[2], bytes[3]]).into()),
                ),
                (key("addr")?, Term::tuple(address)),
                (key("flowinfo")?, Term::Integer((word(4) as i32).into())),
                (key("scope_id")?, Term::Integer((word(24) as i32).into())),
            ]
        }
        _ => {
            return Err(error(
                "prim_net:nif_getaddrinfo/3 socket address of this family is not supported",
            ));
        }
    };
    Ok(Term::Map(
        Map::from_pairs(pairs, atoms).expect("atom keys fit each map form"),
    ))
}

/// Run `nif_getservbyname/2` on a dirty IO thread
/// (`prim_net_nif.c:4089-4149`, `5014`): a name or a protocol that is not
/// a string of at most 255 bytes, or no entry, gives `{error, einval}`.
/// The protocol `"any"` gives NULL. ERTS keeps the port in a `short`, so a
/// port above 32767 is negative.
#[allow(
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    clippy::cast_possible_wrap
)]
fn getservbyname(name: &Term, protocol: &Term) -> Result<BifResult, BeamError> {
    let name = c_string(name, Some(255));
    let protocol = c_string(protocol, Some(255));
    dirty(
        move || {
            let (name, protocol) = (name?, protocol?);
            let protocol = (protocol.as_bytes() != b"any").then_some(protocol);
            netdb::getservbyname(&name, protocol.as_deref())
        },
        |scheduler, port| match port {
            Some(port) => success(Term::Integer((u16::from_be(port as u16) as i16).into())),
            None => failure(&scheduler.atoms, "einval"),
        },
    )
}

/// Run `nif_getservbyport/2` on a dirty IO thread
/// (`prim_net_nif.c:4170-4257`, `5015`): a port that is not an `unsigned
/// int`, or a protocol that is not a string of at most 255 bytes, or no
/// entry, gives `{error, einval}`. `htons` truncates the port to 16 bits.
#[allow(clippy::cast_possible_truncation)]
fn getservbyport(port: &Term, protocol: &Term) -> Result<BifResult, BeamError> {
    let port = uint(port);
    let protocol = c_string(protocol, Some(255));
    dirty(
        move || {
            let (port, protocol) = (port?, protocol?);
            let protocol = (protocol.as_bytes() != b"any").then_some(protocol);
            netdb::getservbyport(c_int::from((port as u16).to_be()), protocol.as_deref())
        },
        |scheduler, name| match name {
            Some(name) => success(latin1(&name)),
            None => failure(&scheduler.atoms, "einval"),
        },
    )
}

/// Raise `error:notsup`, as each Windows function of the library does on
/// Unix (`prim_net_nif.c:1918-1919`, `3128-3129`, `3553-3554`,
/// `3786-3787`).
fn notsup(atoms: &AtomTable) -> Result<BifResult, BeamError> {
    Ok(BifResult::Raise {
        class: ExceptionClass::Error,
        reason: Term::Atom(atoms.put("notsup")?),
    })
}

/// Run `nif_getifaddrs/1` on a dirty IO thread
/// (`prim_net_nif.c:1381-1421`, `5010`). A term that is not a map raises
/// `badarg`. The key `debug` with the value `true` stops the run with a
/// runtime error: ERTS then writes a debug trace, and PBeam has none
/// (`esock_get_bool_from_map`, `socket_util.c:252-269`). On Linux, which
/// defines `HAVE_SETNS`, a key `netns` that is not a string raises `badarg`
/// (`socket_util.c:282-320`), and a string stops the run with a runtime
/// error: PBeam does not change the network namespace. On another host the
/// call stops the run with a runtime error (see [`ifaddrs`]).
fn getifaddrs(atoms: &AtomTable, extra: &Term) -> Result<BifResult, BeamError> {
    let Term::Map(map) = extra else {
        return Ok(BifResult::badarg());
    };
    if map.get(&Term::Atom(atoms.put("debug")?)) == Some(&Term::static_atom("true")) {
        return Err(error(
            "prim_net:nif_getifaddrs/1 debug trace is not supported",
        ));
    }
    #[cfg(target_os = "linux")]
    {
        if let Some(netns) = map.get(&Term::Atom(atoms.put("netns")?)) {
            if c_string(netns, None).is_none() {
                return Ok(BifResult::badarg());
            }
            return Err(error(
                "prim_net:nif_getifaddrs/1 network namespace is not supported",
            ));
        }
        dirty(ifaddrs::read, |scheduler, entries| {
            ifaddrs::result(&scheduler.atoms, entries)
        })
    }
    #[cfg(not(target_os = "linux"))]
    {
        Err(error(
            "prim_net:nif_getifaddrs/1 is supported only on Linux",
        ))
    }
}

/// Run `nif_if_name2index/1` (`prim_net_nif.c:4272-4330`): a name that is
/// not a string of at most `IF_NAMESIZE` (16) bytes gives
/// `{error, einval}`, and a failed `if_nametoindex` gives
/// `{error, Errno}`. `MKI` makes the index signed.
#[allow(clippy::cast_possible_wrap)]
fn if_name2index(atoms: &AtomTable, name: &Term) -> Result<BifResult, BeamError> {
    let Some(name) = c_string(name, Some(16)) else {
        return failure(atoms, "einval");
    };
    match nix::net::if_::if_nametoindex(name.as_c_str()) {
        Ok(index) => success(Term::Integer((index as i32).into())),
        Err(errno) => failure(atoms, &errno_id(errno as c_int)),
    }
}

/// Run `nif_if_index2name/1` (`prim_net_nif.c:4346-4399`): an index that
/// is not an `unsigned int` raises `badarg`, and a failed
/// `if_indextoname` gives `{error, enxio}`. The `nix` wrapper does not see
/// the NULL result of a failed call and gives the empty name of its zeroed
/// buffer, so an empty name is a failure: no interface has an empty name.
fn if_index2name(atoms: &AtomTable, index: &Term) -> Result<BifResult, BeamError> {
    let Some(index) = uint(index) else {
        return Ok(BifResult::badarg());
    };
    match nix::net::if_::if_indextoname(index) {
        Ok(name) if !name.as_bytes().is_empty() => success(latin1(name.as_bytes())),
        _ => failure(atoms, "enxio"),
    }
}

/// Give the result of `nif_if_names/0` (`prim_net_nif.c:4413-4494`) for the
/// result of `if_nameindex`: `{ok, [{Index, Name}]}`, or `{error, Errno}`.
/// `MKI` makes each index signed.
#[allow(clippy::cast_possible_wrap)]
fn if_names(
    atoms: &AtomTable,
    interfaces: nix::Result<nix::net::if_::Interfaces>,
) -> Result<BifResult, BeamError> {
    match interfaces {
        Ok(interfaces) => success(Term::list_from(interfaces.iter().map(|interface| {
            Term::tuple(vec![
                Term::Integer((interface.index() as i32).into()),
                latin1(interface.name().to_bytes()),
            ])
        }))),
        Err(errno) => failure(atoms, &errno_id(errno as c_int)),
    }
}

#[cfg(test)]
#[path = "../../../../tests/runtime/interpreter/native/prim_net.rs"]
mod tests;

//! The control messages of `prim_socket:nif_sendmsg/5` and
//! `nif_recvmsg/5`: `decode_cmsghdrs` (`unix_socket_syncio.c:7214-7543`)
//! writes the `ctrl` list of a message into a control buffer, and
//! `encode_cmsgs` (`unix_socket_syncio.c:9021-9142`) reads a received
//! control buffer into a list of maps. Both use the tables of
//! `prim_socket_nif.c:15645-15758`. The host has no SCTP, so the SCTP table
//! of `prim_socket_nif.c:15761-15964` is not compiled.
//!
//! The buffers have the `cmsghdr` layout of the host, which PBeam reads and
//! writes as bytes: a `cmsg_len` of `size_t` on Linux and of `socklen_t` on
//! macOS, then the `int` level and the `int` type, with the data at the
//! aligned end of the header.

use std::mem::{align_of, size_of};

use super::options::{decode_tos, tos_term};
use super::{BeamError, Scheduler, Term, atom, atom_name, error, field, int, map, pairs};

/// The size of the `cmsg_len` field of the `cmsghdr` of the host:
/// `size_t` on Linux.
#[cfg(target_os = "linux")]
const LENGTH: usize = size_of::<usize>();
/// The size of the `cmsg_len` field of the `cmsghdr` of the host:
/// `socklen_t` on macOS.
#[cfg(not(target_os = "linux"))]
const LENGTH: usize = size_of::<u32>();

/// Return the `cmsg_len` of the `LENGTH` bytes `bytes`.
#[cfg(target_os = "linux")]
fn read_length(bytes: &[u8]) -> usize {
    usize::from_ne_bytes(bytes.try_into().expect("a cmsg_len"))
}

/// Return the `cmsg_len` of the `LENGTH` bytes `bytes`.
#[cfg(not(target_os = "linux"))]
fn read_length(bytes: &[u8]) -> usize {
    u32::from_ne_bytes(bytes.try_into().expect("a cmsg_len")) as usize
}

/// Return the `LENGTH` bytes of the `cmsg_len` `length`.
#[cfg(target_os = "linux")]
fn length_bytes(length: usize) -> Vec<u8> {
    length.to_ne_bytes().to_vec()
}

/// Return the `LENGTH` bytes of the `cmsg_len` `length`, which the control
/// buffer of at most `wCtrlSz` bytes keeps in the `socklen_t` range.
#[cfg(not(target_os = "linux"))]
fn length_bytes(length: usize) -> Vec<u8> {
    (length as u32).to_ne_bytes().to_vec()
}

/// Return the `tv_usec` of the bytes after the `tv_sec` of a `timeval`: a
/// `suseconds_t` of 8 bytes on Linux.
#[cfg(target_os = "linux")]
fn microseconds(bytes: &[u8]) -> i64 {
    i64::from_ne_bytes(bytes[..8].try_into().expect("tv_usec"))
}

/// Return the `tv_usec` of the bytes after the `tv_sec` of a `timeval`: a
/// `suseconds_t` of 4 bytes on macOS.
#[cfg(not(target_os = "linux"))]
fn microseconds(bytes: &[u8]) -> i64 {
    i32::from_ne_bytes(bytes[..4].try_into().expect("tv_usec")).into()
}

/// The size of a `cmsghdr`, `CMSG_LEN(0)`, which is also the offset of
/// `CMSG_DATA`: the header is a multiple of its alignment on Linux and macOS.
const HEADER: usize = size_of::<libc::cmsghdr>();

/// Return `length` rounded up to the alignment of a control message, as
/// `CMSG_ALIGN` does: `size_t` on Linux and 4 bytes on macOS.
const fn align(length: usize) -> usize {
    let alignment = align_of::<libc::cmsghdr>();
    (length + alignment - 1) & !(alignment - 1)
}

/// The index of the host in the type rows of [`SPECS`]: 0 for macOS and 1
/// for Linux.
#[cfg(not(target_os = "linux"))]
const HOST: usize = 0;
/// The index of the host in the type rows of [`SPECS`]: 0 for macOS and 1
/// for Linux.
#[cfg(target_os = "linux")]
const HOST: usize = 1;

/// The `encode` function of a row of the table: it gives the `value` of a
/// received control message.
#[derive(Debug, Clone, Copy)]
enum Encode {
    /// No function: the map has no `value`.
    None,
    /// `esock_cmsg_encode_int` (`prim_socket_nif.c:14375-14387`).
    Int,
    /// `esock_cmsg_encode_uchar` (`prim_socket_nif.c:14435-14447`).
    Uchar,
    /// `esock_cmsg_encode_ip_tos` (`prim_socket_nif.c:14335-14349`).
    Tos,
    /// `esock_cmsg_encode_in_pktinfo` (`prim_socket_nif.c:14453-14490`).
    InPktinfo,
    /// `esock_cmsg_encode_in6_pktinfo` (`prim_socket_nif.c:15605-15626`).
    In6Pktinfo,
    /// `esock_cmsg_encode_timeval` (`prim_socket_nif.c:14266-14277`).
    Timeval,
    /// A function that PBeam does not run yet.
    Unsupported,
}

/// The `decode` function of a row of the table: it gives the data of a
/// control message to send from its `value`.
#[derive(Debug, Clone, Copy)]
enum Decode {
    /// No function: the message needs `data`.
    None,
    /// `esock_cmsg_decode_int` (`prim_socket_nif.c:14390-14407`).
    Int,
    /// `esock_cmsg_decode_ip_tos` (`prim_socket_nif.c:14351-14367`).
    Tos,
    /// A function that PBeam does not run yet.
    Unsupported,
}

/// One row of the tables `cmsgLevelSocket`, `cmsgLevelIP`, and
/// `cmsgLevelIPv6` (`prim_socket_nif.c:15645-15758`): the level, the type
/// on macOS and on Linux (`None` where the host does not define it), the
/// name, and the functions.
type Spec = (i32, [Option<i32>; 2], &'static str, Encode, Decode);

/// The rows of the control message tables, in their order.
const SPECS: [Spec; 17] = [
    (
        libc::SOL_SOCKET,
        [Some(3), Some(2)],
        "credentials",
        Encode::None,
        Decode::None,
    ),
    (
        libc::SOL_SOCKET,
        [Some(1), Some(1)],
        "rights",
        Encode::None,
        Decode::None,
    ),
    (
        libc::SOL_SOCKET,
        [Some(2), Some(29)],
        "timestamp",
        Encode::Timeval,
        Decode::Unsupported,
    ),
    (
        libc::SOL_SOCKET,
        [None, Some(35)],
        "timestampns",
        Encode::Unsupported,
        Decode::Unsupported,
    ),
    (
        libc::IPPROTO_IP,
        [Some(3), Some(1)],
        "tos",
        Encode::Tos,
        Decode::Tos,
    ),
    (
        libc::IPPROTO_IP,
        [Some(4), Some(2)],
        "ttl",
        Encode::Int,
        Decode::Int,
    ),
    (
        libc::IPPROTO_IP,
        [Some(24), Some(12)],
        "recvttl",
        Encode::Uchar,
        Decode::None,
    ),
    (
        libc::IPPROTO_IP,
        [Some(26), Some(8)],
        "pktinfo",
        Encode::InPktinfo,
        Decode::None,
    ),
    (
        libc::IPPROTO_IP,
        [None, Some(20)],
        "origdstaddr",
        Encode::Unsupported,
        Decode::None,
    ),
    (
        libc::IPPROTO_IP,
        [Some(27), Some(13)],
        "recvtos",
        Encode::Tos,
        Decode::None,
    ),
    (
        libc::IPPROTO_IP,
        [None, Some(11)],
        "recverr",
        Encode::Unsupported,
        Decode::None,
    ),
    (
        libc::IPPROTO_IPV6,
        [None, Some(50)],
        "pktinfo",
        Encode::In6Pktinfo,
        Decode::None,
    ),
    (
        libc::IPPROTO_IPV6,
        [None, Some(52)],
        "hoplimit",
        Encode::Int,
        Decode::Int,
    ),
    (
        libc::IPPROTO_IPV6,
        [Some(36), Some(67)],
        "tclass",
        Encode::Int,
        Decode::Int,
    ),
    (
        libc::IPPROTO_IPV6,
        [Some(35), Some(66)],
        "recvtclass",
        Encode::Int,
        Decode::None,
    ),
    (
        libc::IPPROTO_IPV6,
        [None, Some(25)],
        "recverr",
        Encode::Unsupported,
        Decode::None,
    ),
    (
        libc::IPPROTO_IPV6,
        [None, Some(11)],
        "flowinfo",
        Encode::Int,
        Decode::None,
    ),
];

/// Return the row of `level` and the number `kind`, as the linear search of
/// `esock_encode_cmsg` finds it (`prim_socket_nif.c:14187-14206`).
fn spec_of_type(level: i32, kind: i32) -> Option<&'static Spec> {
    SPECS
        .iter()
        .find(|(entry, types, ..)| *entry == level && types[HOST] == Some(kind))
}

/// Return the row of `level` and the name `name`, as
/// `esock_lookup_cmsg_spec` finds it (`prim_socket_nif.c:16000-16047`).
fn spec_of_name(level: i32, name: &str) -> Option<(&'static Spec, i32)> {
    SPECS.iter().find_map(|spec| match spec {
        (entry, types, entry_name, ..) if *entry == level && *entry_name == name => {
            types[HOST].map(|kind| (spec, kind))
        }
        _ => None,
    })
}

/// Return the error of a control message whose function PBeam does not run.
fn unsupported(name: &str, level: i32, kind: &str) -> BeamError {
    error(format!(
        "prim_socket:{name}: the control message {kind} of the level {level} is not supported: see \"NIF stubs\" in docs/erts-differences.md"
    ))
}

/// Return the `int` of the first 4 bytes of `data`.
fn read_int(data: &[u8]) -> Option<i32> {
    Some(i32::from_ne_bytes(data.get(..4)?.try_into().ok()?))
}

/// Return the `{A, B, C, D}` of the `in_addr` of `data`, as
/// `esock_encode_in_addr` does (`socket_util.c:1846-1866`).
fn in_addr_term(data: &[u8]) -> Term {
    Term::tuple(
        data.iter()
            .map(|&byte| Term::Integer(byte.into()))
            .collect(),
    )
}

/// Return the list of the control messages of `ctrl`, the bytes of a
/// received control buffer that the host filled from the start, as
/// `encode_cmsgs` does (`unix_socket_syncio.c:9021-9142`): for each one,
/// `#{level, type, data}` and `value` when the table row has a function and
/// the data is long enough. A message that ends after the buffer gives the
/// atom `bad_data` and ends the list (`unix_socket_syncio.c:9061-9081`).
/// The bytes after the last message are 0, so a `cmsg_len` of 0 ends the
/// list where the `msg_controllen` of the host ends.
///
/// # Errors
///
/// A table row whose function PBeam does not run stops the run. A new atom
/// in a full atom table gives the error of `AtomTable::put`.
pub(in super::super) fn encode_cmsgs(
    scheduler: &mut Scheduler,
    ctrl: &[u8],
) -> Result<Term, BeamError> {
    let mut messages = Vec::new();
    let mut offset = 0;
    while let Some(header) = ctrl.get(offset..offset + HEADER) {
        let (length, rest) = header.split_at(LENGTH);
        let length = read_length(length);
        if length == 0 {
            break;
        }
        let Some(data) = ctrl.get(offset + HEADER..offset + length) else {
            messages.push(atom(scheduler, "bad_data")?);
            break;
        };
        let level = read_int(rest).expect("a cmsg_level");
        let kind = read_int(&rest[4..]).expect("a cmsg_type");
        messages.push(encode_cmsg(scheduler, level, kind, data)?);
        offset += align(length);
    }
    Ok(Term::list_from(messages))
}

/// Return the map of one received control message, as `encode_cmsgs` and
/// `esock_encode_cmsg` make it (`unix_socket_syncio.c:9105-9128`,
/// `prim_socket_nif.c:14176-14215`): the level `socket` or a number, the
/// type name of the table or a number, the data, and the value.
fn encode_cmsg(
    scheduler: &mut Scheduler,
    level: i32,
    kind: i32,
    data: &[u8],
) -> Result<Term, BeamError> {
    let level_term = if level == libc::SOL_SOCKET {
        atom(scheduler, "socket")?
    } else {
        Term::Integer(level.into())
    };
    let mut entries = vec![("level", level_term), ("data", Term::binary(data.to_vec()))];
    let Some(&(_, _, name, encode, _)) = spec_of_type(level, kind) else {
        entries.push(("type", Term::Integer(kind.into())));
        let entries = pairs(scheduler, &entries)?;
        return Ok(map(scheduler, entries));
    };
    entries.push(("type", atom(scheduler, name)?));
    let value = match encode {
        Encode::None => None,
        Encode::Int => read_int(data).map(|value| Term::Integer(value.into())),
        Encode::Uchar => data.first().map(|&value| Term::Integer(value.into())),
        Encode::Tos => match data.first() {
            Some(&value) => Some(tos_term(scheduler, value.into())?),
            None => None,
        },
        Encode::InPktinfo => match data.get(..12) {
            Some(info) => {
                let index = u32::from_ne_bytes(info[..4].try_into().expect("an index"));
                let entries = [
                    ("ifindex", Term::Integer(index.into())),
                    ("spec_dst", in_addr_term(&info[4..8])),
                    ("addr", in_addr_term(&info[8..12])),
                ];
                let entries = pairs(scheduler, &entries)?;
                Some(map(scheduler, entries))
            }
            None => None,
        },
        Encode::In6Pktinfo => match data.get(..20) {
            Some(info) => {
                let segments = info[..16]
                    .chunks(2)
                    .map(|pair| Term::Integer(u16::from_be_bytes([pair[0], pair[1]]).into()))
                    .collect();
                let index = read_int(&info[16..]).expect("an index");
                let entries = [
                    ("addr", Term::tuple(segments)),
                    ("ifindex", Term::Integer(index.into())),
                ];
                let entries = pairs(scheduler, &entries)?;
                Some(map(scheduler, entries))
            }
            None => None,
        },
        Encode::Timeval => match data.get(..size_of::<libc::timeval>()) {
            Some(time) => {
                let seconds = i64::from_ne_bytes(time[..8].try_into().expect("tv_sec"));
                let micro = microseconds(&time[8..]);
                let entries = [
                    ("sec", Term::Integer(seconds)),
                    ("usec", Term::Integer(micro)),
                ];
                let entries = pairs(scheduler, &entries)?;
                Some(map(scheduler, entries))
            }
            None => None,
        },
        Encode::Unsupported => return Err(unsupported("nif_recvmsg/5", level, name)),
    };
    if let Some(value) = value {
        entries.push(("value", value));
    }
    let entries = pairs(scheduler, &entries)?;
    Ok(map(scheduler, entries))
}

/// Return the control buffer of the `ctrl` list `ctrl` of a message to
/// send, with at most `capacity` bytes, the `wCtrlSz` of the socket, as
/// `decode_cmsghdrs` makes it (`unix_socket_syncio.c:7214-7272`), or `None`
/// for a list that does not decode, which gives `{invalid, ctrl}`.
///
/// # Errors
///
/// A `value` whose table function PBeam does not run stops the run.
pub(in super::super) fn decode_cmsgs(
    scheduler: &Scheduler,
    ctrl: &Term,
    capacity: usize,
) -> Result<Option<Vec<u8>>, BeamError> {
    // `GET_LIST_LEN` takes only a proper list (`unix_socket_syncio.c:7234`).
    let Some(mut elements) = ctrl.list_iter() else {
        return Ok(None);
    };
    let list: Vec<&Term> = elements.by_ref().collect();
    if !matches!(elements.rest(), Term::Nil) {
        return Ok(None);
    }
    let mut buffer = Vec::new();
    for element in list {
        let Some((level, kind, data)) = decode_cmsg(scheduler, element)? else {
            return Ok(None);
        };
        // `esock_init_cmsghdr` (`prim_socket_nif.c:16056-16074`).
        let space = HEADER + align(data.len());
        if capacity - buffer.len() < space {
            return Ok(None);
        }
        let start = buffer.len();
        buffer.resize(start + space, 0);
        let header = [
            &length_bytes(HEADER + data.len())[..],
            &level.to_ne_bytes(),
            &kind.to_ne_bytes(),
        ]
        .concat();
        buffer[start..start + HEADER].copy_from_slice(&header);
        buffer[start + HEADER..start + HEADER + data.len()].copy_from_slice(&data);
    }
    Ok(Some(buffer))
}

/// Return the level, the type, and the data of one element of a `ctrl`
/// list, as `decode_cmsghdr` (`unix_socket_syncio.c:7295-7361`),
/// `decode_cmsghdr_value` (`7365-7445`), and `decode_cmsghdr_data`
/// (`7449-7542`) read it, or `None` for an element that does not decode.
///
/// # Errors
///
/// A `value` whose table function PBeam does not run stops the run.
fn decode_cmsg(
    scheduler: &Scheduler,
    element: &Term,
) -> Result<Option<(i32, i32, Vec<u8>)>, BeamError> {
    let atoms = &scheduler.atoms;
    let (Some(level), Some(kind)) = (
        field(atoms, element, "level"),
        field(atoms, element, "type"),
    ) else {
        return Ok(None);
    };
    // `esock_decode_level` (`socket_util.c:3038-3046`).
    let level = match atom_name(scheduler, level) {
        Some("socket") => libc::SOL_SOCKET,
        _ => match int(level) {
            Some(level) => level,
            None => return Ok(None),
        },
    };
    let value = field(atoms, element, "value");
    let Some(data) = field(atoms, element, "data") else {
        let (Some(value), Some(name)) = (value, atom_name(scheduler, kind)) else {
            return Ok(None);
        };
        let Some((&(_, _, name, _, decode), kind)) = spec_of_name(level, name) else {
            return Ok(None);
        };
        let value = match decode {
            Decode::None => None,
            Decode::Int => int(value),
            Decode::Tos => decode_tos(scheduler, value),
            Decode::Unsupported => return Err(unsupported("nif_sendmsg/5", level, name)),
        };
        return Ok(value.map(|value| (level, kind, value.to_ne_bytes().to_vec())));
    };
    if value.is_some() {
        return Ok(None);
    }
    let kind = match (int(kind), atom_name(scheduler, kind)) {
        (Some(kind), _) => kind,
        (None, Some(name)) => match spec_of_name(level, name) {
            Some((_, kind)) => kind,
            None => return Ok(None),
        },
        (None, None) => return Ok(None),
    };
    let data = match (data.binary_bytes(), int(data), atom_name(scheduler, data)) {
        (Some(bytes), ..) => bytes.to_vec(),
        (None, Some(value), _) => value.to_ne_bytes().to_vec(),
        (None, None, Some("true")) => 1i32.to_ne_bytes().to_vec(),
        (None, None, Some("false")) => 0i32.to_ne_bytes().to_vec(),
        _ => return Ok(None),
    };
    Ok(Some((level, kind, data)))
}

#[cfg(test)]
#[path = "../../../../../../tests/runtime/interpreter/native/prim_socket/socket/cmsg.rs"]
mod tests;

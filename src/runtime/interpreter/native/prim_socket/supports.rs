//! The bindings of `prim_socket:nif_supports/0,1`
//! (`prim_socket_nif.c:5272-5687`), which `prim_socket:init/0` calls at
//! the node boot (`prim_socket.erl:184-204`).
//!
//! ERTS takes most values from the C headers of the OTP build. PBeam keeps
//! them in the tables of this file, with one column for each supported
//! host OS: column 0 holds the values of the pinned ERTS on macOS, and
//! column 1 the values of the pinned ERTS on Linux, as `mise.toml` installs
//! them. The rows follow the declaration order of the ERTS tables. Both
//! builds have no `HAVE_SCTP`, so PBeam has no row of the SCTP option level
//! or of the default protocol `IPPROTO_SCTP`, and the flag
//! `MSG_NOTIFICATION` is 0. A request or a default protocol that no
//! supported host defines also has no row. The protocols of the host file
//! `/etc/protocols` come from the file at each call, as `getprotoent`
//! reads them.
//!
//! ERTS sorts the option tables by their numbers at load
//! (`prim_socket_nif.c:4379-4396`). The order of the elements of each list
//! of `nif_supports/1` is not observable: `prim_socket` does not export the
//! function, and `init/0` folds each list into a map
//! (`prim_socket.erl:195-255`). PBeam gives the elements in the reverse
//! declaration order, as the ERTS loops give them for unsorted tables.

use std::{fs, iter, str};

use super::*;

/// The column of the host OS in the tables of this file.
#[cfg(target_os = "macos")]
pub(super) const HOST: usize = 0;
/// The column of the host OS in the tables of this file.
#[cfg(target_os = "linux")]
pub(super) const HOST: usize = 1;

/// The host file that `getprotoent` reads.
const PROTOCOLS_FILE: &str = "/etc/protocols";

/// The features of `nif_supports/0`, in the order of `esock_supports_0`
/// (`prim_socket_nif.c:5300-5345`): `HAVE_SCTP`, `HAVE_IPV6`, `AF_LOCAL`,
/// `HAVE_SETNS`, and `HAVE_SENDFILE`.
const FEATURES: [(&str, [bool; 2]); 5] = [
    ("sctp", [false, false]),
    ("ipv6", [true, true]),
    ("local", [true, true]),
    ("netns", [false, true]),
    ("sendfile", [true, true]),
];

/// The rows of `esock_msg_flags` (`prim_socket_nif.c:526-624`), as
/// `(name, flag)`. A flag that the host does not define is 0.
pub(super) const MSG_FLAGS: [(&str, [i32; 2]); 12] = [
    ("cmsg_cloexec", [0, 1_073_741_824]),
    ("confirm", [0, 2048]),
    ("ctrunc", [32, 8]),
    ("dontroute", [4, 4]),
    ("eor", [8, 128]),
    ("errqueue", [0, 8192]),
    ("more", [0, 32768]),
    ("nosignal", [524_288, 16384]),
    ("oob", [1, 1]),
    ("peek", [2, 2]),
    ("trunc", [16, 32]),
    ("notification", [0, 0]),
];

/// The rows of `esock_ioctl_flags` (`prim_socket_nif.c:627-914`), as
/// `(name, flag)`. A flag that the host does not define is 0.
const IOCTL_FLAGS: [(&str, [i32; 2]); 34] = [
    ("up", [1, 1]),
    ("broadcast", [2, 2]),
    ("debug", [4, 4]),
    ("loopback", [8, 8]),
    ("pointopoint", [16, 16]),
    ("notrailers", [32, 32]),
    ("knowsepoch", [0, 0]),
    ("running", [64, 64]),
    ("noarp", [128, 128]),
    ("promisc", [256, 256]),
    ("allmulti", [512, 512]),
    ("master", [0, 1024]),
    ("oactive", [1024, 0]),
    ("slave", [0, 2048]),
    ("simplex", [2048, 0]),
    ("multicast", [32768, 4096]),
    ("link0", [4096, 0]),
    ("link1", [8192, 0]),
    ("link2", [16384, 0]),
    ("portsel", [0, 8192]),
    ("automedia", [0, 16384]),
    ("dynamic", [0, 32768]),
    ("cantconfig", [0, 0]),
    ("lower_up", [0, 65536]),
    ("ppromisc", [0, 0]),
    ("dormant", [0, 131_072]),
    ("monitor", [0, 0]),
    ("echo", [0, 262_144]),
    ("staticarp", [0, 0]),
    ("dying", [0, 0]),
    ("renaming", [0, 0]),
    ("nogroup", [0, 0]),
    ("dhcprunning", [0, 0]),
    ("private", [0, 0]),
];

/// The requests of `esock_supports_ioctl_requests`
/// (`prim_socket_nif.c:5509-5626`), in source order, as `(name, request)`.
/// `None` is a request that the host does not define.
pub(super) const IOCTL_REQUESTS: [(&str, [Option<u32>; 2]); 20] = [
    ("gifconf", [Some(3_222_038_820), Some(35090)]),
    ("nread", [Some(1_074_030_207), Some(21531)]),
    ("atmark", [Some(1_074_033_415), Some(35077)]),
    ("gifname", [None, Some(35088)]),
    ("gifindex", [None, Some(35123)]),
    ("gifflags", [Some(3_223_349_521), Some(35091)]),
    ("gifaddr", [Some(3_223_349_537), Some(35093)]),
    ("gifdstaddr", [Some(3_223_349_538), Some(35095)]),
    ("gifbrdaddr", [Some(3_223_349_539), Some(35097)]),
    ("gifnetmask", [Some(3_223_349_541), Some(35099)]),
    ("gifmtu", [Some(3_223_349_555), Some(35105)]),
    ("gifhwaddr", [None, Some(35111)]),
    ("gifmap", [None, Some(35184)]),
    ("giftxqlen", [None, Some(35138)]),
    ("sifflags", [Some(2_149_607_696), Some(35092)]),
    ("sifaddr", [Some(2_149_607_692), Some(35094)]),
    ("sifdstaddr", [Some(2_149_607_694), Some(35096)]),
    ("sifbrdaddr", [Some(2_149_607_699), Some(35098)]),
    ("sifmtu", [Some(2_149_607_732), Some(35106)]),
    ("siftxqlen", [None, Some(35139)]),
];

/// The options of one level, as `(name, option number)` for each host.
type Options = [(&'static str, [Option<i32>; 2])];

/// The options of `optLevelSocket` (`prim_socket_nif.c:3217-3490`), as
/// `(name, option number)`. `None` is an option without a set and a get
/// function on the host.
const SOCKET_OPTIONS: [(&str, [Option<i32>; 2]); 38] = [
    ("acceptconn", [Some(2), Some(30)]),
    ("acceptfilter", [None, None]),
    ("bindtodevice", [Some(4404), Some(25)]),
    ("broadcast", [Some(32), Some(6)]),
    ("busy_poll", [None, None]),
    ("bsp_state", [None, None]),
    ("busy_poll", [None, None]),
    ("debug", [Some(1), Some(1)]),
    ("domain", [None, Some(39)]),
    ("dontroute", [Some(16), Some(5)]),
    ("error", [None, None]),
    ("exclusiveaddruse", [None, None]),
    ("keepalive", [Some(8), Some(9)]),
    ("linger", [Some(128), Some(13)]),
    ("mark", [None, None]),
    ("maxdg", [None, None]),
    ("max_msg_size", [None, None]),
    ("oobinline", [Some(256), Some(10)]),
    ("passcred", [None, Some(16)]),
    ("peek_off", [None, Some(42)]),
    ("peercred", [None, None]),
    ("priority", [None, Some(12)]),
    ("protocol", [None, Some(38)]),
    ("rcvbuf", [Some(4098), Some(8)]),
    ("rcvbufforce", [None, None]),
    ("rcvlowat", [Some(4100), Some(18)]),
    ("rcvtimeo", [None, None]),
    ("reuseaddr", [Some(4), Some(2)]),
    ("reuseport", [Some(512), Some(15)]),
    ("rxq_ovfl", [None, None]),
    ("setfib", [None, None]),
    ("sndbuf", [Some(4097), Some(7)]),
    ("sndbufforce", [None, None]),
    ("sndlowat", [Some(4099), Some(19)]),
    ("sndtimeo", [None, None]),
    ("timestamp", [Some(1024), Some(29)]),
    ("timestampns", [None, Some(35)]),
    ("type", [Some(4104), Some(3)]),
];

/// The options of `optLevelIP` (`prim_socket_nif.c:3494-3789`), as
/// [`SOCKET_OPTIONS`] gives them.
const IP_OPTIONS: [(&str, [Option<i32>; 2]); 34] = [
    ("add_membership", [Some(12), Some(35)]),
    ("add_source_membership", [Some(70), Some(39)]),
    ("block_source", [Some(72), Some(38)]),
    ("dontfrag", [None, None]),
    ("drop_membership", [Some(13), Some(36)]),
    ("drop_source_membership", [Some(71), Some(40)]),
    ("freebind", [None, Some(15)]),
    ("hdrincl", [Some(2), Some(3)]),
    ("minttl", [None, Some(21)]),
    ("msfilter", [None, Some(41)]),
    ("mtu", [None, Some(14)]),
    ("mtu_discover", [None, Some(10)]),
    ("multicast_all", [None, Some(49)]),
    ("multicast_if", [Some(9), Some(32)]),
    ("multicast_loop", [Some(11), Some(34)]),
    ("multicast_ttl", [Some(10), Some(33)]),
    ("nodefrag", [None, Some(22)]),
    ("options", [None, None]),
    ("pktinfo", [Some(26), Some(8)]),
    ("pktoptions", [None, Some(9)]),
    ("recvdstaddr", [Some(7), None]),
    ("recverr", [None, Some(11)]),
    ("recvif", [Some(20), None]),
    ("recvopts", [Some(5), Some(6)]),
    ("recvorigdstaddr", [None, Some(20)]),
    ("recvtos", [Some(27), Some(13)]),
    ("recvttl", [Some(24), Some(12)]),
    ("retopts", [Some(8), Some(7)]),
    ("router_alert", [None, Some(5)]),
    ("sendsrcaddr", [None, None]),
    ("tos", [Some(3), Some(1)]),
    ("transparent", [None, Some(19)]),
    ("ttl", [Some(4), Some(2)]),
    ("unblock_source", [Some(73), Some(37)]),
];

/// The options of `optLevelIPV6` (`prim_socket_nif.c:3793-4023`), as
/// [`SOCKET_OPTIONS`] gives them.
const IPV6_OPTIONS: [(&str, [Option<i32>; 2]); 33] = [
    ("addrform", [None, Some(1)]),
    ("add_membership", [None, Some(20)]),
    ("authhdr", [None, Some(10)]),
    ("auth_level", [None, None]),
    ("checksum", [None, None]),
    ("drop_membership", [None, Some(21)]),
    ("dstopts", [None, Some(59)]),
    ("esp_network_level", [None, None]),
    ("esp_trans_level", [None, None]),
    ("faith", [None, None]),
    ("flowinfo", [None, Some(11)]),
    ("hoplimit", [None, Some(52)]),
    ("hopopts", [None, Some(54)]),
    ("ipcomp_level", [None, None]),
    ("join_group", [None, None]),
    ("leave_group", [None, None]),
    ("mtu", [None, Some(24)]),
    ("mtu_discover", [None, Some(23)]),
    ("multicast_hops", [Some(10), Some(18)]),
    ("multicast_if", [Some(9), Some(17)]),
    ("multicast_loop", [Some(11), Some(19)]),
    ("portrange", [None, None]),
    ("pktoptions", [None, None]),
    ("recverr", [None, Some(25)]),
    ("recvhoplimit", [None, Some(51)]),
    ("recvpktinfo", [None, Some(49)]),
    ("recvtclass", [Some(35), Some(66)]),
    ("router_alert", [None, Some(22)]),
    ("rthdr", [None, Some(57)]),
    ("tclass", [Some(36), Some(67)]),
    ("unicast_hops", [Some(4), Some(16)]),
    ("use_min_mtu", [None, None]),
    ("v6only", [Some(27), Some(26)]),
];

/// The options of `optLevelUDP` (`prim_socket_nif.c:4321-4333`), as
/// [`SOCKET_OPTIONS`] gives them.
const UDP_OPTIONS: [(&str, [Option<i32>; 2]); 1] = [("cork", [None, Some(1)])];

/// The options of `optLevelTCP` (`prim_socket_nif.c:4227-4317`), as
/// [`SOCKET_OPTIONS`] gives them.
const TCP_OPTIONS: [(&str, [Option<i32>; 2]); 13] = [
    ("congestion", [None, Some(13)]),
    ("cork", [None, Some(3)]),
    ("info", [None, None]),
    ("keepcnt", [Some(258), Some(6)]),
    ("keepidle", [None, Some(4)]),
    ("keepintvl", [Some(257), Some(5)]),
    ("maxseg", [Some(2), Some(2)]),
    ("md5sig", [None, None]),
    ("nodelay", [Some(1), Some(1)]),
    ("noopt", [None, None]),
    ("nopush", [Some(4), None]),
    ("syncnt", [None, None]),
    ("user_timeout", [None, Some(18)]),
];

/// The option levels of `optLevels` (`prim_socket_nif.c:4343-4375`), as
/// `(name, level number, options)`: `SOL_SOCKET`, `SOL_IP` or
/// `IPPROTO_IP`, `SOL_IPV6` or `IPPROTO_IPV6`, `IPPROTO_UDP`, and
/// `IPPROTO_TCP`.
const OPTION_LEVELS: [(&str, [i32; 2], &Options); 5] = [
    ("socket", [0xffff, 1], &SOCKET_OPTIONS),
    ("ip", [0, 0], &IP_OPTIONS),
    ("ipv6", [41, 41], &IPV6_OPTIONS),
    ("udp", [17, 17], &UDP_OPTIONS),
    ("tcp", [6, 6], &TCP_OPTIONS),
];

/// Return the names `(Level, Option)` of the option `option` of the level
/// number `level` on the host, as `lookupOpt` finds its entry
/// (`prim_socket_nif.c:4424-4441`), or `None` for an option that the
/// host does not have.
pub(super) fn option_name(level: i32, option: i32) -> Option<(&'static str, &'static str)> {
    let (name, _, table) = OPTION_LEVELS
        .iter()
        .find(|(_, numbers, _)| numbers[HOST] == level)?;
    let (option, _) = table
        .iter()
        .find(|(_, numbers)| numbers[HOST] == Some(option))?;
    Some((name, option))
}

/// The default protocols of `esock_supports_protocols`
/// (`prim_socket_nif.c:5451-5503`), in source order: `SOL_IP` or
/// `IPPROTO_IP`, `SOL_IPV6` or `IPPROTO_IPV6`, `IPPROTO_TCP`,
/// `IPPROTO_UDP`, `IPPROTO_IGMP`, and `IPPROTO_MPTCP`.
const DEFAULT_PROTOCOLS: [(&str, [Option<i32>; 2]); 6] = [
    ("ip", [Some(0), Some(0)]),
    ("ipv6", [Some(41), Some(41)]),
    ("tcp", [Some(6), Some(6)]),
    ("udp", [Some(17), Some(17)]),
    ("igmp", [Some(2), Some(2)]),
    ("mptcp", [None, Some(262)]),
];

/// Run `prim_socket:nif_supports()`, as `esock_supports_0` does
/// (`prim_socket_nif.c:5300-5345`): `{Feature, boolean()}` for each row of
/// [`FEATURES`], in order.
///
/// # Examples
///
/// ```rust,ignore
/// // [{sctp,false},{ipv6,true},{local,true},{netns,false},{sendfile,true}] on macOS
/// let BifResult::Value(features) = supports_0(&mut scheduler)? else { unreachable!() };
/// ```
///
/// # Errors
///
/// A new atom that does not fit in the atom table gives the error of
/// `AtomTable::put`.
pub(super) fn supports_0(scheduler: &mut Scheduler) -> Result<BifResult, BeamError> {
    let features = FEATURES
        .iter()
        .map(|&(name, values)| {
            let value = Term::static_atom(if values[HOST] { "true" } else { "false" });
            Ok(Term::tuple(vec![atom(scheduler, name)?, value]))
        })
        .collect::<Result<Vec<_>, BeamError>>()?;
    Ok(BifResult::Value(Term::list_from(features)))
}

/// Run `prim_socket:nif_supports(Key)`, as `esock_supports_1` does
/// (`prim_socket_nif.c:5349-5373`): the list of the key `msg_flags`,
/// `protocols`, `ioctl_requests`, `ioctl_flags`, or `options`, and `[]`
/// for each other term.
///
/// # Examples
///
/// ```rust,ignore
/// let key = Term::Atom(scheduler.atoms.put("other")?);
/// assert!(matches!(supports_1(&mut scheduler, &key)?, BifResult::Value(Term::Nil)));
/// ```
///
/// # Errors
///
/// A new atom that does not fit in the atom table gives the error of
/// `AtomTable::put`.
pub(super) fn supports_1(scheduler: &mut Scheduler, key: &Term) -> Result<BifResult, BeamError> {
    let name = match key {
        Term::Atom(atom) => scheduler.atoms.name_of(atom),
        _ => "",
    };
    let list: fn(&mut Scheduler) -> Result<Term, BeamError> = match name {
        "msg_flags" => |scheduler| flags(scheduler, &MSG_FLAGS),
        "protocols" => |scheduler| {
            let text = fs::read(PROTOCOLS_FILE).unwrap_or_default();
            protocols(scheduler, &protocol_entries(&text))
        },
        "ioctl_requests" => ioctl_requests,
        "ioctl_flags" => |scheduler| flags(scheduler, &IOCTL_FLAGS),
        "options" => options,
        _ => |_| Ok(Term::Nil),
    };
    list(scheduler).map(BifResult::Value)
}

/// Return the atom `name`.
pub(super) fn atom(scheduler: &mut Scheduler, name: &str) -> Result<Term, BeamError> {
    Ok(Term::Atom(scheduler.atoms.put(name)?))
}

/// Return `[{Name, Flag}]` of each row of `table`, as the loops of
/// `esock_supports_msg_flags` and `esock_supports_ioctl_flags` give it
/// (`prim_socket_nif.c:5377-5392`, `5630-5647`).
fn flags(scheduler: &mut Scheduler, table: &[(&str, [i32; 2])]) -> Result<Term, BeamError> {
    let flags = table
        .iter()
        .rev()
        .map(|&(name, values)| {
            Ok(Term::tuple(vec![
                atom(scheduler, name)?,
                Term::Integer(values[HOST].into()),
            ]))
        })
        .collect::<Result<Vec<_>, BeamError>>()?;
    Ok(Term::list_from(flags))
}

/// Return `[{Name, Request}]` of each request of [`IOCTL_REQUESTS`] that
/// the host defines, as `esock_supports_ioctl_requests` gives it
/// (`prim_socket_nif.c:5509-5626`).
fn ioctl_requests(scheduler: &mut Scheduler) -> Result<Term, BeamError> {
    let requests = IOCTL_REQUESTS
        .iter()
        .rev()
        .filter_map(|&(name, values)| Some((name, values[HOST]?)))
        .map(|(name, request)| {
            Ok(Term::tuple(vec![
                atom(scheduler, name)?,
                Term::Integer(request.into()),
            ]))
        })
        .collect::<Result<Vec<_>, BeamError>>()?;
    Ok(Term::list_from(requests))
}

/// Return `[{Level, LevelNumber, Options}]` of [`OPTION_LEVELS`], as
/// `esock_supports_options` gives it (`prim_socket_nif.c:5652-5687`). An
/// option that the host supports is `{Name, OptionNumber}`, and each other
/// option is the atom `Name`.
fn options(scheduler: &mut Scheduler) -> Result<Term, BeamError> {
    let levels = OPTION_LEVELS
        .iter()
        .rev()
        .map(|&(name, numbers, table)| {
            let options = table
                .iter()
                .rev()
                .map(|&(option, values)| {
                    let option = atom(scheduler, option)?;
                    Ok(match values[HOST] {
                        Some(number) => Term::tuple(vec![option, Term::Integer(number.into())]),
                        None => option,
                    })
                })
                .collect::<Result<Vec<_>, BeamError>>()?;
            Ok(Term::tuple(vec![
                atom(scheduler, name)?,
                Term::Integer(numbers[HOST].into()),
                Term::list_from(options),
            ]))
        })
        .collect::<Result<Vec<_>, BeamError>>()?;
    Ok(Term::list_from(levels))
}

/// Return the entries of the protocols file `text`, in file order, as
/// `getprotoent` gives them: the names `[Name | Aliases]` and the number of
/// each line. A `#` starts a comment, and white space separates the fields.
/// A line without a name and a decimal number gives no entry.
///
/// # Examples
///
/// ```rust,ignore
/// let entries = protocol_entries(b"# Internet\nip 0 IP # pseudo\nbad\n");
/// assert_eq!(entries, [(vec![&b"ip"[..], b"IP"], 0)]);
/// ```
fn protocol_entries(text: &[u8]) -> Vec<(Vec<&[u8]>, i32)> {
    text.split(|&byte| byte == b'\n')
        .filter_map(|line| {
            let line = line.split(|&byte| byte == b'#').next()?;
            let mut fields = line
                .split(u8::is_ascii_whitespace)
                .filter(|field| !field.is_empty());
            let name = fields.next()?;
            let number = str::from_utf8(fields.next()?).ok()?.parse().ok()?;
            Some((iter::once(name).chain(fields).collect(), number))
        })
        .collect()
}

/// Return the list of `esock_supports_protocols`
/// (`prim_socket_nif.c:5396-5506`): `{Names, Number}` of each entry of
/// `entries`, with the aliases in reverse order, last entry first, after
/// the rows of [`DEFAULT_PROTOCOLS`] that the host defines, last row
/// first. Each name is an atom of its Latin-1 bytes, as `enif_make_atom`
/// makes it.
fn protocols(scheduler: &mut Scheduler, entries: &[(Vec<&[u8]>, i32)]) -> Result<Term, BeamError> {
    let mut protocols = Vec::new();
    let defaults = DEFAULT_PROTOCOLS
        .iter()
        .rev()
        .filter_map(|&(name, values)| Some((name, values[HOST]?)));
    for (name, number) in defaults {
        let names = Term::list_from([atom(scheduler, name)?]);
        protocols.push(Term::tuple(vec![names, Term::Integer(number.into())]));
    }
    for (names, number) in entries.iter().rev() {
        let mut atoms = vec![latin1_atom(scheduler, names[0])?];
        for alias in names[1..].iter().rev() {
            atoms.push(latin1_atom(scheduler, alias)?);
        }
        protocols.push(Term::tuple(vec![
            Term::list_from(atoms),
            Term::Integer((*number).into()),
        ]));
    }
    Ok(Term::list_from(protocols))
}

/// Return the atom of the Latin-1 bytes `name`.
fn latin1_atom(scheduler: &mut Scheduler, name: &[u8]) -> Result<Term, BeamError> {
    atom(
        scheduler,
        &name.iter().copied().map(char::from).collect::<String>(),
    )
}

#[cfg(test)]
#[path = "../../../../../tests/runtime/interpreter/native/prim_socket/supports.rs"]
mod tests;

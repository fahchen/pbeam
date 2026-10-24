//! `nif_getifaddrs/1` of the `net` NIF library
//! (`prim_net_nif.c:1381-1877`). The safe `nix` wrapper of `getifaddrs`
//! gives each socket address only for the families that it knows, so
//! PBeam reads the list only on Linux, where `getifaddrs` gives only the
//! families `inet`, `inet6`, and `packet`. On another host the call stops
//! the run with a runtime error (row "NIF stubs" of
//! `docs/erts-differences.md`).

// The encoder runs only on Linux; the unit tests run it on each host.
#![cfg_attr(not(target_os = "linux"), allow(dead_code))]

use super::*;

/// A socket address of an entry: the bytes of a full `inet` or `inet6`
/// socket address for [`encode_sockaddr`], or the fields of a
/// `sockaddr_ll`.
pub(super) enum Address {
    Socket(Vec<u8>),
    Packet {
        protocol: u16,
        ifindex: i32,
        hatype: u16,
        pkttype: u8,
        addr: Vec<u8>,
    },
}

/// An entry of the list of `getifaddrs`. `extra` is the key `dstaddr` or
/// `broadaddr` and its address.
pub(super) struct Entry {
    pub(super) name: Vec<u8>,
    pub(super) flags: u32,
    pub(super) addr: Option<Address>,
    pub(super) netmask: Option<Address>,
    pub(super) extra: Option<(&'static str, Address)>,
}

/// The failure of [`read`]: the `errno` of `getifaddrs`, or an address
/// that PBeam cannot read.
pub(super) enum Failure {
    Errno(c_int),
    Unsupported(&'static str),
}

/// The flags of `encode_ifaddrs_flags`, in its order
/// (`prim_net_nif.c:1700-1784`), with the glibc values of `<net/if.h>`.
const FLAGS: [(u32, &str); 15] = [
    (0x1, "up"),
    (0x2, "broadcast"),
    (0x4, "debug"),
    (0x8, "loopback"),
    (0x10, "pointopoint"),
    (0x20, "notrailers"),
    (0x40, "running"),
    (0x80, "noarp"),
    (0x100, "promisc"),
    (0x400, "master"),
    (0x800, "slave"),
    (0x1000, "multicast"),
    (0x2000, "portsel"),
    (0x4000, "automedia"),
    (0x8000, "dynamic"),
];

/// The hardware types of `esock_encode_packet_hatype`
/// (`socket_util.c:2478-2633`), with the values of `<net/if_arp.h>`.
const HATYPES: [(u16, &str); 21] = [
    (0, "netrom"),
    (1, "ether"),
    (2, "eether"),
    (3, "ax25"),
    (4, "pronet"),
    (5, "chaos"),
    (6, "ieee802"),
    (7, "arcnet"),
    (8, "appletlk"),
    (15, "dlci"),
    (19, "atm"),
    (23, "metricom"),
    (24, "ieee1394"),
    (27, "eui64"),
    (32, "infiniband"),
    (768, "tunnel"),
    (769, "tunnel6"),
    (772, "loopback"),
    (773, "localtlk"),
    (0xFFFE, "none"),
    (0xFFFF, "void"),
];

/// The packet types of `esock_encode_packet_pkttype`
/// (`socket_util.c:2651-2722`), with the values of
/// `<linux/if_packet.h>`.
const PKTTYPES: [&str; 8] = [
    "host",
    "broadcast",
    "multicast",
    "otherhost",
    "outgoing",
    "loopback",
    "user",
    "kernel",
];

/// Read the list of `getifaddrs` on Linux, as `enet_getifaddrs` does
/// (`prim_net_nif.c:1455-1508`).
#[cfg(target_os = "linux")]
pub(super) fn read() -> Result<Vec<Entry>, Failure> {
    nix::ifaddrs::getifaddrs()
        .map_err(|errno| Failure::Errno(errno as c_int))?
        .map(entry)
        .collect()
}

/// Copy an entry of `getifaddrs`. The `ifa_ifu` union of the entry is the
/// `dstaddr` of a point-to-point interface, or else the `broadaddr` of a
/// broadcast interface (`prim_net_nif.c:1630-1664`).
pub(super) fn entry(interface: nix::ifaddrs::InterfaceAddress) -> Result<Entry, Failure> {
    let address = |storage: Option<nix::sys::socket::SockaddrStorage>| {
        storage.map(|storage| host_address(&storage)).transpose()
    };
    let extra = match (interface.destination, interface.broadcast) {
        (Some(destination), _) => Some(("dstaddr", host_address(&destination)?)),
        (None, Some(broadcast)) => Some(("broadaddr", host_address(&broadcast)?)),
        (None, None) => None,
    };
    Ok(Entry {
        name: interface.interface_name.into_bytes(),
        flags: interface.flags.bits().cast_unsigned(),
        addr: address(interface.address)?,
        netmask: address(interface.netmask)?,
        extra,
    })
}

/// Copy a socket address of `getifaddrs`: an `inet` or an `inet6` address
/// as its C bytes, or else a link address.
fn host_address(storage: &nix::sys::socket::SockaddrStorage) -> Result<Address, Failure> {
    if let Some(inet) = storage.as_sockaddr_in() {
        let raw: &libc::sockaddr_in = inet.as_ref();
        let mut bytes = header(16, libc::AF_INET);
        bytes.extend(raw.sin_port.to_ne_bytes());
        bytes.extend(raw.sin_addr.s_addr.to_ne_bytes());
        bytes.extend([0; 8]);
        return Ok(Address::Socket(bytes));
    }
    if let Some(inet6) = storage.as_sockaddr_in6() {
        let raw: &libc::sockaddr_in6 = inet6.as_ref();
        let mut bytes = header(28, libc::AF_INET6);
        bytes.extend(raw.sin6_port.to_ne_bytes());
        bytes.extend(raw.sin6_flowinfo.to_ne_bytes());
        bytes.extend(raw.sin6_addr.s6_addr);
        bytes.extend(raw.sin6_scope_id.to_ne_bytes());
        return Ok(Address::Socket(bytes));
    }
    link_address(storage)
}

/// Copy the `sockaddr_ll` of a Linux link address.
#[cfg(target_os = "linux")]
fn link_address(storage: &nix::sys::socket::SockaddrStorage) -> Result<Address, Failure> {
    let link = storage.as_link_addr().ok_or(Failure::Unsupported(
        "prim_net:nif_getifaddrs/1 socket address of this family is not supported",
    ))?;
    let raw: &libc::sockaddr_ll = link.as_ref();
    packet(
        u16::from_be(raw.sll_protocol),
        raw.sll_ifindex,
        raw.sll_hatype,
        raw.sll_pkttype,
        &raw.sll_addr,
        raw.sll_halen,
    )
}

/// A link address of another host is not read (see the module).
#[cfg(not(target_os = "linux"))]
fn link_address(_: &nix::sys::socket::SockaddrStorage) -> Result<Address, Failure> {
    Err(Failure::Unsupported(
        "prim_net:nif_getifaddrs/1 socket address of this family is not supported",
    ))
}

/// Copy the fields of a `sockaddr_ll`, as `esock_encode_sockaddr_ll`
/// reads them (`socket_util.c:1535-1574`): the `halen` first bytes of
/// `sll_addr`. A hardware address longer than the 8 bytes of `sll_addr` is
/// not supported: ERTS reads past the field.
pub(super) fn packet(
    protocol: u16,
    ifindex: i32,
    hatype: u16,
    pkttype: u8,
    sll_addr: &[u8; 8],
    halen: u8,
) -> Result<Address, Failure> {
    let addr = sll_addr
        .get(..usize::from(halen))
        .ok_or(Failure::Unsupported(
            "prim_net:nif_getifaddrs/1 hardware address longer than 8 bytes is not supported",
        ))?;
    Ok(Address::Packet {
        protocol,
        ifindex,
        hatype,
        pkttype,
        addr: addr.to_vec(),
    })
}

/// Give the result of `enet_getifaddrs` (`prim_net_nif.c:1478-1493`) for
/// the result of `read`: `{ok, [Map]}`, `{error, Errno}`, or the
/// runtime error of an address that PBeam cannot read.
pub(super) fn result(
    atoms: &AtomTable,
    entries: Result<Vec<Entry>, Failure>,
) -> Result<BifResult, BeamError> {
    match entries {
        Ok(entries) => Ok(BifResult::Value(encode(atoms, &entries)?)),
        Err(Failure::Errno(errno)) => failure(atoms, &errno_id(errno)),
        Err(Failure::Unsupported(message)) => Err(error(message)),
    }
}

/// Make the list of `enet_getifaddrs_process`: `{ok, [Map]}`
/// (`prim_net_nif.c:1512-1560`).
pub(super) fn encode(atoms: &AtomTable, entries: &[Entry]) -> Result<Term, BeamError> {
    let list = entries
        .iter()
        .map(|entry| entry_map(atoms, entry))
        .collect::<Result<Vec<_>, _>>()?;
    Ok(Term::tuple(vec![
        Term::static_atom("ok"),
        Term::list_from(list),
    ]))
}

/// Make the map of `encode_ifaddrs` and `make_ifaddrs`
/// (`prim_net_nif.c:1592-1877`): `name`, `flags`, and each address that
/// is not NULL.
fn entry_map(atoms: &AtomTable, entry: &Entry) -> Result<Term, BeamError> {
    let key = |name: &str| atoms.put(name).map(Term::Atom);
    let flags = FLAGS
        .iter()
        .filter(|(bit, _)| entry.flags & bit != 0)
        .map(|(_, name)| key(name))
        .collect::<Result<Vec<_>, _>>()?;
    let mut pairs = vec![
        (key("name")?, latin1(&entry.name)),
        (key("flags")?, Term::list_from(flags)),
    ];
    let addresses = [
        entry.addr.as_ref().map(|addr| ("addr", addr)),
        entry.netmask.as_ref().map(|addr| ("netmask", addr)),
        entry.extra.as_ref().map(|(name, addr)| (*name, addr)),
    ];
    for (name, addr) in addresses.into_iter().flatten() {
        pairs.push((key(name)?, address(atoms, addr)?));
    }
    Ok(Term::Map(
        Map::from_pairs(pairs, atoms).expect("atom keys fit each map form"),
    ))
}

/// Encode an address, as `esock_encode_sockaddr` does
/// (`socket_util.c:726-915`, `1535-1574`, `3617-3640`).
fn address(atoms: &AtomTable, address: &Address) -> Result<Term, BeamError> {
    let (protocol, ifindex, hatype, pkttype, addr) = match address {
        Address::Socket(bytes) => return encode_sockaddr(atoms, bytes),
        Address::Packet {
            protocol,
            ifindex,
            hatype,
            pkttype,
            addr,
        } => (protocol, ifindex, hatype, pkttype, addr),
    };
    let key = |name: &str| atoms.put(name).map(Term::Atom);
    let hatype = match HATYPES.iter().find(|(value, _)| value == hatype) {
        Some((_, name)) => key(name)?,
        None => Term::Integer((*hatype).into()),
    };
    let pkttype = match PKTTYPES.get(usize::from(*pkttype)) {
        Some(name) => key(name)?,
        None => Term::Integer((*pkttype).into()),
    };
    let pairs = vec![
        (key("family")?, key("packet")?),
        (key("protocol")?, Term::Integer((*protocol).into())),
        (key("ifindex")?, Term::Integer((*ifindex).into())),
        (key("hatype")?, hatype),
        (key("pkttype")?, pkttype),
        (key("addr")?, Term::binary(addr.clone())),
    ];
    Ok(Term::Map(
        Map::from_pairs(pairs, atoms).expect("atom keys fit each map form"),
    ))
}

//! `nif_getifaddrs/1` of the `net` NIF library
//! (`prim_net_nif.c:1381-1877`). On Linux, PBeam reads the list with the
//! safe `nix` wrapper of `getifaddrs`, because `getifaddrs` there gives
//! only the families `inet`, `inet6`, and `packet`. On macOS, PBeam reads
//! the list with the host call of `host/ifaddrs.rs`, which copies the bytes
//! that ERTS reads, and this module makes the terms. On another host the
//! call stops the run with a runtime error (row "NIF stubs" of
//! `docs/erts-differences.md`).

// Each reader runs on one host; the unit tests run them on each host.
#![cfg_attr(
    not(test),
    expect(
        dead_code,
        reason = "each reader runs on one host; the unit tests run them on each host"
    )
)]

use super::{
    AtomTable, BeamError, BifResult, Map, Term, c_int, encode_sockaddr, errno_id, error, failure,
    header, latin1,
};

/// A socket address of an entry: the bytes of a full `inet` or `inet6`
/// socket address for [`encode_sockaddr`], the fields of a `sockaddr_ll`
/// or of a macOS `sockaddr_dl`, or the macOS family and the data of
/// `esock_encode_sockaddr_native` (`socket_util.c:2830-2859`).
pub(super) enum Address {
    Socket(Vec<u8>),
    Link {
        index: u16,
        kind: u8,
        nlen: u8,
        alen: u8,
        slen: u8,
        data: Vec<u8>,
    },
    Native(c_int, Vec<u8>),
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
#[cfg(not(target_os = "macos"))]
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

/// The flags of `encode_ifaddrs_flags`, in its order
/// (`prim_net_nif.c:1700-1784`), with the macOS values of `<net/if.h>`,
/// which does not define `IFF_MASTER`, `IFF_SLAVE`, `IFF_PORTSEL`,
/// `IFF_AUTOMEDIA`, and `IFF_DYNAMIC`.
#[cfg(target_os = "macos")]
const FLAGS: [(u32, &str); 10] = [
    (0x1, "up"),
    (0x2, "broadcast"),
    (0x4, "debug"),
    (0x8, "loopback"),
    (0x10, "pointopoint"),
    (0x20, "notrailers"),
    (0x40, "running"),
    (0x80, "noarp"),
    (0x100, "promisc"),
    (0x8000, "multicast"),
];

/// `IFF_BROADCAST` and `IFF_POINTOPOINT`, which have the same values in the
/// glibc and the macOS `<net/if.h>`.
const IFF_BROADCAST: u32 = 0x2;
const IFF_POINTOPOINT: u32 = 0x10;

/// The families of the macOS `<sys/socket.h>` that
/// `esock_encode_sockaddr` knows (`socket_util.c:748-913`).
const MACOS_AF_UNSPEC: c_int = 0;
const MACOS_AF_LOCAL: c_int = 1;
const MACOS_AF_INET: c_int = 2;
const MACOS_AF_IMPLINK: c_int = 3;
const MACOS_AF_PUP: c_int = 4;
const MACOS_AF_CHAOS: c_int = 5;
const MACOS_AF_LINK: c_int = 18;
const MACOS_AF_INET6: c_int = 30;

/// The interface types of `esock_encode_if_type`
/// (`socket_util.c:1660-1762`), with the values of the macOS
/// `<net/if_types.h>`, which does not define `IFT_IPV4`, `IFT_IPV6`, and
/// `IFT_6TO4`.
const IF_TYPES: [(u8, &str); 12] = [
    (0x1, "other"),
    (0x3, "hdh1822"),
    (0x4, "x25ddn"),
    (0x5, "x25"),
    (0x6, "ether"),
    (0x17, "ppp"),
    (0x18, "loop"),
    (0x37, "gif"),
    (0x38, "faith"),
    (0x39, "stf"),
    (0xd1, "bridge"),
    (0xff, "cellular"),
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
        .map_err(|errno| Failure::Errno(super::errno_code(errno)))?
        .map(entry)
        .collect()
}

/// Read the list of `getifaddrs` on macOS, as `enet_getifaddrs` does
/// (`prim_net_nif.c:1455-1508`).
#[cfg(target_os = "macos")]
pub(super) fn read() -> Result<Vec<Entry>, Failure> {
    crate::runtime::host::ifaddrs::getifaddrs()
        .map_err(Failure::Errno)?
        .into_iter()
        .map(macos_entry)
        .collect()
}

/// Copy a macOS entry of `getifaddrs`. The `ifa_dstaddr` of the entry is
/// the `dstaddr` of a point-to-point interface, or else the `broadaddr` of
/// a broadcast interface (`prim_net_nif.c:1630-1664`).
pub(super) fn macos_entry(entry: crate::runtime::host::ifaddrs::Entry) -> Result<Entry, Failure> {
    let extra = match entry.dstaddr {
        Some(address) if entry.flags & IFF_POINTOPOINT != 0 => {
            Some(("dstaddr", macos_address(address)?))
        }
        Some(address) if entry.flags & IFF_BROADCAST != 0 => {
            Some(("broadaddr", macos_address(address)?))
        }
        _ => None,
    };
    Ok(Entry {
        name: entry.name,
        flags: entry.flags,
        addr: entry.addr.map(macos_address).transpose()?,
        netmask: entry.netmask.map(macos_address).transpose()?,
        extra,
    })
}

/// Read a macOS socket address as `esock_encode_sockaddr` does with the
/// length -1 (`socket_util.c:723`, `741-913`): an `inet` or `inet6`
/// address of full size, a `sockaddr_dl` of `sdl_len` bytes
/// (`socket_util.c:889-900`, `1599-1656`), or else the family and no data.
/// An `AF_LOCAL` address is not supported: `getifaddrs` gives none.
pub(super) fn macos_address(
    address: crate::runtime::host::ifaddrs::Address,
) -> Result<Address, Failure> {
    let bytes = address.bytes;
    let with_header = |family| {
        let mut socket = header(bytes.len(), family);
        socket.extend(&bytes[2..]);
        Address::Socket(socket)
    };
    Ok(match address.family {
        MACOS_AF_INET => with_header(libc::AF_INET),
        MACOS_AF_INET6 => with_header(libc::AF_INET6),
        MACOS_AF_LINK => match bytes.len() {
            8.. => Address::Link {
                index: u16::from_ne_bytes([bytes[2], bytes[3]]),
                kind: bytes[4],
                nlen: bytes[5],
                alen: bytes[6],
                slen: bytes[7],
                data: bytes[8..].to_vec(),
            },
            0 => Address::Native(MACOS_AF_LINK, Vec::new()),
            // ERTS reads `sdl_len - 2` bytes, which wraps for 1.
            1 => {
                return Err(Failure::Unsupported(
                    "prim_net:nif_getifaddrs/1 link address of 1 byte is not supported",
                ));
            }
            _ => Address::Native(MACOS_AF_LINK, bytes[2..].to_vec()),
        },
        MACOS_AF_LOCAL => {
            return Err(Failure::Unsupported(
                "prim_net:nif_getifaddrs/1 socket address of this family is not supported",
            ));
        }
        family => Address::Native(family, Vec::new()),
    })
}

/// Copy an entry of `getifaddrs`. The `ifa_ifu` union of the entry is the
/// `dstaddr` of a point-to-point interface, or else the `broadaddr` of a
/// broadcast interface (`prim_net_nif.c:1630-1664`).
pub(super) fn entry(interface: nix::ifaddrs::InterfaceAddress) -> Result<Entry, Failure> {
    let address = |storage: Option<nix::sys::socket::SockaddrStorage>| {
        storage.map(|storage| host_address(&storage)).transpose()
    };
    // `nix` replaces the bytes of a name that is not UTF-8, which ERTS
    // gives as they are (`prim_net_nif.c:1693-1695`).
    if interface
        .interface_name
        .contains(char::REPLACEMENT_CHARACTER)
    {
        return Err(Failure::Unsupported(
            "prim_net:nif_getifaddrs/1 interface name that is not UTF-8 is not supported",
        ));
    }
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
        raw.sll_addr,
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
    sll_addr: [u8; 8],
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
    let key = |name: &str| atoms.put(name).map(Term::Atom);
    let (protocol, ifindex, hatype, pkttype, addr) = match address {
        Address::Socket(bytes) => return encode_sockaddr(atoms, bytes),
        Address::Link {
            index,
            kind,
            nlen,
            alen,
            slen,
            data,
        } => return link(atoms, *index, *kind, [*nlen, *alen, *slen], data),
        Address::Native(family, data) => {
            let family = match *family {
                MACOS_AF_UNSPEC => key("unspec")?,
                MACOS_AF_IMPLINK => key("implink")?,
                MACOS_AF_PUP => key("pup")?,
                MACOS_AF_CHAOS => key("chaos")?,
                MACOS_AF_LINK => key("link")?,
                family => Term::Integer(family.into()),
            };
            let pairs = vec![
                (key("family")?, family),
                (key("addr")?, Term::binary(data.clone())),
            ];
            return Ok(Term::Map(
                Map::from_pairs(pairs, atoms).expect("atom keys fit each map form"),
            ));
        }
        Address::Packet {
            protocol,
            ifindex,
            hatype,
            pkttype,
            addr,
        } => (protocol, ifindex, hatype, pkttype, addr),
    };
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

/// Encode the fields of a `sockaddr_dl`, as `esock_encode_sockaddr_dl` and
/// `make_sockaddr_dl` do (`socket_util.c:1599-1656`, `3648-3675`).
fn link(
    atoms: &AtomTable,
    index: u16,
    kind: u8,
    [nlen, alen, slen]: [u8; 3],
    data: &[u8],
) -> Result<Term, BeamError> {
    let key = |name: &str| atoms.put(name).map(Term::Atom);
    let kind = match IF_TYPES.iter().find(|(value, _)| *value == kind) {
        Some((_, name)) => key(name)?,
        None => Term::Integer(kind.into()),
    };
    let pairs = vec![
        (key("family")?, key("link")?),
        (key("index")?, Term::Integer(index.into())),
        (key("type")?, kind),
        (key("nlen")?, Term::Integer(nlen.into())),
        (key("alen")?, Term::Integer(alen.into())),
        (key("slen")?, Term::Integer(slen.into())),
        (key("data")?, Term::binary(data.to_vec())),
    ];
    Ok(Term::Map(
        Map::from_pairs(pairs, atoms).expect("atom keys fit each map form"),
    ))
}

//! The host call of `nif_getifaddrs/1` of the `net` NIF library on macOS
//! (`prim_net_nif.c:1455-1688`): `getifaddrs`. The function copies each
//! entry out of the C memory with the socket address bytes that ERTS reads,
//! and the binding in `interpreter/native/prim_net/ifaddrs.rs` makes the
//! terms in safe Rust.

use std::ffi::{CStr, c_int};

/// An entry of the list of `getifaddrs`: `ifa_name`, `ifa_flags`, and
/// `ifa_addr`, `ifa_netmask`, and `ifa_dstaddr` (see [`copy`]).
#[cfg_attr(not(target_os = "macos"), allow(dead_code))]
pub(crate) struct Entry {
    pub(crate) name: Vec<u8>,
    pub(crate) flags: u32,
    pub(crate) addr: Option<Address>,
    pub(crate) netmask: Option<Address>,
    pub(crate) dstaddr: Option<Address>,
}

/// A socket address of an entry: its family, and the bytes of the address
/// that ERTS reads (see [`copy`]).
#[cfg_attr(not(target_os = "macos"), allow(dead_code))]
pub(crate) struct Address {
    pub(crate) family: c_int,
    pub(crate) bytes: Vec<u8>,
}

/// Run `getifaddrs`, as `enet_getifaddrs` does (`prim_net_nif.c:1455-1508`).
/// Return the entries of the list, or the `errno` of the call.
///
/// As `encode_ifaddrs` does (`prim_net_nif.c:1619-1645`), a netmask of the
/// family `AF_UNSPEC` with an `AF_INET` address, and the `AF_UNSPEC`
/// `ifa_dstaddr` of a point-to-point interface, are read as `AF_INET`.
///
/// # Examples
///
/// ```rust,ignore
/// assert!(getifaddrs().is_ok());
/// ```
#[cfg(target_os = "macos")]
pub(crate) fn getifaddrs() -> Result<Vec<Entry>, c_int> {
    let mut list = std::ptr::null_mut();
    // SAFETY: `getifaddrs` writes the list to `list`, which `freeifaddrs`
    // frees below.
    if unsafe { libc::getifaddrs(&raw mut list) } != 0 {
        return Err(std::io::Error::last_os_error().raw_os_error().unwrap_or(0));
    }
    let mut entries = Vec::new();
    let mut entry = list;
    // SAFETY: each entry of the list and the memory that its pointers
    // point to stay valid until `freeifaddrs`. `ifa_name` is
    // NUL-terminated. For the reads of `copy`, see there.
    unsafe {
        while let Some(interface) = entry.as_ref() {
            let inet = |address: *const libc::sockaddr| {
                address
                    .as_ref()
                    .is_some_and(|address| c_int::from(address.sa_family) == libc::AF_INET)
            };
            let point_to_point = interface.ifa_flags & libc::IFF_POINTOPOINT as u32 != 0;
            entries.push(Entry {
                name: CStr::from_ptr(interface.ifa_name).to_bytes().to_vec(),
                flags: interface.ifa_flags,
                addr: copy(interface.ifa_addr, false),
                netmask: copy(interface.ifa_netmask, inet(interface.ifa_addr)),
                dstaddr: copy(interface.ifa_dstaddr, point_to_point),
            });
            entry = interface.ifa_next;
        }
        libc::freeifaddrs(list);
    }
    Ok(entries)
}

/// Copy the family of a socket address and the bytes that
/// `esock_encode_sockaddr` reads with the length -1 (`socket_util.c:723`,
/// `741-913`): 16 bytes of an `AF_INET` address and 28 bytes of an
/// `AF_INET6` address; the `sdl_len` bytes of an `AF_LINK` address; and no
/// bytes of another family. `unspec_as_inet` reads an `AF_UNSPEC` address
/// as `AF_INET`. NULL gives `None`.
///
/// ERTS reads the 16 or 28 bytes whatever the `sa_len`, so for a shorter
/// address, such as the `AF_UNSPEC` `ifa_dstaddr` of a `utun` interface
/// with `sa_len` 0, it reads the bytes that follow the address in the
/// buffer, or bytes that `getifaddrs` did not write. PBeam copies only the
/// `sa_len` bytes and gives zero for the bytes past them (row "NIF stubs"
/// of `docs/erts-differences.md`).
///
/// # Safety
///
/// `address` is NULL or points to a socket address that `getifaddrs`
/// wrote: its 2 header bytes, and `sa_len` bytes when `sa_len` is larger.
#[cfg(target_os = "macos")]
unsafe fn copy(address: *const libc::sockaddr, unspec_as_inet: bool) -> Option<Address> {
    if address.is_null() {
        return None;
    }
    // SAFETY: see the function.
    let [written, family] = unsafe { address.cast::<[u8; 2]>().read() };
    let mut family = c_int::from(family);
    if unspec_as_inet && family == libc::AF_UNSPEC {
        family = libc::AF_INET;
    }
    let length = match family {
        libc::AF_INET => size_of::<libc::sockaddr_in>(),
        libc::AF_INET6 => size_of::<libc::sockaddr_in6>(),
        libc::AF_LINK => usize::from(written),
        _ => 0,
    };
    let mut bytes = vec![0; length];
    let read = length.min(usize::from(written));
    // SAFETY: see the function; `read` is at most `sa_len`.
    bytes[..read]
        .copy_from_slice(unsafe { std::slice::from_raw_parts(address.cast::<u8>(), read) });
    Some(Address { family, bytes })
}

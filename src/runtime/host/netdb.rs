//! The host calls of the address and service lookups of the `net` NIF
//! library (`prim_net_nif.c:1090-1362`, `4122-4257`): `getaddrinfo`,
//! `getnameinfo`, `getservbyname`, and `getservbyport`. Each function
//! copies the result of the call out of the C memory, and the binding in
//! `interpreter/native/prim_net.rs` makes the terms in safe Rust.

use std::{
    ffi::{CStr, c_char, c_int},
    sync::Mutex,
};

/// The lock of `getservbyname` and `getservbyport`, which return a static
/// `servent` on glibc and macOS.
static SERVICES: Mutex<()> = Mutex::new(());

/// The size of the host and the service buffers of `enet_getnameinfo`:
/// `HOSTNAME_LEN` and `SERVICE_LEN` (`prim_net_nif.c:246-247`).
const NAME_LEN: usize = 256;

/// An entry of the list of `getaddrinfo`: `ai_family`, `ai_socktype`,
/// `ai_protocol`, and the `ai_addrlen` bytes of `ai_addr`.
pub(crate) struct AddressInfo {
    pub(crate) family: c_int,
    pub(crate) socktype: c_int,
    pub(crate) protocol: c_int,
    pub(crate) address: Vec<u8>,
}

/// Run `getaddrinfo(host, service, NULL, &list)`, as `enet_getaddrinfo`
/// does (`prim_net_nif.c:1276`). `None` gives a NULL argument. Return the
/// entries of the list, or the nonzero result of the call.
///
/// # Examples
///
/// ```rust,ignore
/// assert!(getaddrinfo(Some(c"127.0.0.1"), None).is_ok());
/// ```
pub(crate) fn getaddrinfo(
    host: Option<&CStr>,
    service: Option<&CStr>,
) -> Result<Vec<AddressInfo>, c_int> {
    let pointer = |name: Option<&CStr>| name.map_or(std::ptr::null(), CStr::as_ptr);
    let mut list = std::ptr::null_mut();
    // SAFETY: each name is NULL or NUL-terminated, the hints are NULL, and
    // `getaddrinfo` writes the list to `list`, which `freeaddrinfo` frees
    // below.
    let result = unsafe {
        libc::getaddrinfo(
            pointer(host),
            pointer(service),
            std::ptr::null(),
            &raw mut list,
        )
    };
    if result != 0 {
        return Err(result);
    }
    let mut entries = Vec::new();
    let mut entry = list;
    // SAFETY: each entry of the list is valid until `freeaddrinfo`, and its
    // `ai_addr` points to `ai_addrlen` bytes.
    unsafe {
        while let Some(info) = entry.as_ref() {
            let length = usize::try_from(info.ai_addrlen).unwrap_or(0);
            let address = std::slice::from_raw_parts(info.ai_addr.cast::<u8>(), length);
            entries.push(AddressInfo {
                family: info.ai_family,
                socktype: info.ai_socktype,
                protocol: info.ai_protocol,
                address: address.to_vec(),
            });
            entry = info.ai_next;
        }
        libc::freeaddrinfo(list);
    }
    Ok(entries)
}

/// Run `getnameinfo` on the socket address `address` with `flags` and
/// buffers of 256 bytes, as `enet_getnameinfo` does
/// (`prim_net_nif.c:1096-1104`). `address` has at most the size of a
/// `sockaddr_storage`. Return the host and the service, or the nonzero
/// result of the call and the `errno` after it.
///
/// # Examples
///
/// ```rust,ignore
/// let (host, service) = getnameinfo(&address, libc::NI_NUMERICHOST).unwrap();
/// ```
pub(crate) fn getnameinfo(
    address: &[u8],
    flags: c_int,
) -> Result<(Vec<u8>, Vec<u8>), (c_int, c_int)> {
    // SAFETY: a zeroed `sockaddr_storage` is valid.
    let mut storage: libc::sockaddr_storage = unsafe { std::mem::zeroed() };
    let length = address.len().min(size_of::<libc::sockaddr_storage>());
    // SAFETY: `storage` has room for `length` bytes, and the two regions do
    // not overlap.
    unsafe {
        std::ptr::copy_nonoverlapping(address.as_ptr(), (&raw mut storage).cast::<u8>(), length);
    }
    let mut host = [0 as c_char; NAME_LEN];
    let mut service = [0 as c_char; NAME_LEN];
    // SAFETY: `storage` holds `length` bytes of the address, and the call
    // writes at most `NAME_LEN` bytes to each buffer.
    let result = unsafe {
        libc::getnameinfo(
            (&raw const storage).cast(),
            libc::socklen_t::try_from(length).unwrap_or(0),
            host.as_mut_ptr(),
            libc::socklen_t::try_from(NAME_LEN).unwrap_or(0),
            service.as_mut_ptr(),
            libc::socklen_t::try_from(NAME_LEN).unwrap_or(0),
            flags,
        )
    };
    if result != 0 {
        return Err((result, super::errno::errno()));
    }
    // SAFETY: `getnameinfo` wrote a NUL-terminated string to each buffer.
    let text = |buffer: &[c_char; NAME_LEN]| unsafe { CStr::from_ptr(buffer.as_ptr()) };
    Ok((
        text(&host).to_bytes().to_vec(),
        text(&service).to_bytes().to_vec(),
    ))
}

/// Run `getservbyname(name, protocol)`, as `net_getservbyname` does
/// (`prim_net_nif.c:4138-4141`). `None` gives a NULL protocol. Return the
/// `s_port` of the entry, in network byte order, or `None` for NULL.
///
/// # Examples
///
/// ```rust,ignore
/// assert_eq!(getservbyname(c"http", None), Some(i32::from(80u16.to_be())));
/// ```
pub(crate) fn getservbyname(name: &CStr, protocol: Option<&CStr>) -> Option<c_int> {
    let _guard = SERVICES
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let protocol = protocol.map_or(std::ptr::null(), CStr::as_ptr);
    // SAFETY: `name` is NUL-terminated, `protocol` is NULL or
    // NUL-terminated, and `SERVICES` guards the static result.
    unsafe { libc::getservbyname(name.as_ptr(), protocol).as_ref() }.map(|entry| entry.s_port)
}

/// Run `getservbyport(port, protocol)`, as `net_getservbyport` does
/// (`prim_net_nif.c:4228-4232`). `port` is in network byte order, and
/// `None` gives a NULL protocol. Return the bytes of `s_name`, or `None`
/// for NULL.
///
/// # Examples
///
/// ```rust,ignore
/// assert_eq!(getservbyport(i32::from(80u16.to_be()), None), Some(b"http".to_vec()));
/// ```
pub(crate) fn getservbyport(port: c_int, protocol: Option<&CStr>) -> Option<Vec<u8>> {
    let _guard = SERVICES
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let protocol = protocol.map_or(std::ptr::null(), CStr::as_ptr);
    // SAFETY: `protocol` is NULL or NUL-terminated, and `SERVICES` guards
    // the static result, whose `s_name` is NUL-terminated.
    unsafe {
        libc::getservbyport(port, protocol)
            .as_ref()
            .map(|entry| CStr::from_ptr(entry.s_name).to_bytes().to_vec())
    }
}

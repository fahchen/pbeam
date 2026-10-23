//! The `inet_gethost` port program of ERTS
//! (`erts/etc/common/inet_gethost.c`), which `inet_gethost_native` opens
//! with `open_port({spawn_executable, "<bindir>/inet_gethost"}, ...)`
//! (inet_gethost_native.erl:412-421). PBeam runs it in a thread of the
//! node (see [`super::spawn::serve`]), not as an external program, so a
//! bundle needs no `inet_gethost` file.
//!
//! The thread reads the requests from the input pipe and writes the
//! replies to the output pipe, each with a 4-byte big-endian size, as the
//! program does with its fds 0 and 1 (`inet_gethost.c:139-146`). It does
//! each lookup with the host call that the worker of the program uses on
//! a host with `getaddrinfo` and `getnameinfo` (`inet_gethost.c:1707-1992`).
//! It has one worker and no request queue: it answers the requests in
//! order, and `OP_CANCEL_REQUEST` changes nothing (see "Ports" in
//! `docs/erts-differences.md`).

use std::{
    ffi::{CStr, c_char, c_int},
    io::{Read, Write},
    sync::Mutex,
};

/// `OP_GETHOSTBYNAME` (`inet_gethost.c:154`).
const OP_GETHOSTBYNAME: u8 = 1;
/// `OP_GETHOSTBYADDR` (`inet_gethost.c:155`).
const OP_GETHOSTBYADDR: u8 = 2;
/// `OP_CANCEL_REQUEST` (`inet_gethost.c:156`).
const OP_CANCEL_REQUEST: u8 = 3;
/// `OP_CONTROL` (`inet_gethost.c:157`).
const OP_CONTROL: u8 = 4;
/// `PROTO_IPV4` (`inet_gethost.c:162`).
const PROTO_IPV4: u8 = 1;
/// `PROTO_IPV6` (`inet_gethost.c:163`).
const PROTO_IPV6: u8 = 2;
/// `INVALID_SERIAL` (`inet_gethost.c:149`), the serial of `OP_CONTROL`.
const INVALID_SERIAL: u32 = u32::MAX;

/// The `h_errno` values of `<netdb.h>`, the same on Linux and macOS.
const HOST_NOT_FOUND: c_int = 1;
/// `TRY_AGAIN` of `<netdb.h>`.
const TRY_AGAIN: c_int = 2;
/// `NO_RECOVERY` of `<netdb.h>`.
const NO_RECOVERY: c_int = 3;
/// `NO_DATA` (and `NO_ADDRESS`) of `<netdb.h>`.
const NO_DATA: c_int = 4;

/// `EAI_NODATA` of glibc, which ERTS sees because it builds with
/// `-D_GNU_SOURCE` on Linux (erts/configure.ac:543).
#[cfg(target_os = "linux")]
const EAI_NODATA: c_int = -5;
/// `EAI_NODATA` of macOS.
#[cfg(not(target_os = "linux"))]
const EAI_NODATA: c_int = libc::EAI_NODATA;

/// The lock of `gethostbyname` and `gethostbyaddr`, which return a static
/// `hostent` and set a `h_errno` that is not thread-local on macOS. The
/// program calls them in a worker process of its own.
static NETDB: Mutex<()> = Mutex::new(());

unsafe extern "C" {
    fn gethostbyname(name: *const c_char) -> *mut libc::hostent;
    fn gethostbyaddr(
        addr: *const libc::c_void,
        len: libc::socklen_t,
        kind: c_int,
    ) -> *mut libc::hostent;
    #[cfg(target_os = "linux")]
    fn __h_errno_location() -> *mut c_int;
    #[cfg(not(target_os = "linux"))]
    static h_errno: c_int;
}

/// Serve the requests of `input` and write the replies to `output` until
/// the end of file of `input`, as the main loop and the worker loop of the
/// program do (`inet_gethost.c:856-980`, `1725-1992`). Return the exit
/// status of the program: 0 at the end of file, and 1 for a fatal error
/// (`fatal`, `inet_gethost.c:2643-2667`): a request with a size below 5
/// or a part of a request (`inet_gethost.c:1325-1351`), and `OP_CONTROL`
/// with a serial that is not `INVALID_SERIAL` (`inet_gethost.c:896-901`).
/// The program writes the message of a fatal error to its fd 2; the
/// thread writes nothing.
///
/// # Examples
///
/// ```rust,ignore
/// let status = serve(&mut input, &mut output);
/// ```
pub(crate) fn serve(input: &mut impl Read, output: &mut impl Write) -> i32 {
    loop {
        let mut size = [0; 4];
        match read_exact(input, &mut size) {
            Some(0) => return 0,
            Some(4) => {}
            _ => return 1,
        }
        let mut request = vec![0; u32::from_be_bytes(size) as usize];
        if read_exact(input, &mut request) != Some(request.len()) || request.len() < 5 {
            return 1;
        }
        let serial = u32::from_be_bytes(request[..4].try_into().expect("4 bytes"));
        let reply = match request[4] {
            OP_CANCEL_REQUEST => continue,
            // `SETOPT_DEBUG_LEVEL` and other controls change no reply.
            OP_CONTROL if serial == INVALID_SERIAL => continue,
            OP_CONTROL => return 1,
            op @ (OP_GETHOSTBYNAME | OP_GETHOSTBYADDR) => match lookup(op, &request[5..]) {
                Some(reply) => reply,
                None => continue,
            },
            // "Unknown operation requested from erlang", discarded.
            _ => continue,
        };
        let mut packet = Vec::with_capacity(8 + reply.len());
        packet.extend_from_slice(&u32::try_from(4 + reply.len()).unwrap_or(0).to_be_bytes());
        packet.extend_from_slice(&serial.to_be_bytes());
        packet.extend_from_slice(&reply);
        if output.write_all(&packet).is_err() {
            return 1;
        }
    }
}

/// Read bytes into `buffer` until it is full or `input` ends, and return
/// the number of bytes, or `None` for a failed read.
fn read_exact(input: &mut impl Read, buffer: &mut [u8]) -> Option<usize> {
    let mut done = 0;
    while done < buffer.len() {
        match input.read(&mut buffer[done..]) {
            Ok(0) => break,
            Ok(read) => done += read,
            Err(error) if error.kind() == std::io::ErrorKind::Interrupted => {}
            Err(_) => return None,
        }
    }
    Some(done)
}

/// Return the reply after the serial for the operation `op` with the
/// protocol and the data of `rest`, or `None` for a request that the main
/// loop discards: a protocol that is not `PROTO_IPV4` or `PROTO_IPV6`, a
/// name without a byte 0, and an address of the wrong size
/// (`inet_gethost.c:963-979`, `1548-1585`).
fn lookup(op: u8, rest: &[u8]) -> Option<Vec<u8>> {
    let (&proto, data) = rest.split_first()?;
    let result = match (op, proto) {
        (OP_GETHOSTBYNAME, PROTO_IPV4 | PROTO_IPV6) => {
            let name = CStr::from_bytes_until_nul(data).ok()?;
            if proto == PROTO_IPV4 {
                host_by_name(name)
            } else {
                address_info(name)
            }
        }
        (OP_GETHOSTBYADDR, PROTO_IPV4) => host_by_address(<[u8; 4]>::try_from(data).ok()?),
        (OP_GETHOSTBYADDR, PROTO_IPV6) => name_info(<[u8; 16]>::try_from(data).ok()?),
        _ => return None,
    };
    Some(result.unwrap_or_else(error_reply))
}

/// The reply of an error code: the unit 0 and the string of
/// `errcode_to_string` with a byte 0 (`inet_gethost.c:2074-2124`).
fn error_reply(code: Code) -> Vec<u8> {
    let text: &[u8] = match code {
        Code::HostNotFound => b"notfound",
        Code::TryAgain => b"try_again",
        Code::NoRecovery => b"no_recovery",
        Code::NoData => b"no_data",
        Code::Internal => b"netdb_internal",
    };
    [&[0], text, &[0]].concat()
}

/// The internal error codes of the program (`inet_gethost.c:251-256`).
/// `ERRCODE_NOTSUP` has no case: each supported operation has a lookup.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Code {
    HostNotFound,
    TryAgain,
    NoRecovery,
    NoData,
    Internal,
}

/// Map a `h_errno` value, as `map_netdb_error` does
/// (`inet_gethost.c:2008-2037`).
fn netdb_code(value: c_int) -> Code {
    match value {
        HOST_NOT_FOUND => Code::HostNotFound,
        TRY_AGAIN => Code::TryAgain,
        NO_RECOVERY => Code::NoRecovery,
        NO_DATA => Code::NoData,
        _ => Code::Internal,
    }
}

/// Map a `getaddrinfo` or `getnameinfo` error, as `map_netdb_error_ai`
/// does (`inet_gethost.c:2039-2070`).
fn address_info_code(value: c_int) -> Code {
    match value {
        libc::EAI_AGAIN => Code::TryAgain,
        libc::EAI_FAIL | libc::EAI_NONAME | EAI_NODATA => Code::HostNotFound,
        _ => Code::Internal,
    }
}

/// The reply of addresses of `unit` bytes and names, as `build_reply` and
/// `build_reply_ai` write it (`inet_gethost.c:2129-2267`).
fn hosts_reply(unit: u8, addresses: &[Vec<u8>], names: &[Vec<u8>]) -> Vec<u8> {
    let mut reply = vec![unit];
    reply.extend_from_slice(&u32::try_from(addresses.len()).unwrap_or(0).to_be_bytes());
    addresses
        .iter()
        .for_each(|address| reply.extend_from_slice(address));
    reply.extend_from_slice(&u32::try_from(names.len()).unwrap_or(0).to_be_bytes());
    for name in names {
        reply.extend_from_slice(name);
        reply.push(0);
    }
    reply
}

/// Read the current `h_errno`.
fn h_errno_value() -> c_int {
    // SAFETY: glibc and musl return the address of the `h_errno` of the
    // thread; macOS has one global `int`. The caller holds `NETDB`.
    #[cfg(target_os = "linux")]
    unsafe {
        *__h_errno_location()
    }
    #[cfg(not(target_os = "linux"))]
    unsafe {
        h_errno
    }
}

/// Run `gethostbyname(name)`, as the worker does for `PROTO_IPV4`
/// (`inet_gethost.c:1837-1847`).
fn host_by_name(name: &CStr) -> Result<Vec<u8>, Code> {
    let _lock = NETDB.lock().unwrap_or_else(|poison| poison.into_inner());
    // SAFETY: `name` is a NUL-terminated string. The caller holds `NETDB`.
    let host = unsafe { gethostbyname(name.as_ptr()) };
    hostent_reply(host)
}

/// Run `gethostbyaddr(address, 4, AF_INET)`, as the worker does for
/// `PROTO_IPV4` (`inet_gethost.c:1929-1941`).
fn host_by_address(address: [u8; 4]) -> Result<Vec<u8>, Code> {
    let _lock = NETDB.lock().unwrap_or_else(|poison| poison.into_inner());
    // SAFETY: `address` holds the 4 bytes that the call reads. The caller
    // holds `NETDB`.
    let host = unsafe { gethostbyaddr(address.as_ptr().cast(), 4, libc::AF_INET) };
    hostent_reply(host)
}

/// Return the reply of the static `hostent` of `host`, or the code of
/// `h_errno` when `host` is null. The caller holds `NETDB`.
fn hostent_reply(host: *mut libc::hostent) -> Result<Vec<u8>, Code> {
    if host.is_null() {
        return Err(netdb_code(h_errno_value()));
    }
    // SAFETY: a `hostent` that is not null has a NUL-terminated name, a
    // null-terminated alias list (or a null list), and a null-terminated
    // list of addresses of `h_length` bytes. `NETDB` keeps it valid.
    unsafe {
        let host = &*host;
        let unit = u8::try_from(host.h_length).unwrap_or(0);
        let addresses = c_list(host.h_addr_list)
            .map(|address| std::slice::from_raw_parts(address.cast::<u8>(), unit.into()).to_vec())
            .collect::<Vec<_>>();
        let names = std::iter::once(host.h_name)
            .chain(c_list(host.h_aliases))
            .map(|name| CStr::from_ptr(name).to_bytes().to_vec())
            .collect::<Vec<_>>();
        Ok(hosts_reply(unit, &addresses, &names))
    }
}

/// Return the elements of the null-terminated pointer list `list`, or no
/// elements when `list` is null.
///
/// # Safety
///
/// `list` is null or a valid null-terminated list.
unsafe fn c_list(list: *mut *mut c_char) -> impl Iterator<Item = *mut c_char> {
    (0..)
        // SAFETY: the list ends at its first null element. The list of
        // macOS `gethostbyaddr` can be misaligned, so the read is
        // unaligned, as the C code of the program reads it.
        .map(move |index| {
            if list.is_null() {
                std::ptr::null_mut()
            } else {
                unsafe { list.add(index).read_unaligned() }
            }
        })
        .take_while(|element| !element.is_null())
}

/// Run `getaddrinfo(name)` with `AI_CANONNAME`, `SOCK_STREAM`, and
/// `AF_INET6`, as the worker does for `PROTO_IPV6`
/// (`inet_gethost.c:1796-1809`, `1862-1866`).
fn address_info(name: &CStr) -> Result<Vec<u8>, Code> {
    // SAFETY: a zeroed `addrinfo` is a valid hint without flags.
    let mut hints: libc::addrinfo = unsafe { std::mem::zeroed() };
    hints.ai_flags = libc::AI_CANONNAME;
    hints.ai_socktype = libc::SOCK_STREAM;
    hints.ai_family = libc::AF_INET6;
    let mut list = std::ptr::null_mut();
    // SAFETY: `name` is NUL-terminated, and `getaddrinfo` writes the list
    // to `list`, which `freeaddrinfo` frees below.
    let result = unsafe {
        libc::getaddrinfo(
            name.as_ptr(),
            std::ptr::null(),
            &raw const hints,
            &raw mut list,
        )
    };
    if result != 0 {
        return Err(address_info_code(result));
    }
    let mut addresses = Vec::new();
    let mut names = Vec::new();
    let mut entry = list;
    // SAFETY: each entry of the list is valid until `freeaddrinfo`. An
    // `ai_addr` of the family `AF_INET6` is a `sockaddr_in6`.
    unsafe {
        while let Some(info) = entry.as_ref() {
            if !info.ai_addr.is_null() && c_int::from((*info.ai_addr).sa_family) == libc::AF_INET6 {
                let socket = &*info.ai_addr.cast::<libc::sockaddr_in6>();
                addresses.push(socket.sin6_addr.s6_addr.to_vec());
            }
            if !info.ai_canonname.is_null() && info.ai_family == libc::AF_INET6 {
                names.push(CStr::from_ptr(info.ai_canonname).to_bytes().to_vec());
            }
            entry = info.ai_next;
        }
        libc::freeaddrinfo(list);
    }
    Ok(hosts_reply(16, &addresses, &names))
}

/// Run `getnameinfo` with `NI_NAMEREQD` for the IPv6 address `address`, as
/// the worker does for `PROTO_IPV6` (`inet_gethost.c:1877-1899`,
/// `1954-1964`): the reply has the address and the name.
fn name_info(address: [u8; 16]) -> Result<Vec<u8>, Code> {
    // SAFETY: a zeroed `sockaddr_in6` is valid.
    let mut socket: libc::sockaddr_in6 = unsafe { std::mem::zeroed() };
    #[cfg(not(target_os = "linux"))]
    {
        socket.sin6_len = u8::try_from(size_of::<libc::sockaddr_in6>()).unwrap_or(0);
    }
    socket.sin6_family = libc::sa_family_t::try_from(libc::AF_INET6).unwrap_or(0);
    socket.sin6_addr.s6_addr = address;
    let mut name = [0 as c_char; libc::NI_MAXHOST as usize];
    // SAFETY: `socket` is a `sockaddr_in6` of the given size, and the call
    // writes at most `name.len()` bytes to `name`.
    let result = unsafe {
        libc::getnameinfo(
            (&raw const socket).cast(),
            libc::socklen_t::try_from(size_of::<libc::sockaddr_in6>()).unwrap_or(0),
            name.as_mut_ptr(),
            libc::socklen_t::try_from(name.len()).unwrap_or(0),
            std::ptr::null_mut(),
            0,
            libc::NI_NAMEREQD,
        )
    };
    if result != 0 {
        return Err(address_info_code(result));
    }
    // SAFETY: `getnameinfo` wrote a NUL-terminated name.
    let name = unsafe { CStr::from_ptr(name.as_ptr()) }.to_bytes().to_vec();
    Ok(hosts_reply(16, &[address.to_vec()], &[name]))
}

#[cfg(test)]
#[path = "../../../tests/runtime/host/gethost.rs"]
mod tests;

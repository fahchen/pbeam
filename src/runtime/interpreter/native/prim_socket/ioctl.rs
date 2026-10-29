//! The bindings of `prim_socket:nif_ioctl/2,3,4` (`prim_socket_nif.c:13794-13877`),
//! with the request switches `essio_ioctl2`, `essio_ioctl3`, and
//! `essio_ioctl4` of the Unix I/O backend (`unix_socket_syncio.c:5364-5602`).
//!
//! ERTS runs each request with `ioctl(2)`. PBeam has no safe call that
//! gives the same result for these requests, so a request of a switch
//! stops the run (see the row "NIF stubs").

use super::socket::{closed, socket_argument};
use super::supports::{HOST, IOCTL_REQUESTS, atom};
use super::*;
use crate::runtime::resources::lock;
use crate::runtime::sockets::{CLOSED, CLOSING};

/// The requests of each switch, by the argument count of `nif_ioctl`: the
/// requests that the host defines, with `FIONWRITE`, `FIONSPACE`, and
/// `SIOCGENADDR`, which neither host defines, left out
/// (`unix_socket_syncio.c:5364-5405`, `5431-5517`, `5540-5602`).
const SWITCHES: [(usize, &[&str]); 3] = [
    (2, &["gifconf", "nread", "atmark"]),
    (
        3,
        &[
            "gifname",
            "gifindex",
            "gifflags",
            "gifaddr",
            "gifdstaddr",
            "gifbrdaddr",
            "gifnetmask",
            "gifmtu",
            "gifhwaddr",
            "gifmap",
            "giftxqlen",
        ],
    ),
    (
        4,
        &[
            "sifflags",
            "sifaddr",
            "sifdstaddr",
            "sifbrdaddr",
            "sifnetmask",
            "sifmtu",
            "siftxqlen",
            "sifhwaddr",
        ],
    ),
];

/// The requests of `essio_ioctl4` that `esock_supports_ioctl_requests`
/// does not list (`prim_socket_nif.c:5509-5626`), as `(name, request)` in
/// the host columns of [`IOCTL_REQUESTS`]: `SIOCSIFNETMASK` and
/// `SIOCSIFHWADDR`.
const SET_ONLY: [(&str, [Option<u32>; 2]); 2] = [
    ("sifnetmask", [Some(2_149_607_702), Some(35100)]),
    ("sifhwaddr", [None, Some(35108)]),
];

/// Run `prim_socket:nif_ioctl(Socket, Request | Arguments)` with the
/// `arguments` after the socket, as `nif_ioctl` does
/// (`prim_socket_nif.c:13794-13877`): `badarg` for a term that is not a
/// socket or a `Request` that is not an `unsigned long`;
/// `{error, closed}`; and `{error, enotsup}` for a request that is not in
/// the switch of its argument count.
///
/// # Errors
///
/// A request of the switch stops the run: PBeam does not run `ioctl(2)`.
pub(super) fn ioctl(
    scheduler: &mut Scheduler,
    socket: &Term,
    arguments: &[Term],
) -> Result<BifResult, BeamError> {
    let arity = arguments.len() + 1;
    let Some((_, socket)) = socket_argument(scheduler, socket) else {
        return Ok(BifResult::badarg());
    };
    let request = match &arguments[0] {
        Term::Integer(value) => u64::try_from(*value).ok(),
        Term::BigInt(value) => num_traits::ToPrimitive::to_u64(&**value),
        _ => None,
    };
    let Some(request) = request else {
        return Ok(BifResult::badarg());
    };
    if lock(&socket).read & (CLOSING | CLOSED) != 0 {
        return Ok(closed());
    }
    let names = SWITCHES
        .iter()
        .find_map(|(count, names)| (*count == arity).then_some(*names))
        .expect("nif_ioctl has 2, 3, or 4 arguments");
    let known = IOCTL_REQUESTS
        .iter()
        .chain(&SET_ONLY)
        .find(|(name, values)| {
            names.contains(name) && values[HOST].map(u64::from) == Some(request)
        });
    if let Some((name, _)) = known {
        return Err(error(format!(
            "prim_socket:nif_ioctl/{arity}: the request {name} needs ioctl(2), which PBeam does not run"
        )));
    }
    Ok(BifResult::Value(Term::tuple(vec![
        Term::static_atom("error"),
        atom(scheduler, "enotsup")?,
    ])))
}

#[cfg(test)]
#[path = "../../../../../tests/runtime/interpreter/native/prim_socket/ioctl.rs"]
mod tests;

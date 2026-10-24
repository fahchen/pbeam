//! The load-check table of the `prim_socket` NIF library
//! (`erts/emulator/nifs/common/prim_socket_nif.c`), which
//! `prim_socket:on_load/1` loads (`prim_socket.erl:137-182`).
//!
//! PBeam binds `nif_supports/0,1` (see `supports`), which the node boot
//! calls in `prim_socket:init/0` (`prim_socket.erl:184-204`), and
//! `nif_info/0,1`, `nif_open/4`, `nif_bind/2`, `nif_sockname/1`,
//! `nif_close/1`, `nif_finalize_close/1`, `nif_setopt/5`, and
//! `nif_getopt/3,5` (see `socket`), and
//! `nif_connect/1,3`, `nif_listen/2`, `nif_accept/2`, and `nif_cancel/3`
//! (see `select`). A call of each
//! other function of the table stops the run with the runtime error of the
//! static library (`Library::unbound_fails`).

use super::*;
use crate::runtime::resources::lock;
use crate::runtime::sockets::Sockets;

mod atoms;
mod select;
mod socket;
mod supports;

/// The bindings of the `prim_socket` library.
///
/// # Examples
///
/// ```rust,ignore
/// assert_eq!((BINDINGS[0].name, BINDINGS[0].arity), ("nif_supports", 0));
/// ```
pub(super) const BINDINGS: &[Binding] = &[
    Binding {
        name: "nif_accept",
        arity: 2,
        job: Job::Normal,
        run: |scheduler, arguments| select::accept(scheduler, &arguments[0], &arguments[1]),
    },
    Binding {
        name: "nif_bind",
        arity: 2,
        job: Job::Normal,
        run: |scheduler, arguments| socket::bind_2(scheduler, &arguments[0], &arguments[1]),
    },
    Binding {
        name: "nif_cancel",
        arity: 3,
        job: Job::Normal,
        run: |scheduler, arguments| {
            select::cancel(scheduler, &arguments[0], &arguments[1], &arguments[2])
        },
    },
    Binding {
        name: "nif_close",
        arity: 1,
        job: Job::Normal,
        run: |scheduler, arguments| socket::close(scheduler, &arguments[0]),
    },
    Binding {
        name: "nif_connect",
        arity: 1,
        job: Job::Normal,
        run: |scheduler, arguments| select::connect(scheduler, &arguments[0], None),
    },
    Binding {
        name: "nif_connect",
        arity: 3,
        job: Job::Normal,
        run: |scheduler, arguments| {
            select::connect(
                scheduler,
                &arguments[0],
                Some((&arguments[1], &arguments[2])),
            )
        },
    },
    Binding {
        name: "nif_finalize_close",
        arity: 1,
        job: Job::Normal,
        run: |scheduler, arguments| socket::finalize_close(scheduler, &arguments[0]),
    },
    Binding {
        name: "nif_getopt",
        arity: 3,
        job: Job::Normal,
        run: socket::getopt_3,
    },
    Binding {
        name: "nif_getopt",
        arity: 5,
        job: Job::Normal,
        run: socket::getopt_5,
    },
    Binding {
        name: "nif_info",
        arity: 0,
        job: Job::Normal,
        run: |scheduler, _| socket::info_0(scheduler),
    },
    Binding {
        name: "nif_info",
        arity: 1,
        job: Job::Normal,
        run: |scheduler, arguments| socket::info_1(scheduler, &arguments[0]),
    },
    Binding {
        name: "nif_listen",
        arity: 2,
        job: Job::Normal,
        run: |scheduler, arguments| select::listen(scheduler, &arguments[0], &arguments[1]),
    },
    Binding {
        name: "nif_open",
        arity: 4,
        job: Job::Normal,
        run: |scheduler, arguments| {
            socket::open_4(
                scheduler,
                &arguments[0],
                &arguments[1],
                &arguments[2],
                &arguments[3],
            )
        },
    },
    Binding {
        name: "nif_setopt",
        arity: 5,
        job: Job::Normal,
        run: socket::setopt_5,
    },
    Binding {
        name: "nif_sockname",
        arity: 1,
        job: Job::Normal,
        run: |scheduler, arguments| socket::sockname(scheduler, &arguments[0]),
    },
    Binding {
        name: "nif_supports",
        arity: 0,
        job: Job::Normal,
        run: |scheduler, _| supports::supports_0(scheduler),
    },
    Binding {
        name: "nif_supports",
        arity: 1,
        job: Job::Normal,
        run: |scheduler, arguments| supports::supports_1(scheduler, &arguments[0]),
    },
];

/// The functions of the ERTS `prim_socket` NIF library, as `(name, arity)`
/// in the order of `esock_funcs` (`prim_socket_nif.c:18696-18748`).
/// `erlang:load_nif/2` checks each of them in this order. The flags of
/// `nif_sendfile/5`, `nif_sendfile/4`, `nif_sendfile/1`, `nif_ioctl/2`,
/// `nif_ioctl/3`, `nif_ioctl/4`, and `nif_finalize_close/1` are
/// `ERL_NIF_DIRTY_JOB_IO_BOUND` (`prim_socket_nif.c:18720-18722`,
/// `18736-18738`, `18747`), and the flags of each other function are 0.
///
/// # Examples
///
/// ```rust,ignore
/// assert_eq!(FUNCTIONS[0], ("nif_info", 0));
/// assert_eq!(FUNCTIONS[39], ("nif_finalize_close", 1));
/// ```
pub(super) const FUNCTIONS: [(&str, u32); 40] = [
    ("nif_info", 0),
    ("nif_info", 1),
    ("nif_supports", 0),
    ("nif_supports", 1),
    ("nif_command", 1),
    ("nif_open", 2),
    ("nif_open", 4),
    ("nif_bind", 2),
    ("nif_bind", 3),
    ("nif_connect", 1),
    ("nif_connect", 3),
    ("nif_listen", 2),
    ("nif_accept", 2),
    ("nif_peeloff", 2),
    ("nif_send", 4),
    ("nif_sendto", 5),
    ("nif_sendmsg", 5),
    ("nif_sendmmsg", 4),
    ("nif_sendv", 3),
    ("nif_sendfile", 5),
    ("nif_sendfile", 4),
    ("nif_sendfile", 1),
    ("nif_recv", 4),
    ("nif_recvfrom", 4),
    ("nif_recvmsg", 5),
    ("nif_recvmmsg", 6),
    ("nif_close", 1),
    ("nif_shutdown", 2),
    ("nif_setopt", 5),
    ("nif_getopt", 3),
    ("nif_getopt", 5),
    ("nif_sockname", 1),
    ("nif_socknames", 2),
    ("nif_peername", 1),
    ("nif_peernames", 2),
    ("nif_ioctl", 2),
    ("nif_ioctl", 3),
    ("nif_ioctl", 4),
    ("nif_cancel", 3),
    ("nif_finalize_close", 1),
];

/// Run the load callback of the `prim_socket` library with `load_info`, as
/// the ERTS `on_load` does (`prim_socket_nif.c:18785-19202`), and return
/// its veto. A load info that is not a map with a local pid under the key
/// `registry` gives the veto 1 (`prim_socket_nif.c:18862-18867`,
/// `socket_util.c:2958-2971`). Each other load info gives 0 and keeps the
/// pid as the socket registry, as `data.regPid`. ERTS also
/// gives the veto 1 when it cannot start its I/O backend or make its
/// resource type (`prim_socket_nif.c:19161-19164`, `19180-19201`); PBeam
/// starts no backend and makes no resource type, so it has no such veto.
/// A map key `registry` exists only when the atom is in the atom table of
/// the run.
///
/// # Examples
///
/// ```rust,ignore
/// assert_eq!(load(&Term::Integer(0), &scheduler), 1);
/// ```
pub(super) fn load(load_info: &Term, scheduler: &Scheduler) -> i32 {
    // ERTS makes its atoms before it reads the load info
    // (`prim_socket_nif.c:18842-18857`). The atom indexes set the key order
    // of the maps of `nif_info/0,1`. A full atom table stops ERTS here; in
    // PBeam the first use of a missing atom gives the error of
    // `AtomTable::put`.
    for name in atoms::LOAD_ATOMS {
        let _ = scheduler.atoms.put(name);
    }
    let registry = match (load_info, scheduler.atoms.get("registry")) {
        (Term::Map(map), Some(key)) => map.get(&Term::Atom(key)),
        _ => None,
    };
    let Some(Term::Pid(registry)) = registry else {
        return 1;
    };
    lock(&scheduler.node().resources.kind::<Sockets>()).registry = Some(*registry);
    0
}

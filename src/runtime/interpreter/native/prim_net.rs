//! The load-check table of the `net` NIF library
//! (`erts/emulator/nifs/common/prim_net_nif.c`), which the module
//! `prim_net` loads with `erlang:load_nif("net", Extra)`
//! (`prim_net.erl:367-372`). ERTS registers the library under the module
//! `prim_net` (`prim_net_nif.c:5148`).
//!
//! PBeam binds `nif_info/0`, `nif_command/1`, and `nif_gethostname/0`. A
//! call of another function of the table stops the run with the runtime
//! error of the static library (`Library::unbound_fails`). The load callback of the library returns 0
//! for each load info: it reads only the optional `debug` flag of a map
//! (`prim_net_nif.c:5103-5107`, `5116-5146`). ERTS gives the veto 1 only
//! when it cannot make its resource type (`prim_net_nif.c:5135-5145`);
//! PBeam makes no resource type, so it has no such veto.

use std::ffi::OsString;
use std::os::unix::ffi::OsStrExt;

use super::*;

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
        name: "nif_gethostname",
        arity: 0,
        job: Job::Normal,
        run: |_, _| Ok(BifResult::Value(gethostname(nix::unistd::gethostname()))),
    },
    Binding {
        name: "nif_info",
        arity: 0,
        job: Job::Normal,
        run: |scheduler, _| info(&scheduler.atoms),
    },
];

/// Run `nif_info/0` (`prim_net_nif.c:848-870`): the map of the `debug`
/// flag and of `netns`. The debug flag of PBeam is always `false`, because
/// [`command`] does not set it. `netns` is `false` on each host: the
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

#[cfg(test)]
#[path = "../../../../tests/runtime/interpreter/native/prim_net.rs"]
mod tests;

//! The load-check table of the `net` NIF library
//! (`erts/emulator/nifs/common/prim_net_nif.c`), which the module
//! `prim_net` loads with `erlang:load_nif("net", Extra)`
//! (`prim_net.erl:367-372`). ERTS registers the library under the module
//! `prim_net` (`prim_net_nif.c:5148`).
//!
//! PBeam has no binding of a `net` NIF. A call of a function of the table
//! stops the run with the runtime error of the static library
//! (`Library::unbound_fails`). The load callback of the library returns 0
//! for each load info: it reads only the optional `debug` flag of a map
//! (`prim_net_nif.c:5103-5107`, `5116-5146`). ERTS gives the veto 1 only
//! when it cannot make its resource type (`prim_net_nif.c:5135-5145`);
//! PBeam makes no resource type, so it has no such veto.

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

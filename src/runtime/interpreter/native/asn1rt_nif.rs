//! The load-check table of the `asn1rt_nif` NIF library
//! (`lib/asn1/c_src/asn1_erl_nif.c`), which `asn1rt_nif:load_nif/0` loads
//! with the load info 1 (`asn1rt_nif.erl:37-41`, `67`).
//!
//! PBeam has no binding of an `asn1rt_nif` NIF. A call of a function of the
//! table stops the run with the runtime error of the static library
//! (`Library::unbound_fails`).

use super::*;

/// The functions of the `asn1rt_nif` NIF library, as `(name, arity)` in
/// the order of `nif_funcs` (`asn1_erl_nif.c:1343-1347`).
/// `erlang:load_nif/2` checks each of them in this order. The entries have
/// no flags field, so their flags are 0.
///
/// # Examples
///
/// ```rust,ignore
/// assert_eq!(FUNCTIONS[1], ("decode_ber_tlv_raw", 1));
/// ```
pub(super) const FUNCTIONS: [(&str, u32); 3] = [
    ("encode_per_complete", 1),
    ("decode_ber_tlv_raw", 1),
    ("encode_ber_tlv", 1),
];

/// Run the load callback of the `asn1rt_nif` library with `load_info`, as
/// the ERTS `load` does, and return its veto: 0 for the integer 1, and -1
/// for each other term (`asn1_erl_nif.c:1320-1329`).
///
/// # Examples
///
/// ```rust,ignore
/// assert_eq!(load(&Term::Integer(1), &atoms), 0);
/// assert_eq!(load(&Term::Integer(2), &atoms), -1);
/// ```
pub(super) fn load(load_info: &Term, _atoms: &AtomTable) -> i32 {
    if matches!(load_info, Term::Integer(1)) {
        0
    } else {
        -1
    }
}

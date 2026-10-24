//! Unit tests of `src/runtime/interpreter/bif/erts_internal.rs` for an
//! argument that the fixture probes of their tasks cannot check on PBeam:
//! the public API of the runtime has no constructor of a bitstring that is
//! not a whole number of bytes.

use super::*;
use crate::term::Binary;

#[test]
fn gives_badarg_for_a_bitstring_that_is_not_a_whole_number_of_bytes() {
    // `erts_get_aligned_binary_bytes` (`erl_bits.h:606-633`) gives NULL for
    // a term that is not a byte-aligned binary, so
    // `erts_internal:binary_to_integer/2` gives the atom `badarg`. The
    // pinned ERTS gives that atom (measured with the `bits/0` probe of
    // `fixtures/binary_to_integer`). The public API of the runtime has no
    // constructor of such a bitstring, so this unit test builds it with
    // `Binary::bits`.
    let bitstring = Term::Binary(Binary::from(vec![1]).bits(0, 1));
    let result = binary_to_integer(&bitstring, &Term::Integer(10));
    assert!(matches!(
        &result,
        BifResult::Value(Term::Atom(atom)) if atom.is_static("badarg")
    ));
}

#[test]
fn names_a_port_as_erts_does() {
    // `erts_internal:term_type/1` gives `port` for a local port
    // (`erl_map.c:3336-3417`). The size probe runs without a port, so
    // this unit test builds the port term.
    let atoms = AtomTable::new();
    let result = term_type(&atoms, &Term::Port(crate::term::PortId(5))).unwrap();
    assert!(matches!(
        &result,
        BifResult::Value(Term::Atom(atom)) if atom.is_static("port")
    ));
}

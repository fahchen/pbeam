//! Unit tests of `src/runtime/interpreter/bif/float.rs` for an argument that
//! the fixture probes of their tasks cannot check on PBeam: the public API
//! of the runtime has no constructor of a bitstring that is not a whole
//! number of bytes.

use super::*;
use crate::runtime::interpreter::ExceptionClass;
use crate::term::Binary;

#[test]
fn raises_badarg_for_a_bitstring_that_is_not_a_whole_number_of_bytes() {
    // `binary_to_float_1` (`bif.c:3729-3735`) gives `badarg` for a term that
    // is not a bit string with a whole number of bytes. The pinned ERTS
    // gives `badarg` (measured with the `bits/0` probe of
    // `fixtures/binary_to_float`). The public API of the runtime has no
    // constructor of such a bitstring, so this unit test builds it with
    // `Binary::bits`.
    let bitstring = Term::Binary(Binary::from(vec![1]).bits(0, 1));
    let result = from_binary(&bitstring);
    assert!(matches!(
        result,
        BifResult::Raise { class: ExceptionClass::Error, reason }
            if reason == Term::static_atom("badarg")
    ));
}

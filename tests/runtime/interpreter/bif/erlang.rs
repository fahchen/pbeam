//! Unit tests of `src/runtime/interpreter/bif/erlang.rs` for an argument
//! that a fixture cannot check on PBeam: the public API of the runtime has
//! no constructor of a bitstring that is not a whole number of bytes.

use super::*;
use crate::term::Binary;

#[test]
fn raises_badarg_for_a_bitstring_that_is_not_a_whole_number_of_bytes() {
    // `erts_get_aligned_binary_bytes` (`erl_unicode.c:1969-1972`) gives NULL
    // for a term that is not a byte-aligned binary, so `binary_to_atom/2`
    // raises `badarg`. The pinned ERTS gives `badarg` with the frame
    // `{erlang,binary_to_atom,[<<1:1>>,latin1],[{error_info,#{module =>
    // erl_erts_errors}}]}` (measured with `mise run erts-eval`).
    let mut atoms = AtomTable::new();
    let bitstring = Term::Binary(Binary::from(vec![1]).bits(0, 1));
    let result = binary_to_atom(&mut atoms, &bitstring, &Term::static_atom("latin1")).unwrap();
    assert!(matches!(
        result,
        BifResult::Raise { class: ExceptionClass::Error, reason }
            if reason == Term::static_atom("badarg")
    ));
}

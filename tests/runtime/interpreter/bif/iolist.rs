//! Unit tests of the iodata walk of `src/runtime/interpreter/bif/iolist.rs`
//! for a bitstring that is not a whole number of bytes. No compiled fixture
//! makes such a bitstring yet, so the tests make one with `Binary::bits`.

use super::*;
use crate::term::Binary;

#[test]
fn rejects_a_bitstring_as_iodata() {
    // The pinned ERTS raises `badarg` for `iolist_to_binary(B)`,
    // `iolist_size([B])`, `list_to_binary([B])`, and
    // `iolist_to_binary([1|B])` with B = <<1,2:4>> (measured with `mise run
    // erts-eval`).
    let bitstring = || Term::Binary(Binary::from(vec![1, 2 << 4]).bits(0, 12));
    let byte = || Term::Integer(1);
    assert_eq!(iodata_bytes(&bitstring()), None);
    assert_eq!(
        iodata_bytes(&Term::list_from(vec![byte(), bitstring()])),
        None
    );
    let nested = Term::list_from(vec![Term::list_from(vec![bitstring()])]);
    assert_eq!(iodata_bytes(&nested), None);
    assert_eq!(
        iodata_bytes(&Term::list_with_tail(vec![byte()], bitstring())),
        None
    );
    // The same iodata with a binary of whole bytes gives its bytes.
    let whole = Term::binary(vec![1, 2]);
    assert_eq!(
        iodata_bytes(&Term::list_with_tail(vec![byte()], whole)),
        Some(vec![1, 1, 2])
    );
}

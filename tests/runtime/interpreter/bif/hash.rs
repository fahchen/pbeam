//! Unit tests of `src/runtime/interpreter/bif/hash.rs` for the md5 cases that
//! a fixture probe cannot build: a bitstring that is not a whole number of
//! bytes, and a context binary whose buffered length is above 64.

use super::*;

#[test]
fn hashes_abc_and_builds_the_context_of_erts() {
    // `erlang:md5("abc")` gives this digest on the pinned ERTS (measured
    // with `mise run erts-eval`), and `erlang:md5_init()` gives 96 bytes
    // (`erts_md5_state`, `erl_md5.h:31-36`).
    let mut state = Md5::init();
    assert_eq!(
        state.finish().to_vec(),
        [
            212, 29, 140, 217, 143, 0, 178, 4, 233, 128, 9, 152, 236, 248, 66, 126
        ]
    );
    let mut state = Md5::init();
    state.update(b"abc");
    assert_eq!(
        state.finish().to_vec(),
        [
            144, 1, 80, 152, 60, 210, 79, 176, 214, 150, 63, 125, 40, 225, 127, 114
        ]
    );
    assert_eq!(Md5::init().to_binary().len(), CONTEXT_BYTES);
}

#[test]
fn rejects_a_bitstring_that_is_not_a_whole_number_of_bytes() {
    // `erts_get_aligned_binary_bytes` (`erl_bits.h:621-628`) gives NULL for
    // a bitstring that is not byte-aligned, so `md5/1` and `md5_update/2`
    // raise `badarg` (`erl_bif_chksum.c:74-79`, `558-563`). The pinned ERTS
    // gives `badarg` for `erlang:md5(<<1:1>>)` and for
    // `erlang:md5_update(erlang:md5_init(), <<1:1>>)` (measured with
    // `mise run erts-eval`). The loader of a PBeam fixture has no such
    // literal.
    let bitstring = Term::Binary(Binary::from(vec![1]).bits(0, 1));
    let mut budget = 8;
    let mut bytes = Vec::new();
    assert!(
        read(&bitstring, &mut budget, &mut |part| bytes
            .extend_from_slice(part))
        .is_err()
    );
    assert!(context_bytes(&bitstring).is_none());
}

#[test]
fn rejects_a_context_with_a_buffered_length_above_64() {
    // ERTS reads and writes outside the buffer of the state when the
    // buffered length is above 64 (`erl_md5.c:90-99`, `222-243`), which is
    // undefined behavior. PBeam treats such a term as a term that is not a
    // context: see "md5 context" in docs/erts-differences.md.
    let mut bytes = Md5::init().to_binary();
    bytes[24..32].copy_from_slice(&65_u64.to_le_bytes());
    let context = Term::Binary(Binary::from(bytes));
    assert!(context_bytes(&context).and_then(Md5::from_binary).is_none());
}

#[test]
fn reads_the_range_of_phash_and_phash2_as_erts() {
    // `phash_2` reads the range of 1 to 2^32 - 1, and 2^32, and each other
    // term is `badarg` (`bif.c:5324-5334`). A bignum above 2^32 is
    // `badarg` too.
    assert_eq!(parse_range(&Term::Integer(1)), Some(1));
    assert_eq!(parse_range(&Term::Integer((1 << 32) - 1)), Some(u32::MAX));
    assert_eq!(parse_range(&Term::Integer(1 << 32)), Some(0));
    assert_eq!(parse_range(&Term::Integer(0)), None);
    assert_eq!(parse_range(&Term::Integer(-1)), None);
    assert_eq!(parse_range(&Term::Integer((1 << 32) + 1)), None);
    assert_eq!(parse_range(&Term::Float(1.0)), None);
    assert_eq!(parse_range(&Term::static_atom("default")), None);
    let bignum = Term::from_bigint(num_bigint::BigInt::from(1) << 100);
    assert_eq!(parse_range(&bignum), None);
}

#[test]
fn reads_no_byte_of_an_iodata_that_has_no_budget() {
    // A walk with no budget gives the whole term as the rest, which
    // `md5/1` traps with (`erl_bif_chksum.c:60-65`).
    let mut budget = 0;
    let mut bytes = Vec::new();
    let list = Term::list_from([Term::Integer(1), Term::Integer(2)]);
    assert!(
        read(&list, &mut budget, &mut |part| bytes
            .extend_from_slice(part))
        .unwrap()
        .is_some()
    );
    assert!(bytes.is_empty());
    let mut budget = 0;
    assert!(
        read(&Term::nil(), &mut budget, &mut |_| {})
            .unwrap()
            .is_none()
    );
}

#[test]
fn stops_a_walk_at_the_budget_before_the_next_element() {
    // The walk of ERTS stops when its budget is 0, before it reads the next
    // element, so the rest holds that element and the empty list after it
    // (`erl_bif_chksum.c:118-160`, `205-212`). The pinned ERTS gives
    // `erlang:md5([1, 2, []])` the same digest as `erlang:md5([1, 2])`, and
    // a walk with a budget of 2 holds the rest `[[]]`.
    let list = Term::list_from([Term::Integer(1), Term::Integer(2), Term::nil()]);
    let mut budget = 2;
    let mut bytes = Vec::new();
    let rest = read(&list, &mut budget, &mut |part| {
        bytes.extend_from_slice(part)
    })
    .unwrap()
    .expect("the walk does not read the last element");
    assert_eq!(bytes, [1, 2]);
    assert_eq!(rest, Term::list_from([Term::nil()]));
}

#[test]
fn reads_a_binary_of_several_parts_with_one_update() {
    // An append-built binary keeps its parts, and the walk reads them with
    // one update call, as `md5_wrap` reads one bitstring
    // (`erl_bif_chksum.c:74-92`, `280-284`), so the md5 state does not
    // depend on the parts.
    let binary = Term::Binary(Binary::from_chunks(vec![
        std::sync::Arc::new(vec![1; 30]),
        std::sync::Arc::new(vec![2; 6]),
    ]));
    let mut budget = 64;
    let mut calls = 0;
    let mut bytes = Vec::new();
    read(&binary, &mut budget, &mut |part| {
        calls += 1;
        bytes.extend_from_slice(part);
    })
    .unwrap();
    assert_eq!(calls, 1);
    assert_eq!(bytes.len(), 36);
}

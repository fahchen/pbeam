//! Unit tests of the number conversions and of the array calls of
//! `src/runtime/interpreter/bif/atomics.rs`.

use super::*;
use crate::runtime::atomics::AtomicArrays;
use crate::runtime::scheduler::Scheduler;
use crate::term::{AtomTable, Reference};

/// Return the reason of the raise of `result`, or panic.
fn reason(result: Result<u64, BifResult>) -> Term {
    match result {
        Err(BifResult::Raise { reason, .. }) => reason,
        Err(other) => panic!("not a raise: {other:?}"),
        Ok(value) => panic!("not a raise: {value}"),
    }
}

/// Return the word of `result`, or panic.
fn word(result: Result<u64, BifResult>) -> u64 {
    match result {
        Ok(value) => value,
        Err(exception) => panic!("not a word: {exception:?}"),
    }
}

/// Return whether `result` is the `error:badarg` exception.
fn is_badarg(result: &BifResult) -> bool {
    matches!(result, BifResult::Raise { reason, .. } if *reason == Term::static_atom("badarg"))
}

/// Return whether `result` is a value.
fn is_value(result: &BifResult) -> bool {
    matches!(result, BifResult::Value(_))
}

/// Return an array of `size` signed atomics at the reference 0.
fn array(size: usize) -> AtomicArrays {
    let arrays = AtomicArrays::default();
    assert!(arrays.create(Reference(0), true, size));
    arrays
}

#[test]
fn reads_a_machine_word_as_term_to_uword_does() {
    // `term_to_UWord` (`big.c:3564-3605`) takes a non-negative integer of
    // the machine word, gives `badarg` for each other term, and
    // `system_limit` for a positive integer above the word.
    assert_eq!(word(to_uword(&Term::Integer(0))), 0);
    assert_eq!(word(to_uword(&Term::Integer(i64::MAX))), i64::MAX as u64);
    assert_eq!(
        word(to_uword(&Term::from_bigint(BigInt::from(1u64 << 63)))),
        1 << 63
    );
    assert_eq!(
        reason(to_uword(&Term::Integer(-1))),
        Term::static_atom("badarg")
    );
    assert_eq!(
        reason(to_uword(&Term::static_atom("x"))),
        Term::static_atom("badarg")
    );
    assert_eq!(
        reason(to_uword(&Term::from_bigint(BigInt::from(1u64) << 64))),
        Term::static_atom("system_limit")
    );
    // `big_sign` (`big.c:3584-3587`): a negative integer above the signed
    // 64-bit range raises `badarg`, not `system_limit`.
    assert_eq!(
        reason(to_uword(&Term::from_bigint(-(BigInt::from(1u64) << 70u32)))),
        Term::static_atom("badarg")
    );
    assert!(matches!(system_limit(), BifResult::Raise { .. }));
}

#[test]
fn checks_an_index_against_the_size_of_the_array() {
    // `get_ref_ix` (`erl_bif_atomics.c:104-110`) makes the machine word of
    // the index, subtracts one, and checks the result against the size.
    let arrays = array(2);
    let array = arrays.array(&Term::Reference(Reference(0))).unwrap();
    assert_eq!(position(&array, &Term::Integer(1)), Some(0));
    assert_eq!(position(&array, &Term::Integer(2)), Some(1));
    assert_eq!(position(&array, &Term::Integer(0)), None);
    assert_eq!(position(&array, &Term::Integer(3)), None);
    assert_eq!(position(&array, &Term::Integer(i64::MAX)), None);
    assert_eq!(position(&array, &Term::Float(1.5)), None);
    assert_eq!(position(&array, &Term::static_atom("x")), None);
}

#[test]
fn reads_a_value_of_the_range_of_the_array() {
    // `get_value` (`erl_bif_atomics.c:112-117`): a signed array takes the
    // signed 64-bit range, and an unsigned array the unsigned one.
    assert_eq!(value_bits(true, &Term::Integer(i64::MIN)), Some(i64::MIN));
    assert_eq!(value_bits(true, &Term::Integer(i64::MAX)), Some(i64::MAX));
    assert_eq!(
        value_bits(true, &Term::from_bigint(BigInt::from(1u64 << 63))),
        None
    );
    assert_eq!(value_bits(false, &Term::Integer(0)), Some(0));
    assert_eq!(value_bits(false, &Term::Integer(-1)), None);
    assert_eq!(
        value_bits(false, &Term::from_bigint(BigInt::from(u64::MAX))),
        Some(-1)
    );
    assert_eq!(
        value_bits(false, &Term::from_bigint(BigInt::from(1u64) << 64)),
        None
    );
    assert_eq!(value_bits(false, &Term::static_atom("x")), None);
}

#[test]
fn reads_an_increment_of_each_range() {
    // `get_incr` (`erl_bif_atomics.c:119-123`) takes the signed range or
    // the unsigned one.
    assert_eq!(increment_bits(&Term::Integer(-1)), Some(-1));
    assert_eq!(
        increment_bits(&Term::from_bigint(BigInt::from(1u64 << 63))),
        Some(i64::MIN)
    );
    assert_eq!(
        increment_bits(&Term::from_bigint(BigInt::from(u64::MAX))),
        Some(-1)
    );
    assert_eq!(
        increment_bits(&Term::from_bigint(BigInt::from(1u64) << 64)),
        None
    );
    assert_eq!(increment_bits(&Term::Float(1.0)), None);
}

#[test]
fn builds_a_value_of_each_range() {
    // `bld_atomic` (`erl_bif_atomics.c:125-146`) gives the bits of a signed
    // array as a signed integer and of an unsigned array as a non-negative
    // one.
    assert_eq!(value(true, -1), Term::Integer(-1));
    assert_eq!(value(true, i64::MIN), Term::Integer(i64::MIN));
    assert_eq!(value(false, 0), Term::Integer(0));
    assert_eq!(value(false, 5), Term::Integer(5));
    assert_eq!(value(false, -1), Term::from_bigint(BigInt::from(u64::MAX)));
    assert_eq!(
        value(false, i64::MIN),
        Term::from_bigint(BigInt::from(1u64 << 63))
    );
}

#[test]
fn makes_an_array_and_writes_through_it() {
    // The three BIFs of the release path over one array of one atomic:
    // `erts_internal:atomics_new/2`, `atomics:get/2`, and `atomics:add/3`.
    let mut scheduler = Scheduler::new(AtomTable::new());
    let result = new(&mut scheduler, &Term::Integer(1), &Term::Integer(1)).unwrap();
    let BifResult::Value(reference) = result else {
        panic!("not a value: {result:?}");
    };
    assert!(matches!(reference, Term::Reference(_)));
    assert!(is_value(
        &get(&mut scheduler, &reference, &Term::Integer(1)).unwrap()
    ));
    assert!(is_value(
        &add(
            &mut scheduler,
            &reference,
            &Term::Integer(1),
            &Term::Integer(3)
        )
        .unwrap()
    ));
    assert!(matches!(
        get(&mut scheduler, &reference, &Term::Integer(1)).unwrap(),
        BifResult::Value(Term::Integer(3))
    ));
    assert!(is_value(
        &put(
            &mut scheduler,
            &reference,
            &Term::Integer(1),
            &Term::Integer(-2)
        )
        .unwrap()
    ));
    assert!(is_value(&info(&mut scheduler, &reference).unwrap()));
    // A term that is not a reference of an array, an index outside the
    // array, a value outside its range, an increment outside both ranges,
    // a size of 0, a size that is not an integer, and options outside the
    // encoded form.
    assert!(is_value(
        &new(&mut scheduler, &Term::Integer(2), &Term::Integer(0)).unwrap()
    ));
    assert!(is_badarg(
        &get(&mut scheduler, &Term::Integer(0), &Term::Integer(1)).unwrap()
    ));
    assert!(is_badarg(
        &put(
            &mut scheduler,
            &reference,
            &Term::Integer(2),
            &Term::Integer(1)
        )
        .unwrap()
    ));
    assert!(is_badarg(
        &add(
            &mut scheduler,
            &reference,
            &Term::Integer(1),
            &Term::Float(1.0)
        )
        .unwrap()
    ));
    assert!(is_badarg(
        &add(
            &mut scheduler,
            &reference,
            &Term::Integer(2),
            &Term::Integer(1)
        )
        .unwrap()
    ));
    assert!(is_badarg(
        &put(
            &mut scheduler,
            &Term::Integer(0),
            &Term::Integer(1),
            &Term::Integer(1)
        )
        .unwrap()
    ));
    assert!(is_badarg(
        &add(
            &mut scheduler,
            &Term::Integer(0),
            &Term::Integer(1),
            &Term::Integer(1)
        )
        .unwrap()
    ));
    assert!(is_badarg(&info(&mut scheduler, &Term::Integer(0)).unwrap()));
    assert!(is_badarg(
        &new(&mut scheduler, &Term::Integer(0), &Term::Integer(1)).unwrap()
    ));
    assert!(is_badarg(
        &new(&mut scheduler, &Term::Float(1.0), &Term::Integer(1)).unwrap()
    ));
    assert!(is_badarg(
        &new(&mut scheduler, &Term::Integer(1), &Term::Integer(-1)).unwrap()
    ));
    assert!(is_badarg(
        &new(&mut scheduler, &Term::Integer(1), &Term::Float(1.0)).unwrap()
    ));
    // The size above the machine word divided by the bytes of an atomic,
    // the options above the machine word, and an array that PBeam cannot
    // allocate.
    for (size, options) in [
        (
            Term::from_bigint(BigInt::from(1u64) << 61),
            Term::Integer(1),
        ),
        (
            Term::Integer(1),
            Term::from_bigint(BigInt::from(1u64) << 64),
        ),
        (
            Term::from_bigint(BigInt::from(1u64) << 60),
            Term::Integer(1),
        ),
    ] {
        assert!(matches!(
            new(&mut scheduler, &size, &options).unwrap(),
            BifResult::Raise { reason, .. } if reason == Term::static_atom("system_limit")
        ));
    }
}

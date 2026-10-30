//! Unit tests of the atomic arrays of `src/runtime/atomics.rs`.

use super::*;

/// Return the array of `arrays` at the reference `id`, or panic.
fn at(arrays: &AtomicArrays, id: u64) -> Arc<Array> {
    arrays
        .array(&Term::Reference(Reference(id)))
        .expect("the array exists")
}

#[test]
fn creates_an_array_of_zeroed_atomics() {
    let arrays = AtomicArrays::default();
    assert!(arrays.create(Reference(0), Signedness::Signed, 3));
    let array = at(&arrays, 0);
    assert_eq!(array.signedness(), Signedness::Signed);
    assert_eq!(array.len(), 3);
    assert_eq!(array.get(0), 0);
    assert_eq!(array.get(2), 0);
    // An unsigned array of one atomic.
    assert!(arrays.create(Reference(1), Signedness::Unsigned, 1));
    assert_eq!(at(&arrays, 1).signedness(), Signedness::Unsigned);
}

#[test]
fn reads_only_the_reference_of_an_array() {
    let arrays = AtomicArrays::default();
    arrays.create(Reference(0), Signedness::Signed, 1);
    // A term that is not a reference names no array, and neither does the
    // reference of an array that no call made.
    assert!(arrays.array(&Term::Integer(0)).is_none());
    assert!(arrays.array(&Term::Integer(0)).is_none());
    assert!(arrays.array(&Term::Reference(Reference(1))).is_none());
    assert!(arrays.array(&Term::Reference(Reference(1))).is_none());
    assert!(arrays.array(&Term::Reference(Reference(0))).is_some());
}

#[test]
fn writes_and_adds_with_a_wrap_around_at_64_bits() {
    let arrays = AtomicArrays::default();
    arrays.create(Reference(0), Signedness::Signed, 1);
    let array = arrays.array(&Term::Reference(Reference(0))).unwrap();
    array.set(0, i64::MAX);
    assert_eq!(array.get(0), i64::MAX);
    // `erts_atomic64_add_mb` wraps around at 64 bits, so the largest signed
    // value and one more give the smallest one.
    array.add(0, 1);
    assert_eq!(array.get(0), i64::MIN);
    array.add(0, -1);
    assert_eq!(array.get(0), i64::MAX);
    // An unsigned array holds the same bits: `atomics:get/2` reads them as a
    // non-negative integer.
    arrays.create(Reference(1), Signedness::Unsigned, 1);
    let array = arrays.array(&Term::Reference(Reference(1))).unwrap();
    array.set(0, -1);
    assert_eq!(array.get(0), -1);
}

#[test]
fn reports_the_memory_of_an_array() {
    // The pinned ERTS reports the memory 48, 56, and 64 bytes for the sizes
    // 1, 2, and 3 of an array (`atom_counter_info` of `counters_SUITE`).
    let arrays = AtomicArrays::default();
    for (id, size, memory) in [(0, 1, 48), (1, 2, 56), (2, 3, 64)] {
        arrays.create(Reference(id), Signedness::Signed, size);
        assert_eq!(at(&arrays, id).memory(), memory);
        // Each atomic takes 8 bytes after the header of the array.
        assert_eq!(
            MEMORY_HEADER + i64::try_from(ATOMIC_BYTES).unwrap() * i64::try_from(size).unwrap(),
            memory
        );
    }
}

#[test]
fn refuses_an_array_that_does_not_fit() {
    let arrays = AtomicArrays::default();
    // An array of `usize::MAX` atomics is above the address space, and ERTS
    // reports the failure of the allocation as `system_limit`.
    assert!(!arrays.create(Reference(0), Signedness::Signed, usize::MAX));
    assert!(arrays.array(&Term::Reference(Reference(0))).is_none());
}

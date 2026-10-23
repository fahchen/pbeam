//! The atomic arrays of the node: the shared mutable 64-bit atomics of the
//! `atomics` module and of the `counters` module (`erl_bif_atomics.c`). All
//! processes of a run share one store on the node, and each scheduler thread
//! uses an array with real atomic operations. ERTS holds each array in a magic
//! binary and identifies it by the reference of that binary (`AtomicsRef`,
//! `erl_bif_atomics.c:42-47`, `88`).

use std::collections::HashMap;
use std::sync::atomic::{AtomicI64, Ordering};
use std::sync::{Arc, RwLock};

use crate::term::{Reference, Term};

/// The header of an array in bytes: the magic binary of ERTS
/// (`ERTS_MAGIC_BIN_ORIG_SIZE`, `erl_binary.h:137-140`) and the fields of
/// the C `AtomicsRef` (`AtomicsRef.v`, `erl_bif_atomics.c:42-47`). The
/// bytes of each atomic follow it. The pinned ERTS reports 48, 56, and 64
/// bytes for the sizes 1, 2, and 3.
const MEMORY_HEADER: i64 = 40;

/// The bytes of one atomic of an array (`sizeof(p->v[0])`,
/// `erl_bif_atomics.c:74`).
pub(super) const ATOMIC_BYTES: u64 = 8;

/// One array of atomics, as the C `AtomicsRef` (`erl_bif_atomics.c:42-47`).
#[derive(Debug)]
pub(super) struct Array {
    /// Whether `atomics:get/2` reads an atomic of the array as a signed
    /// 64-bit integer and a write accepts the signed 64-bit range
    /// (`AtomicsRef.is_signed` and `get_value`, `erl_bif_atomics.c:45`,
    /// `112-117`).
    signed: bool,
    /// The bits of each atomic. A write wraps around at 64 bits, as the
    /// `erts_atomic64_*` operations of the C array do. Each operation is
    /// sequentially consistent, as the `_mb` (full barrier) operations of
    /// ERTS are.
    values: Box<[AtomicI64]>,
}

impl Array {
    /// Return whether the array holds signed atomics.
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// let arrays = AtomicArrays::default();
    /// arrays.create(Reference(0), true, 1);
    /// assert!(arrays.array(&Term::Reference(Reference(0))).unwrap().signed());
    /// ```
    pub(super) fn signed(&self) -> bool {
        self.signed
    }

    /// Return the number of atomics of the array, as `AtomicsRef.vlen`.
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// let arrays = AtomicArrays::default();
    /// arrays.create(Reference(0), true, 2);
    /// assert_eq!(arrays.array(&Term::Reference(Reference(0))).unwrap().len(), 2);
    /// ```
    pub(super) fn len(&self) -> usize {
        self.values.len()
    }

    /// Return the bits of the atomic at `position`. The caller checks the
    /// position against [`Array::len`].
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// let arrays = AtomicArrays::default();
    /// arrays.create(Reference(0), true, 1);
    /// let array = arrays.array(&Term::Reference(Reference(0))).unwrap();
    /// assert_eq!(array.get(0), 0);
    /// ```
    pub(super) fn get(&self, position: usize) -> i64 {
        self.values[position].load(Ordering::SeqCst)
    }

    /// Set the atomic at `position` to `bits`, as `atomics:put/3` does with
    /// `erts_atomic64_set_mb` (`erl_bif_atomics.c:158`).
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// let arrays = AtomicArrays::default();
    /// arrays.create(Reference(0), true, 1);
    /// arrays.array(&Term::Reference(Reference(0))).unwrap().set(0, -2);
    /// assert_eq!(arrays.array(&Term::Reference(Reference(0))).unwrap().get(0), -2);
    /// ```
    pub(super) fn set(&self, position: usize, bits: i64) {
        self.values[position].store(bits, Ordering::SeqCst);
    }

    /// Add `bits` to the atomic at `position` and wrap around at 64 bits,
    /// as `atomics:add/3` does with `erts_atomic64_add_mb`
    /// (`erl_bif_atomics.c:183`).
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// let arrays = AtomicArrays::default();
    /// arrays.create(Reference(0), true, 1);
    /// let array = arrays.array(&Term::Reference(Reference(0))).unwrap();
    /// array.add(0, i64::MAX);
    /// array.add(0, 1);
    /// assert_eq!(array.get(0), i64::MIN);
    /// ```
    pub(super) fn add(&self, position: usize, bits: i64) {
        self.values[position].fetch_add(bits, Ordering::SeqCst);
    }

    /// Add `bits` to the atomic at `position` with 64-bit wrap and return
    /// the new bits, as `atomics:add_get/3` does with
    /// `erts_atomic64_add_read_mb` (`erl_bif_atomics.c:197`).
    pub(super) fn add_get(&self, position: usize, bits: i64) -> i64 {
        self.values[position]
            .fetch_add(bits, Ordering::SeqCst)
            .wrapping_add(bits)
    }

    /// Set the atomic at `position` to `bits` and return the old bits, as
    /// `atomics:exchange/3` does with `erts_atomic64_xchg_mb`
    /// (`erl_bif_atomics.c:210`).
    pub(super) fn exchange(&self, position: usize, bits: i64) -> i64 {
        self.values[position].swap(bits, Ordering::SeqCst)
    }

    /// Set the atomic at `position` to `desired` when it holds `expected`,
    /// and return the bits that it held, as `atomics:compare_exchange/4`
    /// does with `erts_atomic64_cmpxchg_mb` (`erl_bif_atomics.c:225`).
    pub(super) fn compare_exchange(&self, position: usize, expected: i64, desired: i64) -> i64 {
        match self.values[position].compare_exchange(
            expected,
            desired,
            Ordering::SeqCst,
            Ordering::SeqCst,
        ) {
            Ok(was) | Err(was) => was,
        }
    }

    /// Return the memory of the array in bytes, as `atomics:info/1` reports
    /// it (`atomics_info_1`, `erl_bif_atomics.c:229-268`).
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// let arrays = AtomicArrays::default();
    /// arrays.create(Reference(0), true, 3);
    /// assert_eq!(arrays.array(&Term::Reference(Reference(0))).unwrap().memory(), 64);
    /// ```
    pub(super) fn memory(&self) -> i64 {
        MEMORY_HEADER + ATOMIC_BYTES as i64 * self.len() as i64
    }
}

/// The atomic arrays of the node, by the reference of the magic binary that
/// ERTS makes for each one (`erts_mk_magic_ref`, `erl_bif_atomics.c:88`).
#[derive(Debug, Default)]
pub(super) struct AtomicArrays {
    /// Each array by its reference. ERTS frees an array when no term
    /// references it (`atomics_destructor`, `erl_bif_atomics.c:49-52`).
    /// PBeam keeps each array until the run ends: see "Atomic arrays" in
    /// `docs/erts-differences.md`.
    /// Each scheduler of a run reads the map, and only
    /// `erts_internal:atomics_new/2` writes it.
    arrays: RwLock<HashMap<Reference, Arc<Array>>>,
}

impl AtomicArrays {
    /// Create the array of `size` atomics with the signedness `signed` for
    /// the reference `id`, as `erts_internal_atomics_new_2` makes it
    /// (`erl_bif_atomics.c:56-89`). Return `false` when PBeam cannot
    /// allocate the array, and the caller then raises `error:system_limit`.
    /// ERTS ends the emulator in that case: `erts_create_magic_binary_x`
    /// calls `erts_alloc_n_enomem` (`erl_binary.h:404-407`), which is
    /// `__decl_noreturn` (`erl_alloc.c:2105-2109`). See "Atomic arrays" in
    /// `docs/erts-differences.md`.
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// let arrays = AtomicArrays::default();
    /// assert!(arrays.create(Reference(0), true, 2));
    /// assert!(arrays.array(&Term::Reference(Reference(0))).is_some());
    /// assert!(!arrays.create(Reference(1), true, usize::MAX));
    /// ```
    pub(super) fn create(&self, id: Reference, signed: bool, size: usize) -> bool {
        let mut values = Vec::new();
        if values.try_reserve_exact(size).is_err() {
            return false;
        }
        values.resize_with(size, || AtomicI64::new(0));
        let array = Arc::new(Array {
            signed,
            values: values.into_boxed_slice(),
        });
        self.arrays
            .write()
            .expect("the atomics lock is not poisoned")
            .insert(id, array);
        true
    }

    /// Return the array of `reference`, or `None` when `reference` is not
    /// the reference of an array. `get_ref` (`erl_bif_atomics.c:91-102`)
    /// finds no array for a term that is not the reference of a magic
    /// binary of the atomics type.
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// let arrays = AtomicArrays::default();
    /// arrays.create(Reference(0), true, 1);
    /// assert!(arrays.array(&Term::Reference(Reference(0))).is_some());
    /// assert!(arrays.array(&Term::Integer(0)).is_none());
    /// ```
    pub(super) fn array(&self, reference: &Term) -> Option<Arc<Array>> {
        let Term::Reference(id) = reference else {
            return None;
        };
        self.arrays
            .read()
            .expect("the atomics lock is not poisoned")
            .get(id)
            .cloned()
    }
}

#[cfg(test)]
#[path = "../../tests/runtime/atomics.rs"]
mod tests;

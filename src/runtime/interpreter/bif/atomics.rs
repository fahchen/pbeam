//! Bodies of the `atomics` BIFs and of the `erts_internal:atomics_new/2`
//! BIF that makes an atomic array, and of the `erts_internal` counters BIFs
//! (`erl_bif_counters.c`). The preloaded `counters` module of the node
//! calls the first in its default `atomics` mode and the second in its
//! `write_concurrency` mode (`erts/preloaded/src/counters.erl:100-247`).

use num_bigint::{BigInt, Sign};
use num_traits::ToPrimitive;

use super::BifResult::Value;
use super::{BeamError, BifResult, ExceptionClass, Scheduler, Term};
use crate::runtime::atomics::{ATOMIC_BYTES, Array};
use crate::term::Map;
use std::sync::Arc;

/// The bit of the encoded options of `atomics:new/2` that selects signed
/// atomics (`?OPT_SIGNED`, `atomics.erl:63`; `OPT_SIGNED`,
/// `erl_bif_atomics.c:54`).
const OPT_SIGNED: u64 = 1;

/// Run `erts_internal:atomics_new(Size, Options)`: make the atomic array
/// and return its reference, as `erts_internal_atomics_new_2`
/// (`erl_bif_atomics.c:56-89`) does. `Options` is the encoded options of
/// `atomics:new/2`, and bit 0 selects the signed range.
///
/// The checks keep the ERTS order, so PBeam raises the same exception for
/// each call:
///
/// 1. A `Size` that is not a non-negative integer raises `badarg`, and a
///    positive integer above the machine word raises `system_limit`
///    (`term_to_UWord`, `big.c:3564-3605`).
/// 2. A `Size` of 0 raises `badarg` (`erl_bif_atomics.c:67-69`).
/// 3. `Options` gives these two exceptions in the same way
///    (`erl_bif_atomics.c:70-72`).
/// 4. A `Size` above the machine word divided by the bytes of an atomic
///    raises `system_limit` (`erl_bif_atomics.c:74-75`).
/// 5. An allocation that fails raises `system_limit`
///    (`erl_bif_atomics.c:77-88`).
///
/// # Examples
///
/// ```rust,ignore
/// // `atomics:new(1, [])` encodes the signed option as 1.
/// let result = new(&mut scheduler, &Term::Integer(1), &Term::Integer(1))?;
/// assert!(matches!(result, BifResult::Value(Term::Reference(_))));
/// ```
///
/// # Errors
///
/// A full reference counter gives the error of [`Scheduler::make_ref`].
pub(super) fn new(
    scheduler: &mut Scheduler,
    size: &Term,
    options: &Term,
) -> Result<BifResult, BeamError> {
    let size = match to_uword(size) {
        Ok(size) => size,
        Err(exception) => return Ok(exception),
    };
    if size == 0 {
        return Ok(BifResult::badarg());
    }
    let options = match to_uword(options) {
        Ok(options) => options,
        Err(exception) => return Ok(exception),
    };
    if size > u64::MAX / ATOMIC_BYTES {
        return Ok(system_limit());
    }
    // PBeam runs on 64-bit hosts, and the check above keeps the size of the
    // array in `usize`.
    let id = scheduler.make_magic_ref()?;
    if !scheduler
        .node()
        .atomics
        .create(id, options & OPT_SIGNED == OPT_SIGNED, size as usize)
    {
        return Ok(system_limit());
    }
    Ok(Value(Term::Reference(id)))
}

/// Run `atomics:get(Ref, Ix)`: return the value of the atomic at the index,
/// as `atomics_get_2` (`erl_bif_atomics.c:162-171`) does. A reference that
/// is not the reference of an array and an index that is not an atomic of
/// the array raise `badarg` (`get_ref_ix`, `erl_bif_atomics.c:104-110`). A
/// signed array gives a signed integer, and an unsigned array gives a
/// non-negative integer (`bld_atomic`, `erl_bif_atomics.c:125-146`).
///
/// # Examples
///
/// ```rust,ignore
/// // `counters:get(Ref, 1)` of an empty array gives 0.
/// assert_eq!(get(&mut scheduler, &reference, &Term::Integer(1))?, BifResult::Value(Term::Integer(0)));
/// ```
///
/// # Errors
///
/// A full atom table gives the error of [`Term::from_bigint`].
#[expect(
    clippy::unnecessary_wraps,
    reason = "BIF entry of the dispatch table in `table.rs`; every BIF returns `Result<BifResult, BeamError>`"
)]
pub(super) fn get(
    scheduler: &mut Scheduler,
    reference: &Term,
    index: &Term,
) -> Result<BifResult, BeamError> {
    let Some(array) = scheduler.node().atomics.array(reference) else {
        return Ok(BifResult::badarg());
    };
    let Some(position) = position(&array, index) else {
        return Ok(BifResult::badarg());
    };
    Ok(Value(value(array.signed(), array.get(position))))
}

/// Run `atomics:put(Ref, Ix, Value)`: write `Value` to the atomic at the
/// index, as `atomics_put_3` (`erl_bif_atomics.c:148-160`) does.
///
/// A reference that is not the reference of an array, an index that is not
/// an atomic of the array, and a value outside the range of the array raise
/// `badarg`: a signed array takes the signed 64-bit range, and an unsigned
/// array the unsigned one (`get_ref_ix` and `get_value`,
/// `erl_bif_atomics.c:104-117`).
///
/// # Examples
///
/// ```rust,ignore
/// // `counters:put(Ref, 1, -1)` on a signed array gives ok.
/// let result = put(&mut scheduler, &reference, &Term::Integer(1), &Term::Integer(-1))?;
/// assert!(matches!(result, BifResult::Value(Term::Atom(_))));
/// ```
#[expect(
    clippy::unnecessary_wraps,
    reason = "BIF entry of the dispatch table in `table.rs`; every BIF returns `Result<BifResult, BeamError>`"
)]
pub(super) fn put(
    scheduler: &mut Scheduler,
    reference: &Term,
    index: &Term,
    value: &Term,
) -> Result<BifResult, BeamError> {
    let Some(array) = scheduler.node().atomics.array(reference) else {
        return Ok(BifResult::badarg());
    };
    let Some(position) = position(&array, index) else {
        return Ok(BifResult::badarg());
    };
    let Some(bits) = value_bits(array.signed(), value) else {
        return Ok(BifResult::badarg());
    };
    array.set(position, bits);
    Ok(Value(Term::static_atom("ok")))
}

/// Run `atomics:add(Ref, Ix, Incr)`: add `Incr` to the atomic at the index,
/// as `atomics_add_3` (`erl_bif_atomics.c:173-185`) does. The result wraps
/// around at 64 bits.
///
/// A reference that is not the reference of an array, an index that is not
/// an atomic of the array, and an increment outside the signed and the
/// unsigned 64-bit ranges raise `badarg` (`get_ref_ix` and `get_incr`,
/// `erl_bif_atomics.c:104-110`, `119-123`).
///
/// # Examples
///
/// ```rust,ignore
/// // `counters:add(Ref, 1, 1)` gives ok on each signed and unsigned array.
/// let result = add(&mut scheduler, &reference, &Term::Integer(1), &Term::Integer(1))?;
/// assert!(matches!(result, BifResult::Value(Term::Atom(_))));
/// ```
#[expect(
    clippy::unnecessary_wraps,
    reason = "BIF entry of the dispatch table in `table.rs`; every BIF returns `Result<BifResult, BeamError>`"
)]
pub(super) fn add(
    scheduler: &mut Scheduler,
    reference: &Term,
    index: &Term,
    increment: &Term,
) -> Result<BifResult, BeamError> {
    let Some(array) = scheduler.node().atomics.array(reference) else {
        return Ok(BifResult::badarg());
    };
    let Some(position) = position(&array, index) else {
        return Ok(BifResult::badarg());
    };
    let Some(bits) = increment_bits(increment) else {
        return Ok(BifResult::badarg());
    };
    array.add(position, bits);
    Ok(Value(Term::static_atom("ok")))
}

/// Run `atomics:add_get(Ref, Ix, Incr)`: add `Incr` to the atomic at the
/// index with 64-bit wrap and return the new value, as `atomics_add_get_3`
/// (`erl_bif_atomics.c:187-198`) does. The checks and the `badarg` cases
/// are those of [`add`], and the value has the form of [`get`]
/// (`bld_atomic`, `erl_bif_atomics.c:125-146`).
#[expect(
    clippy::unnecessary_wraps,
    reason = "BIF entry of the dispatch table in `table.rs`; every BIF returns `Result<BifResult, BeamError>`"
)]
pub(super) fn add_get(
    scheduler: &mut Scheduler,
    reference: &Term,
    index: &Term,
    increment: &Term,
) -> Result<BifResult, BeamError> {
    let Some(array) = scheduler.node().atomics.array(reference) else {
        return Ok(BifResult::badarg());
    };
    let Some(position) = position(&array, index) else {
        return Ok(BifResult::badarg());
    };
    let Some(bits) = increment_bits(increment) else {
        return Ok(BifResult::badarg());
    };
    Ok(Value(value(array.signed(), array.add_get(position, bits))))
}

/// Run `atomics:exchange(Ref, Ix, Desired)`: write `Desired` to the atomic
/// at the index and return the old value, as `atomics_exchange_3`
/// (`erl_bif_atomics.c:200-212`) does. The checks and the `badarg` cases
/// are those of [`put`], and the value has the form of [`get`].
#[expect(
    clippy::unnecessary_wraps,
    reason = "BIF entry of the dispatch table in `table.rs`; every BIF returns `Result<BifResult, BeamError>`"
)]
pub(super) fn exchange(
    scheduler: &mut Scheduler,
    reference: &Term,
    index: &Term,
    desired: &Term,
) -> Result<BifResult, BeamError> {
    let Some(array) = scheduler.node().atomics.array(reference) else {
        return Ok(BifResult::badarg());
    };
    let Some(position) = position(&array, index) else {
        return Ok(BifResult::badarg());
    };
    let Some(bits) = value_bits(array.signed(), desired) else {
        return Ok(BifResult::badarg());
    };
    Ok(Value(value(array.signed(), array.exchange(position, bits))))
}

/// Run `atomics:compare_exchange(Ref, Ix, Expected, Desired)`: write
/// `Desired` to the atomic at the index when it holds `Expected`, as
/// `atomics_compare_exchange_4` (`erl_bif_atomics.c:214-227`) does. A
/// write gives `ok`, and a miss gives the value of the atomic in the form
/// of [`get`]. `Expected` and `Desired` take the range of the array, as in
/// [`put`], and each failed check raises `badarg`.
#[expect(
    clippy::unnecessary_wraps,
    reason = "BIF entry of the dispatch table in `table.rs`; every BIF returns `Result<BifResult, BeamError>`"
)]
pub(super) fn compare_exchange(
    scheduler: &mut Scheduler,
    reference: &Term,
    index: &Term,
    expected: &Term,
    desired: &Term,
) -> Result<BifResult, BeamError> {
    let Some(array) = scheduler.node().atomics.array(reference) else {
        return Ok(BifResult::badarg());
    };
    let Some(position) = position(&array, index) else {
        return Ok(BifResult::badarg());
    };
    let (Some(expected), Some(desired)) = (
        value_bits(array.signed(), expected),
        value_bits(array.signed(), desired),
    ) else {
        return Ok(BifResult::badarg());
    };
    let was = array.compare_exchange(position, expected, desired);
    if was == expected {
        return Ok(Value(Term::static_atom("ok")));
    }
    Ok(Value(value(array.signed(), was)))
}

/// Run `atomics:info(Ref)`: return the information map of the array, as
/// `atomics_info_1` (`erl_bif_atomics.c:229-268`) does. The map holds the
/// keys `max`, `min`, `size`, and `memory`, and `memory` is the memory of
/// the array in bytes. A reference that is not the reference of an array
/// raises `badarg`.
///
/// # Examples
///
/// ```rust,ignore
/// // `counters:info(Ref)` of one signed atomic holds
/// // `#{max => 9223372036854775807, memory => 48, min => -9223372036854775808, size => 1}`.
/// assert!(matches!(info(&mut scheduler, &reference)?, BifResult::Value(Term::Map(_))));
/// ```
#[expect(
    clippy::unnecessary_wraps,
    reason = "BIF entry of the dispatch table in `table.rs`; every BIF returns `Result<BifResult, BeamError>`"
)]
pub(super) fn info(scheduler: &mut Scheduler, reference: &Term) -> Result<BifResult, BeamError> {
    let Some(array) = scheduler.node().atomics.array(reference) else {
        return Ok(BifResult::badarg());
    };
    let (min, max) = if array.signed() {
        (Term::Integer(i64::MIN), Term::Integer(i64::MAX))
    } else {
        (Term::Integer(0), Term::from_bigint(BigInt::from(u64::MAX)))
    };
    let pairs = vec![
        (Term::static_atom("max"), max),
        (Term::static_atom("memory"), Term::Integer(array.memory())),
        (Term::static_atom("min"), min),
        (Term::static_atom("size"), Term::Integer(array.len() as i64)),
    ];
    // ponytail: each key is a static atom, so `from_pairs` cannot fail.
    let map = Map::from_pairs(pairs, &scheduler.atoms).expect("static atom keys fit each map form");
    Ok(Value(Term::Map(map)))
}

/// The bytes of an ERTS cache line, `ERTS_CACHE_LINE_SIZE` on the 64-bit
/// hosts of PBeam. A `write_concurrency` array keeps its atomics in cache
/// lines (`erl_bif_counters.c:52-64`).
const CACHE_LINE_BYTES: u64 = 64;

/// The bytes before the cache lines of a `write_concurrency` array in its
/// `memory`: the magic binary header that `orig_size` counts
/// (`ERTS_MAGIC_BIN_ORIG_SIZE`, `erl_binary.h:137-140`) and the `arity`
/// field of `CountersRef` (`erl_bif_counters.c:54-64`). The pinned ERTS
/// with 10 schedulers reports 736 bytes for 1 counter and 1440 for 9.
const COUNTERS_HEADER_BYTES: u64 = 32;

/// Run `erts_internal:counters_new(Size)`: make a `write_concurrency`
/// counter array of `Size` counters and return its reference, as
/// `erts_internal_counters_new_1` (`erl_bif_counters.c:77-113`) does.
///
/// ERTS keeps one atomic for each scheduler and one base atomic for each
/// counter, and a read gives their sum with 64-bit wrap. PBeam keeps one
/// signed atomic for each counter, which gives the same values: each add
/// and put wraps at 64 bits in the same way.
///
/// The checks keep the ERTS order:
///
/// 1. A `Size` that is not a non-negative integer raises `badarg`, and a
///    positive integer above the machine word raises `system_limit`
///    (`erl_bif_counters.c:85-87`).
/// 2. A `Size` of 0 raises `badarg` (`erl_bif_counters.c:88-90`).
/// 3. A `Size` above the machine word divided by 16 bytes for each atomic
///    of a counter raises `system_limit` (`erl_bif_counters.c:92-93`).
/// 4. An allocation that fails raises `system_limit`, where ERTS ends the
///    emulator (see "Atomic arrays" in `docs/erts-differences.md`).
///
/// # Examples
///
/// ```rust,ignore
/// let result = counters_new(&mut scheduler, &Term::Integer(1))?;
/// assert!(matches!(result, BifResult::Value(Term::Reference(_))));
/// ```
///
/// # Errors
///
/// A full reference counter gives the error of [`Scheduler::make_ref`].
pub(super) fn counters_new(scheduler: &mut Scheduler, size: &Term) -> Result<BifResult, BeamError> {
    let size = match to_uword(size) {
        Ok(size) => size,
        Err(exception) => return Ok(exception),
    };
    if size == 0 {
        return Ok(BifResult::badarg());
    }
    if size > u64::MAX / (ATOMIC_BYTES * 2 * atomics_per_counter(scheduler)) {
        return Ok(system_limit());
    }
    let id = scheduler.make_ref()?;
    if !scheduler.node().counters.create(id, true, size as usize) {
        return Ok(system_limit());
    }
    Ok(Value(Term::Reference(id)))
}

/// Run `erts_internal:counters_get(Ref, Ix)`: return the signed value of
/// the counter, as `erts_internal_counters_get_2`
/// (`erl_bif_counters.c:180-195`) does. A reference that is not the
/// reference of a counter array and an index outside the array raise
/// `badarg` (`get_ref_cnt`, `erl_bif_counters.c:128-143`).
///
/// # Examples
///
/// ```rust,ignore
/// assert_eq!(counters_get(&mut scheduler, &reference, &Term::Integer(1))?, BifResult::Value(Term::Integer(0)));
/// ```
#[expect(
    clippy::unnecessary_wraps,
    reason = "BIF entry of the dispatch table in `table.rs`; every BIF returns `Result<BifResult, BeamError>`"
)]
pub(super) fn counters_get(
    scheduler: &mut Scheduler,
    reference: &Term,
    index: &Term,
) -> Result<BifResult, BeamError> {
    let Some((array, position)) = counter(scheduler, reference, index) else {
        return Ok(BifResult::badarg());
    };
    Ok(Value(Term::Integer(array.get(position))))
}

/// Run `erts_internal:counters_add(Ref, Ix, Incr)`: add `Incr` to the
/// counter with 64-bit wrap, as `erts_internal_counters_add_3`
/// (`erl_bif_counters.c:197-210`) does. A bad reference, a bad index, and
/// an increment outside the signed and the unsigned 64-bit ranges raise
/// `badarg` (`get_incr`, `erl_bif_counters.c:162-166`).
///
/// # Examples
///
/// ```rust,ignore
/// let result = counters_add(&mut scheduler, &reference, &Term::Integer(1), &Term::Integer(1))?;
/// assert!(matches!(result, BifResult::Value(Term::Atom(_))));
/// ```
#[expect(
    clippy::unnecessary_wraps,
    reason = "BIF entry of the dispatch table in `table.rs`; every BIF returns `Result<BifResult, BeamError>`"
)]
pub(super) fn counters_add(
    scheduler: &mut Scheduler,
    reference: &Term,
    index: &Term,
    increment: &Term,
) -> Result<BifResult, BeamError> {
    let Some((array, position)) = counter(scheduler, reference, index) else {
        return Ok(BifResult::badarg());
    };
    let Some(bits) = increment_bits(increment) else {
        return Ok(BifResult::badarg());
    };
    array.add(position, bits);
    Ok(Value(Term::static_atom("ok")))
}

/// Run `erts_internal:counters_put(Ref, Ix, Value)`: write `Value` to the
/// counter, as `erts_internal_counters_put_3`
/// (`erl_bif_counters.c:212-236`) does. A bad reference, a bad index, and
/// a value outside the signed 64-bit range raise `badarg`
/// (`erl_bif_counters.c:221-224`).
///
/// # Examples
///
/// ```rust,ignore
/// let result = counters_put(&mut scheduler, &reference, &Term::Integer(1), &Term::Integer(-1))?;
/// assert!(matches!(result, BifResult::Value(Term::Atom(_))));
/// ```
#[expect(
    clippy::unnecessary_wraps,
    reason = "BIF entry of the dispatch table in `table.rs`; every BIF returns `Result<BifResult, BeamError>`"
)]
pub(super) fn counters_put(
    scheduler: &mut Scheduler,
    reference: &Term,
    index: &Term,
    value: &Term,
) -> Result<BifResult, BeamError> {
    let Some((array, position)) = counter(scheduler, reference, index) else {
        return Ok(BifResult::badarg());
    };
    let Some(bits) = signed_bits(value) else {
        return Ok(BifResult::badarg());
    };
    array.set(position, bits);
    Ok(Value(Term::static_atom("ok")))
}

/// Run `erts_internal:counters_info(Ref)`: return the map with the keys
/// `memory` and `size`, as `erts_internal_counters_info_1`
/// (`erl_bif_counters.c:238-259`) does. `memory` is the size of the ERTS
/// magic binary: the header and one cache line of 8 atomics for each
/// scheduler and the base value (`erl_bif_counters.c:95-96`). A reference
/// that is not the reference of a counter array raises `badarg`.
///
/// # Examples
///
/// ```rust,ignore
/// // One counter on one scheduler holds `#{memory => 160, size => 1}`.
/// assert!(matches!(counters_info(&mut scheduler, &reference)?, BifResult::Value(Term::Map(_))));
/// ```
#[expect(
    clippy::unnecessary_wraps,
    reason = "BIF entry of the dispatch table in `table.rs`; every BIF returns `Result<BifResult, BeamError>`"
)]
pub(super) fn counters_info(
    scheduler: &mut Scheduler,
    reference: &Term,
) -> Result<BifResult, BeamError> {
    let Some(array) = scheduler.node().counters.array(reference) else {
        return Ok(BifResult::badarg());
    };
    let size = array.len() as u64;
    let per_line = CACHE_LINE_BYTES / ATOMIC_BYTES;
    let cache_lines = atomics_per_counter(scheduler) * size.div_ceil(per_line);
    let memory = COUNTERS_HEADER_BYTES + cache_lines * CACHE_LINE_BYTES;
    let pairs = vec![
        (
            Term::static_atom("memory"),
            Term::from_bigint(BigInt::from(memory)),
        ),
        (
            Term::static_atom("size"),
            Term::from_bigint(BigInt::from(size)),
        ),
    ];
    // ponytail: each key is a static atom, so `from_pairs` cannot fail.
    let map = Map::from_pairs(pairs, &scheduler.atoms).expect("static atom keys fit each map form");
    Ok(Value(Term::Map(map)))
}

/// Return the atomics of one counter in ERTS: one for each scheduler and
/// one for the base value (`ATOMICS_PER_COUNTER`, `erl_bif_counters.c:50`).
fn atomics_per_counter(scheduler: &Scheduler) -> u64 {
    scheduler.scheduler_count() as u64 + 1
}

/// Return the counter array of `reference` and the position that `index`
/// names, or `None` when either is not valid (`get_ref_cnt`,
/// `erl_bif_counters.c:128-143`).
fn counter(scheduler: &Scheduler, reference: &Term, index: &Term) -> Option<(Arc<Array>, usize)> {
    let array = scheduler.node().counters.array(reference)?;
    let position = position(&array, index)?;
    Some((array, position))
}

/// Return the machine word of `term`, as ERTS `term_to_UWord` gives it
/// (`big.c:3564-3605`, which is `term_to_Uint` on a 64-bit machine). The
/// error is the exception of the failure: `badarg` for a term that is not a
/// non-negative integer, and `system_limit` for a positive integer above
/// the machine word.
///
/// # Examples
///
/// ```rust,ignore
/// assert_eq!(to_uword(&Term::Integer(2)), Ok(2));
/// assert!(matches!(to_uword(&Term::Integer(-1)), Err(BifResult::Raise { .. })));
/// ```
fn to_uword(term: &Term) -> Result<u64, BifResult> {
    match term {
        Term::Integer(value) => u64::try_from(*value).map_err(|_| BifResult::badarg()),
        // `big_sign` (`big.c:3584-3587`): a negative integer of any size
        // raises `badarg`, and a positive integer above the machine word
        // raises `system_limit` (`big.c:3588-3591`).
        Term::BigInt(value) if value.sign() == Sign::Minus => Err(BifResult::badarg()),
        Term::BigInt(value) => value.to_u64().ok_or_else(system_limit),
        _ => Err(BifResult::badarg()),
    }
}

/// Return the `error:system_limit` exception, which ERTS raises with
/// `BIF_ERROR(p, SYSTEM_LIMIT)` for a size above the limit of the machine.
///
/// # Examples
///
/// ```rust,ignore
/// assert!(matches!(system_limit(), BifResult::Raise { .. }));
/// ```
fn system_limit() -> BifResult {
    BifResult::Raise {
        class: ExceptionClass::Error,
        reason: Term::static_atom("system_limit"),
    }
}

/// Return the position of the atomic that `index` names, or `None` when the
/// index is not an atomic of `array`.
///
/// `get_ref_ix` (`erl_bif_atomics.c:104-110`) makes the machine word of the
/// term, subtracts one, and checks the result against the size. ERTS raises
/// `badarg` for each failure, so the reason of [`to_uword`] does not show.
///
/// # Examples
///
/// ```rust,ignore
/// // The index 2 of an array of two atomics is its last position, 1.
/// assert_eq!(position(&array, &Term::Integer(2)), Some(1));
/// assert_eq!(position(&array, &Term::Integer(0)), None);
/// ```
fn position(array: &Array, index: &Term) -> Option<usize> {
    let index = to_uword(index).ok()?.checked_sub(1)?;
    (index < array.len() as u64).then_some(index as usize)
}

/// Return the bits of the value `term` for an array, as `get_value`
/// (`erl_bif_atomics.c:112-117`) reads it: a signed array takes the signed
/// 64-bit range, and an unsigned array the unsigned one. `None` gives
/// `badarg`.
///
/// # Examples
///
/// ```rust,ignore
/// assert_eq!(value_bits(true, &Term::Integer(-1)), Some(-1));
/// assert_eq!(value_bits(false, &Term::Integer(-1)), None);
/// ```
fn value_bits(signed: bool, term: &Term) -> Option<i64> {
    if signed {
        signed_bits(term)
    } else {
        unsigned_bits(term)
    }
}

/// Return the bits of the increment `term`, as `get_incr`
/// (`erl_bif_atomics.c:119-123`) reads it: the signed range or the unsigned
/// range. `None` gives `badarg`.
///
/// # Examples
///
/// ```rust,ignore
/// assert_eq!(increment_bits(&Term::Integer(-1)), Some(-1));
/// assert_eq!(increment_bits(&Term::static_atom("x")), None);
/// ```
fn increment_bits(term: &Term) -> Option<i64> {
    signed_bits(term).or_else(|| unsigned_bits(term))
}

/// Return the signed 64-bit value of `term`, as `term_to_Sint64`
/// (`big.c:3696-3730`) gives it: a signed integer of the range fits, and
/// each other term fails.
///
/// # Examples
///
/// ```rust,ignore
/// assert_eq!(signed_bits(&Term::Integer(i64::MIN)), Some(i64::MIN));
/// assert_eq!(signed_bits(&Term::static_atom("x")), None);
/// ```
fn signed_bits(term: &Term) -> Option<i64> {
    match term {
        Term::Integer(value) => Some(*value),
        _ => None,
    }
}

/// Return the bits of the unsigned 64-bit value of `term`, as
/// `term_to_Uint64` (`big.c:3613-3644`) gives it: a non-negative integer of
/// the range fits, and each other term fails.
///
/// # Examples
///
/// ```rust,ignore
/// assert_eq!(unsigned_bits(&Term::Integer(1)), Some(1));
/// assert_eq!(unsigned_bits(&Term::Integer(-1)), None);
/// ```
fn unsigned_bits(term: &Term) -> Option<i64> {
    match term {
        Term::Integer(value) => u64::try_from(*value).ok().map(|value| value as i64),
        Term::BigInt(value) => value.to_u64().map(|value| value as i64),
        _ => None,
    }
}

/// Return the term of the bits of an atomic, as `bld_atomic`
/// (`erl_bif_atomics.c:125-146`) builds it: a signed array gives the bits
/// as a signed integer, and an unsigned array gives them as a non-negative
/// integer.
///
/// # Examples
///
/// ```rust,ignore
/// assert_eq!(value(true, -1), Term::Integer(-1));
/// assert_eq!(value(false, -1), Term::from_bigint(BigInt::from(u64::MAX)));
/// ```
fn value(signed: bool, bits: i64) -> Term {
    if signed {
        Term::Integer(bits)
    } else {
        Term::from_bigint(BigInt::from(bits as u64))
    }
}

#[cfg(test)]
#[path = "../../../../tests/runtime/interpreter/bif/atomics.rs"]
mod tests;

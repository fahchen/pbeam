//! The comparison cost and the allocation count of `Term`, the measurement
//! of the pair-stack candidate of `docs/data-structure-audit.md` section 4.2.
//!
//! The audit measured `Term::eq` of two integers at 24.692 ns, of two atoms
//! at 25.831 ns, and of two tuples that share one array at 22.915 ns, against
//! 0.317 ns for a plain integer comparison. The comparison allocated a pair
//! stack for every call. This module holds the same rows, so a reader can
//! compare the number of the branch with the number of the audit.
//!
//! Every printed line of the cost report is the minimum of [`RUNS`] timed
//! runs of one operation, and every result passes through [`black_box`]. The
//! allocation report counts the allocations of one comparison with the
//! global allocator of the test binary, as `tests/integration/drop_allocations.rs`
//! does. The run is
//! `cargo test --locked --release --lib -- data_structure_costs --
//! --nocapture --test-threads=1`.

use std::{alloc::System, cell::Cell, hint::black_box, time::Instant};

use super::{Map, Term};

/// The number of timed runs of one operation. The report gives the minimum
/// of the runs, as the audit does.
const RUNS: usize = 9;

/// The number of operations of one timed run of a cheap operation.
const OPERATIONS: usize = 100_000;

/// The number of operations of one timed run of a comparison of two tuples
/// that share one array. The audit uses this count for the same row.
const SHARED_OPERATIONS: usize = 200;

/// The number of allocations that one comparison pays when its work stack
/// overflows the inline array: the one reserve of the heap vector.
const SPILL_ALLOCATIONS: usize = 1;

/// The allocator of the process, with a count of the allocations of each
/// thread.
struct CountingAllocator;

thread_local! {
    /// The number of allocations of the current thread. The count is for each
    /// thread, so a thread of the harness or of the runtime that allocates
    /// beside the measured comparison changes no number of this test. The
    /// `Cell` has a constant value and no destructor, so the allocator reads
    /// it without an allocation of its own.
    static ALLOCATIONS: Cell<usize> = const { Cell::new(0) };
}

// SAFETY: the allocator forwards each operation to `System`, which is a
// valid allocator, and it only adds a count of the current thread.
#[global_allocator]
static ALLOCATOR: CountingAllocator = CountingAllocator;

unsafe impl std::alloc::GlobalAlloc for CountingAllocator {
    unsafe fn alloc(&self, layout: std::alloc::Layout) -> *mut u8 {
        count_one();
        // SAFETY: the caller gives a valid layout, as `GlobalAlloc` requires.
        unsafe { System.alloc(layout) }
    }

    unsafe fn dealloc(&self, pointer: *mut u8, layout: std::alloc::Layout) {
        // SAFETY: the pointer and the layout come from `alloc` of `System`.
        unsafe { System.dealloc(pointer, layout) }
    }

    unsafe fn realloc(&self, pointer: *mut u8, layout: std::alloc::Layout, size: usize) -> *mut u8 {
        count_one();
        // SAFETY: the pointer and the layout come from `alloc` of `System`.
        unsafe { System.realloc(pointer, layout, size) }
    }
}

/// Add one to the allocation count of the current thread.
///
/// # Examples
///
/// ```rust,ignore
/// let before = allocations();
/// count_one();
/// assert_eq!(allocations(), before + 1);
/// ```
fn count_one() {
    ALLOCATIONS.with(|count| count.set(count.get().wrapping_add(1)));
}

/// Return the number of allocations of the current thread.
///
/// # Examples
///
/// ```rust,ignore
/// assert!(allocations() < usize::MAX);
/// ```
fn allocations() -> usize {
    ALLOCATIONS.with(Cell::get)
}

/// Print one measured line: the name of the operation and the nanosecond for
/// each operation.
fn report(name: &str, ns: f64) {
    println!("COST {name} {ns:.3} ns");
}

/// Print one measured line for each shape: the allocations of one comparison
/// of two terms of that shape, against the allocations that the branch
/// claims. The check makes a change of a count visible, and it catches a work
/// stack that keeps a pair after the comparison reads it. Every line prints
/// before a check fails, so one run reports every row.
fn report_allocations(rows: &[(&str, usize, usize)]) {
    for (name, count, _) in rows {
        println!("ALLOC {name} {count}");
    }
    for (name, count, expected) in rows {
        assert_eq!(
            count, expected,
            "{name} allocates {count} times rather than {expected} times"
        );
    }
}

/// Return the nanoseconds for each operation of `body`, as the minimum of
/// [`RUNS`] timed runs. The caller performs exactly `operations` operations.
fn measure(operations: usize, mut body: impl FnMut() -> u64) -> f64 {
    let mut best = f64::INFINITY;
    for _ in 0..RUNS {
        let start = Instant::now();
        let result = body();
        let elapsed = start.elapsed().as_nanos() as f64;
        black_box(result);
        best = best.min(elapsed / operations as f64);
    }
    black_box(best)
}

/// Compare `left` and `right` and return the allocations of the comparison.
/// Both terms stay alive across the comparison, so the count holds the
/// comparison only.
///
/// # Examples
///
/// ```rust,ignore
/// assert_eq!(compared(&Term::Integer(1), &Term::Integer(1)), 0);
/// ```
fn compared(left: &Term, right: &Term) -> usize {
    let start = allocations();
    let equal = black_box(left) == black_box(right);
    let count = allocations() - start;
    black_box(equal);
    count
}

/// Return a tuple of `len` integers.
///
/// # Examples
///
/// ```rust,ignore
/// assert!(matches!(tuple_of(1), Term::Tuple(_)));
/// ```
fn tuple_of(len: usize) -> Term {
    Term::tuple((0..len).map(|value| Term::Integer(value as i64)).collect())
}

/// Return a tuple of `len` elements that holds one nested tuple of the same
/// shape, `depth` times. The element at each level is the nested tuple, so
/// the walk of the comparison holds one pair for each level.
///
/// # Examples
///
/// ```rust,ignore
/// assert!(matches!(nested_tuple(2, 2), Term::Tuple(_)));
/// ```
fn nested_tuple(depth: usize, len: usize) -> Term {
    (0..depth).fold(Term::Integer(0), |inner, _| {
        let mut elements = vec![Term::Integer(0); len];
        elements[0] = inner;
        Term::tuple(elements)
    })
}

/// Return a list of `len` integers.
///
/// # Examples
///
/// ```rust,ignore
/// assert!(matches!(list_of(1), Term::Cons(_)));
/// ```
fn list_of(len: i64) -> Term {
    Term::list_from((0..len).map(Term::Integer))
}

/// Return a map of `keys` integer pairs.
///
/// # Examples
///
/// ```rust,ignore
/// assert!(matches!(map_of(1), Term::Map(_)));
/// ```
fn map_of(keys: i64) -> Term {
    Term::Map(
        Map::from_pairs(
            (0..keys)
                .map(|key| (Term::Integer(key), Term::Integer(key)))
                .collect(),
        )
        .unwrap(),
    )
}

/// Print the allocation count of one comparison of each shape of the report:
/// two terms that share one storage, two terms with equal content and
/// different storage, one deep term, and one long list.
#[test]
fn reports_the_allocation_count_of_one_comparison() {
    let shared_tuple = tuple_of(1000);
    let equal_tuple = tuple_of(1000);
    let deep = nested_tuple(200, 1);
    let deep_copy = nested_tuple(200, 1);
    let list_1000 = list_of(1000);
    let list_1000_copy = list_of(1000);
    let deep_list = list_of(1_000_000);
    let deep_list_copy = list_of(1_000_000);
    let flatmap_32 = map_of(32);
    let flatmap_32_copy = map_of(32);
    let flatmap_4 = map_of(4);
    let flatmap_4_copy = map_of(4);
    let integer = Term::Integer(1);
    let atom = Term::static_atom("ok");
    let tuple_3 = tuple_of(3);
    let tuple_3_copy = tuple_of(3);
    let list_3 = list_of(3);
    let list_3_copy = list_of(3);
    let nested_1 = nested_tuple(1, 2);
    let nested_1_copy = nested_tuple(1, 2);
    let nested_2 = nested_tuple(2, 2);
    let nested_2_copy = nested_tuple(2, 2);
    let nested_3 = nested_tuple(3, 2);
    let nested_3_copy = nested_tuple(3, 2);
    let nested_4 = nested_tuple(4, 2);
    let nested_4_copy = nested_tuple(4, 2);

    report_allocations(&[
        ("term_eq_integer", compared(&integer, &integer), 0),
        ("term_eq_atom", compared(&atom, &atom), 0),
        // The comparison of two tuples with equal content holds one pending
        // pair for each element. The first eight pairs stay in the inline
        // array, and the ninth pair spills them, so the comparison pays one
        // allocation for the whole tuple.
        (
            "term_eq_tuple_3_equal",
            compared(&tuple_3, &tuple_3_copy),
            0,
        ),
        ("term_eq_list_3_equal", compared(&list_3, &list_3_copy), 0),
        (
            "term_eq_tuple_1000_shared",
            compared(&shared_tuple, &shared_tuple),
            0,
        ),
        (
            "term_eq_tuple_1000_equal",
            compared(&shared_tuple, &equal_tuple),
            SPILL_ALLOCATIONS,
        ),
        (
            "term_eq_list_1000_equal",
            compared(&list_1000, &list_1000_copy),
            0,
        ),
        // A map holds one pending pair for each key and one for each value.
        // A flatmap of four keys holds eight pairs, which the inline array
        // keeps, and a flatmap of 32 keys holds 64 pairs, which spills once.
        // The iterator of a hashmap allocates its own node stack
        // (`src/term/map.rs:504-529`), so a hashmap row cannot isolate the
        // work stack and this report measures a flatmap only.
        (
            "term_eq_flatmap_4_equal",
            compared(&flatmap_4, &flatmap_4_copy),
            0,
        ),
        (
            "term_eq_flatmap_32_equal",
            compared(&flatmap_32, &flatmap_32_copy),
            SPILL_ALLOCATIONS,
        ),
        (
            "term_eq_list_1000000_equal",
            compared(&deep_list, &deep_list_copy),
            0,
        ),
        (
            "term_eq_nested_tuple_1",
            compared(&nested_1, &nested_1_copy),
            0,
        ),
        (
            "term_eq_nested_tuple_2",
            compared(&nested_2, &nested_2_copy),
            0,
        ),
        (
            "term_eq_nested_tuple_3",
            compared(&nested_3, &nested_3_copy),
            0,
        ),
        (
            "term_eq_nested_tuple_4",
            compared(&nested_4, &nested_4_copy),
            0,
        ),
        ("term_eq_nested_tuple_200", compared(&deep, &deep_copy), 0),
    ]);
}

/// Print the cost of the comparison rows of the audit, and of the compound
/// shapes that the removal of the pair stack must keep.
#[test]
fn prints_the_comparison_cost() {
    let tuple_1000 = tuple_of(1000);
    let other_1000 = tuple_of(1000);
    let left_map = map_of(1000);
    let right_map = map_of(1000);
    let left_list = list_of(1000);
    let right_list = list_of(1000);
    let left_tuple = tuple_of(3);
    let right_tuple = tuple_of(3);
    let left_list_3 = list_of(3);
    let right_list_3 = list_of(3);

    report(
        "compare_integer_direct",
        measure(OPERATIONS, || {
            let left = 1i64;
            let right = 1i64;
            let mut hits = 0u64;
            for _ in 0..OPERATIONS {
                if black_box(left) == black_box(right) {
                    hits += 1;
                }
            }
            hits
        }),
    );
    report(
        "term_eq_integer",
        measure(OPERATIONS, || {
            let mut hits = 0u64;
            for _ in 0..OPERATIONS {
                if black_box(Term::Integer(1)) == black_box(Term::Integer(1)) {
                    hits += 1;
                }
            }
            hits
        }),
    );
    report(
        "term_eq_atom",
        measure(OPERATIONS, || {
            let left = Term::static_atom("ok");
            let right = Term::static_atom("ok");
            let mut hits = 0u64;
            for _ in 0..OPERATIONS {
                if black_box(&left) == black_box(&right) {
                    hits += 1;
                }
            }
            hits
        }),
    );
    report(
        "term_eq_tuple_1000_shared",
        measure(SHARED_OPERATIONS, || {
            let mut hits = 0u64;
            for _ in 0..SHARED_OPERATIONS {
                if black_box(&tuple_1000) == black_box(&tuple_1000) {
                    hits += 1;
                }
            }
            hits
        }),
    );
    report(
        "term_eq_tuple_1000_equal",
        measure(100, || {
            let mut hits = 0u64;
            for _ in 0..100 {
                if black_box(&tuple_1000) == black_box(&other_1000) {
                    hits += 1;
                }
            }
            hits
        }),
    );
    report(
        "term_eq_tuple_3_equal",
        measure(OPERATIONS, || {
            let mut hits = 0u64;
            for _ in 0..OPERATIONS {
                if black_box(&left_tuple) == black_box(&right_tuple) {
                    hits += 1;
                }
            }
            hits
        }),
    );
    report(
        "term_eq_list_1000_equal",
        measure(100, || {
            let mut hits = 0u64;
            for _ in 0..100 {
                if black_box(&left_list) == black_box(&right_list) {
                    hits += 1;
                }
            }
            hits
        }),
    );
    report(
        "term_eq_list_3_equal",
        measure(OPERATIONS, || {
            let mut hits = 0u64;
            for _ in 0..OPERATIONS {
                if black_box(&left_list_3) == black_box(&right_list_3) {
                    hits += 1;
                }
            }
            hits
        }),
    );
    report(
        "term_eq_map_1000_equal",
        measure(100, || {
            let mut hits = 0u64;
            for _ in 0..100 {
                if black_box(&left_map) == black_box(&right_map) {
                    hits += 1;
                }
            }
            hits
        }),
    );
}

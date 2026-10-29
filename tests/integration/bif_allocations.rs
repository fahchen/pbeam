//! The allocation count of one BIF call of `docs/interpreter-dispatch-plan.md`.
//! Step 5 of the plan gives the pure leaf BIFs borrowed arguments, so the
//! call path builds no argument `Vec<Term>`.
//!
//! The test runs the fixture loop of `fixtures/dispatch/dispatch_probe.erl`
//! twice, at two lengths, and divides the difference by the length
//! difference. The constant cost of one run cancels, so the result is the
//! allocation count of one iteration.

use std::{alloc::System, cell::Cell, hint::black_box};

use pbeam_runtime::{BeamProgram, Term};

const DISPATCH_PROBE: &[u8] = include_bytes!("../../fixtures/dispatch/dispatch_probe.beam");

/// The two lengths of the report. The difference of the counts of the two
/// runs, divided by the difference of the lengths, is the count for one
/// iteration.
const LENGTHS: [usize; 2] = [20_000, 40_000];

/// The length of the warm-up run of each function. The first run of a
/// function pays the one-off setup of the runtime, and the length of that
/// setup does not grow with the length of the run, so the difference of the
/// two measured runs cannot cancel it. The warm-up run pays it first.
const WARM_UP: usize = 100;

/// The allocator of the process, with a count of the allocations of each
/// thread.
struct CountingAllocator;

thread_local! {
    /// The number of allocations of the current thread. The count is for each
    /// thread, so a thread of the harness or of the runtime that allocates
    /// beside the measured run changes no number of this test. The `Cell` has
    /// a constant value and no destructor, so the allocator reads it without
    /// an allocation of its own.
    static ALLOCATIONS: Cell<usize> = const { Cell::new(0) };
}

#[global_allocator]
static ALLOCATOR: CountingAllocator = CountingAllocator;

// SAFETY: each method forwards its arguments to `System`, which is a valid
// allocator, so the `GlobalAlloc` contract holds. The count only changes a
// `Cell` of the current thread, which does not allocate, unwind, or touch
// the memory of the caller.
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
/// let before = counted();
/// count_one();
/// assert_eq!(counted(), before + 1);
/// ```
fn count_one() {
    ALLOCATIONS.with(|count| count.set(count.get().wrapping_add(1)));
}

/// Return the number of allocations of the current thread.
///
/// # Examples
///
/// ```rust,ignore
/// assert_eq!(counted(), 0);
/// ```
fn counted() -> usize {
    ALLOCATIONS.with(Cell::get)
}

/// Run `function` at `length` and return the number of allocations of the
/// run. The result of the run goes to [`black_box`], so the compiler cannot
/// remove the work.
///
/// # Examples
///
/// ```rust,ignore
/// let count = allocations(&program, "floor_loop", 20_000);
/// assert!(count > 0);
/// ```
fn allocations(program: &BeamProgram, function: &str, length: usize) -> usize {
    let counter = Term::Integer(length as i64);
    let tuple = || Term::tuple(vec![Term::Integer(1), Term::Integer(2), Term::Integer(3)]);
    let terms: Vec<Term> = match function {
        "floor_loop" => vec![counter],
        "plus_loop" => vec![counter, Term::Integer(0)],
        "tuple_size_loop" | "element_loop" => vec![counter, Term::Integer(0), tuple()],
        "hd_loop" => vec![
            counter,
            Term::Integer(0),
            Term::list_from([Term::Integer(1)]),
        ],
        other => panic!("unknown probe function {other}"),
    };
    let arguments = terms
        .into_iter()
        .map(|term| program.host_term(term))
        .collect::<Vec<_>>();
    let start = counted();
    let result = program
        .execute("dispatch_probe", function, &arguments)
        .unwrap();
    let difference = counted() - start;
    black_box(&result);
    difference
}

/// Return the allocation count of one iteration of `function`, from the two
/// runs of [`LENGTHS`].
///
/// # Examples
///
/// ```rust,ignore
/// let per_iteration = per_iteration(&program, "plus_loop");
/// assert!(per_iteration > 0);
/// ```
fn per_iteration(program: &BeamProgram, function: &str) -> usize {
    // The first run of a function pays the one-off setup of the runtime,
    // which the difference of the two runs cannot cancel. The warm-up run
    // pays it first, outside the two measured runs.
    let _ = allocations(program, function, WARM_UP);
    let first = allocations(program, function, LENGTHS[0]);
    let second = allocations(program, function, LENGTHS[1]);
    // Round the quotient. One allocation for each iteration is
    // `LENGTHS[1] - LENGTHS[0]` allocations, so a stray allocation of the
    // host moves no count of one iteration. The difference of the count of
    // one iteration from the count of another function stays exact.
    let difference = (second - first) + (LENGTHS[1] - LENGTHS[0]) / 2;
    difference / (LENGTHS[1] - LENGTHS[0])
}

/// Check the allocation count of the borrowed BIF path: none of the added
/// instructions of `docs/interpreter-dispatch-plan.md` adds an allocation to
/// the floor loop, so the path builds no `Vec<Term>` for one call. Step 3 of
/// the plan measured the argument `Vec<Term>` of the `gc_bif` as one of the
/// two allocations of one iteration of the `call_only` floor loop, with the
/// pair stack of `Term::eq` in the `test` instruction as the other.
#[test]
fn passes_borrowed_bif_arguments_with_no_allocation() {
    let program = BeamProgram::load(&[DISPATCH_PROBE]).unwrap();
    let floor = per_iteration(&program, "floor_loop");
    for function in ["plus_loop", "tuple_size_loop", "element_loop", "hd_loop"] {
        let added = per_iteration(&program, function);
        println!("allocations for one iteration: floor_loop {floor}, {function} {added}");
        assert_eq!(
            added, floor,
            "{function} must add no allocation to the floor loop"
        );
    }
}

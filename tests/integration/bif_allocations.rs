//! The allocation count of one BIF call of `docs/interpreter-dispatch-plan.md`.
//! Step 5 of the plan gives the pure leaf BIFs borrowed arguments, so the
//! call path builds no argument `Vec<Term>`.
//!
//! The test runs the fixture loop of `fixtures/dispatch/dispatch_probe.erl`
//! twice, at two lengths, and divides the difference by the length
//! difference. The constant cost of one run cancels, so the result is the
//! allocation count of one iteration.

use std::{alloc::System, hint::black_box, sync::atomic::AtomicUsize};

use pbeam_runtime::{BeamProgram, Term};

const DISPATCH_PROBE: &[u8] = include_bytes!("../../fixtures/dispatch/dispatch_probe.beam");

/// The two lengths of the report. The difference of the counts of the two
/// runs, divided by the difference of the lengths, is the count for one
/// iteration.
const LENGTHS: [usize; 2] = [20_000, 40_000];

/// The allocator of the process, with a count of the allocations.
struct CountingAllocator;

/// The number of allocations of the process.
static ALLOCATIONS: AtomicUsize = AtomicUsize::new(0);

// SAFETY: the allocator forwards each operation to `System`, which is a
// valid allocator, and it only adds a count.
#[global_allocator]
static ALLOCATOR: CountingAllocator = CountingAllocator;

unsafe impl std::alloc::GlobalAlloc for CountingAllocator {
    unsafe fn alloc(&self, layout: std::alloc::Layout) -> *mut u8 {
        ALLOCATIONS.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        // SAFETY: the caller gives a valid layout, as `GlobalAlloc` requires.
        unsafe { System.alloc(layout) }
    }

    unsafe fn dealloc(&self, pointer: *mut u8, layout: std::alloc::Layout) {
        // SAFETY: the pointer and the layout come from `alloc` of `System`.
        unsafe { System.dealloc(pointer, layout) }
    }

    unsafe fn realloc(&self, pointer: *mut u8, layout: std::alloc::Layout, size: usize) -> *mut u8 {
        ALLOCATIONS.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        // SAFETY: the pointer and the layout come from `alloc` of `System`.
        unsafe { System.realloc(pointer, layout, size) }
    }
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
    let arguments: &[Term] = match function {
        "floor_loop" => &[counter],
        "plus_loop" => &[counter, Term::Integer(0)],
        "tuple_size_loop" | "element_loop" => &[counter, Term::Integer(0), tuple()],
        "hd_loop" => &[
            counter,
            Term::Integer(0),
            Term::list_from([Term::Integer(1)]),
        ],
        other => panic!("unknown probe function {other}"),
    };
    let start = ALLOCATIONS.load(std::sync::atomic::Ordering::Relaxed);
    let result = program
        .execute("dispatch_probe", function, arguments)
        .unwrap();
    let count = ALLOCATIONS.load(std::sync::atomic::Ordering::Relaxed) - start;
    black_box(&result);
    count
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
    let first = allocations(program, function, LENGTHS[0]);
    let second = allocations(program, function, LENGTHS[1]);
    (second - first) / (LENGTHS[1] - LENGTHS[0])
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

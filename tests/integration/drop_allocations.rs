//! The allocation count of the term drop path of
//! `docs/interpreter-dispatch-plan.md` step 7.
//!
//! Step 7 removes two allocations of the drop path. `Map::drain_into` built a
//! fresh empty form for every map that it drained, before `Arc::into_inner`
//! read the handle. `Term::drop` built a heap vector for the nested terms of
//! every compound term, also for a term that gives no nested term.
//!
//! The test counts the allocations of the process with a counting allocator
//! and drops one term of each shape. A term that needs no walk must allocate
//! nothing at all. A term whose walk needs a stack keeps its allocation, and
//! the report prints the count and names the structure that keeps it.

use std::{
    alloc::System,
    hint::black_box,
    sync::atomic::{AtomicUsize, Ordering},
};

use pbeam_runtime::{Map, Term};

/// The allocations of the drop of a map that owns its form: the work vector
/// that holds one key and one value of a pair at a time.
const MAP_ALLOCATIONS: usize = 1;

/// The allocations of the drop of a list of 1000000 cells: the work vector
/// that holds the tail and then the head of one cell at a time.
const LIST_ALLOCATIONS: usize = 1;

/// The allocations of the drop of a map of 40 keys: the work vector of the
/// pairs and the node stack of the HAMT walk of `Map::drain_into`.
const HASHMAP_ALLOCATIONS: usize = 10;

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
        ALLOCATIONS.fetch_add(1, Ordering::Relaxed);
        // SAFETY: the caller gives a valid layout, as `GlobalAlloc` requires.
        unsafe { System.alloc(layout) }
    }

    unsafe fn dealloc(&self, pointer: *mut u8, layout: std::alloc::Layout) {
        // SAFETY: the pointer and the layout come from `alloc` of `System`.
        unsafe { System.dealloc(pointer, layout) }
    }

    unsafe fn realloc(&self, pointer: *mut u8, layout: std::alloc::Layout, size: usize) -> *mut u8 {
        ALLOCATIONS.fetch_add(1, Ordering::Relaxed);
        // SAFETY: the pointer and the layout come from `alloc` of `System`.
        unsafe { System.realloc(pointer, layout, size) }
    }
}

/// Return the number of allocations of the process.
///
/// # Examples
///
/// ```rust,ignore
/// assert!(allocations() > 0);
/// ```
fn allocations() -> usize {
    ALLOCATIONS.load(Ordering::Relaxed)
}

/// Drop `term` and return the number of allocations of the drop. The terms of
/// `held` stay alive across the drop, so a shape whose term shares its storage
/// gives the drop of a shared holder.
///
/// # Examples
///
/// ```rust,ignore
/// assert_eq!(dropped(Term::Integer(1), &[]), 0);
/// ```
fn dropped(term: Term, held: &[Term]) -> usize {
    let start = allocations();
    drop(term);
    let count = allocations() - start;
    black_box(held);
    count
}

/// Return a flatmap term of one key.
///
/// # Examples
///
/// ```rust,ignore
/// assert!(matches!(one_key_map(), Term::Map(_)));
/// ```
fn one_key_map() -> Term {
    Term::Map(Map::from_pairs(vec![(Term::Integer(1), Term::Integer(2))]).unwrap())
}

/// Return a hashmap term of `keys` keys.
///
/// # Examples
///
/// ```rust,ignore
/// assert!(matches!(hashmap(40), Term::Map(_)));
/// ```
fn hashmap(keys: i64) -> Term {
    Term::Map(
        Map::from_pairs(
            (1..=keys)
                .map(|key| (Term::Integer(key), Term::Integer(key)))
                .collect(),
        )
        .unwrap(),
    )
}

/// Return a tuple term of three immediate elements.
///
/// # Examples
///
/// ```rust,ignore
/// assert_eq!(three_tuple(), Term::tuple(vec![Term::Integer(1), Term::Integer(2), Term::Integer(3)]));
/// ```
fn three_tuple() -> Term {
    Term::tuple(vec![Term::Integer(1), Term::Integer(2), Term::Integer(3)])
}

/// One shape of the allocation report: its name, the term that the drop
/// releases, and the terms that stay alive across the drop.
struct Shape {
    /// The name of the shape in the report.
    name: &'static str,
    /// The term whose drop the test measures.
    term: Term,
    /// The terms that stay alive across the drop.
    held: Vec<Term>,
}

/// Return the shapes of step 7 that need no walk, with their terms: an
/// immediate, a tuple that owns its array, a tuple that shares its array, a
/// tuple of 20 elements, and a map that shares its form.
///
/// # Examples
///
/// ```rust,ignore
/// assert_eq!(shapes_without_a_walk()[0].name, "an immediate");
/// ```
fn shapes_without_a_walk() -> Vec<Shape> {
    let tuple = three_tuple();
    let shared_tuple = tuple.clone();
    let map = one_key_map();
    let shared_map = map.clone();
    vec![
        // An immediate holds no nested term.
        Shape {
            name: "an immediate",
            term: Term::Integer(1),
            held: Vec::new(),
        },
        // A tuple that owns its array gives that array to the walk, so the
        // walk allocates no second vector.
        Shape {
            name: "a tuple",
            term: three_tuple(),
            held: Vec::new(),
        },
        // A tuple whose array another term holds gives no element, so its
        // drop needs no walk.
        Shape {
            name: "a shared tuple",
            term: shared_tuple,
            held: vec![tuple],
        },
        // A tuple of 20 elements gives its own array in the same way.
        Shape {
            name: "a tuple of 20 elements",
            term: Term::tuple((0..20).map(Term::Integer).collect()),
            held: Vec::new(),
        },
        // A map whose form another term holds gives no pair, and the walk of
        // a map builds its vector only for a pair.
        Shape {
            name: "a shared map",
            term: shared_map,
            held: vec![map],
        },
    ]
}

/// Return the shapes of step 7 whose walk needs a work vector, with their
/// terms.
///
/// # Examples
///
/// ```rust,ignore
/// assert_eq!(shapes_with_a_walk()[0].name, "a map");
/// ```
fn shapes_with_a_walk() -> Vec<Shape> {
    let map = one_key_map();
    let cloned_map = map.clone();
    drop(map);
    vec![
        // The walk of a map holds one pair at a time on its vector.
        Shape {
            name: "a map",
            term: one_key_map(),
            held: Vec::new(),
        },
        // The last holder of a cloned map walks the same pairs.
        Shape {
            name: "a cloned map",
            term: cloned_map,
            held: Vec::new(),
        },
        // The walk of a list holds the tail and then the head of one cell at
        // a time on its vector.
        Shape {
            name: "a list of 1000000 cells",
            term: Term::list_from((0..1_000_000).map(Term::Integer)),
            held: Vec::new(),
        },
        // A hashmap walks its pairs and its HAMT nodes.
        Shape {
            name: "a map of 40 keys",
            term: hashmap(40),
            held: Vec::new(),
        },
    ]
}

/// Drop one term of each shape of `shapes` and return the name and the
/// allocation count of each drop.
///
/// # Examples
///
/// ```rust,ignore
/// assert_eq!(counts(shapes_without_a_walk()).len(), 5);
/// ```
fn counts(shapes: Vec<Shape>) -> Vec<(&'static str, usize)> {
    shapes
        .into_iter()
        .map(|shape| (shape.name, dropped(shape.term, &shape.held)))
        .collect()
}

/// Print the allocation count of each shape of one report.
///
/// # Examples
///
/// ```rust,ignore
/// report(&[("an immediate", 0)]);
/// ```
fn report(counts: &[(&str, usize)]) {
    for (name, count) in counts {
        println!("allocations of the drop of {name}: {count}");
    }
}

/// Check that a term that needs no walk allocates nothing at all: an
/// immediate, a tuple, a shared tuple, a tuple of 20 elements, and a shared
/// map.
#[test]
fn drops_a_term_without_a_walk_with_no_allocation() {
    let counts = counts(shapes_without_a_walk());
    report(&counts);
    assert!(
        counts.iter().all(|(_, count)| *count == 0),
        "the drop of a term that needs no walk allocates: {counts:?}"
    );
}

/// Record the allocation counts that the drop path keeps, and check them so
/// that a later change of a count is visible.
#[test]
fn records_the_allocations_of_the_drop_path_that_stay() {
    let counts = counts(shapes_with_a_walk());
    report(&counts);
    assert_eq!(
        counts,
        [
            ("a map", MAP_ALLOCATIONS),
            ("a cloned map", MAP_ALLOCATIONS),
            ("a list of 1000000 cells", LIST_ALLOCATIONS),
            ("a map of 40 keys", HASHMAP_ALLOCATIONS),
        ],
        "the work vectors of the walk stay"
    );
}

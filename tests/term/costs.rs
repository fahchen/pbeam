//! The cost of one term operation, the committed measurement of the term
//! table of `docs/interpreter-dispatch-measurements.md`.
//!
//! The measurement record reports one `Arc` clone and drop, one `Rc` clone
//! and drop, one bare `Arc` clone and drop, and one deep copy for each term
//! shape. The first form of that table came from a scratch benchmark that
//! the repository did not keep, so a reader cannot retrieve it. This module
//! commits the measurement: it builds each term, measures the operation,
//! consumes every result with [`black_box`], and prints one line for each
//! operation.
//!
//! Each printed line is the minimum of [`RUNS`] timed runs of one operation,
//! in nanosecond for each operation. The timer of one run holds the
//! operation that the line names and nothing else. The test asserts no time,
//! because the numbers move with the host and its load, and a test that
//! asserts a time fails for a reason that is not a defect. The measurement
//! of the fun payload and of the map payload lives in `tests/term/fun.rs`
//! and `tests/term/map.rs`, because those payload types are private to their
//! modules.
//!
//! Run the release measurement with `mise run bench-term`, which writes
//! `target/term-costs.txt` with the revision, the host, the toolchain, the
//! load, and the lines below. CI does not run the measurement tests.

use std::{hint::black_box, rc::Rc, sync::Arc, time::Instant};

use super::{Atom, Binary, Fun, Lambda, Map, Term, list::Cons};

/// The number of timed runs of one operation. The report gives the minimum
/// of the runs, as the design measurement does.
pub(super) const RUNS: usize = 9;

/// The number of operations of one timed run of a shared handle.
pub(super) const HANDLE_OPERATIONS: usize = 200_000;

/// The number of operations of one timed run of a small deep copy. The
/// copies stay alive until the timer stops, so this count bounds the memory
/// of one run.
const SMALL_COPY_OPERATIONS: usize = 500;

/// The number of operations of one timed run of the fun deep copy.
pub(super) const FUN_COPY_OPERATIONS: usize = 20_000;

/// The number of operations of one timed run of a large deep copy. The
/// copies stay alive until the timer stops, so this count bounds the memory
/// of one run.
pub(super) const LARGE_COPY_OPERATIONS: usize = 200;

/// Print one measured line: the name of the operation and the nanosecond for
/// each operation.
pub(super) fn report(name: &str, ns: f64) {
    println!("TERM-COST {name} {ns:.3} ns");
}

/// Print one allocation line: the name of the operation and the number of
/// allocations of one call.
fn report_allocations(name: &str, count: usize) {
    println!("TERM-ALLOC {name} {count}");
}

/// Measure one read of the size of a binary through its view descriptor, and
/// print the line. The read follows the pointer of the handle and then reads
/// the fields of the view that the size needs, so the line holds the
/// dependent load that the boxed descriptor adds to each read of a field of
/// the view. The inner `black_box` sets the handle pointer of one iteration,
/// so the compiler cannot move the read of the view out of the loop, and the
/// outer `black_box` consumes the size that the read gives. The release
/// assembly of the test keeps the load of the view inside the loop.
fn read_binary_view(name: &str, bytes: &Binary) {
    let ns = measure(RUNS, HANDLE_OPERATIONS, || {
        for _ in 0..HANDLE_OPERATIONS {
            black_box(black_box(bytes).bit_bytes().len());
        }
    });
    report(name, ns);
}

/// Return the nanoseconds for each operation of `body`, as the minimum of
/// `runs` timed runs. The caller performs exactly `operations` operations.
/// The timer stops before the value of `body` drops, so a measured copy does
/// not hold the release of its result.
#[expect(
    clippy::as_conversions,
    clippy::cast_precision_loss,
    reason = "a benchmark converts a nanosecond count and an operation count to the nearest f64"
)]
fn measure<T>(runs: usize, operations: usize, mut body: impl FnMut() -> T) -> f64 {
    let mut best = f64::INFINITY;
    for _ in 0..runs {
        let start = Instant::now();
        let kept = body();
        let elapsed = start.elapsed().as_nanos() as f64;
        black_box(&kept);
        drop(kept);
        best = best.min(elapsed / operations as f64);
    }
    black_box(best)
}

/// Measure the clone and the drop of the shared handle `held`, and print the
/// line. The source of the clone stays alive across the whole run, so the
/// drop of the clone is the drop of a shared holder.
pub(super) fn clone_drop<T: Clone>(name: &str, held: &T, operations: usize) {
    let ns = measure(RUNS, operations, || {
        for _ in 0..operations {
            let copy = held.clone();
            black_box(&copy);
            drop(copy);
        }
    });
    report(name, ns);
}

/// Measure the deep copy that `copy` builds, and print the line. The
/// container of the copies is allocated before the timer and cleared between
/// runs, so the timed region holds the copy, the push of the copy into the
/// container, and the storage that the copy allocates itself, and no
/// allocation of the container. The release of the copies is outside the
/// timed region.
#[expect(
    clippy::as_conversions,
    clippy::cast_precision_loss,
    reason = "a benchmark converts a nanosecond count and an operation count to the nearest f64"
)]
pub(super) fn deep_copy<T>(name: &str, operations: usize, mut copy: impl FnMut() -> T) {
    let mut kept: Vec<T> = Vec::with_capacity(operations);
    let mut best = f64::INFINITY;
    for _ in 0..RUNS {
        kept.clear();
        let start = Instant::now();
        for _ in 0..operations {
            kept.push(copy());
        }
        let elapsed = start.elapsed().as_nanos() as f64;
        black_box(&kept);
        best = best.min(elapsed / operations as f64);
    }
    report(name, black_box(best));
}

/// Return a lambda of the module `lists`, for the fun shapes. The fields
/// name a `FunT` entry, and the benchmark does not run the fun.
fn lambda() -> Lambda {
    let module = Atom::from_static("lists").expect("lists is a static atom");
    Lambda::new(module, 0, 0, 7, 53, 1)
}

/// Return a tuple term of 1000 elements, and the element array that its
/// storage holds.
fn tuple_1000() -> (Term, Vec<Term>) {
    let elements: Vec<Term> = (0..1_000).map(Term::Integer).collect();
    (Term::tuple(elements.clone()), elements)
}

/// Return a binary term of 102400 bytes, and the byte array that its storage
/// holds.
fn binary_102400() -> (Term, Vec<u8>) {
    let bytes = vec![7u8; 102_400];
    (Term::binary(bytes.clone()), bytes)
}

/// Return a fun term with two free variables.
fn fun_2_free() -> Term {
    Term::Fun(Fun::new(lambda(), vec![Term::Integer(1), Term::Integer(2)]))
}

/// Return one cons cell, and the cell that its storage holds.
fn cons_1() -> (Term, Arc<Cons>) {
    let cell = Arc::new(Cons {
        head: Term::Integer(1),
        tail: Term::Nil,
    });
    (Term::list_from([Term::Integer(1)]), cell)
}

/// Return the pairs of a 1000-key map.
fn pairs_1000() -> Vec<(Term, Term)> {
    (0..1_000)
        .map(|key| (Term::Integer(key), Term::Integer(key)))
        .collect()
}

/// Return a 1000-key map term.
fn map_1000() -> Term {
    Term::Map(
        Map::from_pairs(pairs_1000(), &crate::term::test_atom_table())
            .expect("1000 integer keys are valid"),
    )
}

/// Return a tuple term that holds a 1000-key map term.
fn tuple_with_map_1000() -> Term {
    Term::tuple(vec![map_1000()])
}

#[test]
fn prints_the_cost_of_a_term_operation() {
    let (tuple, elements) = tuple_1000();
    let (binary, bytes) = binary_102400();
    let (cons, cell) = cons_1();
    let map = map_1000();
    let tuple_with_map = tuple_with_map_1000();
    let fun = fun_2_free();

    // The `Arc` path of one term: `Term::clone` shares the storage of the
    // source, and the clone drops while the source stays alive.
    clone_drop("arc_clone_drop_tuple_1000", &tuple, HANDLE_OPERATIONS);
    clone_drop("arc_clone_drop_binary_102400", &binary, HANDLE_OPERATIONS);
    clone_drop("arc_clone_drop_fun_2_free", &fun, HANDLE_OPERATIONS);
    clone_drop("arc_clone_drop_cons_1", &cons, HANDLE_OPERATIONS);
    clone_drop(
        "arc_clone_drop_tuple_with_map_1000",
        &tuple_with_map,
        HANDLE_OPERATIONS,
    );
    clone_drop("arc_clone_drop_map_1000", &map, HANDLE_OPERATIONS);

    // The `Rc` counterfactual of the same plain payloads, and the bare
    // counters without a term.
    let rc_tuple: Rc<Vec<Term>> = Rc::new(elements.clone());
    let rc_binary: Rc<Vec<u8>> = Rc::new(bytes.clone());
    let rc_cell: Rc<Cons> = Rc::new(Cons {
        head: Term::Integer(1),
        tail: Term::Nil,
    });
    let arc_bare: Arc<u64> = Arc::new(1);
    let rc_bare: Rc<u64> = Rc::new(1);
    clone_drop("rc_clone_drop_tuple_1000", &rc_tuple, HANDLE_OPERATIONS);
    clone_drop("rc_clone_drop_binary_102400", &rc_binary, HANDLE_OPERATIONS);
    clone_drop("rc_clone_drop_cons_1", &rc_cell, HANDLE_OPERATIONS);
    clone_drop("arc_clone_drop_bare", &arc_bare, HANDLE_OPERATIONS);
    clone_drop("rc_clone_drop_bare", &rc_bare, HANDLE_OPERATIONS);

    // The deep copy of one term: a new storage with the contents of the
    // source. This is the copy that the pre-copy-on-write path paid before
    // the term shared its storage.
    deep_copy("deep_copy_tuple_1000", SMALL_COPY_OPERATIONS, || {
        Term::tuple(elements.clone())
    });
    deep_copy("deep_copy_binary_102400", SMALL_COPY_OPERATIONS, || {
        Term::binary(bytes.clone())
    });
    deep_copy("deep_copy_cons_1", SMALL_COPY_OPERATIONS, || {
        Arc::new(Cons {
            head: Term::Integer(1),
            tail: Term::Nil,
        })
    });

    // The shapes stay alive to the end of the test, so the compiler keeps
    // the storage that the measurement shares.
    black_box((&tuple, &binary, &cons, &map, &tuple_with_map, &fun, &cell));
}

#[test]
fn checks_binary_view_sharing() {
    let binary = Binary::from(vec![7u8; 4]);
    let copy = binary.clone();
    assert!(binary.same_view(&copy));
    assert!(!binary.same_view(&Binary::from(b"abcd".to_vec())));
    let part = binary.bits(0, 16);
    assert!(binary.same_bytes(&part));
    let packed = binary.bits(4, 12);
    assert!(packed.packed_backing().is_some());
}

#[test]
fn prints_the_cost_of_a_binary_view() {
    // The boxed descriptor of task [#571] holds one `Arc` to the view for
    // each binary handle. The handle then pays one allocation for the view,
    // and each read of a field of the view follows one dependent load. The
    // allocation count is exact, so a change of the representation fails
    // the test. The count uses the counting allocator of
    // `tests/term/data_structure_costs.rs`, which counts the allocations of
    // the current thread only.
    //
    // [#571]: https://github.com/fahchen/pbeam/issues/571
    use super::data_structure_costs::allocations;

    let source = vec![7u8; 102_400];
    let before = allocations();
    let binary = Binary::from(source);
    let built = allocations() - before;
    report_allocations("build_binary_102400", built);
    assert_eq!(
        built, 2,
        "`Binary::from` allocates the byte array and the view descriptor"
    );
    // The append tree of [#558] adds the chunk list and the joined array
    // to the view, so the descriptor grows from four words to seven. A view
    // that no append built keeps both fields empty or unset. The off-heap
    // mark of [#1237] adds one more word.
    //
    // [#558]: https://github.com/fahchen/pbeam/issues/558
    // [#1237]: https://github.com/fahchen/pbeam/issues/1237
    assert_eq!(
        Binary::view_bytes(),
        8 * std::mem::size_of::<usize>(),
        "the view descriptor is eight words"
    );

    let before = allocations();
    let copy = binary.clone();
    let copied = allocations() - before;
    report_allocations("clone_binary_102400", copied);
    assert_eq!(copied, 0, "a copy of the handle shares the view");
    assert!(binary.same_view(&copy), "a copy shares the same view");
    assert!(
        !binary.same_view(&Binary::from(b"abcd".to_vec())),
        "a new binary owns another view"
    );

    // A byte-aligned sub-binary adds one descriptor and shares the array.
    let before = allocations();
    let part = binary.bits(0, 16);
    let sub = allocations() - before;
    report_allocations("sub_binary_102400", sub);
    assert_eq!(sub, 1, "a byte-aligned sub-binary adds one descriptor");
    assert!(binary.same_bytes(&part), "a sub-binary shares the array");

    // An unaligned sub-binary packs its bits: the vector of the bits and the
    // `Arc` header of the packed array, beside the descriptor.
    let before = allocations();
    let packed = binary.bits(4, 12);
    let pack = allocations() - before;
    report_allocations("packed_sub_binary_102400", pack);
    assert_eq!(
        pack, 3,
        "an unaligned sub-binary adds one descriptor and one packed array"
    );
    assert!(packed.packed_backing().is_some());

    read_binary_view("read_binary_view_102400", &binary);
    black_box((&binary, &copy, &part, &packed));
}

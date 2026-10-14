//! The drop cost of a term of `docs/interpreter-dispatch-plan.md` step 7.
//!
//! The step removes the placeholder form that `Map::drain_into` built before
//! `Arc::into_inner` and the heap work vector that `Term::drop` built for
//! every term. The benchmark measures the drop of one term of each shape
//! that the step names: an immediate, a tuple, a shared map, a cloned map,
//! and a deep list.
//!
//! The report gives the minimum of the timed runs of one shape, in
//! nanosecond for each term. The population of one run is prepared outside
//! the timer, and the timer holds the release of each term and nothing
//! else. A shape whose term shares its storage with another term keeps the
//! other holder alive across the timer, so the timer measures the drop of a
//! shared holder and not the release of the storage.
//!
//! Run the release measurement with `mise run bench-drop`. The check gate
//! runs the same test under the test profile, which proves that every shape
//! runs; its numbers are not the report numbers.

use std::{
    hint::black_box,
    time::{Duration, Instant},
};

use pbeam_runtime::{AtomTable, Map, Term};

/// The number of terms of one timed run of a shape whose term is small.
const BATCH: usize = 100_000;

/// The number of timed runs of one shape with a small term. The report gives
/// the minimum of the runs.
const RUNS: usize = 9;

/// The number of cells of the deep list of the report. `Term::drop` walks
/// the nested terms with a work stack, so the drop of this list must not
/// overflow the Rust stack.
const LIST_CELLS: usize = 1_000_000;

/// The number of timed runs of the deep list. One run builds one list of
/// [`LIST_CELLS`] cells, and the report gives the minimum of the runs.
const LIST_RUNS: usize = 3;

/// One measured shape: its name in the report, the number of terms of one
/// run, the number of timed runs, and the builder of one run.
struct Shape {
    /// The name of the shape in the report.
    name: &'static str,
    /// The number of terms that one run releases inside the timer.
    terms: usize,
    /// The number of timed runs.
    runs: usize,
    /// Build the population of one run: the terms that the timer releases,
    /// and the terms that stay alive across the timer.
    prepare: fn(usize) -> (Vec<Term>, Vec<Term>),
}

/// Return a flatmap term of one key.
///
/// # Examples
///
/// ```rust,ignore
/// assert!(matches!(one_key_map(), Term::Map(_)));
/// ```
fn one_key_map() -> Term {
    Term::Map(
        Map::from_pairs(
            vec![(Term::Integer(1), Term::Integer(2))],
            &AtomTable::new(),
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

/// Return a tuple term of one immediate element, which is the smallest
/// compound term that the walk of `Term::drop` holds.
///
/// # Examples
///
/// ```rust,ignore
/// assert_eq!(one_tuple(), Term::tuple(vec![Term::Integer(1)]));
/// ```
fn one_tuple() -> Term {
    Term::tuple(vec![Term::Integer(1)])
}

/// Return a list term of [`LIST_CELLS`] cells.
///
/// # Examples
///
/// ```rust,ignore
/// assert!(deep_list().list_head().is_some());
/// ```
fn deep_list() -> Term {
    Term::list_from((0..LIST_CELLS as i64).map(Term::Integer))
}

/// Return one population of `count` terms that each own their storage, and
/// no term that stays alive across the timer.
///
/// # Examples
///
/// ```rust,ignore
/// let (measured, held) = own_storage(three_tuple, 10);
/// assert_eq!((measured.len(), held.len()), (10, 0));
/// ```
fn own_storage(make: fn() -> Term, count: usize) -> (Vec<Term>, Vec<Term>) {
    ((0..count).map(|_| make()).collect(), Vec::new())
}

/// Return one population of `count` terms that each share their storage with
/// a term that stays alive across the timer. The timer releases a shared
/// holder, so `Arc::into_inner` gives it no storage.
///
/// # Examples
///
/// ```rust,ignore
/// let (measured, held) = shared_storage(three_tuple, 10);
/// assert_eq!((measured.len(), held.len()), (10, 10));
/// ```
fn shared_storage(make: fn() -> Term, count: usize) -> (Vec<Term>, Vec<Term>) {
    let held: Vec<Term> = (0..count).map(|_| make()).collect();
    (held.clone(), held)
}

/// Return one population of `count` clones whose original has already
/// dropped. The timer releases the last holder of the storage, so
/// `Arc::into_inner` gives it the storage and the drop walks it.
///
/// # Examples
///
/// ```rust,ignore
/// let (measured, held) = last_holder(one_key_map, 10);
/// assert_eq!((measured.len(), held.len()), (10, 0));
/// ```
fn last_holder(make: fn() -> Term, count: usize) -> (Vec<Term>, Vec<Term>) {
    let measured = (0..count)
        .map(|_| {
            let original = make();
            let clone = original.clone();
            drop(original);
            clone
        })
        .collect();
    (measured, Vec::new())
}

/// Return the population of the immediate shape.
///
/// # Examples
///
/// ```rust,ignore
/// assert_eq!(immediate(10).0.len(), 10);
/// ```
fn immediate(count: usize) -> (Vec<Term>, Vec<Term>) {
    (vec![Term::Integer(1); count], Vec::new())
}

/// Return the population of the tuple shape whose term owns its array.
///
/// # Examples
///
/// ```rust,ignore
/// assert_eq!(tuple(10).0.len(), 10);
/// ```
fn tuple(count: usize) -> (Vec<Term>, Vec<Term>) {
    own_storage(three_tuple, count)
}

/// Return the population of the tuple shape whose array another term holds.
///
/// # Examples
///
/// ```rust,ignore
/// assert_eq!(shared_tuple(10).1.len(), 10);
/// ```
fn shared_tuple(count: usize) -> (Vec<Term>, Vec<Term>) {
    shared_storage(three_tuple, count)
}

/// Return the population of the one-element tuple shape whose term owns its
/// array.
///
/// # Examples
///
/// ```rust,ignore
/// assert_eq!(one_tuple_population(10).0.len(), 10);
/// ```
fn one_tuple_population(count: usize) -> (Vec<Term>, Vec<Term>) {
    own_storage(one_tuple, count)
}

/// Return the population of the one-element tuple shape whose array another
/// term holds.
///
/// # Examples
///
/// ```rust,ignore
/// assert_eq!(shared_one_tuple(10).1.len(), 10);
/// ```
fn shared_one_tuple(count: usize) -> (Vec<Term>, Vec<Term>) {
    shared_storage(one_tuple, count)
}

/// Return the population of the map shape whose term owns its form.
///
/// # Examples
///
/// ```rust,ignore
/// assert_eq!(map(10).0.len(), 10);
/// ```
fn map(count: usize) -> (Vec<Term>, Vec<Term>) {
    own_storage(one_key_map, count)
}

/// Return the population of the map shape whose form another term holds.
///
/// # Examples
///
/// ```rust,ignore
/// assert_eq!(shared_map(10).1.len(), 10);
/// ```
fn shared_map(count: usize) -> (Vec<Term>, Vec<Term>) {
    shared_storage(one_key_map, count)
}

/// Return the population of the map shape of a clone whose original has
/// already dropped.
///
/// # Examples
///
/// ```rust,ignore
/// assert_eq!(cloned_map(10).0.len(), 10);
/// ```
fn cloned_map(count: usize) -> (Vec<Term>, Vec<Term>) {
    last_holder(one_key_map, count)
}

/// Return the population of the deep list shape.
///
/// # Examples
///
/// ```rust,ignore
/// assert_eq!(list(1).0.len(), 1);
/// ```
fn list(count: usize) -> (Vec<Term>, Vec<Term>) {
    own_storage(deep_list, count)
}

/// Return the shapes of the report, in table order.
///
/// # Examples
///
/// ```rust,ignore
/// assert_eq!(shapes()[0].name, "immediate");
/// ```
fn shapes() -> Vec<Shape> {
    vec![
        Shape {
            name: "immediate",
            terms: BATCH,
            runs: RUNS,
            prepare: immediate,
        },
        Shape {
            name: "tuple, last holder",
            terms: BATCH,
            runs: RUNS,
            prepare: tuple,
        },
        Shape {
            name: "tuple, shared holder",
            terms: BATCH,
            runs: RUNS,
            prepare: shared_tuple,
        },
        Shape {
            name: "tuple of one element, last holder",
            terms: BATCH,
            runs: RUNS,
            prepare: one_tuple_population,
        },
        Shape {
            name: "tuple of one element, shared holder",
            terms: BATCH,
            runs: RUNS,
            prepare: shared_one_tuple,
        },
        Shape {
            name: "map, last holder",
            terms: BATCH,
            runs: RUNS,
            prepare: map,
        },
        Shape {
            name: "map, shared holder",
            terms: BATCH,
            runs: RUNS,
            prepare: shared_map,
        },
        Shape {
            name: "map clone, last holder after the original drops",
            terms: BATCH,
            runs: RUNS,
            prepare: cloned_map,
        },
        Shape {
            name: "list of 1000000 cells",
            terms: 1,
            runs: LIST_RUNS,
            prepare: list,
        },
    ]
}

/// Return the cost of the release of one term of `shape`, in nanosecond, as
/// the minimum of its timed runs. The preparation of each population stays
/// outside the timer, and only the release of the terms is inside.
///
/// # Examples
///
/// ```rust,ignore
/// assert!(cost(&shapes()[0]) >= 0.0);
/// ```
fn cost(shape: &Shape) -> f64 {
    let mut best = Duration::MAX;
    for _ in 0..shape.runs {
        let (mut measured, held) = (shape.prepare)(shape.terms);
        let start = Instant::now();
        // `clear` releases each term and keeps the buffer of the vector, so
        // the timer holds the release of the terms only.
        measured.clear();
        let elapsed = start.elapsed();
        // The terms that stayed alive across the timer are released here.
        black_box(&held);
        best = best.min(elapsed);
    }
    best.as_nanos() as f64 / shape.terms as f64
}

/// Record the drop cost of one term of each shape of the step. Each number is
/// the minimum of the timed runs of its shape, and the timer holds the
/// release of each term and nothing else.
#[test]
fn records_the_drop_cost_of_a_term() {
    println!(
        "Term drop cost: minimum of the timed runs of each shape, in nanosecond for each term. One timer holds the release of the terms of one population."
    );
    println!();
    println!("| shape | terms for each run | runs | ns for each term |");
    println!("| --- | --- | --- | --- |");
    for shape in shapes() {
        println!(
            "| {} | {} | {} | {:.2} |",
            shape.name,
            shape.terms,
            shape.runs,
            cost(&shape)
        );
    }
    println!();
}

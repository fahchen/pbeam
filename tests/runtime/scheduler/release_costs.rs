//! The pause that the release of a large shared graph adds to the scheduler
//! thread, for row 11 of `docs/term-performance-design-codex-v2.md` and task
//! [#559](https://github.com/fahchen/pbeam/issues/559).
//!
//! The module measures four paths on the scheduler thread:
//!
//! - **synchronous last release**: the direct drop of the only holder of the
//!   graph. This is the walk that the exit of a process performed inline
//!   before this task, and `Term::drop` is the same code at both revisions.
//! - **process exit**: the exit of a process that owns the graph. The exit
//!   moves the terms of the process to the release work list of the
//!   scheduler, and each later scheduler step releases at most
//!   [`RELEASE_BATCH`] of them.
//! - **shared last release**: two processes share the graph, the first exit
//!   drops one holder, and the second exit releases the graph.
//! - **shutdown**: the graph stays in a live process, and the drop of the
//!   `Scheduler` at the end of the run releases it. No process waits to run
//!   at that point.
//!
//! The module lives beside the scheduler, because the pause is the time
//! inside the private exit, release, and drop paths, and no public API shows
//! those boundaries.
//!
//! One number is the pause of one timed run, in microsecond, and the load of
//! the run is printed with it. The report gives the maximum pause of each
//! path, because the row asks for the maximum scheduler pause. The check gate
//! runs the same test under the test profile, which proves that every path
//! runs; those numbers are not the report numbers.
//!
//! Run the release measurement with
//! `cargo test --locked --release --lib release_costs -- --nocapture --test-threads=1`
//! and read the `RELEASE` lines. `mise run bench-release` runs the same test
//! with the revision, the host, the toolchain, and the load of the runs.

use std::{
    hint::black_box,
    time::{Duration, Instant},
};

use super::{RELEASE_BATCH, Release, Scheduler, Turn};
use crate::{
    runtime::{
        capability::{AllowAll, Capability, Policy},
        process::{
            Start,
            tests::{frame, test_scheduler},
        },
    },
    term::{Map, ProcessId, Term},
};

/// The number of cells of the list and of levels of the chain of the report.
const CELLS: usize = 1_000_000;

/// The number of leaves of the tuple tree of the report. The tree then holds
/// 1048575 cells, which is about one million.
const TREE_LEAVES: usize = 1 << 19;

/// The number of keys of the map of the report. The map is a HAMT, and the
/// walk of `Map::drain_into` moves all of its pairs in one call.
const MAP_KEYS: usize = 20_000;

/// The number of timed runs of one path of one shape. The report gives the
/// maximum pause of the runs.
const RUNS: usize = 5;

/// What one timed run of one path of one shape measures.
struct Measurement {
    /// The pause that one process that waits to run sees: the maximum pause
    /// of one scheduler step of the release. A step is the exit step or one
    /// release batch of [`Scheduler::drain_release`].
    max_step: Duration,
    /// The pause of the whole release.
    total: Duration,
    /// The number of scheduler steps that the release used.
    steps: usize,
    /// The largest number of terms that the release work list of the
    /// scheduler held at a step boundary.
    retained_terms: usize,
    /// The number of bytes of the buffer of the release work list at the end
    /// of the release. The list owns each term, and its buffer is the memory
    /// that the bounded release keeps that the synchronous release does not.
    retained_bytes: usize,
}

/// One graph shape: its name, the number of compound nodes that its release
/// walks, and its builder.
struct Shape {
    /// The name of the shape in the report.
    name: &'static str,
    /// The number of compound nodes that the release of one graph of this
    /// shape walks: one cons cell for a list, one tuple for a chain, and one
    /// internal tuple for a tree.
    nodes: usize,
    /// Build one graph of the shape.
    build: fn() -> Term,
}

/// Return a list of [`CELLS`] cons cells.
///
/// # Examples
///
/// ```rust,ignore
/// assert!(list().list_head().is_some());
/// ```
fn list() -> Term {
    Term::list_from((0..CELLS as i64).map(Term::Integer))
}

/// Return a chain of [`CELLS`] nested one-element tuples around [`Term::Nil`].
///
/// # Examples
///
/// ```rust,ignore
/// assert!(matches!(chain(), Term::Tuple(_)));
/// ```
fn chain() -> Term {
    (0..CELLS).fold(Term::Nil, |inner, _| Term::tuple(vec![inner]))
}

/// Return a balanced tuple tree of [`TREE_LEAVES`] immediate leaves.
///
/// # Examples
///
/// ```rust,ignore
/// assert!(matches!(tree(), Term::Tuple(_)));
/// ```
fn tree() -> Term {
    let mut level: Vec<Term> = (0..TREE_LEAVES as i64).map(Term::Integer).collect();
    while level.len() > 1 {
        level = level
            .chunks(2)
            .map(|pair| Term::tuple(pair.to_vec()))
            .collect();
    }
    level.pop().expect("the tree holds one root")
}

/// Return one flat tuple of [`CELLS`] immediate elements. This shape is the
/// worst case of one fanout: one node holds every other term of the graph,
/// and `Term::take_nested` moves each direct child of a node in one call.
///
/// # Examples
///
/// ```rust,ignore
/// assert!(matches!(flat_tuple(), Term::Tuple(_)));
/// ```
fn flat_tuple() -> Term {
    Term::tuple((0..CELLS as i64).map(Term::Integer).collect())
}

/// Return one map of [`MAP_KEYS`] integer keys. This shape is the worst case
/// of the walk of a map: `Map::drain_into` moves every pair of the map in one
/// call.
///
/// # Examples
///
/// ```rust,ignore
/// assert!(matches!(map(), Term::Map(_)));
/// ```
fn map() -> Term {
    Term::Map(
        Map::from_pairs(
            (0..MAP_KEYS as i64)
                .map(|key| (Term::Integer(key), Term::Integer(key)))
                .collect(),
        )
        .expect("the integer keys of a map"),
    )
}

/// Return the shapes of the report, in table order.
///
/// # Examples
///
/// ```rust,ignore
/// assert_eq!(shapes()[0].name, "list of 1000000 cells");
/// ```
fn shapes() -> Vec<Shape> {
    vec![
        Shape {
            name: "list of 1000000 cells",
            nodes: CELLS,
            build: list,
        },
        Shape {
            name: "chain of 1000000 levels",
            nodes: CELLS,
            build: chain,
        },
        Shape {
            name: "tuple tree of 1048575 cells",
            nodes: 2 * TREE_LEAVES - 1,
            build: tree,
        },
        Shape {
            name: "flat tuple of 1048575 cells, one node",
            nodes: 1,
            build: flat_tuple,
        },
        Shape {
            name: "map of 20000 keys, one walk",
            nodes: MAP_KEYS,
            build: map,
        },
    ]
}

/// Return the initial capabilities of the default policy.
///
/// # Examples
///
/// ```rust,ignore
/// assert!(!allow_all().is_empty());
/// ```
fn allow_all() -> Vec<Capability> {
    AllowAll.initial()
}

/// Return a scheduler whose root process owns one holder of `graph`.
///
/// # Examples
///
/// ```rust,ignore
/// let scheduler = scheduler_with(Term::Nil);
/// assert!(scheduler.processes.contains_key(&ProcessId(0)));
/// ```
fn scheduler_with(graph: Term) -> Scheduler {
    let mut scheduler = test_scheduler();
    scheduler
        .start(frame(), vec![graph], allow_all())
        .expect("the root process starts");
    scheduler
}

/// Release each term of the release work list of `scheduler` in scheduler
/// batches, and return the maximum pause of one batch, the total pause, the
/// number of batches, and the largest number of terms that the list held at a
/// batch boundary.
///
/// # Examples
///
/// ```rust,ignore
/// let mut scheduler = test_scheduler();
/// let measured = drain(&mut scheduler);
/// assert_eq!(measured.2, 0);
/// ```
fn drain(scheduler: &mut Scheduler) -> (Duration, Duration, usize, usize) {
    let mut maximum = Duration::ZERO;
    let mut total = Duration::ZERO;
    let mut batches = 0;
    let mut peak = retained(scheduler);
    while !scheduler.release.is_empty() {
        let start = Instant::now();
        scheduler.drain_release(RELEASE_BATCH);
        let pause = start.elapsed();
        maximum = maximum.max(pause);
        total += pause;
        batches += 1;
        peak = peak.max(retained(scheduler));
    }
    (maximum, total, batches, peak)
}

/// Return the number of terms that the release work list of `scheduler`
/// owns: each term of the list and each term of a group that it holds.
///
/// # Examples
///
/// ```rust,ignore
/// assert_eq!(retained(&test_scheduler()), 0);
/// ```
fn retained(scheduler: &Scheduler) -> usize {
    scheduler
        .release
        .iter()
        .map(|item| match item {
            Release::Term(_) => 1,
            Release::Terms(group) => group.len(),
        })
        .sum()
}

/// Return the number of bytes of the buffer of the release work list of
/// `scheduler`. The list holds one item of each term or group, and the buffer
/// is the memory that the bounded release adds to the graph.
///
/// # Examples
///
/// ```rust,ignore
/// assert_eq!(retained_bytes(&test_scheduler()), 0);
/// ```
fn retained_bytes(scheduler: &Scheduler) -> usize {
    scheduler.release.capacity() * std::mem::size_of::<Release>()
}

/// Return the measurement of the direct drop of the only holder of `graph`.
///
/// # Examples
///
/// ```rust,ignore
/// assert!(synchronous_measurement(list()).max_step.as_nanos() > 0);
/// ```
fn synchronous_measurement(graph: Term) -> Measurement {
    let start = Instant::now();
    drop(graph);
    let pause = start.elapsed();
    Measurement {
        max_step: pause,
        total: pause,
        steps: 1,
        retained_terms: 0,
        retained_bytes: 0,
    }
}

/// Return the measurement of the exit of a process that owns `graph`.
///
/// # Examples
///
/// ```rust,ignore
/// assert!(exit_measurement(list()).total.as_nanos() > 0);
/// ```
fn exit_measurement(graph: Term) -> Measurement {
    let mut scheduler = scheduler_with(graph);
    assert_eq!(
        scheduler.schedule_next().expect("the step runs"),
        Turn::Run,
        "the root process runs"
    );
    let start = Instant::now();
    scheduler.finish_current(Term::static_atom("normal"), None);
    let exit = start.elapsed();
    let (maximum, total, batches, peak) = drain(&mut scheduler);
    let retained_terms = peak.max(retained(&scheduler));
    let retained_bytes = retained_bytes(&scheduler);
    black_box(&scheduler);
    Measurement {
        max_step: exit.max(maximum),
        total: exit + total,
        steps: 1 + batches,
        retained_terms,
        retained_bytes,
    }
}

/// Return the measurement of the exit of the second process that shares
/// `graph`. The root process exits first and drops one holder; the child
/// process then exits and releases the graph.
///
/// # Examples
///
/// ```rust,ignore
/// assert!(shared_last_release_measurement(Term::Nil).steps > 0);
/// ```
fn shared_last_release_measurement(graph: Term) -> Measurement {
    let mut scheduler = scheduler_with(graph.clone());
    assert_eq!(
        scheduler.schedule_next().expect("the step runs"),
        Turn::Run,
        "the root process runs"
    );
    let (child, _) = scheduler
        .spawn(
            Start::Entry(frame()),
            vec![graph],
            false,
            false,
            false,
            None,
        )
        .expect("the spawn runs")
        .expect("the spawn is permitted");
    assert_ne!(child, ProcessId(0));
    // The root process exits first and drops one holder of the graph. Its own
    // release work is not part of the measured release.
    scheduler.finish_current(Term::static_atom("normal"), None);
    drain(&mut scheduler);
    assert_eq!(
        scheduler.schedule_next().expect("the step runs"),
        Turn::Run,
        "the child process runs"
    );
    let start = Instant::now();
    scheduler.finish_current(Term::static_atom("normal"), None);
    let exit = start.elapsed();
    let (maximum, total, batches, peak) = drain(&mut scheduler);
    let retained_terms = peak.max(retained(&scheduler));
    let retained_bytes = retained_bytes(&scheduler);
    black_box(&scheduler);
    Measurement {
        max_step: exit.max(maximum),
        total: exit + total,
        steps: 1 + batches,
        retained_terms,
        retained_bytes,
    }
}

/// Return the measurement of the drop of the scheduler of a run whose root
/// process is live and owns `graph`. No process waits to run at that point.
///
/// # Examples
///
/// ```rust,ignore
/// assert!(shutdown_measurement(list()).max_step.as_nanos() > 0);
/// ```
fn shutdown_measurement(graph: Term) -> Measurement {
    let scheduler = scheduler_with(graph);
    let start = Instant::now();
    drop(scheduler);
    let pause = start.elapsed();
    Measurement {
        max_step: pause,
        total: pause,
        steps: 1,
        retained_terms: 0,
        retained_bytes: 0,
    }
}

/// Return the load of the host, or `unknown` when the host shows no source.
/// [`std::fs`] and [`std::process`] are safe, so the measurement needs no
/// foreign call. Linux keeps the load in `/proc/loadavg`, and macOS and the
/// BSDs in the `vm.loadavg` system control.
///
/// # Examples
///
/// ```rust,ignore
/// assert!(!load().is_empty());
/// ```
fn load() -> String {
    if let Ok(text) = std::fs::read_to_string("/proc/loadavg") {
        return text
            .split_whitespace()
            .take(3)
            .collect::<Vec<_>>()
            .join(" ");
    }
    match std::process::Command::new("sysctl")
        .args(["-n", "vm.loadavg"])
        .output()
    {
        Ok(output) => String::from_utf8_lossy(&output.stdout).trim().to_string(),
        Err(_) => "unknown".to_string(),
    }
}

/// One measured path: its name in the report and the function that returns
/// the measurement of one run on one graph.
type Path = (&'static str, fn(Term) -> Measurement);

/// Return the paths of the report, in table order.
///
/// # Examples
///
/// ```rust,ignore
/// assert_eq!(paths()[0].0, "synchronous last release");
/// ```
fn paths() -> Vec<Path> {
    vec![
        ("synchronous last release", synchronous_measurement),
        ("process exit", exit_measurement),
        ("shared last release", shared_last_release_measurement),
        ("shutdown", shutdown_measurement),
    ]
}

/// Print the pause and the load of each timed run of each path of each shape,
/// and the maximum step pause, the total pause, the throughput, and the
/// retained memory of each path.
#[test]
fn records_the_release_pause_of_a_large_graph() {
    println!(
        "RELEASE scheduler release of a large shared graph: one timed run of one path of one shape, in microsecond, with the load of the run."
    );
    println!("RELEASE runs: shape | path | run | load | max step pause us");
    // One entry for each (shape, path) in the order of the two loops below.
    let mut measurements: Vec<Vec<Measurement>> = (0..shapes().len() * paths().len())
        .map(|_| Vec::new())
        .collect();
    let path_count = paths().len();
    for (shape_index, shape) in shapes().into_iter().enumerate() {
        for (path_index, (path, measure)) in paths().into_iter().enumerate() {
            for run in 0..RUNS {
                let graph = (shape.build)();
                let run_load = load();
                let measurement = measure(graph);
                println!(
                    "RELEASE run | {} | {} | {} | {} | {:.3}",
                    shape.name,
                    path,
                    run,
                    run_load,
                    measurement.max_step.as_secs_f64() * 1e6
                );
                measurements[shape_index * path_count + path_index].push(measurement);
            }
        }
    }
    println!();
    println!(
        "RELEASE summary: shape | nodes | path | max step pause us | total pause ms | steps | M nodes/s | retained terms | retained bytes"
    );
    for (shape_index, shape) in shapes().into_iter().enumerate() {
        for (path_index, (path, _)) in paths().into_iter().enumerate() {
            let runs = &measurements[shape_index * path_count + path_index];
            let max_step = runs
                .iter()
                .map(|run| run.max_step)
                .max()
                .expect("a path has runs");
            let total = runs
                .iter()
                .map(|run| run.total)
                .max()
                .expect("a path has runs");
            let steps = runs.iter().map(|run| run.steps).max().unwrap_or(0);
            let retained_terms = runs.iter().map(|run| run.retained_terms).max().unwrap_or(0);
            let retained_bytes = runs.iter().map(|run| run.retained_bytes).max().unwrap_or(0);
            println!(
                "RELEASE summary | {} | {} | {} | {:.3} | {:.3} | {} | {:.2} | {} | {}",
                shape.name,
                shape.nodes,
                path,
                max_step.as_secs_f64() * 1e6,
                total.as_secs_f64() * 1e3,
                steps,
                shape.nodes as f64 / total.as_secs_f64() / 1e6,
                retained_terms,
                retained_bytes
            );
        }
    }
}

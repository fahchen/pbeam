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
            ProcessState, Start,
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

/// The number of entries of the process dictionary of the report, which holds
/// two terms for each entry.
const DICTIONARY_ENTRIES: usize = 100_000;

/// The number of messages of the mailbox of the report, which holds one term
/// for each message.
const MAILBOX_MESSAGES: usize = 100_000;

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

/// One workload of the report: its name, the number of terms that its release
/// moves, the term that the synchronous path releases, and the way to give the
/// workload to a process.
struct Shape {
    /// The name of the shape in the report.
    name: &'static str,
    /// The number of terms that the release of one workload moves: one cons
    /// cell for a list, one tuple for a chain, one internal tuple for a tree,
    /// two terms for each dictionary entry, and one term for each message.
    nodes: usize,
    /// Build one workload outside a process, for the synchronous last release
    /// of a term and for the shared paths. A container workload builds a flat
    /// tuple with as many elements as the container holds terms.
    build: fn() -> Term,
    /// Give one workload to `process`, so that its exit releases it. A term
    /// workload puts the term of `build` in the X registers; a container
    /// workload fills the container of the process.
    install: fn(&mut ProcessState, fn() -> Term),
    /// True when two processes can hold one workload of this shape, so that
    /// the report measures the shared last release of it. A container belongs
    /// to one process, so it has no shared path.
    shared: bool,
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

/// Return one flat tuple of `2 * DICTIONARY_ENTRIES` elements, which holds as
/// many terms as the dictionary workload of the report.
///
/// # Examples
///
/// ```rust,ignore
/// assert!(matches!(dictionary_terms(), Term::Tuple(_)));
/// ```
fn dictionary_terms() -> Term {
    Term::tuple(
        (0..(2 * DICTIONARY_ENTRIES) as i64)
            .map(Term::Integer)
            .collect(),
    )
}

/// Return one flat tuple of [`MAILBOX_MESSAGES`] elements, which holds as many
/// terms as the mailbox workload of the report.
///
/// # Examples
///
/// ```rust,ignore
/// assert!(matches!(mailbox_terms(), Term::Tuple(_)));
/// ```
fn mailbox_terms() -> Term {
    Term::tuple((0..MAILBOX_MESSAGES as i64).map(Term::Integer).collect())
}

/// Put the term of `build` in the X registers of `process`.
///
/// # Examples
///
/// ```rust,ignore
/// let mut process = ProcessState::new(ProcessId(0), Start::Entry(frame()), vec![]);
/// install_registers(&mut process, list);
/// assert_eq!(process.registers.len(), 1);
/// ```
fn install_registers(process: &mut ProcessState, build: fn() -> Term) {
    process.registers = vec![Some(build())];
}

/// Fill the process dictionary of `process` with [`DICTIONARY_ENTRIES`]
/// entries of two terms.
///
/// # Examples
///
/// ```rust,ignore
/// let mut process = ProcessState::new(ProcessId(0), Start::Entry(frame()), vec![]);
/// install_dictionary(&mut process, dictionary_terms);
/// assert_eq!(process.dictionary.len(), DICTIONARY_ENTRIES);
/// ```
fn install_dictionary(process: &mut ProcessState, _build: fn() -> Term) {
    process.dictionary = (0..DICTIONARY_ENTRIES as i64)
        .map(|key| (Term::Integer(key), Term::Integer(key)))
        .collect();
}

/// Fill the mailbox of `process` with [`MAILBOX_MESSAGES`] messages.
///
/// # Examples
///
/// ```rust,ignore
/// let mut process = ProcessState::new(ProcessId(0), Start::Entry(frame()), vec![]);
/// install_mailbox(&mut process, mailbox_terms);
/// assert!(!process.mailbox.is_empty());
/// ```
fn install_mailbox(process: &mut ProcessState, _build: fn() -> Term) {
    for message in 0..MAILBOX_MESSAGES as i64 {
        process.mailbox.push(Term::Integer(message));
    }
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
            install: install_registers,
            shared: true,
        },
        Shape {
            name: "chain of 1000000 levels",
            nodes: CELLS,
            build: chain,
            install: install_registers,
            shared: true,
        },
        Shape {
            name: "tuple tree of 1048575 cells",
            nodes: 2 * TREE_LEAVES - 1,
            build: tree,
            install: install_registers,
            shared: true,
        },
        Shape {
            name: "flat tuple of 1000000 elements, one node",
            nodes: 1,
            build: flat_tuple,
            install: install_registers,
            shared: true,
        },
        Shape {
            name: "map of 20000 keys, one walk",
            nodes: MAP_KEYS,
            build: map,
            install: install_registers,
            shared: true,
        },
        Shape {
            name: "dictionary of 100000 entries",
            nodes: 2 * DICTIONARY_ENTRIES,
            build: dictionary_terms,
            install: install_dictionary,
            shared: false,
        },
        Shape {
            name: "mailbox of 100000 messages",
            nodes: MAILBOX_MESSAGES,
            build: mailbox_terms,
            install: install_mailbox,
            shared: false,
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

/// Return a scheduler whose root process holds one workload of `shape`.
///
/// # Examples
///
/// ```rust,ignore
/// let scheduler = scheduler_with(&shapes()[0]);
/// assert!(scheduler.processes.contains_key(&ProcessId(0)));
/// ```
fn scheduler_with(shape: &Shape) -> Scheduler {
    let mut scheduler = test_scheduler();
    scheduler
        .start(frame(), vec![], allow_all())
        .expect("the root process starts");
    let process = scheduler
        .processes
        .get_mut(&ProcessId(0))
        .expect("the root process is in the table");
    (shape.install)(process, shape.build);
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

/// Return the number of units that the release work list of `scheduler`
/// owns: one for each term, element, register, Y slot, or dictionary pair, and
/// one for a mailbox, a monitor tree, or a map while it holds a term. The
/// count is exact for the terms and the elements, and it is one unit for a
/// container whose terms this module cannot count.
///
/// # Examples
///
/// ```rust,ignore
/// assert_eq!(retained(&test_scheduler()), 0);
/// ```
fn retained(scheduler: &Scheduler) -> usize {
    scheduler.release.iter().map(held_by).sum()
}

/// Return the number of units that one item of the release work list owns.
///
/// # Examples
///
/// ```rust,ignore
/// assert_eq!(held_by(&Release::Term(Term::Integer(1))), 1);
/// ```
fn held_by(item: &Release) -> usize {
    match item {
        Release::Term(_) => 1,
        Release::Terms(group) => group.len(),
        Release::Map(drain) => usize::from(!drain.is_empty()),
        Release::Registers(registers) => registers.len(),
        Release::Frames(frames) => frames.len(),
        Release::Slots(slots) => slots.len(),
        Release::Dictionary(dictionary) => 2 * dictionary.len(),
        Release::Mailbox(mailbox) => usize::from(!mailbox.is_empty()),
        Release::Monitors(monitors) => usize::from(!monitors.is_empty()),
    }
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

/// Return the measurement of the direct drop of the only holder of the term
/// that `shape` builds.
///
/// # Examples
///
/// ```rust,ignore
/// assert!(synchronous_measurement(&shapes()[0]).max_step.as_nanos() > 0);
/// ```
fn synchronous_measurement(shape: &Shape) -> Measurement {
    // The build stays outside the timer, so the pause is the release only.
    let graph = (shape.build)();
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

/// Return the measurement of the exit of a process that holds one workload of
/// `shape`.
///
/// # Examples
///
/// ```rust,ignore
/// assert!(exit_measurement(&shapes()[0]).total.as_nanos() > 0);
/// ```
fn exit_measurement(shape: &Shape) -> Measurement {
    let mut scheduler = scheduler_with(shape);
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

/// Return the measurement of the exit of the second process that shares the
/// graph of `shape`. The root process exits first and drops one holder; the
/// child process then exits and releases the graph. Only a shape that two
/// processes can hold has this path.
///
/// # Examples
///
/// ```rust,ignore
/// assert!(shared_last_release_measurement(&shapes()[0]).steps > 0);
/// ```
fn shared_last_release_measurement(shape: &Shape) -> Measurement {
    let graph = (shape.build)();
    let mut scheduler = test_scheduler();
    scheduler
        .start(frame(), vec![graph.clone()], allow_all())
        .expect("the root process starts");
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
/// process is live and holds one workload of `shape`. No process waits to run
/// at that point.
///
/// # Examples
///
/// ```rust,ignore
/// assert!(shutdown_measurement(&shapes()[0]).max_step.as_nanos() > 0);
/// ```
fn shutdown_measurement(shape: &Shape) -> Measurement {
    let scheduler = scheduler_with(shape);
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

/// One measured path: its name in the report, true when a shape that two
/// processes can hold has this path, and the function that returns the
/// measurement of one run of one shape.
struct Path {
    /// The name of the path in the report.
    name: &'static str,
    /// True for the shared last release, which needs two processes.
    shared: bool,
    /// Measure one run of one shape.
    measure: fn(&Shape) -> Measurement,
}

/// Return the paths of the report, in table order.
///
/// # Examples
///
/// ```rust,ignore
/// assert_eq!(paths()[0].name, "synchronous last release");
/// ```
fn paths() -> Vec<Path> {
    vec![
        Path {
            name: "synchronous last release",
            shared: false,
            measure: synchronous_measurement,
        },
        Path {
            name: "process exit",
            shared: false,
            measure: exit_measurement,
        },
        Path {
            name: "shared last release",
            shared: true,
            measure: shared_last_release_measurement,
        },
        Path {
            name: "shutdown",
            shared: false,
            measure: shutdown_measurement,
        },
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
    // One entry for each (shape, path) that the loops below measure.
    let mut measurements: Vec<(usize, usize, Vec<Measurement>)> = Vec::new();
    for (shape_index, shape) in shapes().into_iter().enumerate() {
        for (path_index, path) in paths().into_iter().enumerate() {
            // A container belongs to one process, so it has no shared path.
            if path.shared && !shape.shared {
                continue;
            }
            let mut runs = Vec::new();
            for run in 0..RUNS {
                let run_load = load();
                let measurement = (path.measure)(&shape);
                println!(
                    "RELEASE run | {} | {} | {} | {} | {:.3}",
                    shape.name,
                    path.name,
                    run,
                    run_load,
                    measurement.max_step.as_secs_f64() * 1e6
                );
                runs.push(measurement);
            }
            measurements.push((shape_index, path_index, runs));
        }
    }
    println!();
    println!(
        "RELEASE summary: shape | nodes | path | max step pause us | total pause ms | steps | M nodes for each second | retained units | retained bytes"
    );
    let shapes = shapes();
    let paths = paths();
    for (shape_index, path_index, runs) in &measurements {
        let shape = &shapes[*shape_index];
        let path = &paths[*path_index];
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
            path.name,
            max_step.as_secs_f64() * 1e6,
            total.as_secs_f64() * 1e3,
            steps,
            shape.nodes as f64 / total.as_secs_f64() / 1e6,
            retained_terms,
            retained_bytes
        );
    }
}

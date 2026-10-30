//! The peak live bytes of one Terra scene, the memory limb of row 6 of
//! `docs/term-performance-design-codex-v2.md` and of `docs/term-layout-design.md`
//! section 5.
//!
//! The test holds one scene of the Terra Counter target
//! (`fixtures/term_live_bytes/term_live_bytes_probe.erl`) and runs it with
//! `BeamProgram::execute_sampled`. The sampler reports the bytes of the
//! guest containers at each scheduling step, so the test records the peak
//! of that number without a global allocator and without `unsafe` code. The
//! peak is the maximum of the sampled live bytes. The bytes added and the
//! bytes released are the changes after the baseline, the first sample, so
//! the test reports the baseline, the totals of both, and the counts of the
//! steps that added and released bytes.
//!
//! The test reports a second, independent number for the same run: the peak
//! resident set size of the test process, sampled from `ps -o rss` by a
//! thread. The two numbers measure different things, and the test states
//! the relation instead of claiming that either is exact.
//!
//! Run the harness on a release build with `mise run bench-live-bytes`. The
//! check gate runs the same test with the test profile.
#![expect(
    clippy::unwrap_used,
    reason = "test helpers report a failure by a panic"
)]

use std::{
    process::Command,
    sync::{
        Arc,
        atomic::{AtomicBool, AtomicUsize, Ordering},
    },
    thread,
    time::Duration,
};

use pbeam_runtime::{BeamProgram, HostTerm, Term};

#[path = "../common/mod.rs"]
mod common;

use common::pinned_beam;

const SCENE: &[u8] = include_bytes!("../../fixtures/term_live_bytes/term_live_bytes_probe.beam");

const RELEASE_PROBE: &[u8] = include_bytes!("../../fixtures/release/release_probe.beam");

/// The number of cells of the graph of the release regression test.
const RELEASE_CELLS: i64 = 20_000;

/// The live bytes of one measured run.
#[derive(Debug, Default, PartialEq, Eq)]
struct LiveBytes {
    /// The largest sample of the live bytes.
    peak: usize,
    /// The total of the increases between two samples.
    added: usize,
    /// The total of the decreases between two samples.
    released: usize,
    /// The number of samples that added bytes.
    increases: usize,
    /// The number of samples that released bytes.
    decreases: usize,
    /// The number of samples.
    samples: usize,
    /// The first sample, the baseline of the run.
    first: Option<usize>,
    /// The previous sample.
    previous: Option<usize>,
}

impl LiveBytes {
    /// Record one sample of the live bytes.
    fn observe(&mut self, live: usize) {
        self.samples += 1;
        self.first.get_or_insert(live);
        self.peak = self.peak.max(live);
        if let Some(previous) = self.previous {
            if live > previous {
                self.added += live - previous;
                self.increases += 1;
            } else if live < previous {
                self.released += previous - live;
                self.decreases += 1;
            }
        }
        self.previous = Some(live);
    }
}

/// Run the scene and return its result with the live-byte report of the run.
fn run(program: &BeamProgram) -> (HostTerm, LiveBytes) {
    let mut report = LiveBytes::default();
    let result = program
        .execute_sampled("term_live_bytes_probe", "scene", &[], &mut |live| {
            report.observe(live);
        })
        .expect("the scene runs");
    (result, report)
}

/// Return the resident set size of this process in kilobytes, as
/// `ps -o rss=` gives it, or 0 when the command fails.
fn rss_kilobytes() -> usize {
    let pid = std::process::id().to_string();
    let output = Command::new("ps").args(["-o", "rss=", "-p", &pid]).output();
    match output {
        Ok(output) => String::from_utf8_lossy(&output.stdout)
            .trim()
            .parse()
            .unwrap_or(0),
        Err(_) => 0,
    }
}

/// A thread that samples the resident set size of this process at a short
/// interval and keeps the maximum.
struct RssSampler {
    stop: Arc<AtomicBool>,
    peak: Arc<AtomicUsize>,
    handle: thread::JoinHandle<()>,
}

impl RssSampler {
    /// Start the sampler.
    fn start() -> Self {
        let stop = Arc::new(AtomicBool::new(false));
        let peak = Arc::new(AtomicUsize::new(0));
        let handle = {
            let stop = Arc::clone(&stop);
            let peak = Arc::clone(&peak);
            thread::spawn(move || {
                let pid = std::process::id().to_string();
                let mut command = Command::new("ps");
                command.args(["-o", "rss=", "-p", &pid]);
                while !stop.load(Ordering::Relaxed) {
                    if let Ok(output) = command.output()
                        && let Ok(kilobytes) =
                            String::from_utf8_lossy(&output.stdout).trim().parse()
                    {
                        peak.fetch_max(kilobytes, Ordering::Relaxed);
                    }
                    thread::sleep(Duration::from_millis(1));
                }
            })
        };
        Self { stop, peak, handle }
    }

    /// Stop the sampler and return its maximum sample. The thread joins
    /// before the maximum is read, so a sample that an in-flight `ps`
    /// command publishes is part of the result.
    fn stop(self) -> usize {
        let Self { stop, peak, handle } = self;
        stop.store(true, Ordering::Relaxed);
        handle.join().expect("the sampler thread joins");
        peak.load(Ordering::Relaxed)
    }
}

/// Record the peak live bytes of one Terra scene, and report the resident
/// set size of the same run as a second, independent number.
#[test]
fn records_the_peak_live_bytes_of_one_terra_scene() {
    let program = BeamProgram::load(&[SCENE]).unwrap();
    let before = rss_kilobytes();
    let sampler = RssSampler::start();
    let (_first_result, first) = run(&program);
    let peak_sample = sampler.stop();
    let after = rss_kilobytes();
    let rss_peak = before.max(after).max(peak_sample);
    // A second run of the same revision gives the spread of the number.
    let (_second_result, second) = run(&program);

    println!("baseline live bytes: {}", first.first.unwrap_or(0));
    println!("peak live bytes: {}", first.peak);
    println!(
        "added {} bytes in {} samples, released {} bytes in {} samples, {} samples",
        first.added, first.increases, first.released, first.decreases, first.samples
    );
    println!("second run peak live bytes: {}", second.peak);
    println!("spread: {} bytes", first.peak.abs_diff(second.peak));
    println!("peak resident set size: {rss_peak} KB");

    assert!(first.peak > 0, "the scene holds live bytes");
    assert_eq!(
        first, second,
        "two runs of the same revision must agree within the printed spread of 0 bytes"
    );
    assert!(
        rss_peak > 0,
        "the resident set size sample must be a number"
    );
    // The resident set size holds the binary, the loaded code, and the
    // allocator arenas of the test process, so it is not the live guest
    // bytes. It can be above the live bytes or below a short peak, and
    // neither number bounds the other. The check above only shows that the
    // sampling produced a value.
}

/// Record the samples of the release work list across the exit of a child and
/// the drain of its graph.
///
/// `release_probe:waits/1` spawns a child that keeps a list of
/// [`RELEASE_CELLS`] cells in its process dictionary and exits. The exit
/// moves the graph to the release work list of the scheduler, and the drain
/// releases it in batches while the root waits for a message that never
/// arrives. A walk that does not visit the release work list reports no bytes
/// for the graph after the sample of the peak, although the scheduler still
/// owns it.
#[test]
fn counts_the_release_work_of_an_exited_child_across_its_drain() {
    let erlang = pinned_beam("erlang");
    let program = BeamProgram::load(&[&erlang, RELEASE_PROBE]).unwrap();
    let mut samples = Vec::new();
    program
        .execute_sampled(
            "release_probe",
            "waits",
            &[program.host_term(Term::Integer(RELEASE_CELLS))],
            &mut |live| samples.push(live),
        )
        .expect_err("the run ends when the root waits for a message that cannot arrive");

    let peak = samples.iter().copied().max().unwrap_or(0);
    assert!(
        peak > 0,
        "the child builds a graph of live bytes: {samples:?}"
    );
    let at_peak = samples
        .iter()
        .position(|sample| *sample == peak)
        .expect("the samples hold the peak");
    // The graph waits on the release work list while the drain releases it in
    // batches, so more than one sample after the peak still holds most of it.
    let counted = samples[at_peak + 1..]
        .iter()
        .filter(|sample| **sample > peak / 2)
        .count();
    assert!(
        counted >= 2,
        "the graph stays counted across the drain steps: peak {peak}, {counted} of {} samples after the peak",
        samples.len() - at_peak - 1
    );
}

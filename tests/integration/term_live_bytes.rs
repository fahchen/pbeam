//! The peak live bytes of one Terra scene, the memory limb of row 6 of
//! `docs/term-performance-design-codex-v2.md` and of `docs/term-layout-design.md`
//! section 5.
//!
//! The test holds one scene of the Terra Counter target
//! (`fixtures/term_live_bytes/term_live_bytes_probe.erl`) and runs it with
//! `BeamProgram::execute_sampled`. The sampler reports the bytes of the
//! guest containers at each scheduling step, so the test records the peak
//! of that number without a global allocator and without `unsafe` code. The
//! peak is the maximum of the bytes added less the bytes released, with the
//! totals of both and the counts of the steps that added and released
//! bytes.
//!
//! The test reports a second, independent number for the same run: the peak
//! resident set size of the test process, sampled from `ps -o rss` by a
//! thread. The two numbers measure different things, and the test states
//! the relation instead of claiming that either is exact.
//!
//! Run the harness on a release build with `mise run bench-live-bytes`. The
//! check gate runs the same test with the test profile.

use std::{
    process::Command,
    sync::{
        Arc,
        atomic::{AtomicBool, AtomicUsize, Ordering},
    },
    thread,
    time::Duration,
};

use pbeam_runtime::{BeamProgram, Term};

const SCENE: &[u8] = include_bytes!("../../fixtures/term_live_bytes/term_live_bytes_probe.beam");

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
fn run(program: &BeamProgram) -> (Term, LiveBytes) {
    let mut report = LiveBytes::default();
    let result = program
        .execute_sampled("term_live_bytes_probe", "scene", &[], &mut |live| {
            report.observe(live)
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

    /// Stop the sampler and return its maximum sample.
    fn stop(self) -> usize {
        self.stop.store(true, Ordering::Relaxed);
        let peak = self.peak.load(Ordering::Relaxed);
        self.handle.join().expect("the sampler thread joins");
        peak
    }
}

/// Record the peak live bytes of one Terra scene, and report the resident
/// set size of the same run as a second, independent number.
#[test]
fn records_the_peak_live_bytes_of_one_terra_scene() {
    let program = BeamProgram::load(&[SCENE]).unwrap();
    let before = rss_kilobytes();
    let sampler = RssSampler::start();
    let (first_result, first) = run(&program);
    let sampled = sampler.stop();
    let after = rss_kilobytes();
    let rss_peak = before.max(after).max(sampled);
    // A second run of the same revision gives the spread of the number.
    let (second_result, second) = run(&program);

    println!("baseline live bytes: {}", first.first.unwrap_or(0));
    println!("peak live bytes: {}", first.peak);
    println!(
        "added {} bytes in {} samples, released {} bytes in {} samples, {} samples",
        first.added, first.increases, first.released, first.decreases, first.samples
    );
    println!("second run peak live bytes: {}", second.peak);
    println!("spread: {} bytes", first.peak.abs_diff(second.peak));
    println!("peak resident set size: {rss_peak} KB");

    assert_eq!(
        first_result, second_result,
        "the scene must give the same result in both runs"
    );
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
    // bytes and neither number bounds the other.
    assert!(
        rss_peak >= first.peak / 1024,
        "the resident set size sample covers the process of the run"
    );
}

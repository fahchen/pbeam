//! The measurement of the two small tables that the run scans.
//!
//! The run searches no `BIFS` entry: the loader stores the identity of a BIF
//! in the instruction. Two linear scans stay on the path of an external
//! call, [`PBEAM_BIFS`](super::PBEAM_BIFS) with 5 entries and
//! [`HEAVY_BIFS`](super::HEAVY_BIFS) with 7 entries. This measurement bounds
//! the cost of each scan. The `call_ext_only` shape of
//! `tests/integration/dispatch_baseline.rs` cannot bound it: the spread of
//! that shape over the runs of one revision is larger than one scan.
//!
//! Run `cargo test --release --lib -- bif::tests -- --nocapture` and read
//! the `SCAN` lines. The test asserts no time, because the load of the host
//! changes it.

use super::{is_heavy_bif, is_pbeam_bif};
use std::{hint::black_box, time::Instant};

/// The number of calls of each measurement.
const CALLS: u64 = 20_000_000;

/// Return the nanoseconds of one call of `is_pbeam_bif` over [`CALLS`]
/// calls.
///
/// # Examples
///
/// ```rust,ignore
/// let ns = measure_pbeam("dispatch_probe", "floor_ext_loop", 1);
/// assert!(ns > 0.0);
/// ```
fn measure_pbeam(module: &str, name: &str, arity: u32) -> f64 {
    let start = Instant::now();
    let mut hits = 0u64;
    for _ in 0..CALLS {
        if is_pbeam_bif(black_box(module), black_box(name), black_box(arity)) {
            hits += 1;
        }
    }
    black_box(hits);
    start.elapsed().as_nanos() as f64 / CALLS as f64
}

/// Return the nanoseconds of one call of `is_heavy_bif` over [`CALLS`]
/// calls.
///
/// # Examples
///
/// ```rust,ignore
/// let ns = measure_heavy("erlang", "put", 2);
/// assert!(ns > 0.0);
/// ```
fn measure_heavy(module: &str, name: &str, arity: u32) -> f64 {
    let start = Instant::now();
    let mut hits = 0u64;
    for _ in 0..CALLS {
        if is_heavy_bif(black_box(module), black_box(name), black_box(arity)) {
            hits += 1;
        }
    }
    black_box(hits);
    start.elapsed().as_nanos() as f64 / CALLS as f64
}

/// Print the cost of each scan for the shape that reads it.
///
/// The `call_ext_only` shape of `tests/integration/dispatch_baseline.rs`
/// calls `dispatch_probe:floor_ext_loop/1`, which is module code, so the run
/// reads `is_pbeam_bif` with an MFA of a guest module and no entry matches.
/// A `call_ext` of a BIF reads `is_heavy_bif` instead, because the identity
/// of the entry is already in the instruction and `is_pbeam_bif` does not
/// run for it.
#[test]
fn prints_the_cost_of_the_two_classification_scans() {
    println!(
        "SCAN is_pbeam_bif(dispatch_probe:floor_ext_loop/1) {:.2} ns",
        measure_pbeam("dispatch_probe", "floor_ext_loop", 1)
    );
    println!(
        "SCAN is_heavy_bif(dispatch_probe:floor_ext_loop/1) {:.2} ns",
        measure_heavy("dispatch_probe", "floor_ext_loop", 1)
    );
    println!(
        "SCAN is_heavy_bif(erlang:put/2) {:.2} ns",
        measure_heavy("erlang", "put", 2)
    );
    println!(
        "SCAN is_heavy_bif(erlang:self/0) {:.2} ns",
        measure_heavy("erlang", "self", 0)
    );
    println!(
        "SCAN is_pbeam_bif(pbeam_cap:serve/2) {:.2} ns",
        measure_pbeam("pbeam_cap", "serve", 2)
    );
}

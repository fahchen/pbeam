//! The measurement of the two small tables that the run scans.
//!
//! The run searches no `BIFS` entry: the loader stores the identity of a BIF
//! in the instruction or in the import. Two linear scans classify each import
//! when the module loads ([`import_kind`](super::import_kind)),
//! [`PBEAM_BIFS`](super::PBEAM_BIFS) with 5 entries and
//! [`HEAVY_BIFS`](super::HEAVY_BIFS) with 7 entries, and an external call
//! reads the result. This measurement bounds the cost of each scan. Before
//! task [#628](https://github.com/fahchen/pbeam/issues/628), each external
//! call ran them. The `call_ext_only` shape of
//! `tests/integration/dispatch_baseline.rs` could not bound them: the spread
//! of that shape over the runs of one revision is larger than one scan.
//!
//! Run `cargo test --release --lib -- bif::tests -- --nocapture` and read
//! the `SCAN` lines. The test asserts no time, because the load of the host
//! changes it.

use super::{
    BifContext, BifId, BifResult, dispatch_leaf, import_kind, is_heavy_bif, is_leaf, is_pbeam_bif,
    mfa, resolve,
};
use crate::{
    beam::ImportKind,
    runtime::scheduler::Scheduler,
    term::{AtomTable, Map, Term},
};
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

/// Print the cost of each scan for the import that reads it.
///
/// The `call_ext_only` shape of `tests/integration/dispatch_baseline.rs`
/// calls `dispatch_probe:floor_ext_loop/1`, which is module code, so the
/// loader reads `is_pbeam_bif` with an MFA of a guest module and no entry
/// matches. The import of a BIF of `BIFS` reads `is_heavy_bif` too.
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

/// Assert the call that the loader decides for each kind of import, in the
/// order of the checks of the removed call-time classification:
/// `erlang:load_nif/2` before the BIFs, `erlang:apply/2,3` apart from the
/// BIFs, and a heavy BIF apart from a light BIF (`jit/arm/ops.tab:615-714`).
#[test]
fn classifies_each_kind_of_import() {
    let cases = [
        (("erlang", "apply", 2), ImportKind::ApplyFun),
        (("erlang", "apply", 3), ImportKind::Apply),
        (("erlang", "load_nif", 2), ImportKind::LoadNif),
        (
            ("erlang", "whereis", 1),
            ImportKind::Bif {
                bif: Some(BifId::ErlangWhereis1),
                heavy: false,
            },
        ),
        (
            ("erlang", "put", 2),
            ImportKind::Bif {
                bif: resolve("erlang", "put", 2),
                heavy: true,
            },
        ),
        (
            ("pbeam_cap", "serve", 2),
            ImportKind::Bif {
                bif: None,
                heavy: false,
            },
        ),
        (("lists", "reverse", 1), ImportKind::Export),
    ];
    for ((module, name, arity), kind) in cases {
        assert_eq!(
            import_kind(module, name, arity),
            kind,
            "{module}:{name}/{arity}"
        );
    }
    assert!(resolve("erlang", "put", 2).is_some());
}

/// Assert that the leaf lane and the owned dispatch agree on each leaf
/// identity, and that the leaf predicate holds exactly for those
/// identities.
///
/// The leaf arms of [`dispatch_leaf`] and the entries of the `bifs!` table
/// hold the same identities in two places (`docs/bif-argument-borrow-design.md`).
/// This test holds them equal. It reaches each leaf arm, so a missing arm
/// fails a test.
#[test]
fn the_leaf_lane_matches_the_owned_dispatch_for_each_leaf_identity() {
    let cases = [
        (
            BifId::ErlangPlus2,
            "erlang",
            "+",
            2,
            vec![Term::Integer(1), Term::Integer(2)],
        ),
        (
            BifId::ErlangMinus2,
            "erlang",
            "-",
            2,
            vec![Term::Integer(3), Term::Integer(2)],
        ),
        (
            BifId::ErlangTupleSize1,
            "erlang",
            "tuple_size",
            1,
            vec![Term::tuple(vec![Term::Integer(1), Term::Integer(2)])],
        ),
        (
            BifId::ErlangElement2,
            "erlang",
            "element",
            2,
            vec![
                Term::Integer(2),
                Term::tuple(vec![Term::Integer(3), Term::Integer(4)]),
            ],
        ),
        (
            BifId::ErlangHd1,
            "erlang",
            "hd",
            1,
            vec![Term::list_from([Term::Integer(5)])],
        ),
        (
            BifId::ErlangMapGet2,
            "erlang",
            "map_get",
            2,
            vec![Term::Integer(1), Term::Map(Map::new())],
        ),
    ];
    for (id, module, name, arity, terms) in cases {
        assert_eq!(
            resolve(module, name, arity),
            Some(id),
            "{module}:{name}/{arity} must resolve to the leaf identity"
        );
        assert!(is_leaf(id), "{module}:{name}/{arity} must be a leaf");
        assert_eq!(mfa(id), (module, name, arity));
        let borrowed: Vec<&Term> = terms.iter().collect();
        let leaf_result = dispatch_leaf(id, &borrowed).expect("the leaf lane runs");
        let mut scheduler = Scheduler::new(AtomTable::new());
        let mut context = BifContext {
            scheduler: &mut scheduler,
        };
        let owned_result =
            super::table::dispatch(&mut context, id, &terms).expect("the owned dispatch runs");
        assert_eq!(
            format!("{leaf_result:?}"),
            format!("{owned_result:?}"),
            "{module}:{name}/{arity}"
        );
    }
}

/// Assert that the fallback arm of the leaf lane reports the same
/// unresolved MFA error as the fallback arm of the owned dispatch, and
/// that the leaf predicate rejects a non-leaf identity.
#[test]
fn dispatch_leaf_reports_an_identity_that_the_leaf_lane_does_not_hold() {
    assert!(!is_leaf(BifId::ErlangByteSize1));
    let error = dispatch_leaf(BifId::ErlangByteSize1, &[&Term::Integer(1)])
        .expect_err("a non-leaf identity has no leaf arm");
    assert_eq!(
        error.to_string(),
        "unresolved external MFA erlang:byte_size/1"
    );
}

/// Assert that the guard-failure path of the leaf arms that have one
/// raises the same exception as the owned dispatch.
#[test]
fn the_leaf_lane_keeps_the_guard_failure_of_each_leaf() {
    let cases: [(BifId, Term); 2] = [
        (BifId::ErlangTupleSize1, Term::Integer(1)),
        (BifId::ErlangHd1, Term::Nil),
    ];
    for (id, argument) in cases {
        let leaf = dispatch_leaf(id, &[&argument]).expect("the leaf lane runs");
        assert!(matches!(&leaf, BifResult::Raise { .. }));
        let mut scheduler = Scheduler::new(AtomTable::new());
        let mut context = BifContext {
            scheduler: &mut scheduler,
        };
        let owned = super::table::dispatch(&mut context, id, std::slice::from_ref(&argument))
            .expect("the owned dispatch runs");
        assert_eq!(format!("{leaf:?}"), format!("{owned:?}"));
    }
}

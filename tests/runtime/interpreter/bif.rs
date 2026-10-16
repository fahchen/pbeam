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
    runtime::process::{ProcessState, Start},
    runtime::scheduler::Scheduler,
    term::{Atom, AtomTable, Binary, Map, ProcessId, Term},
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
        let leaf_result = dispatch_leaf(id, &borrowed, &mut 0).expect("the leaf lane runs");
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
    let error = dispatch_leaf(BifId::ErlangByteSize1, &[&Term::Integer(1)], &mut 0)
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
        let leaf = dispatch_leaf(id, &[&argument], &mut 0).expect("the leaf lane runs");
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

/// Assert that `erlang:binary_to_list/1,3` read a binary that an append
/// built from its chunks and do not join them. The range of one byte that
/// `binary_to_list(Bin, 1, 1)` reads and the chunks of a call of
/// `binary_to_list/1` that traps give the bytes of the binary in order, as
/// ERTS `binary_to_list_chunk` does (`binary.c:201-256`).
#[test]
fn binary_to_list_reads_the_chunks_of_an_appended_binary_with_no_join() {
    let mut binary = Binary::from(vec![7]);
    let mut bytes = vec![7_u8];
    for step in 1..3_000_u32 {
        let suffix = vec![(step % 251) as u8; 1_000];
        binary = Binary::concat(&binary, &suffix);
        bytes.extend(&suffix);
    }
    assert!(binary.chunks().len() > 100);
    let first = super::erlang::b2l_cons(&binary, 0, 1, Term::nil());
    assert_eq!(first, Term::list_from([Term::Integer(7)]));
    assert!(!binary.is_joined());

    let start = Start::Undefined {
        module: Atom::from_static("erlang").unwrap(),
        function: Atom::from_static("binary_to_list").unwrap(),
    };
    let mut process = ProcessState::new(ProcessId(1), start, vec![]);
    let mut result =
        super::erlang::b2l_chunk(&mut process, &binary, 0, bytes.len(), Term::nil(), 4_000);
    let mut turns = 1;
    while let BifResult::TrapBif { arguments, .. } = result {
        let [
            Term::Binary(source),
            Term::Integer(offset),
            Term::Integer(rest),
            list,
        ] = arguments.as_slice()
        else {
            panic!("the trap keeps the binary, the offset, the rest, and the list");
        };
        result = super::erlang::b2l_chunk(
            &mut process,
            source,
            *offset as usize,
            *rest as usize,
            list.clone(),
            4_000,
        );
        turns += 1;
    }
    assert!(turns > 1);
    let BifResult::Value(list) = result else {
        panic!("the last chunk returns the list");
    };
    let codes = bytes.iter().map(|byte| Term::Integer(i64::from(*byte)));
    assert_eq!(list, Term::list_from(codes));
    assert!(!binary.is_joined());
}

/// Return a scheduler with the processes `0..count` and process 0 as the
/// current process, with a budget of `fcalls` reductions.
fn ptab_scheduler(count: u64, fcalls: i32) -> Scheduler {
    let mut scheduler = Scheduler::new(AtomTable::new());
    for id in 0..count {
        let start = Start::Undefined {
            module: Atom::from_static("erlang").unwrap(),
            function: Atom::from_static("ok").unwrap(),
        };
        scheduler.insert(ProcessState::new(ProcessId(id), start, vec![]));
    }
    scheduler.schedule_next().unwrap();
    scheduler.current_process_mut().fcalls = fcalls;
    scheduler
}

/// Return the state of the trap arguments of an `erlang:processes/0` call.
fn ptab_state(arguments: Vec<Term>) -> super::erlang::PtabList {
    let Ok(
        [
            Term::Integer(tix),
            found,
            gone,
            Term::Integer(pid_ix),
            result,
        ],
    ) = <[Term; 5]>::try_from(arguments)
    else {
        panic!("the trap has the five arguments of the state");
    };
    super::erlang::PtabList {
        tix,
        found,
        gone,
        pid_ix,
        result,
    }
}

/// Assert that each turn of `erlang:processes/0`, also the first, reads and
/// builds only the table entries and list cells that its budget pays for.
/// A turn reads at most one chunk of 1000 indices of the table, and builds
/// at most 75 result cells for each reduction (`erl_ptab.c:1006-1037`,
/// `1201-1254`). It does not read the whole table at the start, and it
/// does not copy the state of the call, which is in proportion to the
/// number of processes.
#[test]
fn each_turn_of_processes_touches_only_the_cells_of_its_budget() {
    use super::erlang::{PTAB_CELLS, processes, ptab_list};
    let count = 10_000;
    let mut scheduler = ptab_scheduler(count, 50);
    PTAB_CELLS.set(0);
    let mut result = processes(&mut scheduler).unwrap();
    let mut turns = 0;
    let list = loop {
        assert!(PTAB_CELLS.get() <= 1_000 + 75 * 50);
        let arguments = match result {
            BifResult::TrapBif { arguments, .. } => arguments,
            BifResult::Value(list) => break list,
            _ => panic!("the call traps or returns its result"),
        };
        let state = ptab_state(arguments);
        scheduler.current_process_mut().fcalls = 50;
        PTAB_CELLS.set(0);
        result = ptab_list(&mut scheduler, state).unwrap();
        turns += 1;
    };
    assert!(turns > 900);
    assert_eq!(
        list,
        Term::list_from((0..count).map(|id| Term::Pid(ProcessId(id))))
    );
}

/// Assert that `erlang:processes/0` steps through each process that left
/// the table during the read. A process that was in the table at the
/// start and left it before the call read its index comes after the found
/// pids, in the order of the exits. A process that entered the table after
/// the start is not in the result, but its exit costs a reduction
/// (`erl_ptab.c:1046-1170`). A step that charges 1 reduction more than the
/// budget traps, as ERTS clamps the reductions that remain at zero
/// (`erl_ptab.c:1176-1180`): ten exits cost 11 reductions with a budget of
/// 10.
#[test]
fn processes_traps_when_the_deleted_pids_use_more_than_the_budget() {
    use super::erlang::{processes, ptab_list};
    let mut scheduler = ptab_scheduler(1, 50);
    let start = || Start::Undefined {
        module: Atom::from_static("erlang").unwrap(),
        function: Atom::from_static("ok").unwrap(),
    };
    for id in 1001..1010 {
        scheduler.insert(ProcessState::new(ProcessId(id), start(), vec![]));
    }
    let BifResult::TrapBif { arguments, .. } = processes(&mut scheduler).unwrap() else {
        panic!("the call traps after the first chunk");
    };
    scheduler.insert(ProcessState::new(ProcessId(2000), start(), vec![]));
    for id in (1001..1010).rev().chain([2000]) {
        scheduler.remove_process(ProcessId(id));
    }
    let mut state = ptab_state(arguments);
    state.tix = 1 << 20;
    scheduler.current_process_mut().fcalls = 10;
    let BifResult::TrapBif { arguments, .. } = ptab_list(&mut scheduler, state).unwrap() else {
        panic!("the call traps");
    };
    let state = ptab_state(arguments);
    assert_eq!(state.tix, (1 << 20) + 1);
    scheduler.current_process_mut().fcalls = 4_000;
    let result = ptab_list(&mut scheduler, state).unwrap();
    let ids = [0].into_iter().chain((1001..1010).rev());
    let pids = Term::list_from(ids.map(|id| Term::Pid(ProcessId(id))));
    assert!(matches!(result, BifResult::Value(list) if list == pids));
}

/// Assert that only the oldest of two `erlang:processes/0` reads pays the
/// free cost of the processes that left the table during both reads. ERTS
/// frees a deleted entry only in the read whose invocation entry is first
/// in the deleted list, up to the next invocation entry
/// (`erl_ptab.c:1069-1099`, `1137-1143`). Ten exits cost 1 reduction in
/// the newer read, and 11 in the older read after the newer read ends.
#[test]
fn only_the_oldest_processes_read_pays_the_free_cost_of_the_deleted_pids() {
    use super::erlang::{processes, ptab_list};
    let mut scheduler = ptab_scheduler(2, 50);
    let trap = |result| match result {
        BifResult::TrapBif { arguments, .. } => ptab_state(arguments),
        _ => panic!("the call traps after the first chunk"),
    };
    let older = trap(processes(&mut scheduler).unwrap());
    scheduler.current_process_mut().fcalls = 0;
    scheduler.yield_current();
    scheduler.schedule_next().unwrap();
    scheduler.current_process_mut().fcalls = 50;
    let newer = trap(processes(&mut scheduler).unwrap());
    for id in 1001..=1010 {
        let start = Start::Undefined {
            module: Atom::from_static("erlang").unwrap(),
            function: Atom::from_static("ok").unwrap(),
        };
        scheduler.insert(ProcessState::new(ProcessId(id), start, vec![]));
        scheduler.remove_process(ProcessId(id));
    }
    let both = Term::list_from([0, 1].map(|id| Term::Pid(ProcessId(id))));
    let finish = |scheduler: &mut Scheduler, mut state: super::erlang::PtabList| {
        state.tix = 1 << 20;
        scheduler.current_process_mut().fcalls = 4_000;
        let result = ptab_list(scheduler, state).unwrap();
        assert!(matches!(result, BifResult::Value(ref list) if *list == both));
        4_000 - scheduler.current_process().fcalls
    };
    assert_eq!(scheduler.current_process().id, ProcessId(1));
    assert_eq!(finish(&mut scheduler, newer), 1);
    scheduler.current_process_mut().fcalls = 0;
    scheduler.yield_current();
    scheduler.schedule_next().unwrap();
    assert_eq!(scheduler.current_process().id, ProcessId(0));
    assert_eq!(finish(&mut scheduler, older), 11);
}

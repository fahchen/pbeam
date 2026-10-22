//! The node-wide system flags of a node with more than one scheduler
//! thread. ERTS keeps `backtrace_depth` (`erts_backtrace_depth`) and
//! `system_logger` once for the node, so a set on one scheduler is seen on
//! each scheduler.

use pbeam_runtime::{BeamProgram, Term};

/// The fixture with the roots. `pair_reference/0` runs the pair of roots on
/// the pinned ERTS.
const SYSTEM_FLAGS: &[u8] = include_bytes!("../../fixtures/multischeduler/system_flags.beam");

/// The number of runs of the pair, to exercise different interleavings of
/// the two threads.
const RUNS: usize = 10;

/// The result of the pair on the pinned ERTS with `erl +S 2`.
const REFERENCE: &str = "{8,{3,3,undefined}}\n";

/// A root reads the flags that the root of the other scheduler sets, and
/// the stack trace of the reader holds the depth of the other scheduler.
#[test]
fn reads_the_flags_of_the_other_scheduler() {
    let program = BeamProgram::load(&[SYSTEM_FLAGS]).expect("the fixture loads");
    let undefined = Term::Atom(
        program
            .atom("undefined")
            .expect("the fixture names undefined"),
    );
    for _ in 0..RUNS {
        let results = program
            .execute_on_schedulers(&[("system_flags", "setter"), ("system_flags", "reader")])
            .expect("the two schedulers run");
        assert_eq!(results[0].term, Term::Integer(8.into()), "the setter");
        assert_eq!(
            results[1].term,
            Term::tuple(vec![
                Term::Integer(3.into()),
                Term::Integer(3.into()),
                undefined.clone()
            ]),
            "the reader"
        );
    }
}

/// The pinned ERTS gives the same results with two schedulers.
#[test]
fn the_pinned_erts_gives_the_same_results() {
    let directory = std::env::temp_dir().join(format!(
        "pbeam-multischeduler-system-flags-{}",
        std::process::id()
    ));
    let _ = std::fs::remove_dir_all(&directory);
    std::fs::create_dir_all(&directory).expect("the directory of the reference run");
    std::fs::write(directory.join("system_flags.beam"), SYSTEM_FLAGS).expect("the fixture file");
    let output = std::process::Command::new("erl")
        .args(["+S", "2", "-noshell", "-eval"])
        .arg("io:format(\"~w~n\", [system_flags:pair_reference()]), halt().")
        .arg("-pa")
        .arg(&directory)
        .output()
        .expect("the pinned ERTS runs");
    std::fs::remove_dir_all(&directory).expect("the reference directory leaves");
    assert!(output.status.success(), "{output:?}");
    assert_eq!(String::from_utf8_lossy(&output.stdout), REFERENCE);
}

//! The resources of the node on two scheduler threads. A zlib stream that
//! one root opens resolves on the scheduler of the other root, as an ERTS
//! resource does (`enif_get_resource`, `erl_nif.c`).

#[path = "../common/mod.rs"]
mod common;

use common::pinned_beam;
use pbeam_runtime::{BeamProgram, Term};

/// The fixture with the two roots. `reference/0` runs them on the pinned
/// ERTS.
const NODE_RESOURCES: &[u8] = include_bytes!("../../fixtures/multischeduler/node_resources.beam");

/// The number of runs, to exercise different interleavings of the threads.
const RUNS: usize = 10;

/// The root `zlib_give` gives the control of its stream to the root of the
/// other scheduler. A use after that raises `not_on_controlling_process`,
/// and the other root deflates with the stream.
#[test]
fn a_zlib_stream_resolves_on_another_scheduler() {
    let zlib = pinned_beam("zlib");
    let program = BeamProgram::load(&[&zlib, NODE_RESOURCES]).expect("the fixture loads");
    let atom = |name| Term::Atom(program.atom(name).expect("the fixture names the atom"));
    for _ in 0..RUNS {
        let results = program
            .execute_on_schedulers(&[
                ("node_resources", "zlib_give"),
                ("node_resources", "zlib_use"),
            ])
            .expect("the two schedulers run");
        assert_eq!(results[0].term, atom("not_on_controlling_process"));
        assert_eq!(results[1].term, atom("true"));
    }
}

/// A worker that used its zlib stream can move to the idle scheduler and
/// use the stream there, and each stream gives back its data. ERTS does not
/// guarantee that a worker moves (`steal_reference/0` accepts each
/// boolean), so each run accepts `{ok, true}` and `{ok, false}`, and one run
/// at least must move a worker.
#[test]
fn a_process_that_used_a_zlib_stream_can_move() {
    let (zlib, lists) = (pinned_beam("zlib"), pinned_beam("lists"));
    let program = BeamProgram::load(&[&zlib, &lists, NODE_RESOURCES]).expect("the fixture loads");
    let atom = |name| Term::Atom(program.atom(name).expect("the fixture names the atom"));
    let mut moved_once = false;
    for _ in 0..RUNS {
        let results = program
            .execute_on_schedulers(&[
                ("node_resources", "steal_busy"),
                ("node_resources", "steal_idle"),
            ])
            .expect("the two schedulers run");
        let moved = [atom("true"), atom("false")].map(|moved| Term::tuple(vec![atom("ok"), moved]));
        assert!(moved.contains(&results[0].term), "{:?}", results[0].term);
        assert_eq!(results[1].term, atom("ok"));
        moved_once |= results[0].term == Term::tuple(vec![atom("ok"), atom("true")]);
    }
    assert!(moved_once, "no run moved a worker");
}

/// The two roots give the same results on the pinned ERTS with two
/// schedulers.
#[test]
fn the_pinned_erts_gives_the_same_results() {
    let directory =
        std::env::temp_dir().join(format!("pbeam-node-resources-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&directory);
    std::fs::create_dir_all(&directory).expect("the directory of the reference run");
    std::fs::write(directory.join("node_resources.beam"), NODE_RESOURCES)
        .expect("the fixture file");
    let output = std::process::Command::new("erl")
        .args(["+S", "2", "-noshell"])
        .args([
            "-eval",
            "io:format(\"~w~n~w~n\", [node_resources:reference(), node_resources:steal_reference()]), halt().",
        ])
        .arg("-pa")
        .arg(&directory)
        .output()
        .expect("the pinned ERTS runs");
    std::fs::remove_dir_all(&directory).expect("the reference directory leaves");
    assert!(output.status.success(), "{output:?}");
    assert_eq!(
        String::from_utf8_lossy(&output.stdout),
        "{not_on_controlling_process,true}\n{ok,ok}\n"
    );
}

/// The fixture with the two roots that share one TTY resource.
const TTY_RESOURCES: &[u8] = include_bytes!("../../fixtures/multischeduler/tty_resources.beam");

/// The stand-in `prim_tty` with the NIF stubs of the pinned module.
const PRIM_TTY: &[u8] = include_bytes!("../../fixtures/prim_tty/nif/prim_tty.beam");

/// A TTY resource that `prim_tty:tty_create/1` makes on one scheduler
/// resolves on the other scheduler: `tty_window_size/1` and `tty_select/2`
/// find it, as ERTS keeps one resource object for the node
/// (`prim_tty_nif.c:988-1072`). The pinned ERTS cannot load the stand-in
/// `prim_tty`, so this test checks only PBeam.
#[test]
fn a_tty_resource_resolves_on_another_scheduler() {
    let program = BeamProgram::load(&[PRIM_TTY, TTY_RESOURCES]).expect("the fixture loads");
    let atom = |name| Term::Atom(program.atom(name).expect("the fixture names the atom"));
    for _ in 0..RUNS {
        let results = program
            .execute_on_schedulers(&[
                ("tty_resources", "tty_create"),
                ("tty_resources", "tty_use"),
            ])
            .expect("the two schedulers run");
        assert_eq!(results[0].term, atom("ok"));
        assert_eq!(results[1].term, Term::tuple(vec![atom("ok"), atom("ok")]));
    }
}

//! Checks the scheduling of processes by priority on PBeam against the pinned ERTS with one
//! scheduler.
#![expect(
    clippy::unwrap_used,
    reason = "test helpers report a failure by a panic"
)]

#[path = "../common/mod.rs"]
mod common;

use std::fmt::Write as _;
use std::process::Command;

use common::pinned_beam;
use pbeam_runtime::{AtomTable, BeamProgram, Term};

const PRIORITY_PROBE: &[u8] = include_bytes!("../../fixtures/priority/priority_probe.beam");

/// The results of each function of `priority_probe`, as `~w` prints them,
/// measured on the pinned ERTS with `erl +S 1` (JIT).
const RESULTS: &[(&str, &str)] = &[
    // Each worker sets its priority in its first turn in the `normal`
    // queue. A `high` or `max` worker then runs all its turns, because
    // the other workers wait in the `normal` queue. A `low` worker goes
    // to the end of the `normal` queue at the head until it was there 8
    // times, so the `normal` workers end first.
    (
        "orders",
        "[{l1,1},{n1,1},{h1,6},{m1,6},{l2,1},{n2,1},{h2,6},{m2,6},{n1,1},{n2,1},{n1,1},{n2,1},\
         {n1,1},{n2,1},{n1,1},{n2,1},{n1,1},{n2,1},{l1,1},{l2,1},{l1,1},{l2,1},{l1,1},{l2,1},\
         {l1,1},{l2,1},{l1,1},{l2,1}]",
    ),
    // A `low` worker runs after 8 turns of the `normal` workers, and runs
    // at once when it is the last process of the queue.
    (
        "lows",
        "[[{l,1},{n1,1},{n2,1},{n3,1},{n1,1},{n2,1},{n3,1},{n1,1},{n2,1},{n3,1},{n1,1},{n2,1},\
         {n3,1},{n1,1},{n2,1},{n3,1},{n1,1},{n2,1},{n3,1},{n1,1},{n2,1},{n3,1},{n1,1},{n2,1},\
         {n3,1},{l,1},{n1,1},{n2,1},{n3,1},{n1,1},{n2,1},{n3,1},{n1,1},{n2,1},{n3,1},{n1,1},\
         {n2,1},{n3,1},{l,10}],[{l,1},{n,8},{l,1},{n,4},{l,10}],[{l1,1},{l2,1},{n,8},{l1,1},\
         {l2,1},{n,4},{l1,1},{l2,1},{l1,1},{l2,1},{l1,1},{l2,1},{l1,1},{l2,1},{l1,1},{l2,1},\
         {l1,1},{l2,1},{l1,1},{l2,1},{l1,1},{l2,1},{l1,1},{l2,1},{l1,1},{l2,1}],[{l,3}]]",
    ),
    // An exit that sends more `DOWN` messages than one budget permits
    // continues after the process table at the priority of the process: a
    // `low` exit waits for turns of the `normal` ticker and the parent, and
    // a `max` exit runs before them.
    ("exits", "[{low,21},{normal,1},{max,0}]"),
    // A message wakes a `max` process, but the sender continues its turn.
    ("wakes", "[high,max,high_after,normal,max,normal_after]"),
    (
        "flags",
        "[normal,high,high,low,{priority,max},max,{error,badarg},{error,badarg},\
         {error,badarg},{priority,low}]",
    ),
    (
        "spawns",
        "{[{priority,normal},{priority,low},{priority,max}],{priority,normal},\
         [{error,badarg},{error,badarg}]}",
    ),
];

/// Return a term as `io:format("~w")` prints it. The atom names come from
/// `atoms`, the table that owns the term.
///
/// # Examples
///
/// ```rust,ignore
/// assert_eq!(text(&Term::list_from(vec![Term::Integer(1)]), &atoms), "[1]");
/// ```
fn text(term: &Term, atoms: &AtomTable) -> String {
    let join = |terms: &[Term]| {
        terms
            .iter()
            .map(|term| text(term, atoms))
            .collect::<Vec<_>>()
            .join(",")
    };
    match term {
        Term::Atom(atom) => atoms.name_of(atom).to_owned(),
        Term::Integer(value) => value.to_string(),
        _ if let Some(terms) = term.proper_list() => {
            format!("[{}]", join(&terms.cloned().collect::<Vec<_>>()))
        }
        Term::Tuple(terms) => format!("{{{}}}", join(terms)),
        other => panic!("unexpected term {other:?}"),
    }
}

#[test]
fn schedules_processes_by_priority_as_pinned_erts_with_one_scheduler() {
    // `enqueue_process` puts a `low` process into the `normal` queue with
    // the count `RESCHEDULE_LOW` (`erl_process.c:3953-3971`), and
    // `pick_next_process` runs `max`, then `high`, then the `normal`
    // queue, where `check_requeue_process` moves a `low` process to the
    // end until its count is 0 (`erl_process.c:4039-4054`, `9993-10023`).
    let erlang = pinned_beam("erlang");
    let program = BeamProgram::load(&[&erlang, PRIORITY_PROBE]).unwrap();
    for (function, expected) in RESULTS {
        let result = program.execute("priority_probe", function, &[]).unwrap();
        assert_eq!(text(&result.term, result.atoms()), *expected, "{function}");
    }

    let calls: Vec<String> = RESULTS
        .iter()
        .map(|(function, _)| format!("io:format(\"~w~n\", [priority_probe:{function}()])"))
        .collect();
    let reference = Command::new("erl")
        .args([
            "+S",
            "1",
            "-noshell",
            "-pa",
            "fixtures/priority",
            "-eval",
            &format!(
                "jit = erlang:system_info(emu_flavor), {}, halt().",
                calls.join(", ")
            ),
        ])
        .output()
        .unwrap();
    assert!(
        reference.status.success() && reference.stderr.is_empty(),
        "{reference:?}"
    );
    let expected: String = RESULTS.iter().fold(String::new(), |mut text, (_, result)| {
        let _ = writeln!(text, "{result}");
        text
    });
    assert_eq!(String::from_utf8(reference.stdout).unwrap(), expected);
}

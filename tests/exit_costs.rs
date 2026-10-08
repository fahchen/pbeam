use std::process::Command;

use pbeam_runtime::{BeamProgram, Term};

const EXIT_PROBE: &[u8] = include_bytes!("../fixtures/preempt/exit_probe.beam");

/// The spawn options of the measured processes in the ERTS reference run: a
/// heap of 4,000,000 words and a binary heap of 40,000,000 words, so that no
/// measured process collects garbage. PBeam has no process heap and runs
/// with `[]`.
const ERTS_OPTIONS: &str = "[{min_heap_size,4000000},{min_bin_vheap_size,40000000}]";

/// The cases of `exit_probe:exits/2`: `{Setup, N, K}`. The exiting process
/// `a` has `3997 - K` reductions left when its exit starts. The exit bumps
/// 100 (`erl_process.c:14229`), and leaving the process table uses 50
/// (`erl_process.c:14475`).
const CASES: &str = "[{timers,1,3766},{timers,1,3767},{timers,2,3816},{timers,2,3817},\
                     {timers,100,0},{links,20,3827},{links,21,3827},{monitors,20,3827},\
                     {monitors,21,3827},{watchers,19,3827},{watchers,20,3827},\
                     {signals,600,3200},{signals,600,3343},{signals,600,3344},\
                     {signals,600,3800}]";

/// `exit_probe:exits/2` with [`CASES`] on the pinned ERTS with `erl +S 1`.
///
/// - Timers: each BIF timer costs 80 (`erl_hl_timer.c:2404`). The walk
///   checks the budget before each timer and switches out at 0 or less,
///   and a walk that ends gives 1 when the budget is 0 or less
///   (`erl_rbtree.h:1392-1436`). With 231 left, 1 timer leaves 51 after the
///   50, so `DOWN` comes first. With 230 left, the exit switches out after
///   the process table phase. With 180 left, the second timer finds 0, so
///   the exit switches out in the timer phase and `b` sees the name of
///   `a`. 100 timers at a full budget take three turns: 49, 50, and 1.
/// - Links and monitors that `a` holds: each costs 1, and the walk checks
///   the budget before each entry (`erl_process.c:13797-13977`,
///   `14067-14165`; `erl_rbtree.h:1392-1436`). With 170 left, 20 entries
///   use the budget to 0 and the exit ends; the 21st switches out the
///   exit. The relays of `a` send each `EXIT` message after the turn of
///   `a` that sent its link signal.
/// - Monitors of `a` (`watchers`): each costs 1, and the list walk checks
///   after each entry (`erl_monitor_link.c:517-548`). The first is the
///   monitor of `spawn_opt/4`. With 170 left, 20 `DOWN` messages use the
///   budget to 0, and the 21st comes in the next turn.
/// - Pending signals (`signals`): 402 monitor signals are handled before
///   the exit, and 198 are still in the queue. The exit counts 5 for each
///   monitor signal with a limit of 4 for each reduction left, and switches
///   out when signals remain (`erl_proc_sig_queue.c:7259-7473`). Each gives
///   a `DOWN` message `noproc`.
const EXITS: &str = "[{{timers,1,3766},[{{down,normal},1},{{seen,false},1},{b,2000}]},\
                     {{timers,1,3767},[{{seen,false},1},{b,666},{{down,normal},1},{b,1334}]},\
                     {{timers,2,3816},[{{seen,false},1},{b,666},{{down,normal},1},{b,1334}]},\
                     {{timers,2,3817},[{{seen,true},1},{b,666},{{down,normal},1},{b,1334}]},\
                     {{timers,100,0},[{{seen,true},1},{b,1333},{{down,normal},1},{b,667}]},\
                     {{links,20,3827},[{{down,normal},1},{{seen,false},1},{b,666},\
                     {{exit,normal},20},{b,1334}]},\
                     {{links,21,3827},[{{seen,false},1},{b,666},{{exit,normal},20},\
                     {{down,normal},1},{b,667},{{exit,normal},1},{b,667}]},\
                     {{monitors,20,3827},[{{down,normal},1},{{seen,false},1},{b,2000}]},\
                     {{monitors,21,3827},[{{seen,false},1},{b,666},{{down,normal},1},{b,1334}]},\
                     {{watchers,19,3827},[{{down,normal},20},{{seen,false},1},{b,2000}]},\
                     {{watchers,20,3827},[{{down,normal},20},{{seen,false},1},{b,666},\
                     {{down,normal},1},{b,1334}]},\
                     {{signals,600,3200},[{{down,normal},403},{{down,noproc},115},\
                     {{seen,false},1},{b,666},{{down,noproc},83},{b,1334}]},\
                     {{signals,600,3343},[{{down,normal},403},{{down,noproc},1},\
                     {{seen,false},1},{b,666},{{down,noproc},197},{b,1334}]},\
                     {{signals,600,3344},[{{down,normal},402},{{seen,false},1},{b,666},\
                     {{down,normal},1},{{down,noproc},198},{b,1334}]},\
                     {{signals,600,3800},[{{seen,true},1},{b,666},{{down,normal},403},\
                     {{down,noproc},198},{b,1334}]}]";

/// Return a term as `io:format("~w")` prints it, for the atoms, integers,
/// lists, and tuples of the probe results.
///
/// # Examples
///
/// ```rust,ignore
/// assert_eq!(text(&Term::List(vec![Term::Integer(1)])), "[1]");
/// ```
fn text(term: &Term) -> String {
    let join = |terms: &[Term]| terms.iter().map(text).collect::<Vec<_>>().join(",");
    match term {
        Term::Atom(atom) => atom.name().to_owned(),
        Term::Integer(value) => value.to_string(),
        Term::List(terms) => format!("[{}]", join(terms)),
        Term::Tuple(terms) => format!("{{{}}}", join(terms)),
        other => panic!("unexpected term {other:?}"),
    }
}

/// Return the term of [`CASES`].
///
/// # Examples
///
/// ```rust,ignore
/// let cases = cases(&program);
/// ```
fn cases(program: &BeamProgram) -> Term {
    let atom = |name: &str| Term::Atom(program.atom(name).unwrap());
    let case = |setup: &str, count: i64, budget: i64| {
        Term::Tuple(vec![
            atom(setup),
            Term::Integer(count),
            Term::Integer(budget),
        ])
    };
    let cases: Vec<Term> = CASES
        .trim_start_matches("[{")
        .trim_end_matches("}]")
        .split("},{")
        .map(|text| {
            let parts: Vec<&str> = text.split(',').collect();
            case(
                parts[0],
                parts[1].parse().unwrap(),
                parts[2].parse().unwrap(),
            )
        })
        .collect();
    let cases = Term::List(cases);
    assert_eq!(text(&cases), CASES);
    cases
}

#[test]
fn charges_the_exit_items_as_pinned_erts_with_one_scheduler() {
    let program = BeamProgram::load(&[EXIT_PROBE]).unwrap();
    let exits = program
        .execute(
            "exit_probe",
            "exits",
            &[cases(&program), Term::List(vec![])],
        )
        .unwrap();
    assert_eq!(text(&exits), EXITS);

    // The reference run checks that the pinned `erl` is the JIT flavor, and
    // that `a` and `b` collect no garbage: a collection costs reductions on
    // ERTS (`erl_gc.c:218-230`), and PBeam has no process heap.
    let reference = Command::new("erl")
        .args([
            "+S",
            "1",
            "-noshell",
            "-pa",
            "fixtures/preempt",
            "-eval",
            &format!(
                "{{module, M}} = code:ensure_loaded(exit_probe), \
                 jit = erlang:system_info(emu_flavor), O = {ERTS_OPTIONS}, C = {CASES}, \
                 E = M:exits(C, O), [0] = lists:usort([G || {{_, G}} <- M:gcs(C, O)]), \
                 io:format(\"~w~n\", [E]), halt()."
            ),
        ])
        .output()
        .unwrap();
    assert!(
        reference.status.success() && reference.stderr.is_empty(),
        "{reference:?}"
    );
    assert_eq!(
        String::from_utf8(reference.stdout).unwrap(),
        format!("{EXITS}\n")
    );
}

#[test]
fn gives_no_bif_timer_to_a_process_whose_exit_switched_out() {
    // `erts_pid2proc_opt` finds no exiting process (`erl_process_lock.c:970-984`),
    // so `erlang:start_timer/3` destroys the timer (`erl_hl_timer.c:1833-1861`).
    let program = BeamProgram::load(&[EXIT_PROBE]).unwrap();
    let result = program.execute("exit_probe", "late_timer", &[]).unwrap();
    assert_eq!(text(&result), "false");

    let reference = Command::new("erl")
        .args([
            "+S",
            "1",
            "-noshell",
            "-pa",
            "fixtures/preempt",
            "-eval",
            "{module, M} = code:ensure_loaded(exit_probe), \
             io:format(\"~w~n\", [M:late_timer()]), halt().",
        ])
        .output()
        .unwrap();
    assert!(
        reference.status.success() && reference.stderr.is_empty(),
        "{reference:?}"
    );
    assert_eq!(String::from_utf8(reference.stdout).unwrap(), "false\n");
}

mod common;

use std::process::Command;

use common::pinned_beam;
use pbeam_runtime::{BeamProgram, Term};

const PREEMPT_PROBE: &[u8] = include_bytes!("../fixtures/preempt/preempt_probe.beam");

/// The run lengths of each loop of `preempt_probe:sends/0`, measured on the
/// pinned ERTS with `erl +S 1` (JIT). Each run of `a` is followed by a run
/// of `b` with the same length.
const SENDS: &[(&str, &[i64])] = &[
    // Entry 1, send 1 + 4: the send at budget 0 returns after a switch.
    ("send_op", &[667, 667, 666]),
    ("send_ext", &[667, 667, 666]),
    // Entry 1, send through the export 0 + 4.
    ("send_apply", &[799, 800, 401]),
    ("send_fun", &[800, 800, 400]),
    // Entries 2, send 1 + 4, and the return after `call_ext_only` 1.
    ("send_only", &[500, 500, 500, 500]),
    // Entries 2, `apply_last` of the send through the export 0 + 4 with no
    // return: the send at budget 0 returns from the frame after a switch.
    ("send_apply_only", &[666, 667, 667]),
];

/// The run lengths of each loop of `preempt_probe:calls/0`, as [`SENDS`].
const CALLS: &[(&str, &[i64])] = &[
    // Entries 2, return 1, `whereis/1` 1, send 1 + 4: a turn can end
    // before `whereis/1`, which then runs again.
    ("call_light", &[444, 445, 444, 445, 222]),
    // Entry 1, `put/2` 0, send 1 + 4.
    ("call_heavy", &[667, 667, 666]),
    // Entry 1, `whereis/1` through the export 0, send 1 + 4.
    ("call_apply", &[666, 667, 667]),
    ("call_fun", &[666, 667, 667]),
    // Entries 2, `whereis/1` 1, return 1, send 1 + 4.
    ("call_only", &[444, 445, 444, 445, 222]),
    // Entries 3, returns 2, `whereis/1` 1, send 1 + 4.
    ("call_last", &[363, 364, 364, 363, 364, 182]),
    // Entries 2, `apply_last` of `whereis/1` 0 with no return, send 1 + 4.
    ("call_apply_only", &[571, 571, 572, 286]),
    // Entries 2, `put/2` 0, return 1, send 1 + 4.
    ("call_heavy_only", &[500, 500, 500, 500]),
    // Entry 1, `tuple_size/1` 0, send 1 + 4.
    ("call_guard", &[666, 667, 667]),
];

/// Return the result of `sends/0` or `calls/0` as a term:
/// `[{Loop, [{a, N1}, {b, N1}, {a, N2}, ...]}, ...]`.
///
/// # Examples
///
/// ```rust,ignore
/// // [{send_op, [{a, 667}, {b, 667}]}]
/// let term = runs_term(&program, &[("send_op", &[667])]);
/// ```
fn runs_term(program: &BeamProgram, loops: &[(&str, &[i64])]) -> Term {
    let atom = |name: &str| Term::Atom(program.atom(name).unwrap());
    let run = |tag: &str, length: i64| Term::Tuple(vec![atom(tag), Term::Integer(length)]);
    Term::List(
        loops
            .iter()
            .map(|(name, lengths)| {
                let runs = lengths
                    .iter()
                    .flat_map(|&length| [run("a", length), run("b", length)])
                    .collect();
                Term::Tuple(vec![atom(name), Term::List(runs)])
            })
            .collect(),
    )
}

/// Return the result of `sends/0` or `calls/0` as `io:format("~w")` prints
/// it.
///
/// # Examples
///
/// ```rust,ignore
/// assert_eq!(runs_text(&[("send_op", &[667])]), "[{send_op,[{a,667},{b,667}]}]");
/// ```
fn runs_text(loops: &[(&str, &[i64])]) -> String {
    let loops: Vec<String> = loops
        .iter()
        .map(|(name, lengths)| {
            let runs: Vec<String> = lengths
                .iter()
                .map(|length| format!("{{a,{length}}},{{b,{length}}}"))
                .collect();
            format!("{{{name},[{}]}}", runs.join(","))
        })
        .collect();
    format!("[{}]", loops.join(","))
}

#[test]
fn interleaves_two_busy_processes_as_pinned_erts_with_one_scheduler() {
    // A process has a budget of `CONTEXT_REDS` (4000) for each turn
    // (`erl_vm.h:53`). The JIT costs: function entry 1 (`i_test_yield`),
    // `return` 1, `call_ext` of a light BIF 1, a heavy BIF or a BIF through
    // its export 0, `send` 1 and then `ERTS_VBUMP_REDS(p, 4)`. A process
    // that uses its budget goes to the end of the run queue, so `a` and `b`
    // alternate, and each run of tags is one turn.
    let program = BeamProgram::load(&[PREEMPT_PROBE]).unwrap();
    let run = |function| program.execute("preempt_probe", function, &[]).unwrap();
    assert_eq!(run("sends"), runs_term(&program, SENDS));
    assert_eq!(run("calls"), runs_term(&program, CALLS));

    // The reference run checks that the pinned `erl` is the JIT flavor, and
    // that no loop collects garbage: a collection costs reductions on ERTS
    // (`erl_gc.c:218-230`), and PBeam has no process heap.
    let reference = Command::new("erl")
        .args([
            "+S",
            "1",
            "-noshell",
            "-pa",
            "fixtures/preempt",
            "-eval",
            "jit = erlang:system_info(emu_flavor), \
             [0] = lists:usort([N || {_, N} <- preempt_probe:gcs()]), \
             io:format(\"~w~n~w~n\", [preempt_probe:sends(), preempt_probe:calls()]), halt().",
        ])
        .output()
        .unwrap();
    assert!(
        reference.status.success() && reference.stderr.is_empty(),
        "{reference:?}"
    );
    assert_eq!(
        String::from_utf8(reference.stdout).unwrap(),
        format!("{}\n{}\n", runs_text(SENDS), runs_text(CALLS))
    );
}

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
        Term::Atom(atom) => match atom.name() {
            "fun" => "'fun'".to_owned(),
            name => name.to_owned(),
        },
        Term::Integer(value) => value.to_string(),
        Term::List(terms) => format!("[{}]", join(terms)),
        Term::Tuple(terms) => format!("{{{}}}", join(terms)),
        other => panic!("unexpected term {other:?}"),
    }
}

/// Return the runs of a race whose runs of `a` and `b` all have the same
/// lengths, as `~w` prints them.
///
/// # Examples
///
/// ```rust,ignore
/// assert_eq!(same_runs(&[(1, 2)]), "[{a,1},{b,1},{a,1},{b,1}]");
/// ```
fn same_runs(runs: &[(i64, usize)]) -> String {
    let runs: Vec<String> = runs
        .iter()
        .flat_map(|&(length, count)| vec![format!("{{a,{length}}},{{b,{length}}}"); count])
        .collect();
    format!("[{}]", runs.join(","))
}

#[test]
fn runs_erlang_yield_as_the_pinned_erts_instruction() {
    // `i_yield` writes `true` and always switches out the process with no
    // reduction cost (`jit/arm/ops.tab:664-666`;
    // `jit/arm/instr_common.cpp:3121-3124`; `jit/arm/instr_call.cpp:30-36`).
    // Through `apply/3`, `yield/0` of `erlang.beam` runs.
    let yields = format!(
        "[{}]",
        ["yield_op", "yield_only", "yield_last", "yield_apply"]
            .map(|name| format!("{{{name},{}}}", same_runs(&[(1, 30)])))
            .join(",")
    );

    let erlang = pinned_beam("erlang");
    let program = BeamProgram::load(&[&erlang, PREEMPT_PROBE]).unwrap();
    let run = |function| text(&program.execute("preempt_probe", function, &[]).unwrap());
    assert_eq!(run("yields"), yields);

    // The reference run checks that the pinned `erl` is the JIT flavor.
    // `preempt_probe:gcs/0` in the test above checks that no loop collects
    // garbage.
    let reference = Command::new("erl")
        .args([
            "+S",
            "1",
            "-noshell",
            "-pa",
            "fixtures/preempt",
            "-eval",
            "jit = erlang:system_info(emu_flavor), \
             io:format(\"~w~n\", [preempt_probe:yields()]), halt().",
        ])
        .output()
        .unwrap();
    assert!(
        reference.status.success() && reference.stderr.is_empty(),
        "{reference:?}"
    );
    assert_eq!(
        String::from_utf8(reference.stdout).unwrap(),
        format!("{yields}\n")
    );
}

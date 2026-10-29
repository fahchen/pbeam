//! Checks the preemption of busy processes, the reduction charge of `receive`, and `erlang:yield`
//! on PBeam against the pinned ERTS.

#[path = "../common/mod.rs"]
mod common;

use std::process::Command;

use common::pinned_beam;
use pbeam_runtime::{AtomTable, BeamProgram, Term};

const PREEMPT_PROBE: &[u8] = include_bytes!("../../fixtures/preempt/preempt_probe.beam");

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
    let run = |tag: &str, length: i64| Term::tuple(vec![atom(tag), Term::Integer(length)]);
    Term::list_from(loops.iter().map(|(name, lengths)| {
        let runs: Vec<_> = lengths
            .iter()
            .flat_map(|&length| [run("a", length), run("b", length)])
            .collect();
        Term::tuple(vec![atom(name), Term::list_from(runs)])
    }))
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
    assert_eq!(run("sends").term, runs_term(&program, SENDS));
    assert_eq!(run("calls").term, runs_term(&program, CALLS));

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

const RECEIVE_PROBE: &[u8] = include_bytes!("../../fixtures/preempt/receive_probe.beam");

/// The run lengths of each loop of `receive_probe:receives/0`, measured on
/// the pinned ERTS with `erl +S 1` (JIT), as [`SENDS`].
const RECEIVES: &[(&str, &[i64])] = &[
    // Entry 1, send to self 1 + 4, a fetch that finds the message 4, send
    // 1 + 4.
    ("recv_fetch", &[266, 267, 267, 266, 267, 267, 266, 134]),
    // Entry 1, a fetch that finds no message 4, send 1 + 4.
    ("recv_poll", &[400, 400, 400, 400, 400]),
    // Entry 1, a fetched message 0, send 1 + 4.
    ("recv_queued", &[667, 667, 666]),
    // Entry 1, a fetched message 0, `loop_rec_end` 1, send 1 + 4.
    ("recv_skip", &[571, 571, 572, 286]),
    // Entries 3, send to self 1 + 4, returns 2, fetch 4, send 1 + 4. A
    // return can leave a budget of 0: `loop_rec` then switches out before
    // the fetch, which costs 4 in the next turn.
    (
        "recv_return",
        &[210, 211, 210, 211, 211, 210, 211, 210, 211, 105],
    ),
];

/// The result of `receive_probe:orders/0`, measured on the pinned ERTS with
/// `erl +S 1`: for each kind of receive, the order of the messages for the
/// two values of N.
const ORDERS: &[(&str, [[&str; 3]; 2])] = &[
    ("woken", [["w", "q1", "q2"], ["q1", "w", "q2"]]),
    ("queued", [["w", "q1", "q2"], ["q1", "w", "q2"]]),
    ("info", [["w", "q1", "q2"], ["q1", "w", "q2"]]),
    ("named", [["w", "q1", "q2"], ["q1", "w", "q2"]]),
    ("hibernated", [["q1", "w", "q2"], ["q1", "q2", "w"]]),
];

/// Return [`ORDERS`] as a term: `[{Kind, [[w, q1, q2], ...]}, ...]`.
///
/// # Examples
///
/// ```rust,ignore
/// // [{woken, [[w, q1, q2], [q1, w, q2]]}, ...]
/// let term = orders_term(&program);
/// ```
fn orders_term(program: &BeamProgram) -> Term {
    let atom = |name: &str| Term::Atom(program.atom(name).unwrap());
    let order = |names: &[&str; 3]| Term::list_from(names.iter().map(|name| atom(name)));
    Term::list_from(ORDERS.iter().map(|(kind, orders)| {
        Term::tuple(vec![atom(kind), Term::list_from(orders.iter().map(order))])
    }))
}

/// Return [`ORDERS`] as `io:format("~w")` prints it.
///
/// # Examples
///
/// ```rust,ignore
/// assert!(orders_text().starts_with("[{woken,[[w,q1,q2],[q1,w,q2]]},"));
/// ```
fn orders_text() -> String {
    let kinds: Vec<String> = ORDERS
        .iter()
        .map(|(kind, orders)| {
            let orders: Vec<String> = orders
                .iter()
                .map(|names| format!("[{}]", names.join(",")))
                .collect();
            format!("{{{kind},[{}]}}", orders.join(","))
        })
        .collect();
    format!("[{}]", kinds.join(","))
}

#[test]
fn charges_the_receive_reductions_as_pinned_erts_with_one_scheduler() {
    // `loop_rec` costs 0 for a fetched message. At the end of the fetched
    // messages it fetches for 4 reductions, also when no message arrived
    // (`erl_proc_sig_queue.c:7771-7799`). `loop_rec_end` costs 1, and
    // `loop_rec` switches out before the peek when the budget is 0 or less
    // (`jit/arm/instr_msg.cpp:123-127`, `258-268`). `process_info/2` of the
    // caller fetches with no cost for each item (`erl_bif_info.c:1355-1356`,
    // `1237-1244`).
    //
    // In `orders/0`, the first value of N lets the receiving process send
    // `w` in the turn of the receive, and the next value does not. Woken: a
    // fetch of 4 in the new turn, N + 1 entries, a return, and a send.
    // Queued: entry 1, one fetch of 4 for 11 messages, 11 entries and a
    // return of `recv/1`, N + 1 entries, a return, and a send. Info: as
    // queued, with no fetch, and with 1 for the `process_info/2` item
    // (`erl_bif_info.c:1601`). Named: as info, with `registered_name`. Hibernated:
    // `erlang:hibernate/0` fetches (`beam_common.c:1586-1587`), so the next
    // turn has no fetch.
    let program = BeamProgram::load(&[RECEIVE_PROBE]).unwrap();
    let run = |function| program.execute("receive_probe", function, &[]).unwrap();
    assert_eq!(run("receives").term, runs_term(&program, RECEIVES));
    assert_eq!(run("orders").term, orders_term(&program));

    let reference = Command::new("erl")
        .args([
            "+S",
            "1",
            "-noshell",
            "-pa",
            "fixtures/preempt",
            "-eval",
            "jit = erlang:system_info(emu_flavor), \
             [0] = lists:usort([N || {_, N} <- receive_probe:gcs()]), \
             io:format(\"~w~n~w~n\", [receive_probe:receives(), receive_probe:orders()]), \
             halt().",
        ])
        .output()
        .unwrap();
    assert!(
        reference.status.success() && reference.stderr.is_empty(),
        "{reference:?}"
    );
    assert_eq!(
        String::from_utf8(reference.stdout).unwrap(),
        format!("{}\n{}\n", runs_text(RECEIVES), orders_text())
    );
}

/// Return a term as `io:format("~w")` prints it, for the atoms, integers,
/// lists, and tuples of the probe results.
///
/// The atom names come from `atoms`, the table that owns the term.
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
        Term::Atom(atom) => match atoms.name_of(atom) {
            "fun" => "'fun'".to_owned(),
            name => name.to_owned(),
        },
        Term::Integer(value) => value.to_string(),
        _ if let Some(terms) = term.proper_list() => {
            format!("[{}]", join(&terms.cloned().collect::<Vec<_>>()))
        }
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
    let run = |function| {
        let result = program.execute("preempt_probe", function, &[]).unwrap();
        text(&result.term, result.atoms())
    };
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

#[test]
fn runs_bump_reductions_and_reads_the_reduction_total_as_pinned_erts() {
    // `bump_reductions/1` is a light BIF: `call_ext` costs 1, and the BIF
    // uses `min(N, CONTEXT_REDS)` down to 0 (`bif.c:5403-5415`;
    // `bif.h:71-78`, `221-224`). Each iteration of `bump_loop` costs entry
    // 1, send 1 + 4, and 1 + 100.
    let bumps = format!(
        "[{{bump_loop,{}}},{{values,[true,true]}},{{errors,[{}]}}]",
        same_runs(&[(38, 7), (34, 1)]),
        ["{error,badarg}"; 4].join(",")
    );
    // The change of `process_info(self(), reductions)` over 100 iterations:
    // 101 entries, 1 return, and 1 for the first read
    // (`erl_bif_info.c:1424`, `1601`), plus the cost of 100 operations.
    // The total is `p->reds` plus `REDS_IN - fcalls - virtual_reds` for the
    // caller (`erl_bif_info.c:2114-2121`; `beam_common.c:2393-2409`), so a
    // send shows 1 and not its 4 virtual reductions (`bif.h:85-96`), and
    // each switch out adds the turn (`erl_process.c:9678-9700`). With
    // `bump_reductions(5000)`, each turn ends at the entry with a budget of
    // -1 and uses 4001. Another process shows only its `p->reds`: entry 1,
    // `call_ext` 1, and 100.
    let reductions = "[{empty,103},{body,303},{light,203},{heavy,103},{send,203},{bump,1203},\
                      {clamp,400098},{yield,103},{yield_apply,303},{apply,103},{'fun',103},\
                      {guard,103},{other,[102,2]}]";

    let erlang = pinned_beam("erlang");
    let program = BeamProgram::load(&[&erlang, PREEMPT_PROBE]).unwrap();
    let run = |function| {
        let result = program.execute("preempt_probe", function, &[]).unwrap();
        text(&result.term, result.atoms())
    };
    assert_eq!(run("bumps"), bumps);
    assert_eq!(run("reductions"), reductions);

    // The reference run checks that the pinned `erl` is the JIT flavor.
    // `preempt_probe:gcs/0` in the first test checks that no loop and no
    // measured window collects garbage.
    let reference = Command::new("erl")
        .args([
            "+S",
            "1",
            "-noshell",
            "-pa",
            "fixtures/preempt",
            "-eval",
            "jit = erlang:system_info(emu_flavor), \
             io:format(\"~w~n~w~n\", [preempt_probe:bumps(), preempt_probe:reductions()]), \
             halt().",
        ])
        .output()
        .unwrap();
    assert!(
        reference.status.success() && reference.stderr.is_empty(),
        "{reference:?}"
    );
    assert_eq!(
        String::from_utf8(reference.stdout).unwrap(),
        format!("{bumps}\n{reductions}\n")
    );
}

#[test]
fn switches_the_process_at_each_bif_trap_to_erlang_code_as_pinned_erts() {
    // `erlang:demonitor/2` with `flush` and no active monitor traps to
    // `erts_internal:flush_monitor_messages/3` (`bif.c:640-649`). ERTS
    // switches out the process at each `BIF_TRAP`, through `call_ext` and
    // through the export (`jit/arm/instr_bif.cpp:525-536`, `676-701`). Thus
    // each turn of `a` and `b` sends one tag. `preempt_probe:gcs/0` in the
    // other test checks that these loops collect no garbage on ERTS.
    const TRAPS: &[(&str, &[i64])] = &[("trap_ext", &[1; 30]), ("trap_apply", &[1; 30])];
    let erts_internal = pinned_beam("erts_internal");
    let program = BeamProgram::load(&[&erts_internal, PREEMPT_PROBE]).unwrap();
    assert_eq!(
        program.execute("preempt_probe", "traps", &[]).unwrap().term,
        runs_term(&program, TRAPS)
    );

    let reference = Command::new("erl")
        .args([
            "+S",
            "1",
            "-noshell",
            "-pa",
            "fixtures/preempt",
            "-eval",
            "jit = erlang:system_info(emu_flavor), \
             io:format(\"~w~n\", [preempt_probe:traps()]), halt().",
        ])
        .output()
        .unwrap();
    assert!(
        reference.status.success() && reference.stderr.is_empty(),
        "{reference:?}"
    );
    assert_eq!(
        String::from_utf8(reference.stdout).unwrap(),
        format!("{}\n", runs_text(TRAPS))
    );
}

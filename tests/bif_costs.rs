use std::process::Command;

use pbeam_runtime::{BeamProgram, Term};

const BIF_COST_PROBE: &[u8] = include_bytes!("../fixtures/preempt/bif_cost_probe.beam");

/// The spawn options of the measured processes in the ERTS reference run: a
/// heap of 4,000,000 words and a binary heap of 40,000,000 words, so that no
/// measured process collects garbage. PBeam has no process heap and runs
/// with `[]`.
const ERTS_OPTIONS: &str = "[{min_heap_size,4000000},{min_bin_vheap_size,40000000}]";

/// `bif_cost_probe:puts/1` on the pinned ERTS with `erl +S 1`: the cost of
/// `persistent_term:put/2` for the new keys of `put_keys/0` in an empty
/// table, then for the same value and for a new value. A put that grows the
/// table costs one reduction for each 32 slots of the old and the new table
/// (`erl_bif_persistent.c:708-718`, `1034-1139`).
const PUTS: &str =
    "{[{1,0},{2,0},{4,0},{5,0},{8,0},{9,1},{10,0},{17,3},{33,6},{34,0},{65,12}],0,0}";

/// `bif_cost_probe:put_trap/0` on the pinned ERTS with `erl +S 1`: the
/// reductions of a put that grows a table of 16 slots with 1 reduction
/// left. The put uses all reductions and traps, and the copy of the other
/// 16 slots costs nothing in the next turn: 1 for `process_info/2`, 3997 for
/// `bump_reductions/1`, 1 for the call of the put, and 1 for the trap.
const PUT_TRAP: &str = "4000";

/// `bif_cost_probe:costs/1` on the pinned ERTS with `erl +S 1`.
///
/// - `maps:from_list/1` (`erl_map.c:321-1182`; `utils.c:3499-3819`): 40
///   YCF units for each reduction. A list of one key uses 10 units for each
///   pair: the check, the hash, the sort, and the loops that remove the
///   pairs of a key that comes again, with 2 and 4 units in nested loops.
///   `{dups, 40000}` traps two times. A `badarg` costs nothing.
/// - `maps:keys/1` (`erl_map.c:1207-1253`, `2801-2834`): 15 units for each
///   reduction, one unit for each key of a hashmap. `{hash, 100000}` traps.
const COSTS: &str = "[{maps_from_list,[{nil,0},{{flat,32},0},{{dups,33},8},{{dups,80},19},\
                     {{dups,400},99},{{dups,40000},9999},{{bad,400},0},{{improper,400},0},\
                     {atom,0}]},\
                     {maps_keys,[{{flat,32},0},{{hash,33},2},{{hash,100},6},\
                     {{hash,10000},666},{{hash,100000},6666}]},\
                     {characters_to_binary,[{{list,3},0},{{list,10},1},{{list,49},4},\
                     {{list,50},6},{{list,100},12},{{list,10000},1200},{{list,50000},7000},\
                     {{bin,3},0},{{bin,10000},0},{{one_bin,10000},0},{{bins,100},1002},\
                     {nested,0},{{list_bin,50000},7010},{{split,39990},5598},\
                     {{split_bin,39990},3999},{leftover_trap,3997}]},\
                     {characters_to_list,[{{list,3},0},{{list,10},2},{{list,100},22},\
                     {{list,10000},2200},{{list,50000},12000},{{bin,3},0},{{bin,10000},2000},\
                     {{bin,39999},7999},{{bin,40000},8000},{{bin,50000},10000},\
                     {{bin,100000},20000},{{one_bin,10000},2000},{{bins,100},2002},{nested,0},\
                     {{list_bin,50000},12020},{bad_code,0},{bad_bin,0},{short_bin,0},\
                     {short_list,0},{{bad_after,50000},12000},{leftover_trap,6852}]},\
                     {characters_to_list_latin1,[{{list,3},0},{{list,100},22},\
                     {{list,10000},2200},{{bin,3},0},{{bin,10000},2200},{{high_bin,10000},2400},\
                     {{list,50000},12000},{{bin,50000},12000}]}]";

/// `bif_cost_probe:longs/1` on the pinned ERTS with `erl +S 1`: the runs of
/// `a`, which calls a BIF that traps three times, and of `b`, which sends
/// 667 tags in each turn. `a` switches out at each trap.
const LONGS: &str = "[{maps_from_list,[{b,1334},{a,1},{b,2001},{a,1},{b,1334},{a,1},{b,331}]},\
                     {maps_keys,[{b,667},{a,1},{b,1334},{a,1},{b,1334},{a,1},{b,1665}]},\
                     {characters_to_binary,[{b,667},{a,1},{b,1334},{a,1},{b,1334},{a,1},\
                     {b,1665}]},\
                     {characters_to_list,[{b,3335},{a,1},{b,1665},{a,2}]},\
                     {characters_to_list,[{b,2001},{a,1},{b,2001},{a,1},{b,998},{a,1}]}]";

/// `bif_cost_probe:results/0` on the pinned ERTS: the results of
/// `unicode:characters_to_list/2` with `unicode` and `latin1` for nested
/// chardata and for chardata that gives an error or an incomplete result.
const RESULTS: &str = "[[97,98,99,100,101,102,103],[97,98,99,100,101,102,103],\
                       {error,[97,98],[[-1],99]},{error,[97,98],[[-1],99]},\
                       {error,[97],[<<255,98>>,99]},[97,255,98,99],\
                       {incomplete,[97],<<226,130>>},[97,226,130],\
                       {incomplete,[97],<<226,130>>},[97,226,130],\
                       {error,[97,97,97],[-1]},{error,[97,97,97],[-1]}]";

/// `bif_cost_probe:sorts/1` on PBeam: the cost of `maps:from_list/1` for the
/// keys 1..N. ERTS picks the pivots of its sort from addresses, so its cost
/// changes between runs. The reference run checks that the ERTS cost is
/// within 10% of each value.
const SORTS: &[(&str, i64)] = &[
    ("{distinct,40}", 18),
    ("{distinct,100}", 51),
    ("{distinct,1000}", 638),
    ("{distinct,10000}", 7949),
];

/// `bif_cost_probe:exits/0` on the pinned ERTS with `erl +S 1`. The exit of
/// `a` bumps 100 reductions (`erl_process.c:14229`). With 150 or fewer
/// reductions left (`K` 3846 and more), `a` switches out before its
/// monitors, so `b` sends 666 messages before the `DOWN` message. With 100
/// or fewer (`K` 3896 and more), it also switches out before it leaves the
/// process table, so `b` sees its name (`erl_process.c:14315-14316`, `14475`,
/// `14541-14543`).
const EXITS: &str = "[{3845,[{down,0,normal},{seen,false},{done,1000}]},\
                     {3846,[{seen,false},{down,666,normal},{done,334}]},\
                     {3895,[{seen,false},{down,666,normal},{done,334}]},\
                     {3896,[{seen,true},{down,666,normal},{done,334}]},\
                     {3990,[{seen,true},{down,666,normal},{done,334}]}]";

/// Return [`SORTS`] as `io:format("~w")` prints the result of
/// `bif_cost_probe:sorts/1`.
///
/// # Examples
///
/// ```rust,ignore
/// // [{{distinct,40},18},...]
/// let text = sorts_text();
/// ```
fn sorts_text() -> String {
    let sorts: Vec<String> = SORTS
        .iter()
        .map(|(input, cost)| format!("{{{input},{cost}}}"))
        .collect();
    format!("[{}]", sorts.join(","))
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
        Term::Atom(atom) => atom.name().to_owned(),
        Term::Integer(value) => value.to_string(),
        Term::List(terms) => format!("[{}]", join(terms)),
        Term::Tuple(terms) => format!("{{{}}}", join(terms)),
        Term::Binary(bytes) => {
            let bytes: Vec<String> = bytes.iter().map(u8::to_string).collect();
            format!("<<{}>>", bytes.join(","))
        }
        other => panic!("unexpected term {other:?}"),
    }
}

#[test]
fn charges_the_remaining_bif_costs_and_the_exit_as_pinned_erts_with_one_scheduler() {
    let program = BeamProgram::load(&[BIF_COST_PROBE]).unwrap();
    let run = |function, arguments: &[Term]| {
        text(
            &program
                .execute("bif_cost_probe", function, arguments)
                .unwrap(),
        )
    };
    let options = [Term::List(vec![])];
    assert_eq!(run("puts", &options), PUTS);
    assert_eq!(run("put_trap", &[]), PUT_TRAP);
    assert_eq!(run("costs", &options), COSTS);
    assert_eq!(run("sorts", &options), sorts_text());
    assert_eq!(run("longs", &options), LONGS);
    assert_eq!(run("exits", &[]), EXITS);
    assert_eq!(run("results", &[]), RESULTS);

    // The reference run checks that the pinned `erl` is the JIT flavor, and
    // that no measured process collects garbage: a collection costs
    // reductions on ERTS (`erl_gc.c:218-230`), and PBeam has no process
    // heap. It first erases the persistent terms of the boot, because a
    // PBeam run starts with none.
    let tolerances: Vec<String> = SORTS
        .iter()
        .map(|(input, cost)| format!("{{{input},{}}}", cost / 10))
        .collect();
    let reference = Command::new("erl")
        .args([
            "+S",
            "1",
            "-noshell",
            "-pa",
            "fixtures/preempt",
            "-eval",
            &format!(
                "{{module, M}} = code:ensure_loaded(bif_cost_probe), \
                 jit = erlang:system_info(emu_flavor), O = {ERTS_OPTIONS}, \
                 true = erts_internal:erase_persistent_terms(), \
                 P = M:puts(O), C = M:costs(O), L = M:longs(O), E = M:exits(), \
                 R = M:results(), true = erts_internal:erase_persistent_terms(), \
                 T = M:put_trap(), \
                 [true = abs(Cost - Expected) =< Tolerance \
                  || {{{{Input, Cost}}, {{Input, Expected}}, {{Input, Tolerance}}}} \
                  <- lists:zip3(M:sorts(O), {sorts}, [{tolerances}])], \
                 [0] = lists:usort([element(tuple_size(G), G) || G <- M:gcs(O)]), \
                 io:format(\"~w~n~w~n~w~n~w~n~w~n~w~n\", [P, C, L, E, R, T]), halt().",
                sorts = sorts_text(),
                tolerances = tolerances.join(","),
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
        format!("{PUTS}\n{COSTS}\n{LONGS}\n{EXITS}\n{RESULTS}\n{PUT_TRAP}\n")
    );
}

#[test]
fn stops_maps_from_list_that_finds_a_bad_element_after_a_trap() {
    let program = BeamProgram::load(&[BIF_COST_PROBE]).unwrap();
    let failure = program
        .execute("bif_cost_probe", "bad_long", &[])
        .unwrap_err()
        .to_string();
    assert!(
        failure.contains("maps:from_list/1 finds a bad element after a trap"),
        "{failure}"
    );
}

#[test]
fn switches_out_at_each_persistent_term_put() {
    let program = BeamProgram::load(&[BIF_COST_PROBE]).unwrap();
    let runs = program.execute("bif_cost_probe", "put_race", &[]).unwrap();
    // Each put of a new value switches out `a` (`ERTS_BIF_YIELD_RETURN`,
    // `erl_bif_persistent.c:782-783`). ERTS also suspends `a` until thread
    // progress, so `b` runs more turns, for example
    // [{b,1334},{a,1},{b,1334},{a,1},{b,667},{a,1},{b,1665}] (row "Persistent terms" in
    // docs/erts-differences.md).
    assert_eq!(
        text(&runs),
        "[{b,667},{a,1},{b,667},{a,1},{b,667},{a,1},{b,2999}]"
    );
}

#[test]
fn stops_characters_to_binary_with_an_error_result() {
    let program = BeamProgram::load(&[BIF_COST_PROBE]).unwrap();
    let failure = program
        .execute("bif_cost_probe", "bad_binary", &[])
        .unwrap_err()
        .to_string();
    // ERTS returns {error, <<>>, <<255>>} (row "Chardata conversion").
    assert!(
        failure.contains("unsupported unicode:characters_to_binary/2 result"),
        "{failure}"
    );
}

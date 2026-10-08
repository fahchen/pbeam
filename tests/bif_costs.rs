use std::process::Command;

use pbeam_runtime::{BeamProgram, Term};

const BIF_COST_PROBE: &[u8] = include_bytes!("../fixtures/preempt/bif_cost_probe.beam");

/// The spawn options of the measured processes in the ERTS reference run: a
/// heap of 4,000,000 words and a binary heap of 40,000,000 words, so that no
/// measured process collects garbage. PBeam has no process heap and runs
/// with `[]`.
const ERTS_OPTIONS: &str = "[{min_heap_size,4000000},{min_bin_vheap_size,40000000}]";

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
                     {{hash,10000},666},{{hash,100000},6666}]}]";

/// `bif_cost_probe:longs/1` on the pinned ERTS with `erl +S 1`: the runs of
/// `a`, which calls a BIF that traps three times, and of `b`, which sends
/// 667 tags in each turn. `a` switches out at each trap.
const LONGS: &str = "[{maps_from_list,[{b,1334},{a,1},{b,2001},{a,1},{b,1334},{a,1},{b,331}]},\
                     {maps_keys,[{b,667},{a,1},{b,1334},{a,1},{b,1334},{a,1},{b,1665}]}]";

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
    assert_eq!(run("costs", &options), COSTS);
    assert_eq!(run("sorts", &options), sorts_text());
    assert_eq!(run("longs", &options), LONGS);
    assert_eq!(run("exits", &[]), EXITS);

    // The reference run checks that the pinned `erl` is the JIT flavor, and
    // that no measured process collects garbage: a collection costs
    // reductions on ERTS (`erl_gc.c:218-230`), and PBeam has no process
    // heap.
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
                 C = M:costs(O), L = M:longs(O), E = M:exits(), \
                 [true = abs(Cost - Expected) =< Tolerance \
                  || {{{{Input, Cost}}, {{Input, Expected}}, {{Input, Tolerance}}}} \
                  <- lists:zip3(M:sorts(O), {sorts}, [{tolerances}])], \
                 [0] = lists:usort([element(tuple_size(G), G) || G <- M:gcs(O)]), \
                 io:format(\"~w~n~w~n~w~n\", [C, L, E]), halt().",
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
        format!("{COSTS}\n{LONGS}\n{EXITS}\n")
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

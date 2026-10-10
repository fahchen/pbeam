use std::process::Command;

use pbeam_runtime::{BeamProgram, Term};

const INPUT_PROBE: &[u8] = include_bytes!("../../fixtures/preempt/input_probe.beam");

/// The spawn options of the measured processes in the ERTS reference run: a
/// heap of 4,000,000 words and a binary heap of 40,000,000 words, so that no
/// measured process collects garbage, also for a large off-heap binary of
/// `erlang:iolist_to_iovec/1`. PBeam has no process heap and runs with
/// `[]`.
const ERTS_OPTIONS: &str = "[{min_heap_size,4000000},{min_bin_vheap_size,40000000}]";

/// An input, as `io:format("~w")` prints it, and the reductions of the BIF
/// for it.
type Cost = (&'static str, i64);

/// The reductions of each BIF for each input of `input_probe:costs/1`,
/// measured on the pinned ERTS with `erl +S 1` (JIT). Each BIF runs with a
/// budget of 3997 reductions.
const COSTS: &[(&str, &[Cost])] = &[
    // A bignum costs one reduction for each 16 digits of
    // `big_integer_estimate` (`bif.c:3283-3299`), which is 1 in the 64-bit
    // range. A small integer costs nothing.
    (
        "integer_to_list",
        &[
            ("small", 0),
            ("max_small", 0),
            ("min_big", 1),
            ("min_small", 0),
            ("max_negative_big", 1),
            ("max_64", 1),
            ("min_64", 1),
        ],
    ),
    // `integer_to_binary/1` has the same cost as `integer_to_list/1`
    // (`binary.c:92-108`).
    (
        "integer_to_binary",
        &[
            ("small", 0),
            ("max_small", 0),
            ("min_big", 1),
            ("min_small", 0),
            ("max_negative_big", 1),
            ("max_64", 1),
            ("min_64", 1),
        ],
    ),
    // `integer_to_binary/2` estimates the digits of a bignum in its base
    // with `lg2_lookup` (`big.c:2589-2597`, `4087-4092`): 2^63 - 1 has 64
    // digits and costs 4 in base 2, and 13 digits and costs 0 in base 36.
    (
        "integer_to_binary_2",
        &[
            ("{small,2}", 0),
            ("{max_small,2}", 0),
            ("{min_big,2}", 3),
            ("{max_negative_big,2}", 3),
            ("{max_64,2}", 4),
            ("{min_64,2}", 4),
            ("{max_64,10}", 1),
            ("{max_64,36}", 0),
        ],
    ),
    // Two flatmaps, and an empty flatmap with a hashmap, cost nothing. Each
    // other pair costs one reduction for each 8 steps of `hashmap_merge`
    // (`erl_map.c:1507`, `1736`, `1823`). The last pair traps.
    (
        "maps_merge",
        &[
            ("{{1,10},{5,20}}", 0),
            ("{{1,0},{1,40}}", 0),
            ("{{1,10},{100,140}}", 2),
            ("{{100,140},{1,10}}", 2),
            ("{{1,40},{41,80}}", 4),
            ("{{1,40},{1,40}}", 8),
            ("{{1,100},{50,150}}", 16),
            ("{{1,1000},{500,1500}}", 174),
            ("{{1,10000},{5000,15000}}", 1844),
            ("{{1,40000},{20000,60000}}", 7034),
        ],
    ),
    // A flatmap costs nothing. A hashmap call takes at most `3997 / 4`
    // pairs, at most 32 at path 0, and costs 4 for each pair
    // (`erl_map.c:3783-3800`, `4126`). An ordered iterator path takes at
    // most `3997 / 32` keys and costs 32 for each key, or all reductions
    // when keys remain (`erl_map.c:3662-3703`).
    (
        "map_next",
        &[
            ("{10,0,[]}", 0),
            ("{33,0,[]}", 128),
            ("{33,1,iterator}", 124),
            ("{100,1,[]}", 384),
            ("{1000,1,[]}", 3772),
            ("{10000,1,iterator}", 3996),
            ("{ordered,200,1}", 32),
            ("{ordered,200,124}", 3968),
            ("{ordered,200,125}", 3997),
        ],
    ),
    // A list of at least two objects uses 8 units for each reduction: 1
    // for each object of the check, then 2 for each object of the insert
    // when the units that remain are at least the number of objects, else
    // 1 for each object of a copy and 2 for each object of the insert, with
    // traps (`erl_db.c:1812-1949`, `2002-2116`, `2152-2336`). The cost of
    // 15000 objects is more than the budget, and stops at 0. A bad object
    // stops the check, and a table that the caller cannot write stops the
    // insert, after a removal of the copies with 1 unit for each object. A
    // table that does not exist costs nothing.
    (
        "ets_insert",
        &[
            ("object", 0),
            ("nil", 0),
            ("one", 0),
            ("{list,2}", 0),
            ("{list,3}", 1),
            ("{list,8}", 3),
            ("{list,100}", 37),
            ("{list,1000}", 375),
            ("{list,10000}", 3750),
            ("{list,15000}", 3997),
            ("{list,16000}", 8000),
            ("improper", 12),
            ("bad", 6),
            ("{protected,100}", 12),
            ("{protected,40000}", 15000),
            ("{missing,100}", 0),
        ],
    ),
    // One reduction for each persistent term, and a trap at the end of the
    // budget (`erl_bif_persistent.c:324-356`, `805-900`).
    (
        "persistent_term_get",
        &[
            ("0", 0),
            ("1", 1),
            ("100", 100),
            ("1000", 1000),
            ("4000", 4000),
            ("10000", 10000),
        ],
    ),
    // One reduction for each 8 steps, rounded up, and at least 1: one step
    // for each cell and for the end of each list (`erl_iolist.c:113-290`,
    // `1004-1094`). A walk of more than 31975 steps traps, and the step of
    // the trap runs again in the next turn.
    (
        "iolist_size",
        &[
            ("bin", 1),
            ("nil", 1),
            ("{ones,1}", 1),
            ("{ones,7}", 1),
            ("{ones,8}", 2),
            ("{ones,15}", 2),
            ("{ones,16}", 3),
            ("nested", 2),
            ("binaries", 1),
            ("{ones,1000}", 126),
            ("{ones,10000}", 1251),
            ("{ones,31990}", 3999),
            ("{ones,100000}", 12501),
            ("bad", 1),
            ("bad_tail", 1),
            ("atom", 1),
            ("{bad_after,40000}", 5001),
            ("{tail_bin,15983}", 1998),
        ],
    ),
    // A binary, a list of one binary, and `[]` cost nothing. Each other
    // list walks for the size and then for the copy, and a copy uses one
    // more step for each 32 bytes of a binary of at most 512 bytes
    // (`erl_iolist.c:354-649`, `1121-1310`).
    (
        "iolist_to_iovec",
        &[
            ("bin", 0),
            ("nil", 0),
            ("empty_bin_list", 0),
            ("one_bin", 0),
            ("{ones,1}", 2),
            ("{ones,8}", 4),
            ("{ones,16}", 6),
            ("nested", 4),
            ("binaries", 2),
            ("{ones,1000}", 252),
            ("{ones,31970}", 7994),
            ("{ones,31990}", 7998),
            ("{ones,100000}", 25002),
            ("bad", 1),
            ("bad_tail", 1),
            ("atom", 0),
            ("{bins,1995}", 4241),
            ("{tail_bin,15983}", 3998),
        ],
    ),
    // A binary, a list of one binary, `[]`, and a term that is not a list
    // cost nothing. Each other list walks for the size and then for the
    // copy, and a copy uses one more step for each 32 bytes of each binary
    // (`erl_iolist.c:354-649`, `710-928`).
    (
        "iolist_to_binary",
        &[
            ("bin", 0),
            ("nil", 0),
            ("empty_bin_list", 0),
            ("one_bin", 0),
            ("{ones,1}", 2),
            ("{ones,8}", 4),
            ("{ones,16}", 6),
            ("nested", 4),
            ("binaries", 8),
            ("{ones,1000}", 252),
            ("{ones,31970}", 7994),
            ("{ones,31990}", 7998),
            ("{ones,100000}", 25002),
            ("bad", 1),
            ("bad_tail", 1),
            ("atom", 0),
            ("{bins,1995}", 4241),
            ("{tail_bin,15983}", 3998),
        ],
    ),
    // `erlang:bump_reductions(Bump)` and then `iolist_to_binary/1`. With at
    // most 100 reductions left, the call uses them as virtual reductions,
    // which the total does not count, and runs again in the next turn. With
    // fewer than 1000, it walks with a budget of 1000, and its budget
    // becomes 0 (`erl_iolist.c:34-35`, `857-874`).
    (
        "iolist_to_binary_late",
        &[
            ("{3900,{ones,10}}", 3905),
            ("{3900,atom}", 3901),
            ("{3500,nested}", 3505),
            ("{3500,{ones,4000}}", 3999),
            ("{3500,{ones,8000}}", 4999),
        ],
    ),
    // One reduction for each 10 search steps: one step for each byte of
    // an Aho-Corasick search, for each comparison of a Boyer-Moore search,
    // for each 8 bytes of its memchr skip, and for each part of a global
    // result. A search that does not end in its turn traps, and the trap
    // export bumps only the steps of its last turn
    // (`erl_bif_binary.c:637-994`, `1433-1645`, `1835-1952`).
    (
        "binary_split",
        &[
            ("first", 0),
            ("global", 0),
            ("none", 0),
            ("long_needle", 0),
            ("ac_first", 0),
            ("ac_global", 0),
            ("trim", 1),
            ("bad", 0),
            ("{commas,10}", 2),
            ("{commas,1000}", 200),
            ("{commas,20000}", 4000),
            ("{commas,50000}", 6000),
            ("{plain,1000}", 12),
            ("{plain,100000}", 1250),
            ("{plain,400000}", 5000),
            ("{plain_global,100000}", 1250),
            ("{plain_ac,1000}", 100),
            ("{plain_ac,100000}", 6000),
            ("{mixed,1000}", 600),
            ("{mixed,30000}", 6000),
            ("{wrap,319700}", 3996),
            ("{wrap,319745}", 3997),
            ("{wrap,319760}", 3997),
        ],
    ),
    // Each item that PBeam supports costs 1 after the read, also for a
    // process with 10000 messages (`erl_bif_info.c:1417-1424`, `1601`).
    // `process_info/2` is a heavy BIF, so its call costs nothing.
    (
        "process_info",
        &[
            ("{self,registered_name}", 1),
            ("{self,message_queue_len}", 1),
            ("{self,reductions}", 1),
            ("{self,current_stacktrace}", 1),
            ("{idle,message_queue_len}", 1),
            ("{idle,registered_name}", 1),
            ("{idle,reductions}", 1),
            ("{busy,message_queue_len}", 1),
        ],
    ),
];

/// The runs of `input_probe:longs/1`, measured on the pinned ERTS with
/// `erl +S 1`: the tags of `a`, which calls a BIF that traps three times,
/// and of `b`, which sends 667 tags in each turn.
const LONGS: &[(&str, &[(&str, i64)])] = &[
    // 7034 reductions for each `maps:merge/2` of two maps of 40000 keys.
    (
        "maps_merge",
        &[
            ("b", 667),
            ("a", 1),
            ("b", 1334),
            ("a", 1),
            ("b", 1334),
            ("a", 1),
            ("b", 1665),
        ],
    ),
    // 8000 reductions for each `ets:insert/2` of 16000 objects.
    (
        "ets_insert",
        &[
            ("b", 1334),
            ("a", 1),
            ("b", 1334),
            ("a", 1),
            ("b", 1334),
            ("a", 1),
            ("b", 998),
        ],
    ),
    // 10000 reductions for each `persistent_term:get/0` of 10000 terms.
    (
        "persistent_term_get",
        &[
            ("b", 1334),
            ("a", 1),
            ("b", 2001),
            ("a", 1),
            ("b", 1334),
            ("a", 1),
            ("b", 331),
        ],
    ),
    // 12501 reductions for each `iolist_size/1` of 100000 ones.
    (
        "iolist_size",
        &[
            ("b", 2001),
            ("a", 1),
            ("b", 2001),
            ("a", 1),
            ("b", 998),
            ("a", 1),
        ],
    ),
    // 25002 reductions for each `iolist_to_iovec/1` of 100000 ones.
    (
        "iolist_to_iovec",
        &[("b", 4002), ("a", 1), ("b", 998), ("a", 2)],
    ),
    // 4241 reductions for each `iolist_to_iovec/1` of 1995 binaries.
    (
        "iolist_to_iovec",
        &[
            ("b", 667),
            ("a", 1),
            ("b", 667),
            ("a", 1),
            ("b", 667),
            ("a", 1),
            ("b", 2999),
        ],
    ),
    // 25002 reductions for each `iolist_to_binary/1` of 100000 ones.
    (
        "iolist_to_binary",
        &[("b", 4002), ("a", 1), ("b", 998), ("a", 2)],
    ),
    // 4241 reductions for each `iolist_to_binary/1` of 1995 binaries.
    (
        "iolist_to_binary",
        &[
            ("b", 667),
            ("a", 1),
            ("b", 667),
            ("a", 1),
            ("b", 667),
            ("a", 1),
            ("b", 2999),
        ],
    ),
    // `erlang:bump_reductions(3900)` and then `iolist_to_binary/1` of 10
    // ones: the call switches out before it walks.
    (
        "iolist_to_binary_late",
        &[
            ("b", 667),
            ("a", 1),
            ("b", 667),
            ("a", 1),
            ("b", 667),
            ("a", 1),
            ("b", 2999),
        ],
    ),
    // A global `binary:split/3` with 50000 matches: its search and its
    // result build trap.
    (
        "binary_split",
        &[
            ("b", 1334),
            ("a", 1),
            ("b", 2001),
            ("a", 1),
            ("b", 1334),
            ("a", 1),
            ("b", 331),
        ],
    ),
    // An Aho-Corasick search of 200000 bytes with no match.
    (
        "binary_split",
        &[("b", 3335), ("a", 1), ("b", 1665), ("a", 2)],
    ),
];

/// Return [`COSTS`] as `io:format("~w")` prints the result of
/// `input_probe:costs/1`.
///
/// # Examples
///
/// ```rust,ignore
/// // [{integer_to_list,[{small,0},...]},...]
/// let text = costs_text();
/// ```
fn costs_text() -> String {
    let bifs: Vec<String> = COSTS
        .iter()
        .map(|(bif, costs)| {
            let costs: Vec<String> = costs
                .iter()
                .map(|(input, cost)| format!("{{{input},{cost}}}"))
                .collect();
            format!("{{{bif},[{}]}}", costs.join(","))
        })
        .collect();
    format!("[{}]", bifs.join(","))
}

/// Return [`LONGS`] as `io:format("~w")` prints the result of
/// `input_probe:longs/1`.
///
/// # Examples
///
/// ```rust,ignore
/// // [{maps_merge,[{b,667},{a,1},...]},...]
/// let text = longs_text();
/// ```
fn longs_text() -> String {
    let bifs: Vec<String> = LONGS
        .iter()
        .map(|(bif, runs)| {
            let runs: Vec<String> = runs
                .iter()
                .map(|(tag, length)| format!("{{{tag},{length}}}"))
                .collect();
            format!("{{{bif},[{}]}}", runs.join(","))
        })
        .collect();
    // The owner of the table exits while the insert of 16000 objects traps
    // after the copy, so the insert raises `badarg` (`erl_db.c:2215-2253`).
    // The size of the iodata and of each binary of its iovec, for inputs
    // whose copy traps or stops in a binary.
    let sizes = "{iovec_sizes,[{binaries,1500,[800,100,600]},{{ones,100000},100000,[100000]},\
                 {{bins,1995},997500,[997500]},{{tail_bin,15983},16483,[16483]}]}";
    format!("[{},{{ets_vanish,[badarg]}},{sizes}]", bifs.join(","))
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
fn charges_and_traps_the_input_dependent_bifs_as_pinned_erts_with_one_scheduler() {
    let program = BeamProgram::load(&[INPUT_PROBE]).unwrap();
    let run = |function| {
        text(
            &program
                .execute("input_probe", function, &[Term::List(vec![])])
                .unwrap(),
        )
    };
    assert_eq!(run("costs"), costs_text());
    assert_eq!(run("longs"), longs_text());

    // The reference run checks that the pinned `erl` is the JIT flavor, and
    // that no measured process collects garbage: a collection costs
    // reductions on ERTS (`erl_gc.c:218-230`), and PBeam has no process
    // heap. It first erases the persistent terms of the boot, because a
    // PBeam run starts with none. `gcs/1` runs last, because it puts the
    // persistent terms again.
    let reference = Command::new("erl")
        .args([
            "+S",
            "1",
            "-noshell",
            "-pa",
            "fixtures/preempt",
            "-eval",
            &format!(
                "{{module, M}} = code:ensure_loaded(input_probe), \
                 jit = erlang:system_info(emu_flavor), O = {ERTS_OPTIONS}, \
                 true = erts_internal:erase_persistent_terms(), \
                 C = M:costs(O), L = M:longs(O), \
                 [0] = lists:usort([element(tuple_size(G), G) || G <- M:gcs(O)]), \
                 io:format(\"~w~n~w~n\", [C, L]), halt()."
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
        format!("{}\n{}\n", costs_text(), longs_text())
    );
}

//! Checks the reduction costs of the remaining BIFs and of the process exit on PBeam against the
//! pinned ERTS.

use std::process::Command;

use pbeam_runtime::{AtomTable, BeamProgram, HostTerm, Term};

const BIF_COST_PROBE: &[u8] = include_bytes!("../../fixtures/preempt/bif_cost_probe.beam");

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

/// `bif_cost_probe:erase_costs/1` on the pinned ERTS with `erl +S 1`:
/// the reductions of `persistent_term:erase/1` for the keys 1 to 10 of an
/// empty table, in this order. The table grows to 32 slots at the ninth
/// key. The erase of key 1 copies the 32 slots of the same-size table
/// twice and 32 more, the erase of key 3 shrinks the table from 32 to 16
/// slots, and the erase of key 7 shrinks it from 16 to 8 slots. A key
/// whose next slot is free charges nothing. The probe yields before each
/// erase, so the erase does not wait for thread progress.
const ERASE_COSTS: &str = "[3,0,2,0,0,0,1,0,0,0]";

/// `bif_cost_probe:erase_trap/0` on the pinned ERTS with `erl +S 1`: the
/// reductions of an erase that rehashes a table of 16 slots with 1
/// reduction left. The temporary copy brings 16 slots and the rehashed
/// copy 32 slots, so each copy fits its own limit of 32 slots and the
/// erase completes in this call. 4000 is 1 for `process_info/2`, 3996 for
/// `bump_reductions/1`, 1 for the call of the erase, 1 for its charge, and
/// 1 for the switch out.
const ERASE_TRAP: &str = "4000";

/// `bif_cost_probe:erase_trap_32/0` on the pinned ERTS with `erl +S 1`: the
/// reductions of an erase that rehashes a table of 32 slots with 1
/// reduction left. The temporary copy goes through its 32 slots within the
/// limit of 32, and the rehashed copy traps after 32 slots of its 64. ERTS
/// gives each copy its own limit (`erl_bif_persistent.c:275-293`).
const ERASE_TRAP_32: &str = "4001";

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
///   `{dups, 40000}` traps two times. The bignum key of `{big_dups, 33}`
///   costs the same as a small key. The sort of `{two_keys, 9, 73}` and
///   `{two_keys, 73, 9}` uses 81 units for the partition of the pairs
///   around the pivot key, then 72 units for the pairs of the other key: a
///   partition of 73 pairs or the selection sort of 9 pairs
///   (`utils.c:3499-3588`, `3636-3640`). Thus the cost is the same for each
///   pivot. A `badarg` costs nothing.
/// - `maps:keys/1` (`erl_map.c:1207-1253`, `2801-2834`): 15 units for each
///   reduction, one unit for each key of a hashmap. `{hash, 100000}` traps.
/// - `unicode:characters_to_binary/2` with `latin1`
///   (`erl_unicode.c:342-505`, `931-1022`): the `utf8_need` walk costs one
///   step for each byte of a binary and for each integer, and one reduction
///   for each 50 steps. A binary of bytes below 128 then returns itself.
///   Else the conversion costs one reduction for each 10 characters.
///   `{high_bin, 50000}` and `{list, 50000}` trap.
/// - `erlang:list_to_binary/1` (`erl_iolist.c:113-290`, `424-649`,
///   `706-914`): `[]`, a list of one binary, and a term that is not a list
///   cost nothing. Else the size walk and the copy walk each use one step
///   for each list cell and each list end, 8 steps for each reduction, and
///   the copy one more step for each 32 bytes of a binary. A walk costs at
///   least 1. A `badarg` costs the size walk up to the bad element.
///   `{list, 50000}` traps in the size walk. The size walk of
///   `{list, 31970}` uses all reductions, so the copy starts in the next
///   turn. `{big_bin, 1100000}` and `{big_tail, 1100000}` trap in the copy
///   of the binary.
const COSTS: &str = "[{maps_from_list,[{nil,0},{{flat,32},0},{{dups,33},8},{{dups,80},19},\
                     {{dups,400},99},{{dups,40000},9999},{{big_dups,33},8},{{two_keys,9,73},23},\
                     {{two_keys,73,9},23},{{bad,400},0},{{improper,400},0},{atom,0}]},\
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
                     {{list,50000},12000},{{bin,50000},12000}]},\
                     {characters_to_binary_latin1,[{{list,3},0},{{list,100},12},\
                     {{list,10000},1200},{{list,50000},7000},{{bin,3},0},{{bin,10000},1200},\
                     {{high_bin,3},0},{{high_bin,10000},1400},{{high_bin,50000},8990},\
                     {{one_bin,10000},1200},{{bins,100},1200},{nested,0}]},\
                     {list_to_binary,[{nil,0},{{one_bin,100},0},{{list,3},2},{{list,100},26},\
                     {{list,10000},2502},{{list,31970},7994},{{list,50000},12502},{{bins,100},64},{nested,6},\
                     {{big_bin,1100000},4299},{{big_tail,1100000},4299},{{bad_after,400},51},\
                     {{improper,400},51},{atom,0}]}]";

/// `bif_cost_probe:longs/1` on the pinned ERTS with `erl +S 1`: the runs of
/// `a`, which calls a BIF that traps three times, and of `b`, which sends
/// 667 tags in each turn. `a` switches out at each trap.
const LONGS: &str = "[{maps_from_list,[{b,1334},{a,1},{b,2001},{a,1},{b,1334},{a,1},{b,331}]},\
                     {maps_keys,[{b,667},{a,1},{b,1334},{a,1},{b,1334},{a,1},{b,1665}]},\
                     {characters_to_binary,[{b,667},{a,1},{b,1334},{a,1},{b,1334},{a,1},\
                     {b,1665}]},\
                     {characters_to_list,[{b,3335},{a,1},{b,1665},{a,2}]},\
                     {characters_to_list,[{b,2001},{a,1},{b,2001},{a,1},{b,998},{a,1}]},\
                     {characters_to_binary_latin1,[{b,1334},{a,1},{b,1334},{a,1},{b,1334},\
                     {a,1},{b,998}]},\
                     {list_to_binary,[{b,4002},{a,1},{b,998},{a,2}]}]";

/// `bif_cost_probe:lows/1` on the pinned ERTS with `erl +S 1`: the
/// reductions of `erlang:list_to_binary/1` of 10000 bytes after the process
/// uses `K` reductions, with `K`. The call has `3996 - K` reductions left.
/// With 100 or fewer (`K` 3896 and 3995), it uses them as virtual
/// reductions, switches out, and runs again from the start in the next
/// turn (`ERTS_BIF_YIELD1`). With fewer than 1000 (`K` 2997 and 3895), it
/// walks with a budget of 1000 (`erl_iolist.c:866-876`): the size walk of
/// 10001 steps traps after 8000 steps, so the process uses its budget and
/// continues in the next turn.
const LOWS: &str = "[{3895,5503},{3896,6403},{3995,6502},{2996,5503},{2997,5503},{0,2507}]";

/// `bif_cost_probe:results/0` on the pinned ERTS: the results of
/// `unicode:characters_to_list/2` with `unicode` and `latin1` for nested
/// chardata and for chardata that gives an error or an incomplete result.
const RESULTS: &str = "[[97,98,99,100,101,102,103],[97,98,99,100,101,102,103],\
                       {error,[97,98],[[-1],99]},{error,[97,98],[[-1],99]},\
                       {error,[97],[<<255,98>>,99]},[97,255,98,99],\
                       {incomplete,[97],<<226,130>>},[97,226,130],\
                       {incomplete,[97],<<226,130>>},[97,226,130],\
                       {error,[97,97,97],[-1]},{error,[97,97,97],[-1]}]";

/// `bif_cost_probe:binary_results/0` on the pinned ERTS: the results of
/// `unicode:characters_to_binary/2` with `latin1` and its `badarg` inputs,
/// of `unicode:bin_is_7bit/1`, of `erlang:list_to_binary/1`, and of
/// `erlang:split_binary/2`. Each binary is `{binary, Bytes}`.
const BINARY_RESULTS: &str = "[[{binary,[97,98,99]},{binary,[195,136,65]},\
                              {binary,[195,136,65,195,191]},{binary,[97]},{binary,[195,136]},\
                              {binary,[]},{binary,[65,195,136]},\
                              {binary,[97,98,99,100,101,102,103]}],\
                              [{error,badarg},{error,badarg},{error,badarg},{error,badarg}],\
                              [true,true,false,false,false,false,false],\
                              [{binary,[]},{binary,[1]},{binary,[1,2,3,4]},{binary,[]},\
                              {binary,[]},{binary,[1,2]},{binary,[1,2]},{error,badarg},\
                              {error,badarg},{error,badarg},{error,badarg},{error,badarg},\
                              {error,badarg},{error,badarg}],\
                              [{{binary,[]},{binary,[97,98,99]}},{{binary,[97]},{binary,[98,99]}},\
                              {{binary,[97,98,99]},{binary,[]}},{{binary,[]},{binary,[]}},\
                              {error,badarg},{error,badarg},{error,badarg},{error,badarg}]]";

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
        Term::Atom(atom) => atoms.name_of(atom).to_owned(),
        Term::Integer(value) => value.to_string(),
        _ if let Some(terms) = term.proper_list() => {
            format!("[{}]", join(&terms.cloned().collect::<Vec<_>>()))
        }
        Term::Tuple(terms) => format!("{{{}}}", join(terms)),
        Term::Binary(bytes) => {
            let bytes: Vec<String> = bytes.bytes().unwrap().iter().map(u8::to_string).collect();
            format!("<<{}>>", bytes.join(","))
        }
        other => panic!("unexpected term {other:?}"),
    }
}

#[test]
fn charges_the_remaining_bif_costs_and_the_exit_as_pinned_erts_with_one_scheduler() {
    let program = BeamProgram::load(&[BIF_COST_PROBE]).unwrap();
    let run = |function, arguments: &[HostTerm]| {
        let result = program
            .execute("bif_cost_probe", function, arguments)
            .unwrap();
        text(&result.term, result.atoms())
    };
    let options = [program.host_term(Term::nil())];
    assert_eq!(run("puts", &options), PUTS);
    assert_eq!(run("put_trap", &[]), PUT_TRAP);
    assert_eq!(run("erase_costs", &options), ERASE_COSTS);
    assert_eq!(run("erase_trap", &[]), ERASE_TRAP);
    assert_eq!(run("erase_trap_32", &[]), ERASE_TRAP_32);
    assert_eq!(run("costs", &options), COSTS);
    assert_eq!(run("longs", &options), LONGS);
    assert_eq!(run("exits", &[]), EXITS);
    assert_eq!(run("results", &[]), RESULTS);
    assert_eq!(run("lows", &options), LOWS);
    assert_eq!(run("binary_results", &[]), BINARY_RESULTS);

    // The reference run checks that the pinned `erl` is the JIT flavor, and
    // that no measured process collects garbage: a collection costs
    // reductions on ERTS (`erl_gc.c:218-230`), and PBeam has no process
    // heap. It first erases the persistent terms of the boot, because a
    // PBeam run starts with none.
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
                 R = M:results(), W = M:lows(O), B = M:binary_results(), \
                 true = erts_internal:erase_persistent_terms(), T = M:put_trap(), \
                 true = erts_internal:erase_persistent_terms(), X = M:erase_costs(O), \
                 true = erts_internal:erase_persistent_terms(), Y = M:erase_trap(), \
                 true = erts_internal:erase_persistent_terms(), Z = M:erase_trap_32(), \
                 [0] = lists:usort([element(tuple_size(G), G) || G <- M:gcs(O)]), \
                 io:format(\"~w~n~w~n~w~n~w~n~w~n~w~n~w~n~w~n~w~n~w~n~w~n\", [P, C, L, E, R, W, B, T, X, Y, Z]), \
                 halt()."
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
        format!(
            "{PUTS}\n{COSTS}\n{LONGS}\n{EXITS}\n{RESULTS}\n{LOWS}\n{BINARY_RESULTS}\n{PUT_TRAP}\n{ERASE_COSTS}\n{ERASE_TRAP}\n{ERASE_TRAP_32}\n"
        )
    );
}

#[test]
fn stops_characters_to_binary_with_a_latin1_error_result() {
    let program = BeamProgram::load(&[BIF_COST_PROBE]).unwrap();
    let failure = program
        .execute("bif_cost_probe", "bad_latin1", &[])
        .unwrap_err()
        .to_string();
    // ERTS returns {error, <<>>, [256]} (row "Chardata conversion").
    assert!(
        failure.contains("unsupported unicode:characters_to_binary/2 result"),
        "{failure}"
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
    // Each put of a new value suspends `a` until thread progress
    // (`erl_bif_persistent.c:774-783`), so `b` runs two or more turns.
    // ERTS gives, for example,
    // [{b,1334},{a,1},{b,1334},{a,1},{b,667},{a,1},{b,1665}]. PBeam makes
    // the table visible after three scheduling steps (row "Persistent
    // terms" in docs/erts-differences.md).
    assert_eq!(
        text(&runs.term, runs.atoms()),
        "[{b,1334},{a,1},{b,1334},{a,1},{b,1334},{a,1},{b,998}]"
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

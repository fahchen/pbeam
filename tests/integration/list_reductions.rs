use std::process::Command;

use pbeam_runtime::{BeamProgram, Term};

const LIST_PROBE: &[u8] = include_bytes!("../../fixtures/preempt/list_probe.beam");

/// The spawn options of the measured processes in the ERTS reference run: a
/// heap of 2,000,000 words, so that `lists:reverse/2` and `erlang:'++'/2`
/// copy on the heap (`erl_bif_lists.c:1258-1262`, `246-250`) and no loop
/// collects garbage. PBeam has no process heap and runs with `[]`.
const ERTS_OPTIONS: &str = "[{min_heap_size,2000000}]";

/// A list length and the run lengths of `a` for it.
type Lengths = (i64, &'static [i64]);

/// The run lengths of `list_probe:costs/1` for each BIF and list length,
/// measured on the pinned ERTS with `erl +S 1` (JIT). Each run of `a` is
/// followed by a run of `b` with the same length. Each iteration costs
/// entry 1, send 1 + 4, `call_ext` 1 (not for the guard BIF `length/1`),
/// and the cost of the BIF.
const COSTS: &[(&str, &[Lengths])] = &[
    // `lists:member/2`: one reduction for each 16 compared elements, rounded
    // up: 0, 1, 1, 7, 63, and 625.
    (
        "member",
        &[
            (0, &[571, 571, 358]),
            (1, &[500, 500, 500]),
            (10, &[500, 500, 500]),
            (100, &[286, 285, 286, 143]),
            (1000, &[58, 57, 57, 57, 57, 14]),
            (10000, &[7, 6, 6, 7, 4]),
        ],
    ),
    // `lists:reverse/2` and `erlang:'++'/2` with a nonempty right list: one
    // reduction for each 60 elements: 0, 0, 0, 1, 16, and 166.
    (
        "reverse",
        &[
            (0, &[571, 571, 358]),
            (1, &[571, 571, 358]),
            (10, &[571, 571, 358]),
            (100, &[500, 500]),
            (1000, &[174, 126]),
            (10000, &[24, 6]),
        ],
    ),
    (
        "append",
        &[
            (0, &[571, 571, 358]),
            (1, &[571, 571, 358]),
            (10, &[571, 571, 358]),
            (100, &[500, 500]),
            (1000, &[174, 126]),
            (10000, &[24, 6]),
        ],
    ),
    // `erlang:'++'/2` with `[]` on the right: one reduction for each 32
    // elements: 0, 0, 0, 3, 31, and 312.
    (
        "append_nil",
        &[
            (0, &[571, 571, 358]),
            (1, &[571, 571, 358]),
            (10, &[571, 571, 358]),
            (100, &[400, 400, 200]),
            (1000, &[106, 105, 89]),
            (10000, &[13, 13, 4]),
        ],
    ),
    // The guard BIF `length/1` with no call cost: one reduction for each 16
    // elements, rounded down: 0, 0, 0, 6, 62, and 625.
    (
        "length",
        &[
            (0, &[666, 667, 167]),
            (1, &[666, 667, 167]),
            (10, &[666, 667, 167]),
            (100, &[333, 334, 333]),
            (1000, &[59, 59, 59, 59, 59, 5]),
            (10000, &[7, 6, 7, 6, 4]),
        ],
    ),
    // `lists:keyfind/3` and `lists:keymember/3`: no cost up to 40,000
    // elements.
    (
        "keyfind",
        &[
            (0, &[571, 571, 358]),
            (1, &[571, 571, 358]),
            (10, &[571, 571, 358]),
            (100, &[571, 429]),
            (1000, &[300]),
            (10000, &[30]),
        ],
    ),
    (
        "keymember",
        &[
            (0, &[571, 571, 358]),
            (1, &[571, 571, 358]),
            (10, &[571, 571, 358]),
            (100, &[571, 429]),
            (1000, &[300]),
            (10000, &[30]),
        ],
    ),
];

/// The runs of `list_probe:longs/1`, measured on the pinned ERTS with
/// `erl +S 1`: the tags of `a`, which calls a BIF on a list of 100,000
/// elements three times, and of `b`, which sends 667 tags in each turn.
const LONGS: &[(&str, &[(&str, i64)])] = &[
    // 6250 reductions for each call: the first call traps in the first
    // turn of `a`, and continues in the next turn.
    (
        "length_guard",
        &[
            ("b", 667),
            ("a", 1),
            ("b", 1334),
            ("a", 1),
            ("b", 667),
            ("a", 1),
            ("b", 2332),
        ],
    ),
    (
        "length_body",
        &[
            ("b", 667),
            ("a", 1),
            ("b", 1334),
            ("a", 1),
            ("b", 667),
            ("a", 1),
            ("b", 2332),
        ],
    ),
    (
        "length_apply",
        &[
            ("b", 667),
            ("a", 1),
            ("b", 1334),
            ("a", 1),
            ("b", 667),
            ("a", 1),
            ("b", 2332),
        ],
    ),
    // 1666 reductions for each call: the third call traps.
    (
        "reverse_ext",
        &[("a", 2), ("b", 667), ("a", 1), ("b", 4333)],
    ),
    (
        "reverse_apply_only",
        &[("a", 2), ("b", 667), ("a", 1), ("b", 4333)],
    ),
    (
        "member_ext",
        &[
            ("b", 667),
            ("a", 1),
            ("b", 1334),
            ("a", 1),
            ("b", 667),
            ("a", 1),
            ("b", 2332),
        ],
    ),
    // No cost, but a trap after each 40,000 elements.
    (
        "keyfind_ext",
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
    // The same runs for `lists:keymember/3`, which finds `{x}` after the
    // 100,000 elements and returns `true` after its traps.
    (
        "keymember_ext",
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
];

/// Return the runs of [`COSTS`] as `io:format("~w")` prints the result of
/// `list_probe:costs/1`.
///
/// # Examples
///
/// ```rust,ignore
/// // [{member,[{0,[{a,571},{b,571},...]},...]},...]
/// let text = costs_text();
/// ```
fn costs_text() -> String {
    let bifs: Vec<String> = COSTS
        .iter()
        .map(|(bif, sizes)| {
            let sizes: Vec<String> = sizes
                .iter()
                .map(|(size, lengths)| {
                    let runs: Vec<String> = lengths
                        .iter()
                        .map(|length| format!("{{a,{length}}},{{b,{length}}}"))
                        .collect();
                    format!("{{{size},[{}]}}", runs.join(","))
                })
                .collect();
            format!("{{{bif},[{}]}}", sizes.join(","))
        })
        .collect();
    format!("[{}]", bifs.join(","))
}

/// Return the runs of [`LONGS`] as `io:format("~w")` prints the result of
/// `list_probe:longs/1`.
///
/// # Examples
///
/// ```rust,ignore
/// // [{length_guard,[{b,667},{a,1},...]},...]
/// let text = longs_text();
/// ```
fn longs_text() -> String {
    let loops: Vec<String> = LONGS
        .iter()
        .map(|(name, runs)| {
            let runs: Vec<String> = runs
                .iter()
                .map(|(tag, length)| format!("{{{tag},{length}}}"))
                .collect();
            format!("{{{name},[{}]}}", runs.join(","))
        })
        .collect();
    format!("[{}]", loops.join(","))
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
fn charges_and_traps_the_list_bifs_as_pinned_erts_with_one_scheduler() {
    // Each list BIF reads the budget (`ERTS_BIF_REDS_LEFT`, `bif.h:218-219`)
    // and works on one chunk: `lists:member/2` and `length/1` 16 elements
    // for each reduction (`erl_bif_lists.c:1108-1142`;
    // `erl_bif_guard.c:270-313`), `lists:reverse/2` and `'++'/2` 60
    // (`erl_bif_lists.c:1194-1244`, `176-235`), `'++'/2` with `[]` 32
    // (`erl_bif_lists.c:61-89`), and `lists:keyfind/3` and
    // `lists:keymember/3` `10 * CONTEXT_REDS` with no cost
    // (`erl_bif_lists.c:1301-1381`). When elements remain, the BIF traps to
    // its export, and the process switches out
    // (`jit/arm/instr_bif.cpp:525-536`, `676-701`). The export continues the
    // BIF when the process runs again. The guard BIF `length/1` keeps its
    // trap state and runs the instruction again
    // (`jit/arm/instr_bif.cpp:198-337`).
    let program = BeamProgram::load(&[LIST_PROBE]).unwrap();
    let run = |function| {
        text(
            &program
                .execute("list_probe", function, &[Term::List(vec![])])
                .unwrap(),
        )
    };
    assert_eq!(run("costs"), costs_text());
    assert_eq!(run("longs"), longs_text());

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
            &format!(
                "jit = erlang:system_info(emu_flavor), O = {ERTS_OPTIONS}, \
                 [0] = lists:usort([element(tuple_size(G), G) || G <- list_probe:gcs(O)]), \
                 io:format(\"~w~n~w~n\", [list_probe:costs(O), list_probe:longs(O)]), halt()."
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

use std::process::{Command, Stdio};

use pbeam_runtime::{BeamProgram, HostTerm, Term};

const BINARY_FIND_PROBE: &[u8] =
    include_bytes!("../../fixtures/binary_find/binary_find_probe.beam");

/// An Erlang fun `T` for the `-eval` of an ERTS run: it gives the text of a
/// term as [`text`] does.
const ERTS_TEXT: &str = "T = fun T(X) when is_binary(X) -> \
                         [\"<<\", lists:join(\",\", [integer_to_list(B) || <<B>> <= X]), \">>\"]; \
                         T(X) when is_list(X) -> [\"[\", lists:join(\",\", [T(E) || E <- X]), \"]\"]; \
                         T(X) when is_tuple(X) -> \
                         [\"{\", lists:join(\",\", [T(E) || E <- tuple_to_list(X)]), \"}\"]; \
                         T(X) -> io_lib:write(X) end";

/// The spawn options of the measured processes in the ERTS reference run: a
/// heap of 4,000,000 words, so that no measured process collects garbage.
/// PBeam has no process heap and runs with `[]`.
const ERTS_OPTIONS: &str = "[{min_heap_size,4000000},{min_bin_vheap_size,40000000}]";

/// `binary_find_probe:results/0` on the pinned ERTS: the Boyer-Moore results
/// of `binary:match/2` for one needle (`erl_bif_binary.c:834-880`), the
/// Aho-Corasick results for two or more needles, which give the match that
/// starts first, and of these the longest (`erl_bif_binary.c:637-727`), and
/// `badarg` for each
/// subject that is not a binary and each pattern that is not a binary or a
/// proper list of binaries with at least one byte each
/// (`erl_bif_binary.c:1004-1043`, `1244-1270`), also for a tuple.
const RESULTS: &str = "[{6,3},{2,1},nomatch,{1,2},{0,9},nomatch,nomatch,{0,2},{2,4},{3,4},\
                       {1,5},{2,2},nomatch,{0,3},{1,3},{error,badarg},{error,badarg},\
                       {error,badarg},{error,badarg},\
                       {error,badarg},{error,badarg},{error,badarg},{error,badarg},\
                       {error,badarg},{error,badarg},{error,badarg},{error,badarg},\
                       {error,badarg},{error,badarg},{error,badarg},{error,badarg}]";

/// `binary_find_probe:costs/1` on the pinned ERTS with `erl +S 1`: the
/// result and the reductions of `binary:match/2` for a subject of `N` bytes
/// `$a` and a tail, with 1 for the `call_ext`. The search uses one
/// reduction for each 10 steps, and the `memchr` scan one step for each 8
/// bytes (`erl_bif_binary.c:824-880`). A subject of 319906 bytes or more is
/// longer than the scan of one turn: the first call uses all reductions and
/// traps, and the trap continues in the next turn
/// (`erl_bif_binary.c:1568-1571`, `1935-1951`). A needle of 21 bytes, whose
/// last byte the subject does not have, uses no step for the scan of 980
/// bytes, because the scan stops after the bytes that a match needs.
const COSTS: &str = "[{10,0,1,nomatch,1},{10,1,1,{10,1},1},{100000,0,1,nomatch,1251},\
                     {100000,1,1,{100000,1},1251},{319905,0,1,nomatch,3999},\
                     {319906,0,1,nomatch,3999},{1000000,0,1,nomatch,4501},\
                     {1000000,1,1,{1000000,1},4501},{10000,0,2,nomatch,501},\
                     {10000,2,2,{10000,2},501},{100000,0,2,nomatch,5001},\
                     {100000,2,2,{100000,2},5001},{100000,10,10,{100000,10},1001},\
                     {1000,0,21,nomatch,13}]";

/// Return a term as the `T` fun of [`ERTS_TEXT`] gives it: the `~w` text,
/// with each binary as its bytes, for the atoms, integers, binaries, lists,
/// and tuples of the probe results.
///
/// # Examples
///
/// ```rust,ignore
/// assert_eq!(text(&Term::list_from(vec![Term::binary(vec![97])])), "[<<97>>]");
/// ```
fn text(term: &Term) -> String {
    let join = |terms: &[Term]| terms.iter().map(text).collect::<Vec<_>>().join(",");
    match term {
        Term::Atom(atom) => atom.name().to_owned(),
        Term::Integer(value) => value.to_string(),
        _ if let Some(terms) = term.proper_list() => {
            format!("[{}]", join(&terms.cloned().collect::<Vec<_>>()))
        }
        Term::Tuple(terms) => format!("{{{}}}", join(terms)),
        Term::Binary(bytes) => {
            let bytes: Vec<String> = bytes.iter().map(u8::to_string).collect();
            format!("<<{}>>", bytes.join(","))
        }
        other => panic!("unexpected term {other:?}"),
    }
}

#[test]
fn matches_binaries_as_pinned_erts_with_one_scheduler() {
    let program = BeamProgram::load(&[BINARY_FIND_PROBE]).unwrap();
    let run = |function, arguments: &[HostTerm]| {
        text(
            &program
                .execute("binary_find_probe", function, arguments)
                .unwrap()
                .term,
        )
    };
    assert_eq!(run("results", &[]), RESULTS);
    assert_eq!(run("costs", &[program.host_term(Term::nil())]), COSTS);

    let reference = Command::new("erl")
        .args([
            "+S",
            "1",
            "-noshell",
            "-pa",
            "fixtures/binary_find",
            "-eval",
            &format!(
                "{ERTS_TEXT}, M = binary_find_probe, jit = erlang:system_info(emu_flavor), \
                 io:format(\"~s~n~s~n\", [T(M:results()), T(M:costs({ERTS_OPTIONS}))]), halt()."
            ),
        ])
        .stdin(Stdio::null())
        .output()
        .unwrap();
    assert!(
        reference.status.success() && reference.stderr.is_empty(),
        "{reference:?}"
    );
    assert_eq!(
        String::from_utf8(reference.stdout).unwrap(),
        format!("{RESULTS}\n{COSTS}\n")
    );
}

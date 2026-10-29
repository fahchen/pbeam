//! Runs `binary_to_list` on PBeam with one scheduler and compares the results and the reductions
//! with the pinned ERTS.
#![expect(
    clippy::unwrap_used,
    reason = "test helpers report a failure by a panic"
)]

use std::process::Command;

use pbeam_runtime::{AtomTable, BeamProgram, HostTerm, Term};

#[path = "../common/mod.rs"]
mod common;

const B2L_PROBE: &[u8] = include_bytes!("../../fixtures/preempt/b2l_probe.beam");

/// The spawn options of the measured processes in the ERTS reference run: a
/// heap of 10,000,000 words, so that no measured process collects garbage
/// for a list of 3,000,000 bytes. PBeam has no process heap and runs with
/// `[]`.
const ERTS_OPTIONS: &str = "[{min_heap_size,10000000},{min_bin_vheap_size,40000000}]";

/// `b2l_probe:results/0` on the pinned ERTS: `binary_to_list/1` of `<<>>`,
/// `<<1,2,3>>`, a binary at bit offset 4, an atom, a list, and an integer;
/// `binary_to_list/3` of parts and of bad positions and terms
/// (`binary.c:306-411`).
const RESULTS: &str = "{[[],[1,2,3],[188,222],{error,badarg},{error,badarg},{error,badarg}],\
                       [[1,2,3],[2],[3],[2,3],[188,222],[222],{error,badarg},{error,badarg},\
                       {error,badarg},{error,badarg},{error,badarg},{error,badarg},\
                       {error,badarg},{error,badarg},{error,badarg},{error,badarg}]}";

/// `b2l_probe:costs/1` on the pinned ERTS with `erl +S 1`: 1 for the call,
/// and one reduction for each 256 bytes (`binary.c:182`, `213-219`,
/// `262-265`). A chunk uses at most the budget, so a call that traps loses
/// 1 of the 3998 reductions of its first chunk and 1 of the 4001 of each
/// next chunk. `binary_to_list/3` decides on one chunk with the size of the
/// whole binary (`binary.c:385-398`).
const COSTS: &str = "[{{one,1},2},{{one,256},2},{{one,257},3},{{one,10000},41},\
                     {{one,1023231},3998},{{one,1023232},3998},{{one,2000000},7813},\
                     {{one,3000000},11718},{{three,10,1,10},2},{{three,2000000,1,10},2},\
                     {{three,2000000,1000001,2000000},3908},{{three,3000000,2,2999999},11718}]";

/// `b2l_probe:lows/1` on the pinned ERTS with `erl +S 1`: with 100 or fewer
/// reductions left, a binary of fewer than 256,000 bytes switches out
/// first, and the rest of the budget is virtual (`binary.c:331-342`,
/// `387-398`). With 101, it runs in one chunk.
const LOWS: &str = "[{{one,100000},101,4001},{{one,100000},100,4292},{{one,100000},0,4393},\
                    {{three,200000,1,10},101,3901},{{three,200000,1,10},100,3902},\
                    {{three,200000,1,10},0,4003},{{one,300000},101,5071},\
                    {{one,300000},100,5072},{{one,300000},0,5174}]";

/// `b2l_probe:pretty/0` on the pinned ERTS, as Latin-1 characters: the
/// text of `~p` for `<<1,2>>`, `<<"hello">>`, `<<"h", 233, "llo">>`, and
/// 100 times `$a` (`io_lib_pretty.erl:1255-1285`).
const PRETTY: &str = "<<1,2>>\n<<\"hello\">>\n<<\"h\u{e9}llo\">>\n<<\"aaaaaaaaaaaaaaaaaaaaaaaaa\
                      aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa\">>";

/// Return the text of `~w` for a term of the probe.
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
fn runs_binary_to_list_as_pinned_erts_with_one_scheduler() {
    let otp: Vec<Vec<u8>> = [
        "epp",
        "io_lib",
        "io_lib_format",
        "io_lib_pretty",
        "lists",
        "string",
        "unicode",
        "unicode_util",
    ]
    .map(common::pinned_beam)
    .into();
    let mut modules: Vec<&[u8]> = otp.iter().map(Vec::as_slice).collect();
    modules.push(B2L_PROBE);
    let program = BeamProgram::load(&modules).unwrap();
    let run = |function, arguments: &[HostTerm]| {
        let result = program.execute("b2l_probe", function, arguments).unwrap();
        text(&result.term, result.atoms())
    };
    let options = [program.host_term(Term::nil())];
    let pretty: Vec<String> = PRETTY.chars().map(|c| (c as u32).to_string()).collect();
    let pretty = format!("<<{}>>", pretty.join(","));
    assert_eq!(run("results", &[]), RESULTS);
    assert_eq!(run("costs", &options), COSTS);
    assert_eq!(run("lows", &options), LOWS);
    assert_eq!(run("pretty", &[]), pretty);

    let reference = Command::new("erl")
        .args([
            "+S",
            "1",
            "-noshell",
            "-pa",
            "fixtures/preempt",
            "-eval",
            &format!(
                "{{module, M}} = code:ensure_loaded(b2l_probe), \
                 jit = erlang:system_info(emu_flavor), O = {ERTS_OPTIONS}, \
                 io:format(\"~w~n~w~n~w~n~w~n\", [M:results(), M:costs(O), M:lows(O), M:pretty()]), \
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
        format!("{RESULTS}\n{COSTS}\n{LOWS}\n{pretty}\n")
    );
}

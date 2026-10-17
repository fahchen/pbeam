use std::process::{Command, Stdio};

use pbeam_runtime::{AtomTable, BeamProgram, HostTerm, Term};

const BINARY_MODULE_PROBE: &[u8] =
    include_bytes!("../../fixtures/binary_module/binary_module_probe.beam");

/// An Erlang fun `T` for the `-eval` of an ERTS run: it gives the text of a
/// term as [`text`] does.
const ERTS_TEXT: &str = "T = fun T(X) when is_binary(X) -> \
                         [\"<<\", lists:join(\",\", [integer_to_list(B) || <<B>> <= X]), \">>\"]; \
                         T(X) when is_list(X) -> [\"[\", lists:join(\",\", [T(E) || E <- X]), \"]\"]; \
                         T(X) when is_tuple(X) -> \
                         [\"{\", lists:join(\",\", [T(E) || E <- tuple_to_list(X)]), \"}\"]; \
                         T(X) -> io_lib:write(X) end";

/// The spawn options of the measured processes in the ERTS reference run: a
/// heap of 4,000,000 words and a binary vheap of 40,000,000 words, so that no
/// measured process collects garbage. PBeam has no process heap and runs with
/// `[]`.
const ERTS_OPTIONS: &str = "[{min_heap_size,4000000},{min_bin_vheap_size,40000000}]";

/// `binary_module_probe:results/0` on the pinned ERTS: the value of each
/// `binary:at/2`, `binary:part/2,3`, `binary:matches/2`, `binary:copy/1,2`,
/// and `binary:list_to_bin/1` call of the probe, and the `badarg` of each
/// call that ERTS rejects.
const RESULTS: &str = "[1,5,{error,badarg},{error,badarg},{error,badarg},{error,badarg},\
                        {error,badarg},{error,badarg},{error,badarg},99,<<1,2,3,4,5>>,<<2,3,4>>,\
                        <<>>,<<4,5>>,<<1,2>>,{error,badarg},{error,badarg},{error,badarg},\
                        {error,badarg},{error,badarg},{error,badarg},{error,badarg},<<2,3,4>>,\
                        <<>>,<<4,5>>,<<1,2,3,4,5>>,{error,badarg},{error,badarg},{error,badarg},\
                        {error,badarg},{error,badarg},{error,badarg},{error,badarg},{error,badarg},\
                        {error,badarg},[{0,3},{3,3}],[{6,3}],[],[{0,9}],[],[],[{0,2},{2,2}],\
                        [{1,4}],[{2,1},{5,1}],{error,badarg},{error,badarg},{error,badarg},\
                        {error,badarg},{error,badarg},{error,badarg},{error,badarg},<<1,2>>,\
                        <<1,2,1,2>>,<<>>,<<>>,{error,badarg},{error,badarg},{error,badarg},\
                        {error,badarg},{error,system_limit},{error,system_limit},<<97,98,99,3>>,\
                        <<>>,<<>>,{error,badarg},{error,badarg},{error,badarg},<<97,98>>,\
                        {error,badarg},<<>>,<<1,2,3>>]";

/// `binary_module_probe:traces/0` on the pinned ERTS: the reason, the
/// module, the function, and the arguments of the first stack frame of each
/// error, and the module of its `error_info` location. `save_stacktrace`
/// builds the frame `{Module, Function, Args, Location}` of a BIF with the
/// arguments of the call and the location
/// `[{error_info, #{module => Format}}]` (`beam_common.c:939-1005`), and
/// the format module of the `binary` module is `erl_stdlib_errors`.
const TRACES: &str = "[{badarg,{binary,at,[<<1,2>>,5]},erl_stdlib_errors},\
                      {badarg,{binary,part,[<<1,2>>,{0,3}]},erl_stdlib_errors},\
                      {badarg,{binary,part,[<<1,2>>,0,-3]},erl_stdlib_errors},\
                      {badarg,{binary,matches,[<<97>>,<<>>]},erl_stdlib_errors},\
                      {badarg,{binary,copy,[<<1>>,-1]},erl_stdlib_errors},\
                      {badarg,{binary,list_to_bin,[[256]]},erl_stdlib_errors}]";

/// `binary_module_probe:last/0` on the pinned ERTS: the last byte of each
/// subject, including an append-built binary of 70001 bytes, and the
/// `badarg` of the empty binary and of a term that is not a binary
/// (`binary_last_1`, `erl_bif_binary.c:2311-2333`).
const LAST: &str = "[5,7,255,{error,badarg},{error,badarg},{error,badarg}]";

/// `binary_module_probe:last_traces/0` on the pinned ERTS: the first stack
/// frame of each `binary:last/1` error, as [`TRACES`] gives it.
const LAST_TRACES: &str = "[{badarg,{binary,last,[<<>>]},erl_stdlib_errors},\
                           {badarg,{binary,last,[foo]},erl_stdlib_errors}]";

/// `binary_module_probe:costs/1` on the pinned ERTS with `erl +S 1`: the
/// summary of the matches and the reductions of `binary:matches/2` for a
/// subject of `N` bytes `$a` and `Tail`, with 1 for the `call_ext`. The
/// search uses one reduction for each 10 steps (`erl_bif_binary.c:635`,
/// `824`), and the result build one step for each match of its integer
/// phase and of its cons phase (`do_match_global_result`,
/// `erl_bif_binary.c:1699-1730`). A search or a build longer than one turn
/// uses all reductions of the turn and traps to `erlang:binary_find_trap/3`
/// (`erl_bif_binary.c:1568-1571`, `1935-1951`).
const COSTS: &str = "[{10,0,<<97>>,{10,{0,1},10},29},{1000,0,<<97>>,{1000,{0,1},1000},2368},\
                     {1000,1,<<98>>,{1,{1000,1},1},20},\
                     {100000,0,<<97>>,{100000,{0,1},100000},212256},\
                     {10000,0,<<97,97>>,{5000,{0,2},10000},11818},\
                     {1000,0,[<<97>>,<<97,97>>],{500,{0,2},1000},1237},\
                     {1000,0,<<98>>,{0,none,0},18},\
                     {10,0,<<97,97,97,97,97,97,97,97,97,97>>,{1,{0,10},10},8},\
                     {1000,1,<<97,97>>,{500,{0,2},1000},1187},\
                     {100000,1,[<<97>>,<<98>>],{100001,{0,1},100001},212259}]";

/// Return a term as the `T` fun of [`ERTS_TEXT`] gives it: the `~w` text,
/// with each binary as its bytes, for the atoms, integers, binaries, proper
/// and improper lists, and tuples of the probe results.
///
/// The atom names come from `atoms`, the table that owns the term.
///
/// # Examples
///
/// ```rust,ignore
/// assert_eq!(text(&Term::list_from(vec![Term::binary(vec![97])]), &atoms), "[<<97>>]");
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
        Term::Nil | Term::Cons(_) => {
            let (elements, tail) = term.list_cells().expect("the term is a list");
            let elements: Vec<String> = elements.map(|element| text(element, atoms)).collect();
            match tail {
                None => format!("[{}]", elements.join(",")),
                Some(tail) => format!("[{}|{}]", elements.join(","), text(tail, atoms)),
            }
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
fn runs_the_binary_module_bifs_as_pinned_erts() {
    // The pinned compiler emits each call of a `binary` BIF as a `bif` or
    // `gc_bif` instruction and the calls of the helpers of the probe as
    // external calls, so the fixture runs each path.
    let program = BeamProgram::load(&[BINARY_MODULE_PROBE]).unwrap();
    let run = |function, arguments: &[HostTerm]| {
        let result = program
            .execute("binary_module_probe", function, arguments)
            .unwrap();
        text(&result.term, result.atoms())
    };
    assert_eq!(run("results", &[]), RESULTS);
    assert_eq!(run("traces", &[]), TRACES);
    assert_eq!(run("costs", &[program.host_term(Term::nil())]), COSTS);

    let reference = Command::new("erl")
        .args([
            "+S",
            "1",
            "-noshell",
            "-pa",
            "fixtures/binary_module",
            "-eval",
            &format!(
                "{ERTS_TEXT}, M = binary_module_probe, \
                 io:format(\"~s~n~s~n~s~n\", [T(M:results()), T(M:traces()), T(M:costs({ERTS_OPTIONS}))]), halt()."
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
        format!("{RESULTS}\n{TRACES}\n{COSTS}\n")
    );
}

#[test]
fn runs_binary_last_as_pinned_erts() {
    // The pinned compiler emits a `binary:last/1` call as an external call,
    // so the probe runs it through the import path of the interpreter.
    let program = BeamProgram::load(&[BINARY_MODULE_PROBE]).unwrap();
    let run = |function| {
        let result = program
            .execute("binary_module_probe", function, &[])
            .unwrap();
        text(&result.term, result.atoms())
    };
    assert_eq!(run("last"), LAST);
    assert_eq!(run("last_traces"), LAST_TRACES);

    let reference = Command::new("erl")
        .args([
            "+S",
            "1",
            "-noshell",
            "-pa",
            "fixtures/binary_module",
            "-eval",
            &format!(
                "{ERTS_TEXT}, M = binary_module_probe, \
                 io:format(\"~s~n~s~n\", [T(M:last()), T(M:last_traces())]), halt()."
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
        format!("{LAST}\n{LAST_TRACES}\n")
    );
}

#[test]
fn gives_parts_that_share_the_bytes_of_their_source_and_copies_that_do_not() {
    // `erts_binary_part` makes a sub-binary that refers to the source
    // (`erl_bif_binary.c:1954-2012`), so a part points into the array of its
    // source. `binary:copy/1,2` always makes a new binary, also for the
    // count 1 (`do_binary_copy`, `erl_bif_binary.c:2400-2447`), so a copy
    // points outside the array of its source.
    let program = BeamProgram::load(&[BINARY_MODULE_PROBE]).unwrap();
    let result = program
        .execute("binary_module_probe", "shared", &[])
        .unwrap();
    let Term::Tuple(terms) = &result.term else {
        panic!("the probe gives a tuple: {:?}", result.term);
    };
    let Term::Binary(source) = &terms[0] else {
        panic!("the source is a binary");
    };
    assert_eq!(&source[..], b"\x01\x02\x03\x04\x05");
    let start = source.as_ptr() as usize;
    let end = start + source.len();
    for (index, term) in terms[1..].iter().enumerate() {
        let Term::Binary(part) = term else {
            panic!("a part or a copy is a binary: {term:?}");
        };
        let pointer = part.as_ptr() as usize;
        if index < 2 {
            assert_eq!(&part[..], b"\x02\x03\x04", "the parts of the probe");
            assert!(
                (start..end).contains(&pointer),
                "the part {pointer:#x} is not in the array of the source {start:#x}"
            );
        } else {
            assert_eq!(&part[..], &source[..], "the copies of the probe");
            assert!(
                !(start..end).contains(&pointer),
                "the copy {pointer:#x} is in the array of the source {start:#x}"
            );
        }
    }
}

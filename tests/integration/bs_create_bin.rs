use std::process::Command;

use pbeam_runtime::{BeamProgram, Map, Term};

const BS_CREATE_BIN_PROBE: &[u8] =
    include_bytes!("../../fixtures/bs_create_bin/bs_create_bin_probe.beam");
const BS_CREATE_BIN_ALLOC_PROBE: &[u8] =
    include_bytes!("../../fixtures/bs_create_bin/bs_create_bin_alloc_probe.beam");
const BS_CREATE_BIN_ALLOC_LIMIT: &[u8] =
    include_bytes!("../../fixtures/bs_create_bin/bs_create_bin_alloc_limit.beam");

/// Evaluate an expression on the pinned ERTS with `bs_create_bin_probe` in
/// the code path and return its `~w` output.
fn reference(expression: &str) -> String {
    let reference = Command::new("erl")
        .args([
            "-noshell",
            "-pa",
            "fixtures/bs_create_bin",
            "-eval",
            &format!("io:format(\"~w~n\", [{expression}]), halt()."),
        ])
        .output()
        .unwrap();
    assert!(
        reference.status.success() && reference.stderr.is_empty(),
        "{reference:?}"
    );
    String::from_utf8(reference.stdout).unwrap()
}

/// Run `bs_create_bin_probe:Function/1` with one argument.
fn run(program: &BeamProgram, function: &str, argument: Term) -> Term {
    program
        .execute("bs_create_bin_probe", function, &[argument])
        .unwrap()
}

#[test]
fn builds_string_and_binary_segments_as_pinned_erts() {
    // `prefix/1` is `<<"count: ", X/binary>>`, as Elixir compiles
    // `"count: " <> X`, and `around/1` is `<<"worker ", X/binary,
    // " crashed">>`. The empty binary and the bytes that are not ASCII
    // are copied as they are.
    let program = BeamProgram::load(&[BS_CREATE_BIN_PROBE]).unwrap();
    let binary = |bytes: &[u8]| Term::Binary(bytes.to_vec());
    assert_eq!(
        [
            run(&program, "prefix", binary(b"3")),
            run(&program, "prefix", binary(b"")),
            run(&program, "around", binary(b"12")),
            run(&program, "around", binary(&[0, 195, 169, 255])),
        ],
        [
            binary(b"count: 3"),
            binary(b"count: "),
            binary(b"worker 12 crashed"),
            binary(b"worker \x00\xC3\xA9\xFF crashed"),
        ]
    );
    assert_eq!(
        reference(
            "[bs_create_bin_probe:prefix(<<\"3\">>), bs_create_bin_probe:prefix(<<>>), \
             bs_create_bin_probe:around(<<\"12\">>), \
             bs_create_bin_probe:around(<<0,195,169,255>>)]"
        ),
        "[<<99,111,117,110,116,58,32,51>>,<<99,111,117,110,116,58,32>>,\
         <<119,111,114,107,101,114,32,49,50,32,99,114,97,115,104,101,100>>,\
         <<119,111,114,107,101,114,32,0,195,169,255,32,99,114,97,115,104,101,100>>]\n"
    );
}

#[test]
fn jumps_to_the_fail_label_for_a_term_that_is_not_a_binary() {
    // The guard of `guard/1` compiles to `bs_create_bin` with the fail
    // label of the next clause. A term that is not a binary jumps to it.
    let program = BeamProgram::load(&[BS_CREATE_BIN_PROBE]).unwrap();
    let atom = |name: &str| Term::Atom(program.atom(name).unwrap());
    assert_eq!(
        [
            run(&program, "guard", Term::Binary(b"b".to_vec())),
            run(&program, "guard", Term::Binary(b"bc".to_vec())),
            run(&program, "guard", atom("other")),
        ],
        [atom("ab"), atom("other"), atom("other")]
    );
    assert_eq!(
        reference("[bs_create_bin_probe:guard(T) || T <- [<<\"b\">>, <<\"bc\">>, other]]"),
        "[ab,other,other]\n"
    );
}

#[test]
fn raises_badarg_with_the_erts_error_info_for_a_term_that_is_not_a_binary() {
    // `catch_bad/1` catches the exception of `prefix/1`. ERTS gives
    // `badarg` with the `error_info` cause `{2, binary, type, Value}` in the
    // frame of `prefix/1`; 2 is the segment number of the compiler. The
    // fixture has no line table, so the frame has no `file` or `line`.
    // PBeam gives only this frame (row "Exception stack trace").
    let program = BeamProgram::load(&[BS_CREATE_BIN_PROBE]).unwrap();
    let atom = |name: &str| Term::Atom(program.atom(name).unwrap());
    let caught = |value: Term| {
        let cause = Term::Tuple(vec![Term::Integer(2), atom("binary"), atom("type"), value]);
        let error_info = Map::from_pairs(vec![
            (atom("cause"), cause),
            (atom("function"), atom("format_bs_fail")),
            (atom("module"), atom("erl_erts_errors")),
        ])
        .unwrap();
        let frame = Term::Tuple(vec![
            atom("bs_create_bin_probe"),
            atom("prefix"),
            Term::Integer(1),
            Term::List(vec![Term::Tuple(vec![
                atom("error_info"),
                Term::Map(error_info),
            ])]),
        ]);
        Term::Tuple(vec![atom("error"), atom("badarg"), Term::List(vec![frame])])
    };
    let values = [
        atom("other"),
        Term::Integer(1),
        Term::List(Vec::new()),
        Term::Tuple(vec![atom("other")]),
    ];
    for value in values {
        assert_eq!(run(&program, "catch_bad", value.clone()), caught(value));
    }
    assert_eq!(
        reference(
            "[begin {C, R, S} = bs_create_bin_probe:catch_bad(T), {C, R, hd(S)} end \
             || T <- [other, 1, [], {other}]]"
        ),
        "[{error,badarg,{bs_create_bin_probe,prefix,1,[{error_info,#{cause => {2,binary,type,other},\
         function => format_bs_fail,module => erl_erts_errors}}]}},\
         {error,badarg,{bs_create_bin_probe,prefix,1,[{error_info,#{cause => {2,binary,type,1},\
         function => format_bs_fail,module => erl_erts_errors}}]}},\
         {error,badarg,{bs_create_bin_probe,prefix,1,[{error_info,#{cause => {2,binary,type,[]},\
         function => format_bs_fail,module => erl_erts_errors}}]}},\
         {error,badarg,{bs_create_bin_probe,prefix,1,[{error_info,#{cause => {2,binary,type,{other}},\
         function => format_bs_fail,module => erl_erts_errors}}]}}]\n"
    );
}

#[test]
fn exits_with_the_badarg_and_its_error_info_when_no_catch_holds_it() {
    // With no catch, the root process exits with `{badarg, Stack}`. The
    // first frame of the ERTS stack trace is the frame of `prefix/1`.
    let program = BeamProgram::load(&[BS_CREATE_BIN_PROBE]).unwrap();
    let other = Term::Atom(program.atom("other").unwrap());
    assert_eq!(
        program
            .execute("bs_create_bin_probe", "prefix", &[other])
            .unwrap_err()
            .to_string(),
        "root process exited with {badarg,[{bs_create_bin_probe,prefix,1,[{error_info,\
         #{cause=>{2,binary,type,other},function=>format_bs_fail,\
         module=>erl_erts_errors}}]}]}"
    );
    assert_eq!(
        reference(
            "begin {'EXIT', {R, [F | _]}} = catch bs_create_bin_probe:prefix(other), {R, F} end"
        ),
        "{badarg,{bs_create_bin_probe,prefix,1,[{error_info,#{cause => {2,binary,type,other},\
         function => format_bs_fail,module => erl_erts_errors}}]}}\n"
    );
}

#[test]
fn uses_one_reduction_for_each_1024_bytes_of_a_binary_segment() {
    // `cost/1` gives the reductions between two `process_info/2` calls
    // around one `bs_create_bin`: 1 for the second call, and the
    // reductions of `erts_bs_put_binary_all` (erl_bits.c:1311-1332): the
    // bit size of the binary divided by 8192. The ERTS run gives the
    // process a large heap, so that no garbage collection occurs.
    let sizes = [0, 1023, 1024, 2047, 2048, 100_000];
    let program = BeamProgram::load(&[BS_CREATE_BIN_PROBE]).unwrap();
    for size in sizes {
        let mut expected = vec![b'x'];
        expected.extend(std::iter::repeat_n(1, size));
        assert_eq!(
            run(&program, "cost", Term::Binary(vec![1; size])),
            Term::Tuple(vec![
                Term::Integer(1 + size as i64 / 1024),
                Term::Binary(expected)
            ])
        );
    }
    assert_eq!(
        reference(
            "begin Self = self(), [begin spawn_opt(fun() -> {C, B} = \
             bs_create_bin_probe:cost(binary:copy(<<1>>, N)), Self ! {N, C, byte_size(B)} end, \
             [{min_heap_size,4000000},{min_bin_vheap_size,40000000}]), \
             receive R -> R end end || N <- [0, 1023, 1024, 2047, 2048, 100000]] end"
        ),
        "[{0,1,1},{1023,1,1024},{1024,2,1025},{2047,2,2048},{2048,3,2049},{100000,98,100001}]\n"
    );
}

#[test]
fn stops_at_a_segment_type_that_is_not_implemented() {
    // `byte/1` is `<<"b", X>>`: a string segment and an integer segment.
    // ERTS gives `<<98,7>>`. PBeam stops at the integer segment (row
    // "Binary construction").
    let program = BeamProgram::load(&[BS_CREATE_BIN_PROBE]).unwrap();
    assert_eq!(
        program
            .execute("bs_create_bin_probe", "byte", &[Term::Integer(7)])
            .unwrap_err()
            .to_string(),
        "unsupported bs_create_bin segment integer"
    );
    assert_eq!(reference("bs_create_bin_probe:byte(7)"), "<<98,7>>\n");
}

#[test]
fn loads_an_allocation_list_as_the_alloc_operand() {
    // `wrap/1` builds a fun after the binary, so erlc gives its
    // `bs_create_bin` the `Alloc` operand {alloc,[{words,1},{floats,0},
    // {funs,1}]}, as `Elixir.Code.Formatter:binary_op_to_algebra/8` has.
    // `run/1` calls the fun.
    let program = BeamProgram::load(&[BS_CREATE_BIN_ALLOC_PROBE]).unwrap();
    let run = |bytes: &[u8]| {
        program
            .execute(
                "bs_create_bin_alloc_probe",
                "run",
                &[Term::Binary(bytes.to_vec())],
            )
            .unwrap()
    };
    assert_eq!(
        [run(b"bc"), run(b"")],
        [Term::Binary(b"abc".to_vec()), Term::Binary(b"a".to_vec())]
    );
    assert_eq!(
        reference(
            "[bs_create_bin_alloc_probe:run(<<\"bc\">>), bs_create_bin_alloc_probe:run(<<>>)]"
        ),
        "[<<97,98,99>>,<<97>>]\n"
    );
}

#[test]
fn rejects_an_allocation_list_that_erts_rejects() {
    // `fixtures/bs_create_bin/bs_create_bin_alloc_limit.asm` gives
    // `bs_create_bin` an allocation list of 2147483648 heap words. ERTS
    // rejects a sum larger than `i32::MAX` (`beam_file.c:2268-2270`). The
    // logger level `none` keeps the ERTS load error report out of the
    // output.
    assert_eq!(
        BeamProgram::load(&[BS_CREATE_BIN_ALLOC_LIMIT])
            .unwrap_err()
            .to_string(),
        "BEAM allocation list exceeds the heap word limit"
    );
    assert_eq!(
        reference(
            "begin logger:set_primary_config(level, none), \
             code:ensure_loaded(bs_create_bin_alloc_limit) end"
        ),
        "{error,badfile}\n"
    );
}

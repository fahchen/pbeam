use std::{process::Command, sync::Arc};

use num_bigint::BigInt;
use pbeam_runtime::{AtomTable, BeamProgram, Map, Term};

const BS_CREATE_BIN_PROBE: &[u8] =
    include_bytes!("../../fixtures/bs_create_bin/bs_create_bin_probe.beam");
const BS_CREATE_BIN_APPEND_PROBE: &[u8] =
    include_bytes!("../../fixtures/bs_create_bin/bs_create_bin_append_probe.beam");
const BS_CREATE_BIN_ALLOC_PROBE: &[u8] =
    include_bytes!("../../fixtures/bs_create_bin/bs_create_bin_alloc_probe.beam");
const BS_CREATE_BIN_INTEGER_PROBE: &[u8] =
    include_bytes!("../../fixtures/bs_create_bin/bs_create_bin_integer_probe.beam");
const BS_CREATE_BIN_WRITABLE_PROBE: &[u8] =
    include_bytes!("../../fixtures/bs_create_bin/bs_create_bin_writable_probe.beam");
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
        .execute(
            "bs_create_bin_probe",
            function,
            &[program.host_term(argument)],
        )
        .unwrap()
        .term
}

#[test]
fn builds_string_and_binary_segments_as_pinned_erts() {
    // `prefix/1` is `<<"count: ", X/binary>>`, as Elixir compiles
    // `"count: " <> X`, and `around/1` is `<<"worker ", X/binary,
    // " crashed">>`. The empty binary and the bytes that are not ASCII
    // are copied as they are.
    let program = BeamProgram::load(&[BS_CREATE_BIN_PROBE]).unwrap();
    let binary = |bytes: &[u8]| Term::binary(bytes.to_vec());
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
            run(&program, "guard", Term::binary(b"b".to_vec())),
            run(&program, "guard", Term::binary(b"bc".to_vec())),
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
        let cause = Term::tuple(vec![Term::Integer(2), atom("binary"), atom("type"), value]);
        let error_info = Map::from_pairs(
            vec![
                (atom("cause"), cause),
                (atom("function"), atom("format_bs_fail")),
                (atom("module"), atom("erl_erts_errors")),
            ],
            program.atoms(),
        )
        .unwrap();
        let frame = Term::tuple(vec![
            atom("bs_create_bin_probe"),
            atom("prefix"),
            Term::Integer(1),
            Term::list_from(vec![Term::tuple(vec![
                atom("error_info"),
                Term::Map(error_info),
            ])]),
        ]);
        Term::tuple(vec![
            atom("error"),
            atom("badarg"),
            Term::list_from(vec![frame]),
        ])
    };
    let values = [
        atom("other"),
        Term::Integer(1),
        Term::nil(),
        Term::tuple(vec![atom("other")]),
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
            .execute("bs_create_bin_probe", "prefix", &[program.host_term(other)])
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
            run(&program, "cost", Term::binary(vec![1; size])),
            Term::tuple(vec![
                Term::Integer(1 + size as i64 / 1024),
                Term::binary(expected)
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
fn builds_a_utf16_segment_after_a_string_segment_as_pinned_erts() {
    // `utf16/1` is `<<"b", X/utf16>>`: a `string` segment, then a `utf16`
    // segment that writes `0` and `X` in big-endian order. ERTS gives
    // `<<98,0,7>>`.
    let program = BeamProgram::load(&[BS_CREATE_BIN_PROBE]).unwrap();
    assert_eq!(
        program
            .execute(
                "bs_create_bin_probe",
                "utf16",
                &[program.host_term(Term::Integer(7))],
            )
            .unwrap()
            .term,
        Term::binary(vec![98, 0, 7])
    );
    assert_eq!(reference("bs_create_bin_probe:utf16(7)"), "<<98,0,7>>\n");
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
                &[program.host_term(Term::binary(bytes.to_vec()))],
            )
            .unwrap()
    };
    assert_eq!(
        [run(b"bc"), run(b"")].map(|host| host.term),
        [Term::binary(b"abc".to_vec()), Term::binary(b"a".to_vec())]
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

/// Run `bs_create_bin_append_probe:Function` with its arguments.
fn run_append(program: &BeamProgram, function: &str, arguments: &[Term]) -> Term {
    let arguments = arguments
        .iter()
        .cloned()
        .map(|term| program.host_term(term))
        .collect::<Vec<_>>();
    program
        .execute("bs_create_bin_append_probe", function, &arguments)
        .unwrap()
        .term
}

#[test]
fn builds_an_append_segment_as_pinned_erts() {
    // `two/2` is `<<A/binary, B/binary>>`, as Elixir compiles `A <> B`, and
    // `tail/1` is `<<A/binary, "!">>`. erlc makes `A` an `append` segment.
    // `built/1` appends to a binary that a `string` and a `binary` segment
    // built in the same function, and `again/1` appends to a binary that
    // an `append` built. `unit16/2` and `bits/2` have an `append` segment
    // of unit 16 and unit 1.
    let program = BeamProgram::load(&[BS_CREATE_BIN_APPEND_PROBE]).unwrap();
    let binary = |bytes: &[u8]| Term::binary(bytes.to_vec());
    assert_eq!(
        [
            run_append(&program, "two", &[binary(b"ab"), binary(b"cd")]),
            run_append(&program, "two", &[binary(b""), binary(b"")]),
            run_append(&program, "two", &[binary(&[1, 2]), binary(b"")]),
            run_append(&program, "tail", &[binary(b"hi")]),
            run_append(&program, "tail", &[binary(b"")]),
            run_append(&program, "built", &[binary(b"bc")]),
            run_append(&program, "again", &[binary(b"x")]),
            run_append(&program, "unit16", &[binary(&[1, 2]), binary(&[3])]),
            run_append(&program, "unit16", &[binary(b""), binary(&[3])]),
            run_append(&program, "bits", &[binary(&[1, 2, 3]), binary(&[4])]),
        ],
        [
            binary(b"abcd"),
            binary(b""),
            binary(&[1, 2]),
            binary(b"hi!"),
            binary(b"!"),
            binary(b"abcz"),
            binary(b"xyz"),
            binary(&[1, 2, 3]),
            binary(&[3]),
            binary(&[1, 2, 3, 4]),
        ]
    );
    assert_eq!(
        reference(
            "begin M = bs_create_bin_append_probe, \
             [M:two(<<\"ab\">>, <<\"cd\">>), M:two(<<>>, <<>>), M:two(<<1,2>>, <<>>), \
             M:tail(<<\"hi\">>), M:tail(<<>>), M:built(<<\"bc\">>), M:again(<<\"x\">>), \
             M:unit16(<<1,2>>, <<3>>), M:unit16(<<>>, <<3>>), M:bits(<<1,2,3>>, <<4>>)] end"
        ),
        "[<<97,98,99,100>>,<<>>,<<1,2>>,<<104,105,33>>,<<33>>,<<97,98,99,122>>,<<120,121,122>>,\
         <<1,2,3>>,<<3>>,<<1,2,3,4>>]\n"
    );
}

#[test]
fn jumps_to_the_fail_label_for_an_append_segment_that_fails() {
    // The guard of `guard/2` compiles to `bs_create_bin` with an `append`
    // segment and the fail label of the next clause.
    let program = BeamProgram::load(&[BS_CREATE_BIN_APPEND_PROBE]).unwrap();
    let atom = |name: &str| Term::Atom(program.atom(name).unwrap());
    let binary = |bytes: &[u8]| Term::binary(bytes.to_vec());
    let cases = [
        (binary(b"a"), binary(b"b")),
        (binary(b"ab"), binary(b"")),
        (binary(b"a"), binary(b"c")),
        (atom("other"), binary(b"b")),
        (binary(b"a"), atom("ab")),
    ];
    assert_eq!(
        cases.map(|(a, b)| run_append(&program, "guard", &[a, b])),
        [
            atom("ab"),
            atom("ab"),
            atom("other"),
            atom("other"),
            atom("other")
        ]
    );
    assert_eq!(
        reference(
            "[bs_create_bin_append_probe:guard(A, B) || {A, B} <- \
             [{<<\"a\">>, <<\"b\">>}, {<<\"ab\">>, <<>>}, {<<\"a\">>, <<\"c\">>}, \
             {other, <<\"b\">>}, {<<\"a\">>, ab}]]"
        ),
        "[ab,ab,other,other,other]\n"
    );
}

#[test]
fn raises_badarg_with_the_erts_error_info_for_an_append_segment() {
    // ERTS checks the `binary` segments before the `append` segment, so
    // with two bad terms the cause names segment 2. A binary whose bit
    // size is not a multiple of 16 fails the `append` segment of unit 16
    // with the reason `unit`.
    let program = BeamProgram::load(&[BS_CREATE_BIN_APPEND_PROBE]).unwrap();
    let atom = |name: &str| Term::Atom(program.atom(name).unwrap());
    let binary = |bytes: &[u8]| Term::binary(bytes.to_vec());
    let caught = |function: &str, segment: i64, reason: &str, value: Term| {
        let cause = Term::tuple(vec![
            Term::Integer(segment),
            atom("binary"),
            atom(reason),
            value,
        ]);
        let error_info = Map::from_pairs(
            vec![
                (atom("cause"), cause),
                (atom("function"), atom("format_bs_fail")),
                (atom("module"), atom("erl_erts_errors")),
            ],
            program.atoms(),
        )
        .unwrap();
        let frame = Term::tuple(vec![
            atom("bs_create_bin_append_probe"),
            atom(function),
            Term::Integer(2),
            Term::list_from(vec![Term::tuple(vec![
                atom("error_info"),
                Term::Map(error_info),
            ])]),
        ]);
        Term::tuple(vec![
            atom("error"),
            atom("badarg"),
            Term::list_from(vec![frame]),
        ])
    };
    let cases = [
        ("two", atom("other"), binary(b""), 1, "type", atom("other")),
        (
            "two",
            Term::Integer(1),
            binary(b"x"),
            1,
            "type",
            Term::Integer(1),
        ),
        ("two", Term::nil(), binary(b""), 1, "type", Term::nil()),
        ("two", binary(b"a"), atom("ab"), 2, "type", atom("ab")),
        ("two", atom("other"), atom("ab"), 2, "type", atom("ab")),
        (
            "unit16",
            binary(&[1, 2, 3]),
            binary(b""),
            1,
            "unit",
            binary(&[1, 2, 3]),
        ),
        (
            "unit16",
            binary(&[1]),
            binary(&[4]),
            1,
            "unit",
            binary(&[1]),
        ),
        (
            "unit16",
            binary(&[1, 2, 3]),
            atom("ab"),
            2,
            "type",
            atom("ab"),
        ),
        (
            "unit16",
            atom("other"),
            binary(b""),
            1,
            "type",
            atom("other"),
        ),
    ];
    for (function, a, b, segment, reason, value) in cases {
        assert_eq!(
            run_append(&program, &format!("catch_{function}"), &[a, b]),
            caught(function, segment, reason, value)
        );
    }
    let frame = |function: &str, cause: &str| {
        format!(
            "{{error,badarg,{{bs_create_bin_append_probe,{function},2,[{{error_info,\
             #{{cause => {{{cause}}},function => format_bs_fail,module => erl_erts_errors}}}}]}}}}"
        )
    };
    let expected = [
        frame("two", "1,binary,type,other"),
        frame("two", "1,binary,type,1"),
        frame("two", "1,binary,type,[]"),
        frame("two", "2,binary,type,ab"),
        frame("two", "2,binary,type,ab"),
        frame("unit16", "1,binary,unit,<<1,2,3>>"),
        frame("unit16", "1,binary,unit,<<1>>"),
        frame("unit16", "2,binary,type,ab"),
        frame("unit16", "1,binary,type,other"),
    ];
    assert_eq!(
        reference(
            "begin M = bs_create_bin_append_probe, \
             [begin {C, R, S} = M:F(A, B), {C, R, hd(S)} end || {F, A, B} <- \
             [{catch_two, other, <<>>}, {catch_two, 1, <<\"x\">>}, {catch_two, [], <<>>}, \
             {catch_two, <<\"a\">>, ab}, {catch_two, other, ab}, \
             {catch_unit16, <<1,2,3>>, <<>>}, {catch_unit16, <<1>>, <<4>>}, \
             {catch_unit16, <<1,2,3>>, ab}, {catch_unit16, other, <<>>}]] end"
        ),
        format!("[{}]\n", expected.join(","))
    );
}

#[test]
fn uses_one_reduction_for_each_1024_bytes_of_an_appended_binary() {
    // `cost/2` gives the reductions between two `process_info/2` calls
    // around `<<A/binary, B/binary>>`: 1 for the second call, then the
    // copy of `A` by `erts_bs_append_checked` for a binary that is not
    // writable (erl_bits.c:1916-1921) and the copy of `B`, each its bit
    // size divided by 8192. When `B` is empty, the result is `A` and no
    // copy occurs (erl_bits.c:1882-1888). The ERTS run gives the process
    // a large heap, so that no garbage collection occurs.
    // (size of A, size of B, reductions, size of the result)
    let results = [
        (0, 0, 1, 0),
        (1, 0, 1, 1),
        (8192, 0, 1, 8192),
        (8192, 1, 9, 8193),
        (1023, 1, 1, 1024),
        (1024, 1, 2, 1025),
        (2048, 1, 3, 2049),
        (100_000, 0, 1, 100_000),
        (100_000, 1, 98, 100_001),
        (0, 1024, 2, 1024),
        (1024, 1024, 3, 2048),
        (100_000, 100_000, 195, 200_000),
    ];
    let program = BeamProgram::load(&[BS_CREATE_BIN_APPEND_PROBE]).unwrap();
    for (a, b, reductions, size) in results {
        assert_eq!(
            run_append(
                &program,
                "cost",
                &[Term::binary(vec![1; a]), Term::binary(vec![2; b])],
            ),
            Term::tuple(vec![Term::Integer(reductions), Term::Integer(size)]),
            "cost(<<1:{a}/unit:8>>, <<2:{b}/unit:8>>)"
        );
    }
    assert_eq!(
        reference(
            "begin Self = self(), [begin spawn_opt(fun() -> {C, S} = \
             bs_create_bin_append_probe:cost(binary:copy(<<1>>, A), binary:copy(<<2>>, B)), \
             Self ! {A, B, C, S} end, \
             [{min_heap_size,4000000},{min_bin_vheap_size,40000000}]), \
             receive R -> R end end || {A, B} <- [{0,0},{1,0},{8192,0},{8192,1},{1023,1},\
             {1024,1},{2048,1},{100000,0},{100000,1},{0,1024},{1024,1024},{100000,100000}]] end"
        ),
        "[{0,0,1,0},{1,0,1,1},{8192,0,1,8192},{8192,1,9,8193},{1023,1,1,1024},{1024,1,2,1025},\
         {2048,1,3,2049},{100000,0,1,100000},{100000,1,98,100001},{0,1024,2,1024},\
         {1024,1024,3,2048},{100000,100000,195,200000}]\n"
    );
}

#[test]
fn copies_a_binary_that_an_append_built_where_erts_appends_in_place() {
    // `cost_again/2` appends `B` to `<<A/binary, "y">>`, a binary that an
    // `append` built. ERTS appends in place to this writable binary and
    // uses reductions only for the copy of `B` while the binary has room
    // (erl_bits.c:1773-1849). PBeam has no writable binary, so it copies
    // the start of the result as for each other binary (row "Binary
    // construction").
    // (size of A, size of B, PBeam reductions, size of the result); the
    // ERTS reductions follow in the reference run.
    let results = [
        (0, 0, 1, 1),
        (0, 1, 1, 2),
        (1024, 1, 2, 1026),
        (100_000, 1, 98, 100_002),
        (100_000, 0, 1, 100_001),
        (0, 1024, 2, 1025),
    ];
    let program = BeamProgram::load(&[BS_CREATE_BIN_APPEND_PROBE]).unwrap();
    for (a, b, reductions, size) in results {
        assert_eq!(
            run_append(
                &program,
                "cost_again",
                &[Term::binary(vec![1; a]), Term::binary(vec![2; b])],
            ),
            Term::tuple(vec![Term::Integer(reductions), Term::Integer(size)]),
            "cost_again(<<1:{a}/unit:8>>, <<2:{b}/unit:8>>)"
        );
    }
    assert_eq!(
        reference(
            "begin Self = self(), [begin spawn_opt(fun() -> {C, S} = \
             bs_create_bin_append_probe:cost_again(binary:copy(<<1>>, A), binary:copy(<<2>>, B)), \
             Self ! {A, B, C, S} end, \
             [{min_heap_size,4000000},{min_bin_vheap_size,40000000}]), \
             receive R -> R end end || {A, B} <- [{0,0},{0,1},{1024,1},{100000,1},{100000,0},\
             {0,1024}]] end"
        ),
        "[{0,0,1,1},{0,1,1,2},{1024,1,1,1026},{100000,1,1,100002},{100000,0,1,100001},\
         {0,1024,2,1025}]\n"
    );
}

/// Render an integer, an atom, or a binary as `io:format("~w")` does. Each
/// atom name comes from `atoms`, the table that owns the term.
fn render(term: &Term, atoms: &AtomTable) -> String {
    match term {
        Term::Integer(value) => value.to_string(),
        Term::Atom(atom) => atoms.name_of(atom).to_owned(),
        Term::Binary(bytes) => {
            let bytes: Vec<String> = bytes.iter().map(u8::to_string).collect();
            format!("<<{}>>", bytes.join(","))
        }
        other => panic!("render has no form for {other:?}"),
    }
}

/// Run `bs_create_bin_integer_probe:Function` with its arguments.
fn run_integer(program: &BeamProgram, function: &str, arguments: &[Term]) -> Term {
    let arguments = arguments
        .iter()
        .cloned()
        .map(|term| program.host_term(term))
        .collect::<Vec<_>>();
    program
        .execute("bs_create_bin_integer_probe", function, &arguments)
        .unwrap()
        .term
}

/// Return the ERTS expression that calls `bs_create_bin_integer_probe:Function`
/// for each case, in a list.
fn integer_calls<'a>(
    atoms: &AtomTable,
    calls: impl Iterator<Item = (&'a str, &'a [Term])>,
) -> String {
    let calls: Vec<String> = calls
        .map(|(function, arguments)| {
            let arguments: Vec<String> = arguments
                .iter()
                .map(|argument| render(argument, atoms))
                .collect();
            format!(
                "bs_create_bin_integer_probe:{function}({})",
                arguments.join(", ")
            )
        })
        .collect();
    format!("[{}]", calls.join(", "))
}

#[test]
fn builds_integer_and_utf8_segments_as_pinned_erts() {
    // Each case runs one segment form that the compiler writes: sizes 0, 8,
    // 16, and 72, the big, little, and native byte orders, sizes in a
    // register of unit 1 and unit 8, segments that do not start on a byte
    // boundary, and utf8 code points of 1 to 4 bytes. A value is written
    // modulo 2^Size, and a size above 64 repeats the sign bit.
    let program = BeamProgram::load(&[BS_CREATE_BIN_INTEGER_PROBE]).unwrap();
    let int = Term::Integer;
    let cases: Vec<(&str, Vec<Term>, Vec<u8>)> = vec![
        ("byte", vec![int(1)], vec![1]),
        ("byte", vec![int(263)], vec![7]),
        ("byte", vec![int(-1)], vec![255]),
        ("big16", vec![int(0x1234)], vec![0x12, 0x34]),
        ("big16", vec![int(-2)], vec![255, 254]),
        ("little16", vec![int(0x1234)], vec![0x34, 0x12]),
        ("native16", vec![int(0x1234)], vec![0x34, 0x12]),
        ("wide", vec![int(-1)], vec![255; 9]),
        (
            "wide",
            vec![int(0x0102_0304_0506_0708)],
            vec![0, 1, 2, 3, 4, 5, 6, 7, 8],
        ),
        ("wide", vec![int(1 << 62)], vec![0, 64, 0, 0, 0, 0, 0, 0, 0]),
        (
            "wide_little",
            vec![int(0x0102_0304_0506_0708)],
            vec![8, 7, 6, 5, 4, 3, 2, 1, 0],
        ),
        (
            "wide_little",
            vec![int(-2)],
            vec![254, 255, 255, 255, 255, 255, 255, 255, 255],
        ),
        ("nibbles", vec![int(0xA), int(0xB)], vec![0xAB]),
        ("nibbles", vec![int(0x1F), int(-1)], vec![0xFF]),
        ("little12", vec![int(0xABC), int(0xD)], vec![0xBC, 0xAD]),
        ("odd", vec![int(5), int(0x1234)], vec![166, 146]),
        ("odd", vec![int(-1), int(-1)], vec![255, 255]),
        ("sized", vec![int(1), int(16)], vec![0, 1]),
        ("sized", vec![int(-1), int(0)], vec![]),
        ("bytes", vec![int(0x01_0203), int(3)], vec![1, 2, 3]),
        ("bytes", vec![int(-1), int(0)], vec![]),
        ("little_bytes", vec![int(0x01_0203), int(3)], vec![3, 2, 1]),
        (
            "little_bytes",
            vec![int(-3), int(9)],
            vec![253, 255, 255, 255, 255, 255, 255, 255, 255],
        ),
        ("zero", vec![int(5)], b"a".to_vec()),
        ("pair", vec![int(1), int(2)], vec![1, 2]),
        ("little_pair", vec![int(1), int(2)], vec![1, 2]),
        (
            "before_binary",
            vec![int(1), Term::binary(vec![2, 3])],
            vec![1, 2, 3],
        ),
        (
            "append",
            vec![Term::binary(vec![1, 2]), int(3)],
            vec![1, 2, 3],
        ),
        ("append", vec![Term::binary(Vec::new()), int(300)], vec![44]),
        (
            "append_sized",
            vec![Term::binary(vec![1, 2]), int(5), int(0)],
            vec![1, 2],
        ),
        ("code", vec![int(0)], vec![0]),
        ("code", vec![int(0x7F)], vec![0x7F]),
        ("code", vec![int(0x80)], vec![0xC2, 0x80]),
        ("code", vec![int(0x7FF)], vec![0xDF, 0xBF]),
        ("code", vec![int(0x800)], vec![0xE0, 0xA0, 0x80]),
        ("code", vec![int(0xD7FF)], vec![0xED, 0x9F, 0xBF]),
        ("code", vec![int(0xE000)], vec![0xEE, 0x80, 0x80]),
        ("code", vec![int(0xFFFF)], vec![0xEF, 0xBF, 0xBF]),
        ("code", vec![int(0x10000)], vec![0xF0, 0x90, 0x80, 0x80]),
        ("code", vec![int(0x10_FFFF)], vec![0xF4, 0x8F, 0xBF, 0xBF]),
        ("code_tail", vec![int(0xE9)], vec![0xC3, 0xA9, b'!']),
        (
            "code_append",
            vec![Term::binary(vec![1]), int(0x1_F600)],
            vec![1, 0xF0, 0x9F, 0x98, 0x80],
        ),
        (
            "sized_code",
            vec![int(0x41), int(8), int(0xE9)],
            vec![0x41, 0xC3, 0xA9],
        ),
        (
            "sized_code",
            vec![int(0x1234), int(16), int(0x41)],
            vec![0x12, 0x34, 0x41],
        ),
    ];
    for (function, arguments, bytes) in &cases {
        assert_eq!(
            run_integer(&program, function, arguments),
            Term::binary(bytes.clone()),
            "{function}{arguments:?}"
        );
    }
    let expected: Vec<String> = cases
        .iter()
        .map(|(_, _, bytes)| render(&Term::binary(bytes.clone()), program.atoms()))
        .collect();
    assert_eq!(
        reference(&integer_calls(
            program.atoms(),
            cases
                .iter()
                .map(|(function, arguments, _)| (*function, arguments.as_slice()))
        )),
        format!("[{}]\n", expected.join(","))
    );
}

#[test]
fn jumps_to_the_fail_label_for_integer_and_utf8_segments_that_fail() {
    // The guards of `guard/1` and `code_guard/1` compile to `bs_create_bin`
    // with the fail label of the next clause.
    let program = BeamProgram::load(&[BS_CREATE_BIN_INTEGER_PROBE]).unwrap();
    let atom = |name: &str| Term::Atom(program.atom(name).unwrap());
    let cases = [
        ("guard", Term::Integer(1)),
        ("guard", Term::Integer(2)),
        ("guard", atom("other")),
        ("code_guard", Term::Integer(0xE9)),
        ("code_guard", Term::Integer(-1)),
        ("code_guard", atom("other")),
    ];
    let results: Vec<Term> = cases
        .iter()
        .map(|(function, argument)| run_integer(&program, function, std::slice::from_ref(argument)))
        .collect();
    assert_eq!(
        results,
        [
            atom("one"),
            atom("other"),
            atom("other"),
            atom("e"),
            atom("other"),
            atom("other")
        ]
    );
    assert_eq!(
        reference(&integer_calls(
            program.atoms(),
            cases
                .iter()
                .map(|(function, argument)| (*function, std::slice::from_ref(argument)))
        )),
        "[one,other,other,e,other,other]\n"
    );
}

/// A call of `bs_create_bin_integer_probe` that fails: the function, the
/// arguments, the exception reason, and the segment, operation, info, and
/// value of the `error_info` cause.
type FailureCase<'a> = (&'a str, Vec<Term>, &'a str, i64, &'a str, &'a str, Term);

#[test]
fn raises_the_erts_error_info_for_integer_and_utf8_segments() {
    // The size pass checks the sizes of `integer` segments, the code
    // points of `utf8` segments, and the terms of `binary` segments in
    // segment order. Then the `append` segment is checked. Then the
    // construct pass checks the term of each `integer` segment, and checks
    // a group of little-endian segments in reverse order. A size that is a
    // positive bignum, or a size of 2^52 or more with a unit other than 1,
    // raises `system_limit`.
    let program = BeamProgram::load(&[BS_CREATE_BIN_INTEGER_PROBE]).unwrap();
    let atom = |name: &str| Term::Atom(program.atom(name).unwrap());
    let int = Term::Integer;
    let (one, other) = (atom("one"), atom("other"));
    let cases: Vec<FailureCase> = vec![
        (
            "byte",
            vec![other.clone()],
            "badarg",
            1,
            "integer",
            "type",
            other.clone(),
        ),
        (
            "byte",
            vec![Term::binary(vec![1])],
            "badarg",
            1,
            "integer",
            "type",
            Term::binary(vec![1]),
        ),
        (
            "sized",
            vec![int(1), int(-1)],
            "badarg",
            1,
            "integer",
            "size",
            int(-1),
        ),
        (
            "sized",
            vec![int(1), other.clone()],
            "badarg",
            1,
            "integer",
            "size",
            other.clone(),
        ),
        (
            "sized",
            vec![other.clone(), int(-1)],
            "badarg",
            1,
            "integer",
            "size",
            int(-1),
        ),
        (
            "sized",
            vec![int(1), int(1 << 59)],
            "system_limit",
            1,
            "integer",
            "size",
            int(1 << 59),
        ),
        (
            "sized",
            vec![int(1), int(-(1 << 59) - 1)],
            "badarg",
            1,
            "integer",
            "size",
            int(-(1 << 59) - 1),
        ),
        (
            "sized",
            vec![other.clone(), int(0)],
            "badarg",
            1,
            "integer",
            "type",
            other.clone(),
        ),
        (
            "bytes",
            vec![int(1), int(1 << 52)],
            "system_limit",
            1,
            "integer",
            "size",
            int(1 << 52),
        ),
        (
            "bytes",
            vec![int(1), int(-1)],
            "badarg",
            1,
            "integer",
            "size",
            int(-1),
        ),
        (
            "little_bytes",
            vec![other.clone(), int(1 << 52)],
            "system_limit",
            1,
            "integer",
            "size",
            int(1 << 52),
        ),
        (
            "zero",
            vec![other.clone()],
            "badarg",
            1,
            "integer",
            "type",
            other.clone(),
        ),
        (
            "pair",
            vec![other.clone(), one.clone()],
            "badarg",
            1,
            "integer",
            "type",
            other.clone(),
        ),
        (
            "pair",
            vec![int(1), one.clone()],
            "badarg",
            2,
            "integer",
            "type",
            one.clone(),
        ),
        (
            "little_pair",
            vec![other.clone(), one.clone()],
            "badarg",
            2,
            "integer",
            "type",
            one.clone(),
        ),
        (
            "little_pair",
            vec![other.clone(), int(2)],
            "badarg",
            1,
            "integer",
            "type",
            other.clone(),
        ),
        (
            "little12",
            vec![other.clone(), one.clone()],
            "badarg",
            1,
            "integer",
            "type",
            other.clone(),
        ),
        (
            "odd",
            vec![other.clone(), one.clone()],
            "badarg",
            1,
            "integer",
            "type",
            other.clone(),
        ),
        (
            "before_binary",
            vec![other.clone(), one.clone()],
            "badarg",
            2,
            "binary",
            "type",
            one.clone(),
        ),
        (
            "before_binary",
            vec![other.clone(), Term::binary(Vec::new())],
            "badarg",
            1,
            "integer",
            "type",
            other.clone(),
        ),
        (
            "append",
            vec![other.clone(), one.clone()],
            "badarg",
            1,
            "binary",
            "type",
            other.clone(),
        ),
        (
            "append",
            vec![Term::binary(Vec::new()), one.clone()],
            "badarg",
            2,
            "integer",
            "type",
            one.clone(),
        ),
        (
            "append_sized",
            vec![Term::binary(vec![1]), other.clone(), int(0)],
            "badarg",
            2,
            "integer",
            "type",
            other.clone(),
        ),
        ("code", vec![int(-1)], "badarg", 1, "utf8", "type", int(-1)),
        (
            "code",
            vec![int(0xD800)],
            "badarg",
            1,
            "utf8",
            "type",
            int(0xD800),
        ),
        (
            "code",
            vec![int(0xDFFF)],
            "badarg",
            1,
            "utf8",
            "type",
            int(0xDFFF),
        ),
        (
            "code",
            vec![int(0x11_0000)],
            "badarg",
            1,
            "utf8",
            "type",
            int(0x11_0000),
        ),
        (
            "code",
            vec![other.clone()],
            "badarg",
            1,
            "utf8",
            "type",
            other.clone(),
        ),
        (
            "code_append",
            vec![other.clone(), int(-1)],
            "badarg",
            2,
            "utf8",
            "type",
            int(-1),
        ),
        (
            "sized_code",
            vec![other.clone(), one.clone(), int(-1)],
            "badarg",
            1,
            "integer",
            "size",
            one.clone(),
        ),
        (
            "sized_code",
            vec![other.clone(), int(8), int(-1)],
            "badarg",
            2,
            "utf8",
            "type",
            int(-1),
        ),
        (
            "sized_code",
            vec![other.clone(), int(8), int(0x41)],
            "badarg",
            1,
            "integer",
            "type",
            other.clone(),
        ),
    ];
    let caught = |function: &str, arity: usize, reason: &str, cause: Term| {
        let error_info = Map::from_pairs(
            vec![
                (atom("cause"), cause),
                (atom("function"), atom("format_bs_fail")),
                (atom("module"), atom("erl_erts_errors")),
            ],
            program.atoms(),
        )
        .unwrap();
        let frame = Term::tuple(vec![
            atom("bs_create_bin_integer_probe"),
            atom(function),
            Term::Integer(arity as i64),
            Term::list_from(vec![Term::tuple(vec![
                atom("error_info"),
                Term::Map(error_info),
            ])]),
        ]);
        Term::tuple(vec![atom("error"), atom(reason), frame])
    };
    for (function, arguments, reason, segment, operation, info, value) in &cases {
        let cause = Term::tuple(vec![
            int(*segment),
            atom(operation),
            atom(info),
            value.clone(),
        ]);
        assert_eq!(
            run_integer(
                &program,
                "catch_call",
                &[atom(function), Term::list_from(arguments.clone())]
            ),
            caught(function, arguments.len(), reason, cause),
            "{function}{arguments:?}"
        );
    }
    // The arm64 JIT of the macOS arm64 target fails a size of 2^52 or more
    // with a unit other than 1 (jit/arm/instr_bs.cpp:1685). The x86_64 JIT
    // fails only when the size times the unit overflows 64 bits
    // (jit/x86/instr_bs.cpp:1942-1943), so ERTS on an x86_64 host stops at
    // the allocation. Compare these cases with ERTS only on arm64.
    let reference_cases: Vec<&FailureCase> = cases
        .iter()
        .filter(|(.., value)| cfg!(target_arch = "aarch64") || *value != int(1 << 52))
        .collect();
    let calls: Vec<String> = reference_cases
        .iter()
        .map(|(function, arguments, ..)| {
            let arguments: Vec<String> = arguments
                .iter()
                .map(|argument| render(argument, program.atoms()))
                .collect();
            format!(
                "bs_create_bin_integer_probe:catch_call({function}, [{}])",
                arguments.join(",")
            )
        })
        .collect();
    let expected: Vec<String> = reference_cases
        .iter()
        .map(
            |(function, arguments, reason, segment, operation, info, value)| {
                format!(
                    "{{error,{reason},{{bs_create_bin_integer_probe,{function},{},[{{error_info,\
                 #{{cause => {{{segment},{operation},{info},{}}},function => format_bs_fail,\
                 module => erl_erts_errors}}}}]}}}}",
                    arguments.len(),
                    render(value, program.atoms())
                )
            },
        )
        .collect();
    assert_eq!(
        reference(&format!("[{}]", calls.join(", "))),
        format!("[{}]\n", expected.join(","))
    );
}

#[test]
fn uses_no_reduction_for_an_integer_segment() {
    // `cost/2` gives the reductions between two `process_info/2` calls
    // around `<<X:N>>`: 1 for the second call. `erts_bs_put_integer_be`
    // uses no reductions (erl_bits.c:926-1028). The ERTS run gives the
    // process a large heap and binary heap, so that no garbage collection
    // occurs.
    let program = BeamProgram::load(&[BS_CREATE_BIN_INTEGER_PROBE]).unwrap();
    let cases = [(1, 8), (1, 800_000), (-1, 8_000_000)];
    for (value, bits) in cases {
        assert_eq!(
            run_integer(
                &program,
                "cost",
                &[Term::Integer(value), Term::Integer(bits)]
            ),
            Term::tuple(vec![Term::Integer(1), Term::Integer(bits / 8)])
        );
    }
    assert_eq!(
        reference(
            "begin Self = self(), [begin spawn_opt(fun() -> \
             Self ! bs_create_bin_integer_probe:cost(X, N) end, \
             [{min_heap_size,4000000},{min_bin_vheap_size,40000000}]), \
             receive R -> R end end || {X, N} <- [{1, 8}, {1, 800000}, {-1, 8000000}]] end"
        ),
        "[{1,1},{1,100000},{1,1000000}]\n"
    );
}

#[test]
fn stops_at_a_bitstring_that_is_not_a_binary() {
    // `nibble/1` is `<<X:4>>`. ERTS gives the bitstring `<<1:4>>`. PBeam
    // has no bitstring term and stops (row "Binary construction").
    let program = BeamProgram::load(&[BS_CREATE_BIN_INTEGER_PROBE]).unwrap();
    assert_eq!(
        program
            .execute(
                "bs_create_bin_integer_probe",
                "nibble",
                &[program.host_term(Term::Integer(1))],
            )
            .unwrap_err()
            .to_string(),
        "unsupported bs_create_bin result: a bitstring of 4 bits, which is not a binary"
    );
    assert_eq!(
        reference("bs_create_bin_integer_probe:nibble(1)"),
        "<<1:4>>\n"
    );
}

/// Run `bs_create_bin_writable_probe:Function` with its arguments, and give
/// the runtime error as text.
fn run_writable(program: &BeamProgram, function: &str, arguments: &[Term]) -> Result<Term, String> {
    let arguments = arguments
        .iter()
        .cloned()
        .map(|term| program.host_term(term))
        .collect::<Vec<_>>();
    program
        .execute("bs_create_bin_writable_probe", function, &arguments)
        .map(|host| host.term)
        .map_err(|error| error.to_string())
}

#[test]
fn appends_with_private_append_segments_as_pinned_erts() {
    // Each loop starts with `bs_init_writable` and appends with a
    // `private_append` segment and an `integer`, `binary`, or `float`
    // segment, as erlc compiles `<<Acc/binary, X:8>>` in a loop.
    // `sizes/1` reads the size of each step, and `shared/1` keeps each step
    // in a list, which makes the compiler use an `append` segment.
    let program = BeamProgram::load(&[BS_CREATE_BIN_WRITABLE_PROBE]).unwrap();
    let binary = |bytes: &[u8]| Term::binary(bytes.to_vec());
    let int = Term::Integer;
    let list = |items: Vec<Term>| Term::list_from(items);
    let big = Term::BigInt(std::sync::Arc::new(num_bigint::BigInt::from(1u128 << 80)));
    let float_bytes = [
        &[63, 248, 0, 0, 0, 0, 0, 0][..],
        &[64, 0, 0, 0, 0, 0, 0, 0],
        &[128, 0, 0, 0, 0, 0, 0, 0],
        &[68, 240, 0, 0, 0, 0, 0, 0],
    ]
    .concat();
    let cases: Vec<(&str, Vec<Term>, Term)> = vec![
        ("bytes", vec![int(5)], binary(&[5, 4, 3, 2, 1])),
        ("bytes", vec![int(0)], binary(&[])),
        (
            "chunks",
            vec![list(vec![binary(&[1, 2]), binary(&[]), binary(&[3])])],
            binary(&[1, 2, 3]),
        ),
        (
            "floats",
            vec![list(vec![
                Term::Float(1.5),
                int(2),
                Term::Float(-0.0),
                big.clone(),
            ])],
            binary(&float_bytes),
        ),
        (
            "sizes",
            vec![int(3)],
            Term::tuple(vec![binary(&[3, 2, 1]), list(vec![int(3), int(2), int(1)])]),
        ),
        (
            "shared",
            vec![int(3)],
            Term::tuple(vec![
                binary(&[3, 2, 1]),
                list(vec![binary(&[3, 2]), binary(&[3]), binary(&[])]),
            ]),
        ),
    ];
    let results: Vec<_> = cases
        .iter()
        .map(|(function, arguments, _)| run_writable(&program, function, arguments))
        .collect();
    let expected: Vec<_> = cases.iter().map(|(_, _, term)| Ok(term.clone())).collect();
    assert_eq!(results, expected);
    assert_eq!(
        run_writable(&program, "bytes", &[int(100_000)]).map(|term| match &term {
            Term::Binary(bytes) => bytes.len(),
            other => panic!("not a binary: {other:?}"),
        }),
        Ok(100_000)
    );
    assert_eq!(
        reference(
            "begin M = bs_create_bin_writable_probe, \
             [M:bytes(5), M:bytes(0), M:chunks([<<1,2>>, <<>>, <<3>>]), \
             M:floats([1.5, 2, -0.0, 1 bsl 80]), M:sizes(3), M:shared(3), \
             byte_size(M:bytes(100000))] end"
        ),
        "[<<5,4,3,2,1>>,<<>>,<<1,2,3>>,<<63,248,0,0,0,0,0,0,64,0,0,0,0,0,0,0,128,0,0,0,0,0,0,0,\
         68,240,0,0,0,0,0,0>>,{<<3,2,1>>,[3,2,1]},{<<3,2,1>>,[<<3,2>>,<<3>>,<<>>]},100000]\n"
    );
}

#[test]
fn uses_no_reduction_for_a_private_append_as_the_pinned_erts_on_arm64() {
    // `reds/1` gives the reductions between two `process_info/2` calls
    // around a loop that appends 1024 bytes N times with a
    // `private_append` segment: 1 for each of the N + 1 calls of the loop,
    // 1 for the second `process_info/2` call, and 1 for the call of the
    // loop from `reds/1`. `erts_bs_private_append_checked` calls
    // `BUMP_REDS` when it grows the buffer (erl_bits.c:1974, 2010), but
    // the arm JIT enters the runtime without `Update::eReductions`
    // (jit/arm/instr_bs.cpp:1876-1880), so the bump is lost. The x86 JIT
    // keeps the bump (jit/x86/instr_bs.cpp:1578-1584, 2116-2131), so the
    // reference run is only on an arm64 host (row "Binary construction"
    // of `docs/erts-differences.md`).
    // (N, size of the result, reductions)
    let results = [
        (1, 1024, 4),
        (10, 10_240, 13),
        (100, 102_400, 103),
        (300, 307_200, 303),
    ];
    let program = BeamProgram::load(&[BS_CREATE_BIN_WRITABLE_PROBE]).unwrap();
    for (n, size, reductions) in results {
        assert_eq!(
            run_writable(&program, "reds", &[Term::Integer(n)]),
            Ok(Term::tuple(vec![
                Term::Integer(size),
                Term::Integer(reductions)
            ])),
            "reds({n})"
        );
    }
    #[cfg(target_arch = "aarch64")]
    assert_eq!(
        reference("[bs_create_bin_writable_probe:reds(N) || N <- [1, 10, 100, 300]]"),
        "[{1024,4},{10240,13},{102400,103},{307200,303}]\n"
    );
}

#[test]
fn builds_float_segments_and_raises_their_failures_as_pinned_erts() {
    // A float segment of 32 or 64 bits converts a small integer with one C
    // cast and a bignum through a double, and a value too large for 32
    // bits gives an infinity (erl_bits.c:1340-1452). The size checks are
    // the size checks of an `integer` segment, and the construct pass fails
    // with `invalid`, `type`, or `no_float`.
    let program = BeamProgram::load(&[BS_CREATE_BIN_WRITABLE_PROBE]).unwrap();
    let atom = |name: &str| Term::Atom(program.atom(name).unwrap());
    let binary = |bytes: &[u8]| Term::binary(bytes.to_vec());
    let int = Term::Integer;
    let huge = Term::BigInt(std::sync::Arc::new(
        num_bigint::BigInt::from(1u8) << 1100u32,
    ));
    let limit = Term::BigInt(std::sync::Arc::new(num_bigint::BigInt::from(1u128 << 70)));
    let cause = |reason: &str, segment: i64, info: &str, value: Term| {
        Term::tuple(vec![
            atom(reason),
            Term::tuple(vec![int(segment), atom("float"), atom(info), value]),
        ])
    };
    let caught = |function: &str, arguments: Vec<Term>| {
        run_writable(
            &program,
            "cause",
            &[atom(function), Term::list_from(arguments)],
        )
    };
    let results = [
        run_writable(&program, "float_size", &[Term::Float(1.5), int(32)]),
        run_writable(&program, "float_size", &[int(-3), int(32)]),
        run_writable(&program, "float_size", &[Term::Float(1.0e300), int(32)]),
        run_writable(&program, "float_size", &[Term::Float(3.0e38), int(32)]),
        run_writable(
            &program,
            "float_size",
            &[
                Term::BigInt(std::sync::Arc::new(num_bigint::BigInt::from(1u128 << 80))),
                int(32),
            ],
        ),
        run_writable(&program, "float_size", &[int(1 << 62), int(64)]),
        run_writable(
            &program,
            "append_float",
            &[binary(&[1]), Term::Float(1.5), int(64)],
        ),
        caught("float_size", vec![Term::Float(1.5), int(8)]),
        caught("float_size", vec![Term::Float(1.5), int(-1)]),
        caught("float_size", vec![Term::Float(1.5), Term::nil()]),
        caught("float_size", vec![huge.clone(), int(64)]),
        caught("float_bad", vec![Term::nil()]),
        caught("float_size", vec![Term::Float(1.5), limit.clone()]),
        caught("append_float", vec![binary(&[1]), Term::Float(1.5), int(0)]),
        caught("append_float", vec![binary(&[1]), Term::nil(), int(64)]),
    ];
    // The failures make the atoms `invalid` and `no_float`, so the expected
    // terms read them after the runs.
    assert_eq!(
        results,
        [
            Ok(binary(&[63, 192, 0, 0])),
            Ok(binary(&[192, 64, 0, 0])),
            Ok(binary(&[127, 128, 0, 0])),
            Ok(binary(&[127, 97, 177, 230])),
            Ok(binary(&[103, 128, 0, 0])),
            Ok(binary(&[67, 208, 0, 0, 0, 0, 0, 0])),
            Ok(binary(&[1, 63, 248, 0, 0, 0, 0, 0, 0])),
            Ok(cause("badarg", 1, "invalid", int(8))),
            Ok(cause("badarg", 1, "size", int(-1))),
            Ok(cause("badarg", 1, "size", Term::nil())),
            Ok(cause("badarg", 1, "no_float", huge)),
            Ok(cause("badarg", 1, "type", Term::nil())),
            Ok(cause("system_limit", 1, "size", limit)),
            Ok(cause("badarg", 2, "invalid", int(0))),
            Ok(cause("badarg", 2, "type", Term::nil())),
        ]
    );
    assert_eq!(
        reference(
            "begin M = bs_create_bin_writable_probe, \
             [M:float_size(1.5, 32), M:float_size(-3, 32), M:float_size(1.0e300, 32), \
             M:float_size(3.0e38, 32), M:float_size(1 bsl 80, 32), M:float_size(1 bsl 62, 64), \
             M:append_float(<<1>>, 1.5, 64), M:cause(float_size, [1.5, 8]), \
             M:cause(float_size, [1.5, -1]), M:cause(float_size, [1.5, []]), \
             M:cause(float_size, [1 bsl 1100, 64]) =:= {badarg, {1, float, no_float, 1 bsl 1100}}, \
             M:cause(float_bad, [[]]), M:cause(float_size, [1.5, 1 bsl 70]), \
             M:cause(append_float, [<<1>>, 1.5, 0]), M:cause(append_float, [<<1>>, [], 64])] end"
        ),
        "[<<63,192,0,0>>,<<192,64,0,0>>,<<127,128,0,0>>,<<127,97,177,230>>,<<103,128,0,0>>,\
         <<67,208,0,0,0,0,0,0>>,<<1,63,248,0,0,0,0,0,0>>,{badarg,{1,float,invalid,8}},\
         {badarg,{1,float,size,-1}},{badarg,{1,float,size,[]}},true,{badarg,{1,float,type,[]}},\
         {system_limit,{1,float,size,1180591620717411303424}},{badarg,{2,float,invalid,0}},\
         {badarg,{2,float,type,[]}}]\n"
    );
}

#[test]
fn stops_at_a_16_bit_float_and_at_a_private_append_of_a_bitstring() {
    // PBeam does not write 16-bit floats, and has no bitstring that is not
    // a binary, so these runs stop with a runtime error where ERTS gives a
    // binary.
    let program = BeamProgram::load(&[BS_CREATE_BIN_WRITABLE_PROBE]).unwrap();
    assert_eq!(
        [
            run_writable(
                &program,
                "float_size",
                &[Term::Integer(1), Term::Integer(16)]
            ),
            run_writable(&program, "bits", &[Term::Integer(2)]),
        ],
        [
            Err("unsupported bs_create_bin float segment of 16 bits".into()),
            Err(
                "unsupported bs_create_bin result: a bitstring of 4 bits, which is not a binary"
                    .into()
            ),
        ]
    );
    assert_eq!(
        reference(
            "[bs_create_bin_writable_probe:float_size(1, 16), bs_create_bin_writable_probe:bits(2)]"
        ),
        "[<<60,0>>,<<33>>]\n"
    );
}

#[test]
fn raises_the_failure_of_a_segment_after_a_private_append_as_pinned_erts() {
    // `ints/1` appends each integer of a list with a `private_append`
    // segment and an `integer` segment. The `integer` segment fails in the
    // construct pass, before the append writes in the binary.
    let program = BeamProgram::load(&[BS_CREATE_BIN_WRITABLE_PROBE]).unwrap();
    let ints = |items: Vec<Term>| {
        run_writable(
            &program,
            "cause",
            &[
                Term::Atom(program.atom("ints").unwrap()),
                Term::list_from(vec![Term::list_from(items)]),
            ],
        )
    };
    let result = ints(vec![Term::Integer(1), Term::nil()]);
    let atom = |name: &str| Term::Atom(program.atom(name).unwrap());
    assert_eq!(
        [
            result,
            run_writable(
                &program,
                "ints",
                &[Term::list_from(vec![Term::Integer(1), Term::Integer(2)])]
            )
        ],
        [
            Ok(Term::tuple(vec![
                atom("badarg"),
                Term::tuple(vec![
                    Term::Integer(2),
                    atom("integer"),
                    atom("type"),
                    Term::nil()
                ]),
            ])),
            Ok(Term::binary(vec![1, 2])),
        ]
    );
    assert_eq!(
        reference(
            "[bs_create_bin_writable_probe:cause(ints, [[1, []]]), \
             bs_create_bin_writable_probe:ints([1, 2])]"
        ),
        "[{badarg,{2,integer,type,[]}},<<1,2>>]\n"
    );
}
const BS_CREATE_BIN_UTF_PROBE: &[u8] =
    include_bytes!("../../fixtures/bs_create_bin/bs_create_bin_utf_probe.beam");

/// Run `bs_create_bin_utf_probe:Function/N` with each call of arguments and
/// return the results.
fn run_utf(program: &BeamProgram, function: &str, calls: &[Vec<Term>]) -> Vec<Term> {
    calls
        .iter()
        .map(|arguments| {
            let imported: Vec<_> = arguments
                .iter()
                .cloned()
                .map(|term| program.host_term(term))
                .collect();
            program
                .execute("bs_create_bin_utf_probe", function, &imported)
                .unwrap()
                .term
        })
        .collect()
}

#[test]
fn builds_utf16_and_utf32_segments_as_pinned_erts() {
    // A `utf16` segment writes two bytes for a code point below 0x10000 and
    // four bytes for each other one, in the byte order of the flags
    // (`erts_bs_put_utf16`, erl_bits.c:1221-1282; `am_utf16` in
    // jit/arm/instr_bs.cpp:1751-1765, 2407-2427). A `utf32` segment writes
    // the code point as a 32-bit integer, as an `integer` segment of 32
    // bits does (jit/arm/instr_bs.cpp:2428-2452).
    let program = BeamProgram::load(&[BS_CREATE_BIN_UTF_PROBE]).unwrap();
    let binary = |bytes: &[u8]| Term::binary(bytes.to_vec());
    let face = Term::Integer(0x1F600);
    let calls: Vec<Vec<Term>> = vec![
        vec![Term::Integer(65)],
        vec![face.clone()],
        vec![Term::Integer(0)],
    ];
    assert_eq!(
        run_utf(&program, "utf16", &calls),
        [
            binary(&[0, 65]),
            binary(&[216, 61, 222, 0]),
            binary(&[0, 0]),
        ]
    );
    assert_eq!(
        run_utf(
            &program,
            "utf16_little",
            &[vec![Term::Integer(65)], vec![face.clone()]]
        ),
        [binary(&[65, 0]), binary(&[61, 216, 0, 222])]
    );
    // The `native` flag gives the byte order of the host, as the ERTS
    // loader reads it (`$NativeEndian`, generators.tab).
    assert_eq!(
        run_utf(
            &program,
            "utf16_native",
            &[vec![Term::Integer(65)], vec![face.clone()]]
        ),
        [
            binary(if cfg!(target_endian = "little") {
                &[65, 0]
            } else {
                &[0, 65]
            }),
            binary(&[61, 216, 0, 222]),
        ]
    );
    assert_eq!(
        run_utf(
            &program,
            "utf32",
            &[vec![face.clone()], vec![Term::Integer(65)]]
        ),
        [binary(&[0, 1, 246, 0]), binary(&[0, 0, 0, 65])]
    );
    assert_eq!(
        run_utf(&program, "utf32_little", &[vec![face.clone()]]),
        [binary(&[0, 246, 1, 0])]
    );
    assert_eq!(
        run_utf(&program, "utf32_native", &[vec![face.clone()]]),
        [binary(if cfg!(target_endian = "little") {
            &[0, 246, 1, 0]
        } else {
            &[0, 1, 246, 0]
        })]
    );
    // A utf16 or utf32 segment after other segments, also at a bit offset
    // that is not a multiple of 8.
    assert_eq!(
        run_utf(
            &program,
            "utf16_between",
            &[vec![Term::Integer(65)], vec![face.clone()]]
        ),
        [binary(b"a\x00Ab"), binary(b"a\xD8=\xDE\x00b")]
    );
    assert_eq!(
        run_utf(&program, "utf32_between", &[vec![face.clone()]]),
        [binary(b"a\x00\x01\xF6\x00b")]
    );
    assert_eq!(
        run_utf(
            &program,
            "utf16_bits",
            &[vec![Term::Integer(65)], vec![face.clone()]]
        ),
        [binary(&[0, 4, 16]), binary(&[13, 131, 221, 224, 0])]
    );
    assert_eq!(
        run_utf(&program, "utf32_bits", &[vec![face.clone()]]),
        [binary(&[0, 0, 0x1F, 0x60, 0])]
    );
    assert_eq!(
        run_utf(
            &program,
            "mixed",
            &[
                vec![face.clone(), Term::Integer(8)],
                vec![Term::Integer(65), Term::Integer(8)],
            ]
        ),
        [binary(&[1, 216, 61, 222, 0]), binary(&[1, 0, 65])]
    );
    assert_eq!(
        run_utf(
            &program,
            "append_utf16",
            &[
                vec![Term::binary(vec![97]), Term::Integer(65)],
                vec![Term::binary(Vec::new()), face.clone()],
            ]
        ),
        [binary(&[97, 0, 65]), binary(&[216, 61, 222, 0])]
    );
    assert_eq!(
        reference(
            "begin P = bs_create_bin_utf_probe, F = 16#1F600, \
             [P:utf16(65), P:utf16(F), P:utf16(0), P:utf16_little(65), \
             P:utf16_little(F), P:utf16_native(65), P:utf32(F), P:utf32(65), \
             P:utf32_little(F), P:utf32_native(F), P:utf16_between(65), \
             P:utf32_between(F), P:utf16_bits(65), P:utf16_bits(F), \
             P:utf32_bits(F), P:mixed(F, 8), P:mixed(65, 8), \
             P:append_utf16(<<97>>, 65), P:append_utf16(<<>>, F)] end"
        ),
        "[<<0,65>>,<<216,61,222,0>>,<<0,0>>,<<65,0>>,<<61,216,0,222>>,\
         <<65,0>>,<<0,1,246,0>>,<<0,0,0,65>>,<<0,246,1,0>>,<<0,246,1,0>>,\
         <<97,0,65,98>>,<<97,0,1,246,0,98>>,<<0,4,16>>,<<13,131,221,224,0>>,\
         <<0,0,31,96,0>>,<<1,216,61,222,0>>,<<1,0,65>>,<<97,0,65>>,\
         <<216,61,222,0>>]\n"
    );
}

/// A failing call of `bs_create_bin_utf_probe`: the function, the
/// arguments, the segment, the operation, the info, and the value of the
/// `error_info` cause.
type UtfFailureCase<'a> = (&'a str, Vec<Term>, i64, &'a str, &'a str, Term);

#[test]
fn raises_the_erts_error_info_for_utf16_and_utf32_segments() {
    // A `utf16` segment fails at the construct pass for a term that is not
    // a code point from 0 to 16#10FFFF outside 16#D800 to 16#DFFF, with
    // the cause `{Segment, utf16, type, Value}` (`erts_bs_put_utf16`,
    // erl_bits.c:1231-1237; jit/arm/instr_bs.cpp:2418-2425). A `utf32`
    // segment fails at the size pass, with the cause
    // `{Segment, utf32, type, Value}` (jit/arm/instr_bs.cpp:1766-1799).
    // The size pass runs over each segment before the construct pass, so
    // `ordered(other, -1)` fails on the size of the `integer` segment 2 and
    // `ordered(other, 8)` fails on the term of the `utf16` segment 1.
    let program = BeamProgram::load(&[BS_CREATE_BIN_UTF_PROBE]).unwrap();
    let atom = |name: &str| Term::Atom(program.atom(name).unwrap());
    let int = Term::Integer;
    let other = atom("other");
    let bignum = Term::BigInt(Arc::new(BigInt::from(1u8) << 64u32));
    let cases: Vec<UtfFailureCase> = vec![
        ("utf16", vec![int(-1)], 1, "utf16", "type", int(-1)),
        ("utf16", vec![int(0xD800)], 1, "utf16", "type", int(0xD800)),
        ("utf16", vec![int(0xDFFF)], 1, "utf16", "type", int(0xDFFF)),
        (
            "utf16",
            vec![int(0x11_0000)],
            1,
            "utf16",
            "type",
            int(0x11_0000),
        ),
        (
            "utf16",
            vec![other.clone()],
            1,
            "utf16",
            "type",
            other.clone(),
        ),
        (
            "utf16",
            vec![bignum.clone()],
            1,
            "utf16",
            "type",
            bignum.clone(),
        ),
        ("utf16_little", vec![int(-1)], 1, "utf16", "type", int(-1)),
        ("utf32", vec![int(-1)], 1, "utf32", "type", int(-1)),
        ("utf32", vec![int(0xD800)], 1, "utf32", "type", int(0xD800)),
        ("utf32", vec![int(0xDFFF)], 1, "utf32", "type", int(0xDFFF)),
        (
            "utf32",
            vec![int(0x11_0000)],
            1,
            "utf32",
            "type",
            int(0x11_0000),
        ),
        (
            "utf32",
            vec![other.clone()],
            1,
            "utf32",
            "type",
            other.clone(),
        ),
        (
            "utf32",
            vec![bignum.clone()],
            1,
            "utf32",
            "type",
            bignum.clone(),
        ),
        (
            "utf32_little",
            vec![int(0x11_0000)],
            1,
            "utf32",
            "type",
            int(0x11_0000),
        ),
        (
            "ordered",
            vec![other.clone(), int(8)],
            1,
            "utf16",
            "type",
            other.clone(),
        ),
        (
            "ordered",
            vec![other.clone(), int(-1)],
            2,
            "integer",
            "size",
            int(-1),
        ),
        (
            "ordered",
            vec![other.clone(), other.clone()],
            2,
            "integer",
            "size",
            other.clone(),
        ),
        (
            "ordered",
            vec![int(0x41), int(-1)],
            2,
            "integer",
            "size",
            int(-1),
        ),
        // A size of 0x110000 bits passes the size pass, so the construct
        // pass fails for the `utf16` term first.
        (
            "ordered",
            vec![other.clone(), int(0x11_0000)],
            1,
            "utf16",
            "type",
            other.clone(),
        ),
    ];
    let caught = |function: &str, arity: usize, cause: Term| {
        let error_info = Map::from_pairs(
            vec![
                (atom("cause"), cause),
                (atom("function"), atom("format_bs_fail")),
                (atom("module"), atom("erl_erts_errors")),
            ],
            program.atoms(),
        )
        .unwrap();
        let frame = Term::tuple(vec![
            atom("bs_create_bin_utf_probe"),
            atom(function),
            Term::Integer(arity as i64),
            Term::list_from(vec![Term::tuple(vec![
                atom("error_info"),
                Term::Map(error_info),
            ])]),
        ]);
        Term::tuple(vec![atom("error"), atom("badarg"), frame])
    };
    for (function, arguments, segment, operation, info, value) in &cases {
        let cause = Term::tuple(vec![
            int(*segment),
            atom(operation),
            atom(info),
            value.clone(),
        ]);
        assert_eq!(
            run_utf(
                &program,
                "call",
                &[vec![atom(function), Term::list_from(arguments.clone())]]
            ),
            [caught(function, arguments.len(), cause)],
            "{function} {arguments:?}"
        );
    }
    // The same causes from a pinned ERTS run of the same fixture function,
    // in the order of `cases`.
    assert_eq!(
        reference(
            "[begin {error, badarg, {_, _, _, [{error_info, #{cause := Cause}}]}} = \
             bs_create_bin_utf_probe:call(F, A), Cause end || {F, A} <- [\
             {utf16,[-1]},{utf16,[55296]},{utf16,[57343]},{utf16,[1114112]},\
             {utf16,[other]},{utf16,[1 bsl 64]},{utf16_little,[-1]},\
             {utf32,[-1]},{utf32,[55296]},{utf32,[57343]},{utf32,[1114112]},\
             {utf32,[other]},{utf32,[1 bsl 64]},{utf32_little,[1114112]},\
             {ordered,[other,8]},{ordered,[other,-1]},{ordered,[other,other]},\
             {ordered,[65,-1]},{ordered,[other,1114112]}]]"
        ),
        "[{1,utf16,type,-1},{1,utf16,type,55296},{1,utf16,type,57343},\
         {1,utf16,type,1114112},{1,utf16,type,other},\
         {1,utf16,type,18446744073709551616},{1,utf16,type,-1},{1,utf32,type,-1},\
         {1,utf32,type,55296},{1,utf32,type,57343},{1,utf32,type,1114112},\
         {1,utf32,type,other},{1,utf32,type,18446744073709551616},\
         {1,utf32,type,1114112},{1,utf16,type,other},{2,integer,size,-1},\
         {2,integer,size,other},{2,integer,size,-1},{1,utf16,type,other}]\n"
    );
}

#[test]
fn stops_at_a_binary_segment_of_a_size_in_a_register() {
    // `binary_sized/2` is `<<X:Size/binary>>`: a `binary` segment of a size
    // that is not `all`. The loader keeps it as an unsupported segment, so
    // the run stops (row "Binary construction"). ERTS gives the prefix of
    // the size.
    let program = BeamProgram::load(&[BS_CREATE_BIN_PROBE]).unwrap();
    assert_eq!(
        program
            .execute(
                "bs_create_bin_probe",
                "binary_sized",
                &[
                    program.host_term(Term::binary(vec![1, 2, 3])),
                    program.host_term(Term::Integer(2)),
                ],
            )
            .unwrap_err()
            .to_string(),
        "unsupported bs_create_bin segment binary"
    );
    assert_eq!(
        reference("bs_create_bin_probe:binary_sized(<<1,2,3>>, 2)"),
        "<<1,2>>\n"
    );
}

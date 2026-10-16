#[path = "../common/mod.rs"]
mod common;

use std::{process::Command, sync::Arc};

use common::pinned_beam;
use num_bigint::BigInt;
use pbeam_runtime::{BeamProgram, Term};

const BINARY_MATCH_PROBE: &[u8] =
    include_bytes!("../../fixtures/binary_match/binary_match_probe.beam");
const BS_MATCH_PROBE: &[u8] = include_bytes!("../../fixtures/binary_match/bs_match_probe.beam");
const BS_GET_PROBE: &[u8] = include_bytes!("../../fixtures/binary_match/bs_get_probe.beam");
const BS_FLOAT_PROBE: &[u8] = include_bytes!("../../fixtures/binary_match/bs_float_probe.beam");
const BS_UTF_PROBE: &[u8] = include_bytes!("../../fixtures/binary_match/bs_utf_probe.beam");
const MATCH_COPY_PROBE: &[u8] = include_bytes!("../../fixtures/binary_match/match_copy_probe.beam");

/// Evaluate an expression on the pinned ERTS with `binary_match_probe` in
/// the code path and return its `~w` output.
fn reference(expression: &str) -> Vec<u8> {
    let reference = Command::new("erl")
        .args([
            "-noshell",
            "-pa",
            "fixtures/binary_match",
            "-eval",
            &format!("io:format(\"~w~n\", [{expression}]), halt()."),
        ])
        .output()
        .unwrap();
    assert!(reference.status.success());
    assert!(reference.stderr.is_empty());
    reference.stdout
}

#[test]
fn fails_the_bs_start_match3_test_for_each_term_that_is_not_a_binary() {
    // `binary_match_probe:kinds/0` runs `bs_start_match3` of
    // `binary_match_probe:kind/1` on one term of each kind that PBeam has,
    // except a binary. 2^60 is an ERTS bignum, which is boxed but not a
    // bitstring (emu bs_instrs.tab:1199-1238).
    let program = BeamProgram::load(&[BINARY_MATCH_PROBE]).unwrap();
    let other = Term::Atom(program.atom("other").unwrap());
    assert_eq!(
        program
            .execute("binary_match_probe", "kinds", &[])
            .unwrap()
            .term,
        Term::list_from(vec![other; 11])
    );
    assert_eq!(
        reference("binary_match_probe:kinds()"),
        b"[other,other,other,other,other,other,other,other,other,other,other]\n"
    );
}

#[test]
fn runs_string_uppercase_of_a_list_as_pinned_erts() {
    // `binary_match_probe:upper/0` runs `string:uppercase/1` on a list, as
    // `logger_formatter` does (logger_formatter.erl:616).
    // `unicode_util:cp/1` runs `bs_start_match3` on the list, and the test
    // fails to the list clause.
    let erlang = pinned_beam("erlang");
    let lists = pinned_beam("lists");
    let string = pinned_beam("string");
    let unicode_util = pinned_beam("unicode_util");
    let program =
        BeamProgram::load(&[&erlang, &lists, &string, &unicode_util, BINARY_MATCH_PROBE]).unwrap();
    let text = |text: &[u8]| Term::list_from(text.iter().map(|&byte| Term::Integer(byte.into())));
    assert_eq!(
        program
            .execute("binary_match_probe", "upper", &[])
            .unwrap()
            .term,
        Term::list_from(vec![text(b"ERROR"), text(b"NOTICE"), text(b"R")])
    );
    assert_eq!(
        reference("binary_match_probe:upper()"),
        b"[[69,82,82,79,82],[78,79,84,73,67,69],[82]]\n"
    );
}

/// Run `binary_match_probe:Function/1` on each input and return the
/// results.
fn run_each(program: &BeamProgram, function: &str, inputs: &[&[u8]]) -> Vec<Term> {
    run_each_in(program, "binary_match_probe", function, inputs)
        .into_iter()
        .map(Result::unwrap)
        .collect()
}

/// Run `Module:Function/1` on each input and return the results, or the
/// text of each error.
fn run_each_in(
    program: &BeamProgram,
    module: &str,
    function: &str,
    inputs: &[&[u8]],
) -> Vec<Result<Term, String>> {
    inputs
        .iter()
        .map(|input| {
            program
                .execute(
                    module,
                    function,
                    &[program.host_term(Term::binary(input.to_vec()))],
                )
                .map(|host| host.term)
                .map_err(|error| error.to_string())
        })
        .collect()
}

/// Return the Erlang list expression of the binaries of `inputs`, for
/// example `[<<65>>,<<>>]`.
fn erlang_binaries(inputs: &[&[u8]]) -> String {
    let binaries: Vec<String> = inputs
        .iter()
        .map(|input| {
            let bytes: Vec<String> = input.iter().map(u8::to_string).collect();
            format!("<<{}>>", bytes.join(","))
        })
        .collect();
    format!("[{}]", binaries.join(","))
}

#[test]
fn runs_bs_start_match3_of_a_binary_as_pinned_erts() {
    // ERTS makes a match context for a binary (erl_bits.c:81-105), and
    // `binary_match_probe:first/1` gives `{utf8,233}`.
    let program = BeamProgram::load(&[BINARY_MATCH_PROBE]).unwrap();
    let utf8 = Term::Atom(program.atom("utf8").unwrap());
    assert_eq!(
        program
            .execute(
                "binary_match_probe",
                "first",
                &[program.host_term(Term::binary(vec![195, 169]))]
            )
            .unwrap()
            .term,
        Term::tuple(vec![utf8, Term::Integer(233)])
    );
    assert_eq!(
        reference("binary_match_probe:first(<<195,169>>)"),
        b"{utf8,233}\n"
    );
}

#[test]
fn runs_the_binary_match_of_is_usascii_as_pinned_erts() {
    // `binary_match_probe:is_usascii/1` has the instructions of
    // `prim_tty:is_usascii/1` (prim_tty.erl:1316-1321): `bs_start_match3`,
    // `bs_get_position`, `bs_get_utf8`, `bs_match` with `ensure_at_least`,
    // `call_only` with the match context in `x0`, `bs_set_position`, and
    // `bs_match` with `ensure_exactly`. The inputs are ASCII, `<<>>`,
    // non-ASCII UTF-8, invalid UTF-8, and a byte that does not start a
    // UTF-8 character.
    let inputs: [&[u8]; 5] = [b"Hello, world!\n", b"", b"a\xC3\xA9", b"ab\xFF", b"\xC8"];
    let program = BeamProgram::load(&[BINARY_MATCH_PROBE]).unwrap();
    let true_ = Term::Atom(program.atom("true").unwrap());
    let false_ = Term::Atom(program.atom("false").unwrap());
    assert_eq!(
        run_each(&program, "usascii", &inputs),
        [true_.clone(), true_, false_.clone(), false_.clone(), false_]
    );
    assert_eq!(
        reference(&format!(
            "[binary_match_probe:usascii(B) || B <- {}]",
            erlang_binaries(&inputs)
        )),
        b"[true,true,false,false,false]\n"
    );
}

#[test]
fn reads_utf8_and_rejects_invalid_forms_as_pinned_erts() {
    // `bs_get_utf8` gives the code point of each valid form and fails for
    // short input, a byte that does not start a character, a bad
    // continuation byte, an overlong form, a surrogate, and a value above
    // 0x10FFFF (`erts_bs_get_utf8`, erl_bits.c:2128-2221).
    let valid: [(&[u8], i64); 7] = [
        (b"A", 65),
        (&[127], 127),
        (&[195, 169], 233),
        (&[226, 130, 172], 8364),
        (&[240, 159, 152, 128], 128_512),
        (&[244, 143, 191, 191], 0x10_FFFF),
        (&[237, 159, 191], 0xD7FF),
    ];
    let invalid: [&[u8]; 25] = [
        // Short input.
        b"",
        &[195],
        &[226, 130],
        &[240, 159, 152],
        // A bad continuation byte.
        &[195, 40],
        &[226, 40, 172],
        &[226, 130, 40],
        &[240, 40, 152, 128],
        &[240, 159, 40, 128],
        &[240, 159, 152, 40],
        // A byte that does not start a character.
        &[128],
        &[191],
        &[248, 136, 128, 128],
        &[255],
        // An overlong form.
        &[192, 128],
        &[193, 191],
        &[224, 128, 128],
        &[224, 159, 191],
        &[240, 128, 128, 128],
        &[240, 143, 191, 191],
        // A surrogate.
        &[237, 160, 128],
        &[237, 191, 191],
        // A value above 0x10FFFF.
        &[244, 144, 128, 128],
        &[245, 128, 128, 128],
        &[247, 191, 191, 191],
    ];
    let inputs: Vec<&[u8]> = valid
        .iter()
        .map(|(input, _)| *input)
        .chain(invalid)
        .collect();
    let program = BeamProgram::load(&[BINARY_MATCH_PROBE]).unwrap();
    let false_ = Term::Atom(program.atom("false").unwrap());
    let expected: Vec<Term> = valid
        .iter()
        .map(|(_, code_point)| Term::Integer(*code_point))
        .chain(invalid.iter().map(|_| false_.clone()))
        .collect();
    assert_eq!(run_each(&program, "utf8", &inputs), expected);
    assert_eq!(
        reference(&format!(
            "[binary_match_probe:utf8(B) || B <- {}]",
            erlang_binaries(&inputs)
        )),
        format!(
            "[65,127,233,8364,128512,1114111,55295{}]\n",
            ",false".repeat(25)
        )
        .as_bytes()
    );
}

#[test]
fn runs_the_integer_command_of_binary_match_probe_byte_as_pinned_erts() {
    // `binary_match_probe:byte/1` runs `bs_match` with `ensure_at_least`
    // and then `integer`. It gives the first byte, or `false` when no byte
    // remains.
    let program = BeamProgram::load(&[BINARY_MATCH_PROBE]).unwrap();
    let inputs: [&[u8]; 2] = [&[7, 8], b""];
    assert_eq!(
        run_each(&program, "byte", &inputs),
        [Term::Integer(7), Term::Atom(program.atom("false").unwrap())]
    );
    assert_eq!(
        reference("[binary_match_probe:byte(B) || B <- [<<7,8>>,<<>>]]"),
        b"[7,false]\n"
    );
}

/// Run `bs_match_probe:Function/1` on each input in PBeam, and in pinned
/// ERTS with `~w` output.
fn bs_match_each(function: &str, inputs: &[&[u8]]) -> (Vec<Result<Term, String>>, Vec<u8>) {
    let program = BeamProgram::load(&[BS_MATCH_PROBE]).unwrap();
    (
        run_each_in(&program, "bs_match_probe", function, inputs),
        reference(&format!(
            "[bs_match_probe:{function}(B) || B <- {}]",
            erlang_binaries(inputs)
        )),
    )
}

/// The atom `name` of `bs_match_probe`.
fn probe_atom(name: &str) -> Term {
    let program = BeamProgram::load(&[BS_MATCH_PROBE]).unwrap();
    Term::Atom(program.atom(name).unwrap())
}

/// A list of integers.
fn integers(values: &[i64]) -> Term {
    Term::list_from(values.iter().copied().map(Term::Integer))
}

#[test]
fn runs_unsigned_big_endian_integer_commands_as_pinned_erts() {
    // `bs_match_probe:unsigned/1` reads integers of 1 to 16 bits, at
    // positions that are not byte-aligned, and `F:2/unit:8`, which has
    // size 2 and unit 8 (`emit_extract_integer`, jit/arm/instr_bs.cpp:
    // 2750-2879). Seven bytes fail `ensure_exactly`.
    let inputs: [&[u8]; 2] = [
        &[0xAB, 0xCD, 0xEF, 0x12, 0x34, 0x56, 0x78, 0x9A],
        &[1, 2, 3, 4, 5, 6, 7],
    ];
    let (results, erts) = bs_match_each("unsigned", &inputs);
    assert_eq!(
        results,
        [
            Ok(integers(&[5, 11, 3294, 15, 4660, 22136, 1, 26])),
            Ok(probe_atom("false"))
        ]
    );
    assert_eq!(erts, b"[[5,11,3294,15,4660,22136,1,26],false]\n");
}

#[test]
fn runs_little_native_and_signed_integer_commands_as_pinned_erts() {
    // `bs_match_probe:flagged/1` reads `little`, `native`, and `signed`
    // integers. A little-endian segment of 12 bits has its last 4 bits as
    // the most significant part (jit/arm/instr_bs.cpp:2801-2827), and
    // `native` is little-endian (`bs_get_flags`,
    // jit/beam_jit_bs.cpp:255-285).
    let inputs: [&[u8]; 3] = [
        &[1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14],
        &[
            255, 254, 253, 252, 251, 250, 249, 248, 247, 246, 245, 244, 243, 242,
        ],
        b"",
    ];
    let (results, erts) = bs_match_each("flagged", &inputs);
    assert_eq!(
        results,
        [
            Ok(integers(&[513, 3, 4, 5, 6, 7, 185_207_048, 920_844])),
            Ok(integers(&[
                65279,
                4093,
                12,
                -5,
                -6,
                -7,
                4_126_603_256,
                -855_052
            ])),
            Ok(probe_atom("false"))
        ]
    );
    assert_eq!(
        erts,
        b"[[513,3,4,5,6,7,185207048,920844],[65279,4093,12,-5,-6,-7,4126603256,-855052],false]\n"
    );
}

#[test]
fn runs_wide_integer_commands_and_stops_at_a_bignum() {
    // `bs_match_probe:wide/1` reads integers of 64, 68, 60, and 59 bits.
    // The 68-bit segment uses `erts_bs_get_integer_2` (erl_bits.c:170-398).
    // ERTS gives 2^63 as a bignum. PBeam has no bignum term, so it stops
    // the run (row "Integer range").
    let mut first = vec![
        0, 0, 0, 0, 0, 0, 0, 1, 255, 255, 255, 255, 255, 255, 255, 254, 253,
    ];
    first.extend([255; 7]);
    first.extend([248, 0, 0, 0, 0, 0, 0, 5, 128, 0, 0, 0, 0, 0, 0, 17]);
    let mut second = vec![127, 255, 255, 255, 255, 255, 255, 255, 128];
    second.extend([0; 14]);
    second.extend([
        64, 0, 0, 0, 0, 0, 0, 0, 0, 127, 255, 255, 255, 255, 255, 255, 224,
    ]);
    let mut big = vec![128];
    big.extend([0; 39]);
    let mut little_big = vec![0; 40];
    little_big[23] = 128;
    let inputs: [&[u8]; 4] = [&first, &second, &big, &little_big];
    let (results, erts) = bs_match_each("wide", &inputs);
    let bignum =
        Err("unsupported bs_match integer outside the 64-bit range: needs a bignum".into());
    assert_eq!(
        results,
        [
            Ok(integers(&[1, -2, -3, (1 << 59) + 5, -(1 << 58), 17])),
            Ok(integers(&[
                i64::MAX,
                i64::MIN,
                1 << 62,
                0,
                (1 << 58) - 1,
                0
            ])),
            bignum.clone(),
            bignum
        ]
    );
    assert_eq!(
        String::from_utf8(erts).unwrap(),
        "[[1,-2,-3,576460752303423493,-288230376151711744,17],\
         [9223372036854775807,-9223372036854775808,4611686018427387904,0,288230376151711743,0],\
         [9223372036854775808,0,0,0,0,0],[0,0,9223372036854775808,0,0,0]]\n"
    );
}

#[test]
fn runs_the_equal_and_get_tail_commands_as_pinned_erts() {
    // `bs_match_probe:prefix/1` matches strings with `=:=`: the compiler
    // splits "hello world" in parts of at most 31 bits
    // (beam_ssa_codegen.erl:2817-2833). A failed `=:=` jumps to the next
    // clause, which restores the position with `bs_set_position`.
    // `get_tail` gives the rest of the binary.
    let inputs: [&[u8]; 8] = [
        b"ab",
        b"abc",
        b"hello world!",
        b"hello there",
        b"hello",
        &[0x1F, 9],
        b"x",
        b"",
    ];
    let (results, erts) = bs_match_each("prefix", &inputs);
    let tagged = |tag: &str, rest: &[u8]| {
        Ok(Term::tuple(vec![
            probe_atom(tag),
            Term::binary(rest.to_vec()),
        ]))
    };
    assert_eq!(
        results,
        [
            tagged("ab", b""),
            tagged("ab", b"c"),
            tagged("hello", b"!"),
            tagged("hi", b" there"),
            tagged("hi", b""),
            Ok(Term::tuple(vec![probe_atom("nibble"), Term::Integer(15)])),
            Ok(probe_atom("other")),
            Ok(probe_atom("other"))
        ]
    );
    assert_eq!(
        erts,
        b"[{ab,<<>>},{ab,<<99>>},{hello,<<33>>},{hi,<<32,116,104,101,114,101>>},{hi,<<>>},{nibble,15},other,other]\n"
    );
}

#[test]
fn runs_the_skip_and_binary_commands_as_pinned_erts() {
    // `bs_match_probe:skip/1` skips 16, 3, and 104 bits.
    // `bs_match_probe:binaries/1` reads binaries of 2 bytes, of 1 byte at
    // bit 20, and of 9 bytes, which is above 64 bits (`GET_BITSTRING`,
    // jit/arm/instr_bs.cpp:3136-3176). `bs_match_probe:tail/1` skips a
    // byte and reads the tail.
    let skip: [&[u8]; 2] = [
        &[
            1, 2, 3, 0xE5, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 16, 17,
        ],
        &[1],
    ];
    let (results, erts) = bs_match_each("skip", &skip);
    assert_eq!(
        results,
        [Ok(integers(&[3, 5, 17])), Ok(probe_atom("false"))]
    );
    assert_eq!(erts, b"[[3,5,17],false]\n");
    let binaries: [&[u8]; 2] = [&[1, 2, 0xAB, 0xCD, 3, 4, 5, 6, 7, 8, 9, 10, 11], &[1]];
    let (results, erts) = bs_match_each("binaries", &binaries);
    assert_eq!(
        results,
        [
            Ok(Term::list_from(vec![
                Term::binary(vec![1, 2]),
                Term::binary(vec![0xBC]),
                Term::binary(vec![3, 4, 5, 6, 7, 8, 9, 10, 11])
            ])),
            Ok(probe_atom("false"))
        ]
    );
    assert_eq!(erts, b"[[<<1,2>>,<<188>>,<<3,4,5,6,7,8,9,10,11>>],false]\n");
    let tail: [&[u8]; 2] = [&[1, 2, 3], b""];
    let (results, erts) = bs_match_each("tail", &tail);
    assert_eq!(
        results,
        [Ok(Term::binary(vec![2, 3])), Ok(probe_atom("false"))]
    );
    assert_eq!(erts, b"[<<2,3>>,false]\n");
}

#[test]
fn stops_at_a_binary_or_get_tail_command_that_needs_a_bitstring() {
    // `bs_match_probe:bits/1` reads a binary of 3 bits, and
    // `bs_match_probe:bit_tail/1` reads a tail of 4 bits. ERTS gives the
    // bitstrings `<<5:3>>` and `<<5:4>>`. PBeam has no bitstring term, so
    // it stops the run (row "Binary matching").
    let inputs: [&[u8]; 2] = [&[0xA5], b""];
    let (results, erts) = bs_match_each("bits", &inputs);
    assert_eq!(
        results,
        [
            Err("unsupported bs_match binary of 3 bits: needs a bitstring term".into()),
            Ok(probe_atom("false"))
        ]
    );
    assert_eq!(erts, b"[<<5:3>>,false]\n");
    let (results, erts) = bs_match_each("bit_tail", &inputs);
    assert_eq!(
        results,
        [
            Err("unsupported bs_match get_tail of 4 bits: needs a bitstring term".into()),
            Ok(probe_atom("false"))
        ]
    );
    assert_eq!(erts, b"[<<5:4>>,false]\n");
}

#[test]
fn reads_utf8_after_a_skip_to_an_unaligned_position_as_pinned_erts() {
    // `bs_match_probe:unaligned_utf8/1` skips 4 bits with `bs_match` and
    // then runs `bs_get_utf8`. ERTS aligns up to 4 whole bytes after the
    // position and fails when a byte of the character is not there
    // (`erts_align_utf8_bytes` and `erts_bs_get_utf8`, erl_bits.c:
    // 2106-2221).
    let inputs: [&[u8]; 7] = [
        &[0x04, 0x10],
        &[0x04],
        &[0x0C, 0x3A, 0x90],
        &[0x0C, 0x3A],
        &[0x0E, 0x28, 0x2A, 0xC0],
        &[0x0F, 0x09, 0xF9, 0x88, 0x00],
        &[0x0C, 0x00],
    ];
    let (results, erts) = bs_match_each("unaligned_utf8", &inputs);
    let false_atom = || Ok(probe_atom("false"));
    assert_eq!(
        results,
        [
            Ok(Term::Integer(65)),
            false_atom(),
            Ok(Term::Integer(233)),
            false_atom(),
            Ok(Term::Integer(8364)),
            Ok(Term::Integer(128_512)),
            false_atom(),
        ]
    );
    assert_eq!(erts, b"[65,false,233,false,8364,128512,false]\n");
}

/// Run `bs_get_probe:Function` with each argument list, and return the
/// results or the text of each error.
fn bs_get_each(function: &str, calls: &[Vec<Term>]) -> Vec<Result<Term, String>> {
    let program = BeamProgram::load(&[BS_GET_PROBE]).unwrap();
    calls
        .iter()
        .map(|arguments| {
            let imported = arguments
                .iter()
                .cloned()
                .map(|term| program.host_term(term))
                .collect::<Vec<_>>();
            program
                .execute("bs_get_probe", function, &imported)
                .map(|host| host.term)
                .map_err(|error| error.to_string())
        })
        .collect()
}

/// The atom `name` of `bs_get_probe`.
fn bs_get_atom(name: &str) -> Term {
    let program = BeamProgram::load(&[BS_GET_PROBE]).unwrap();
    Term::Atom(program.atom(name).unwrap())
}

/// The tuple `{Head, Tail}` of two binaries.
fn halves(head: &[u8], tail: &[u8]) -> Result<Term, String> {
    Ok(Term::tuple(vec![
        Term::binary(head.to_vec()),
        Term::binary(tail.to_vec()),
    ]))
}

#[test]
fn runs_bs_start_match4_bs_get_binary2_and_bs_get_tail_as_pinned_erts() {
    // `bs_get_probe:split/2` runs `bs_start_match4` with `no_fail`, then
    // `bs_get_binary2` with the size in `x1` and unit 8, and then
    // `bs_get_tail`. A size that is not a non-negative integer fails the
    // field size check (`emit_bs_get_field_size`, jit/arm/instr_bs.cpp:
    // 38-116). A size larger than the rest fails in
    // `erts_bs_get_binary_2` (erl_bits.c:400-424). 2^59 is an ERTS bignum.
    let bytes = || Term::binary(vec![1, 2, 3]);
    let calls = [
        vec![bytes(), Term::Integer(0)],
        vec![bytes(), Term::Integer(1)],
        vec![bytes(), Term::Integer(3)],
        vec![bytes(), Term::Integer(4)],
        vec![Term::binary(Vec::new()), Term::Integer(0)],
        vec![Term::binary(Vec::new()), Term::Integer(1)],
        vec![bytes(), Term::Integer(-1)],
        vec![bytes(), bs_get_atom("short")],
        vec![bytes(), Term::Integer(1 << 59)],
        vec![bytes(), Term::Integer(1 << 62)],
    ];
    let short = || Ok(bs_get_atom("short"));
    assert_eq!(
        bs_get_each("split", &calls),
        [
            halves(b"", &[1, 2, 3]),
            halves(&[1], &[2, 3]),
            halves(&[1, 2, 3], b""),
            short(),
            halves(b"", b""),
            short(),
            short(),
            short(),
            short(),
            short(),
        ]
    );
    assert_eq!(
        reference(
            "[bs_get_probe:split(B, N) || {B, N} <- [{<<1,2,3>>,0},{<<1,2,3>>,1},\
             {<<1,2,3>>,3},{<<1,2,3>>,4},{<<>>,0},{<<>>,1},{<<1,2,3>>,-1},\
             {<<1,2,3>>,short},{<<1,2,3>>,1 bsl 59},{<<1,2,3>>,1 bsl 62}]]"
        ),
        b"[{<<>>,<<1,2,3>>},{<<1>>,<<2,3>>},{<<1,2,3>>,<<>>},short,{<<>>,<<>>},\
          short,short,short,short,short]\n"
    );
}

#[test]
fn multiplies_the_bs_get_binary2_size_by_its_unit_as_pinned_erts() {
    // `bs_get_probe:split_units/2` runs `bs_get_binary2` with unit 16.
    let calls = [
        vec![Term::binary(vec![1, 2, 3]), Term::Integer(1)],
        vec![Term::binary(vec![1, 2, 3, 4]), Term::Integer(2)],
        vec![Term::binary(vec![1, 2, 3]), Term::Integer(2)],
    ];
    assert_eq!(
        bs_get_each("split_units", &calls),
        [
            halves(&[1, 2], &[3]),
            halves(&[1, 2, 3, 4], b""),
            Ok(bs_get_atom("short")),
        ]
    );
    assert_eq!(
        reference(
            "[bs_get_probe:split_units(B, N) || {B, N} <- [{<<1,2,3>>,1},{<<1,2,3,4>>,2},\
             {<<1,2,3>>,2}]]"
        ),
        b"[{<<1,2>>,<<3>>},{<<1,2,3,4>>,<<>>},short]\n"
    );
}

#[test]
fn stops_at_a_bs_get_binary2_that_needs_a_bitstring() {
    // `bs_get_probe:split_bits/2` runs `bs_get_binary2` with unit 1. ERTS
    // gives the bitstring `<<0:3>>` for 3 bits. PBeam has no bitstring
    // term, so it stops the run (row "Binary matching").
    let calls: Vec<Vec<Term>> = [8, 16, 17, 3]
        .into_iter()
        .map(|size| vec![Term::binary(vec![1, 2]), Term::Integer(size)])
        .collect();
    assert_eq!(
        bs_get_each("split_bits", &calls),
        [
            halves(&[1], &[2]),
            halves(&[1, 2], b""),
            Ok(bs_get_atom("short")),
            Err("unsupported bs_get_binary2 of 3 bits: needs a bitstring term".into()),
        ]
    );
    assert_eq!(
        reference("[bs_get_probe:split_bits(<<1,2>>, N) || N <- [8,16,17,3]]"),
        b"[{<<1>>,<<2>>},{<<1,2>>,<<>>},short,{<<0:3>>,<<8,2:5>>}]\n"
    );
}

#[test]
fn resumes_a_match_context_and_reads_its_tail_as_pinned_erts() {
    // `bs_get_probe:nibbles/2` gets the match context of `nibbles/1` and
    // runs `bs_start_match4` with `resume`. At the fail label, it runs
    // `bs_set_position` and then `bs_get_tail`. A tail of 12 bits is a
    // bitstring in ERTS, so PBeam stops the run (row "Binary matching").
    let calls: Vec<Vec<Term>> = [&b"a"[..], b"a\x12\xF3", b"a\x1F\x23", b"a\x12", b"b"]
        .into_iter()
        .map(|input| vec![Term::binary(input.to_vec())])
        .collect();
    let nibbles = |values: &[i64], tail: &[u8]| {
        Ok(Term::tuple(vec![
            Term::list_from(values.iter().copied().map(Term::Integer)),
            Term::binary(tail.to_vec()),
        ]))
    };
    assert_eq!(
        bs_get_each("nibbles", &calls),
        [
            nibbles(&[], b""),
            nibbles(&[2, 1], &[0xF3]),
            Err("unsupported bs_get_tail of 12 bits: needs a bitstring term".into()),
            nibbles(&[2, 1], b""),
            Ok(bs_get_atom("none")),
        ]
    );
    assert_eq!(
        reference(
            "[bs_get_probe:nibbles(B) || B <- [<<\"a\">>,<<$a,16#12,16#F3>>,<<$a,16#1F,16#23>>,\
             <<$a,16#12>>,<<\"b\">>]]"
        ),
        b"[{[],<<>>},{[2,1],<<243>>},{[1],<<242,3:4>>},{[2,1],<<>>},none]\n"
    );
}

#[test]
fn shares_a_match_context_that_move_copies_as_pinned_erts() {
    // `match_copy_probe:decode_codepoint/1` has the instructions of
    // `Terra.Input.Parser.decode_codepoint/1`: `bs_start_match4` with
    // `resume`, `move` of the match context from `x0` to `y1`, then
    // `bs_set_position`, `byte_size/1`, and `bs_get_tail` on the copy in
    // `y1`, and `bs_start_match4` with `no_fail` on a binary in `x3` with
    // the destination `x1`. The inputs are ASCII, characters of 2, 3, and 4
    // bytes, short input, a byte that does not start a character, a bad
    // continuation byte, an overlong form, a surrogate, and a value above
    // 0x10FFFF.
    let inputs: [&[u8]; 11] = [
        b"",
        b"ab",
        b"a\xC3\xA9b",
        b"\xE2\x82\xAC\xF0\x9F\x98\x80z",
        b"a\xE2\x82",
        b"\xF4",
        b"\xFFa",
        b"\xC3(",
        b"\xE0\x80\x80",
        b"\xED\xA0\x80",
        b"\xF4\x90\x80\x80",
    ];
    let erlang = pinned_beam("erlang");
    let lists = pinned_beam("lists");
    let program = BeamProgram::load(&[&erlang, &lists, MATCH_COPY_PROBE]).unwrap();
    let atom = |name| Term::Atom(program.atom(name).unwrap());
    let invalid = || Term::list_from(vec![atom("invalid")]);
    assert_eq!(
        run_each_in(&program, "match_copy_probe", "parse", &inputs),
        [
            Ok(integers(&[])),
            Ok(integers(&[97, 98])),
            Ok(integers(&[97, 233, 98])),
            Ok(integers(&[8364, 128_512, 122])),
            Ok(Term::list_from(vec![Term::Integer(97), atom("incomplete")])),
            Ok(Term::list_from(vec![atom("incomplete")])),
            Ok(invalid()),
            Ok(invalid()),
            Ok(invalid()),
            Ok(invalid()),
            Ok(invalid()),
        ]
    );
    assert_eq!(
        reference(&format!(
            "[match_copy_probe:parse(B) || B <- {}]",
            erlang_binaries(&inputs)
        )),
        b"[[],[97,98],[97,233,98],[8364,128512,122],[97,incomplete],[incomplete],\
[invalid],[invalid],[invalid],[invalid],[invalid]]\n"
    );
}

/// Run `bs_float_probe:Function` with each argument list, and return the
/// results or the text of each error.
fn bs_float_each(function: &str, calls: &[Vec<Term>]) -> Vec<Result<Term, String>> {
    let program = BeamProgram::load(&[BS_FLOAT_PROBE]).unwrap();
    calls
        .iter()
        .map(|arguments| {
            let imported = arguments
                .iter()
                .cloned()
                .map(|term| program.host_term(term))
                .collect::<Vec<_>>();
            program
                .execute("bs_float_probe", function, &imported)
                .map(|host| host.term)
                .map_err(|error| error.to_string())
        })
        .collect()
}

/// The calls `Function(Bin, N)` of each binary and size.
fn float_calls(cases: &[(&[u8], Term)]) -> Vec<Vec<Term>> {
    cases
        .iter()
        .map(|(bytes, size)| vec![Term::binary(bytes.to_vec()), size.clone()])
        .collect()
}

/// The result `{F, Rest}` of a float segment.
fn float_rest(value: f64, rest: &[u8]) -> Result<Term, String> {
    Ok(Term::tuple(vec![
        Term::Float(value),
        Term::binary(rest.to_vec()),
    ]))
}

#[test]
fn runs_bs_get_float2_of_each_size_and_endianness_as_pinned_erts() {
    // `bs_float_probe:big/2`, `little/2`, and `native/2` run
    // `bs_get_float2` with the size in a register. A size of 0 gives 0.0
    // and reads no bits. A size that is not 16, 32, or 64, a size that is
    // not a non-negative integer, fewer bits than the size, an infinity,
    // and a NaN fail (`erts_bs_get_float_2`, erl_bits.c:549-611;
    // `emit_bs_get_field_size`, jit/arm/instr_bs.cpp:38-116). A 16-bit
    // segment is a half-precision float. `native` is the byte order of the
    // host, which is little endian on each host that PBeam has. 2^64 and
    // -2^64 are bignums, and 2^59 is an ERTS bignum; each fails.
    let short = Ok(Term::Atom(
        BeamProgram::load(&[BS_FLOAT_PROBE])
            .unwrap()
            .atom("short")
            .unwrap(),
    ));
    let bignum = |value: BigInt| Term::BigInt(Arc::new(value));
    let big: [(&[u8], Term); 22] = [
        (&[64, 9, 33, 251, 84, 68, 45, 24, 7], Term::Integer(64)),
        (&[63, 192, 0, 0], Term::Integer(32)),
        (&[60, 0, 1], Term::Integer(16)),
        (&[1, 2], Term::Integer(0)),
        (&[1, 2, 3], Term::Integer(24)),
        (&[1, 2], Term::Integer(32)),
        (&[1, 2, 3, 4], Term::Integer(-1)),
        (&[1, 2, 3, 4], Term::Nil),
        (&[1, 2, 3, 4], Term::Float(1.0)),
        (&[127, 128, 0, 0], Term::Integer(32)),
        (&[127, 192, 0, 0], Term::Integer(32)),
        (&[124, 0], Term::Integer(16)),
        (&[126, 0], Term::Integer(16)),
        (&[255, 240, 0, 0, 0, 0, 0, 0], Term::Integer(64)),
        (&[127, 248, 0, 0, 0, 0, 0, 1], Term::Integer(64)),
        (&[0, 1], Term::Integer(16)),
        (&[123, 255], Term::Integer(16)),
        (&[128, 0], Term::Integer(16)),
        (&[1, 2, 3, 4], Term::Integer(i64::MAX)),
        (&[1, 2, 3, 4], bignum(BigInt::from(1u8) << 64u32)),
        (&[1, 2, 3, 4], bignum(-(BigInt::from(1u8) << 64u32))),
        (&[1, 2, 3, 4], Term::Integer(1 << 59)),
    ];
    assert_eq!(
        bs_float_each("big", &float_calls(&big)),
        [
            float_rest(std::f64::consts::PI, &[7]),
            float_rest(1.5, &[]),
            float_rest(1.0, &[1]),
            float_rest(0.0, &[1, 2]),
            short.clone(),
            short.clone(),
            short.clone(),
            short.clone(),
            short.clone(),
            short.clone(),
            short.clone(),
            short.clone(),
            short.clone(),
            short.clone(),
            short.clone(),
            float_rest(5.960464477539063e-8, &[]),
            float_rest(65504.0, &[]),
            float_rest(-0.0, &[]),
            short.clone(),
            short.clone(),
            short.clone(),
            short.clone(),
        ]
    );
    assert_eq!(
        reference(
            "[bs_float_probe:big(B, N) || {B, N} <- [{<<64,9,33,251,84,68,45,24,7>>,64}, \
             {<<63,192,0,0>>,32}, {<<60,0,1>>,16}, {<<1,2>>,0}, {<<1,2,3>>,24}, {<<1,2>>,32}, \
             {<<1,2,3,4>>,-1}, {<<1,2,3,4>>,[]}, {<<1,2,3,4>>,1.0}, {<<127,128,0,0>>,32}, \
             {<<127,192,0,0>>,32}, {<<124,0>>,16}, {<<126,0>>,16}, {<<255,240,0,0,0,0,0,0>>,64}, \
             {<<127,248,0,0,0,0,0,1>>,64},              {<<0,1>>,16}, {<<123,255>>,16}, {<<128,0>>,16}, {<<1,2,3,4>>,1 bsl 63 - 1}, \
             {<<1,2,3,4>>,1 bsl 64}, {<<1,2,3,4>>,-(1 bsl 64)}, {<<1,2,3,4>>,1 bsl 59}]]"
        ),
        b"[{3.141592653589793,<<7>>},{1.5,<<>>},{1.0,<<1>>},{0.0,<<1,2>>},short,short,short,\
short,short,short,short,short,short,short,short,{5.960464477539063e-8,<<>>},{65504.0,<<>>},\
{-0.0,<<>>},short,short,short,short]\n"
    );
    let little: [(&[u8], Term); 4] = [
        (&[0, 0, 192, 63], Term::Integer(32)),
        (&[0, 60], Term::Integer(16)),
        (&[24, 45, 68, 84, 251, 33, 9, 64], Term::Integer(64)),
        (&[0, 0, 128, 127], Term::Integer(32)),
    ];
    let expected = [
        float_rest(1.5, &[]),
        float_rest(1.0, &[]),
        float_rest(std::f64::consts::PI, &[]),
        short.clone(),
    ];
    assert_eq!(bs_float_each("little", &float_calls(&little)), expected);
    assert_eq!(bs_float_each("native", &float_calls(&little)), expected);
    let little_erts = "[{<<0,0,192,63>>,32}, {<<0,60>>,16}, {<<24,45,68,84,251,33,9,64>>,64}, \
                       {<<0,0,128,127>>,32}]";
    for function in ["little", "native"] {
        assert_eq!(
            reference(&format!(
                "[bs_float_probe:{function}(B, N) || {{B, N}} <- {little_erts}]"
            )),
            b"[{1.5,<<>>},{1.0,<<>>},{3.141592653589793,<<>>},short]\n"
        );
    }
}

#[test]
fn runs_bs_get_float2_with_a_unit_a_literal_size_and_an_unaligned_position_as_pinned_erts() {
    // `bs_float_probe:units/2` has unit 16, so the size is N * 16 bits.
    // `half/1` has the literal size 16. `huge/1` has the literal size 2^64,
    // which is not a small integer, so it jumps to the fail label
    // (`emit_bs_get_field_size`, jit/arm/instr_bs.cpp:58-86).
    // `unaligned/1` skips 4 bits with `bs_match` and then reads 32 bits.
    let short = Ok(Term::Atom(
        BeamProgram::load(&[BS_FLOAT_PROBE])
            .unwrap()
            .atom("short")
            .unwrap(),
    ));
    let units: [(&[u8], Term); 4] = [
        (&[63, 192, 0, 0], Term::Integer(2)),
        (&[1, 2, 3, 4, 5, 6], Term::Integer(3)),
        (&[64, 9, 33, 251, 84, 68, 45, 24], Term::Integer(4)),
        (&[1], Term::Integer(0)),
    ];
    assert_eq!(
        bs_float_each("units", &float_calls(&units)),
        [
            float_rest(1.5, &[]),
            short.clone(),
            float_rest(std::f64::consts::PI, &[]),
            float_rest(0.0, &[1]),
        ]
    );
    assert_eq!(
        reference(
            "[bs_float_probe:units(B, N) || {B, N} <- [{<<63,192,0,0>>,2}, {<<1,2,3,4,5,6>>,3}, \
             {<<64,9,33,251,84,68,45,24>>,4}, {<<1>>,0}]]"
        ),
        b"[{1.5,<<>>},short,{3.141592653589793,<<>>},{0.0,<<1>>}]\n"
    );
    let half = [vec![Term::binary(vec![60, 0])], vec![Term::binary(vec![1])]];
    assert_eq!(
        bs_float_each("half", &half),
        [float_rest(1.0, &[]), short.clone()]
    );
    assert_eq!(
        reference("[bs_float_probe:half(B) || B <- [<<60,0>>, <<1>>]]"),
        b"[{1.0,<<>>},short]\n"
    );
    let huge = [vec![Term::binary(vec![60, 0])], vec![Term::binary(vec![])]];
    assert_eq!(bs_float_each("huge", &huge), [short.clone(), short.clone()]);
    assert_eq!(
        reference("[bs_float_probe:huge(B) || B <- [<<60,0>>, <<>>]]"),
        b"[short,short]\n"
    );
    let unaligned = [
        vec![Term::binary(vec![3, 252, 0, 0, 0])],
        vec![Term::binary(vec![7, 248, 0, 0, 0])],
    ];
    assert_eq!(
        bs_float_each("unaligned", &unaligned),
        [Ok(Term::Float(1.5)), short]
    );
    assert_eq!(
        reference("[bs_float_probe:unaligned(B) || B <- [<<3,252,0,0,0>>, <<7,248,0,0,0>>]]"),
        b"[1.5,short]\n"
    );
}

#[test]
fn runs_erl_parse_of_a_term_with_a_binary_through_eval_bits_as_pinned_erts() {
    // `bs_float_probe:parse/0` parses the tokens of
    // `{short, <<1, 2:16, "xy", 1.5/float>>}.`. `erl_parse:normalise/1` builds
    // each binary with `eval_bits:expr_grp/3` (erl_parse.yrl:1960-1966),
    // which starts with `bs_init_writable` and appends each field with a
    // `private_append` segment. The 1.5 field is a `float` segment of 64
    // bits. The modules are the pinned OTP modules that the call reaches.
    let modules = [
        "erl_parse",
        "eval_bits",
        "erl_bits",
        "erl_anno",
        "lists",
        "erl_scan",
    ]
    .map(pinned_beam);
    let mut beams: Vec<&[u8]> = modules.iter().map(Vec::as_slice).collect();
    beams.push(BS_FLOAT_PROBE);
    let program = BeamProgram::load(&beams).unwrap();
    let atom = |name: &str| Term::Atom(program.atom(name).unwrap());
    let result = program
        .execute("bs_float_probe", "parse", &[])
        .map(|host| host.term)
        .map_err(|error| error.to_string());
    let binary = Term::binary(vec![1, 0, 2, 120, 121, 63, 248, 0, 0, 0, 0, 0, 0]);
    assert_eq!(
        result,
        Ok(Term::tuple(vec![
            atom("ok"),
            Term::tuple(vec![atom("short"), binary]),
        ]))
    );
    assert_eq!(
        reference("bs_float_probe:parse()"),
        b"{ok,{short,<<1,0,2,120,121,63,248,0,0,0,0,0,0>>}}\n"
    );
}

/// Run `bs_utf_probe:Function(Bin)` with each binary, and return the
/// results.
fn bs_utf_each(function: &str, binaries: &[Vec<u8>]) -> Vec<Term> {
    let program = BeamProgram::load(&[BS_UTF_PROBE]).unwrap();
    binaries
        .iter()
        .map(|bytes| {
            let argument = program.host_term(Term::binary(bytes.clone()));
            program
                .execute("bs_utf_probe", function, &[argument])
                .unwrap()
                .term
        })
        .collect()
}

/// Reverse the bytes of each whole code unit of `width` bytes and drop the
/// bytes after the last whole unit, as the ERTS run of the fixture makes
/// the little-endian binaries.
fn little_units(binaries: &[Vec<u8>], width: usize) -> Vec<Vec<u8>> {
    binaries
        .iter()
        .map(|bytes| {
            bytes
                .chunks_exact(width)
                .flat_map(|unit| unit.iter().rev().copied())
                .collect()
        })
        .collect()
}

#[test]
fn runs_the_utf16_and_utf32_match_instructions_as_pinned_erts() {
    // `bs_get_utf16` and `bs_skip_utf16` call `erts_bs_get_utf16`
    // (erl_bits.c:2223-2285; jit/arm/instr_bs.cpp:905-934). A unit that is
    // not a surrogate is one character, and a high and a low surrogate are
    // one character. A lone surrogate and fewer than 16 bits fail.
    // `bs_get_utf32` and `bs_skip_utf32` read a 32-bit integer and fail for
    // a surrogate or a value above 16#10FFFF, with the position moved back
    // (jit/arm/ops.tab:1086-1095, instr_bs.cpp:936-983). The expected
    // values are the results of the pinned ERTS.
    let program = BeamProgram::load(&[BS_UTF_PROBE]).unwrap();
    let short = Term::Atom(program.atom("short").unwrap());
    let int = Term::Atom(program.atom("int").unwrap());
    let pair = |code_point: i64, rest: &[u8]| {
        Term::tuple(vec![Term::Integer(code_point), Term::binary(rest.to_vec())])
    };
    let rest = |bytes: &[u8]| Term::binary(bytes.to_vec());
    let big16: Vec<Vec<u8>> = [
        &[0, 65, 1][..],
        &[216, 61, 222, 0, 7],
        &[216, 0, 220, 0],
        &[219, 255, 223, 255],
        &[215, 255],
        &[224, 0],
        &[255, 255],
        &[216, 61],
        &[216, 61, 0, 65],
        &[216, 61, 222],
        &[220, 0, 0, 0],
        &[0],
        &[],
    ]
    .map(<[u8]>::to_vec)
    .to_vec();
    let little16 = little_units(&big16, 2);
    let values16 = [65, 0x1F600, 0x10000, 0x10FFFF, 0xD7FF, 0xE000, 0xFFFF];
    let mut big_results: Vec<_> = values16
        .iter()
        .zip([&[1][..], &[7], &[], &[], &[], &[], &[]])
        .map(|(&code_point, tail)| pair(code_point, tail))
        .collect();
    big_results.extend([short.clone(), short.clone(), short.clone()]);
    big_results.extend([short.clone(), short.clone(), short.clone()]);
    let mut little_results: Vec<_> = values16
        .iter()
        .map(|&code_point| pair(code_point, &[]))
        .collect();
    little_results.extend(vec![short.clone(); 6]);
    assert_eq!(bs_utf_each("get16", &big16), big_results);
    assert_eq!(bs_utf_each("get16_little", &little16), little_results);
    assert_eq!(bs_utf_each("get16_native", &little16), little_results);
    let mut skipped = vec![rest(&[1]), rest(&[7])];
    skipped.extend(vec![rest(&[]); 5]);
    skipped.extend(vec![short.clone(); 6]);
    assert_eq!(bs_utf_each("skip16", &big16), skipped);
    let mut skipped_little = vec![rest(&[]); 7];
    skipped_little.extend(vec![short.clone(); 6]);
    assert_eq!(bs_utf_each("skip16_little", &little16), skipped_little);

    let big32: Vec<Vec<u8>> = [
        &[0, 0, 0, 65, 1][..],
        &[0, 1, 246, 0],
        &[0, 16, 255, 255],
        &[0, 0, 215, 255],
        &[0, 0, 224, 0],
        &[0, 17, 0, 0],
        &[0, 0, 216, 0],
        &[0, 0, 223, 255],
        &[255, 255, 255, 255],
        &[0, 0, 0],
        &[],
    ]
    .map(<[u8]>::to_vec)
    .to_vec();
    let little32 = little_units(&big32, 4);
    let values32 = [65, 0x1F600, 0x10FFFF, 0xD7FF, 0xE000];
    let mut big_results = vec![pair(65, &[1])];
    big_results.extend(
        values32[1..]
            .iter()
            .map(|&code_point| pair(code_point, &[])),
    );
    big_results.extend(vec![short.clone(); 6]);
    let mut little_results: Vec<_> = values32
        .iter()
        .map(|&code_point| pair(code_point, &[]))
        .collect();
    little_results.extend(vec![short.clone(); 6]);
    assert_eq!(bs_utf_each("get32", &big32), big_results);
    assert_eq!(bs_utf_each("get32_little", &little32), little_results);
    assert_eq!(bs_utf_each("get32_native", &little32), little_results);
    let mut skipped = vec![rest(&[1])];
    skipped.extend(vec![rest(&[]); 4]);
    skipped.extend(vec![short.clone(); 6]);
    assert_eq!(bs_utf_each("skip32", &big32), skipped);
    let mut skipped_little = vec![rest(&[]); 5];
    skipped_little.extend(vec![short.clone(); 6]);
    assert_eq!(bs_utf_each("skip32_little", &little32), skipped_little);
    // The next clause of `retract32/1` reads the same 32 bits as an
    // integer after `bs_get_utf32` fails.
    let mut retracted = big_results[..5].to_vec();
    retracted.extend(
        [0x110000, 0xD800, 0xDFFF, 0xFFFF_FFFF]
            .map(|value: i64| Term::tuple(vec![int.clone(), Term::Integer(value), rest(&[])])),
    );
    retracted.extend([short.clone(), short.clone()]);
    assert_eq!(bs_utf_each("retract32", &big32), retracted);

    // A segment at the bit offset 4: `<<15:4, C/utf16, 0:4, 9>>` and the
    // same for `utf32`.
    let offset16: Vec<Vec<u8>> = [
        &[240, 4, 16, 9][..],
        &[253, 131, 221, 224, 0, 9],
        &[253, 131, 208],
        &[253, 192, 0],
    ]
    .map(<[u8]>::to_vec)
    .to_vec();
    assert_eq!(
        bs_utf_each("offset16", &offset16),
        [
            pair(65, &[9]),
            pair(0x1F600, &[9]),
            short.clone(),
            short.clone()
        ]
    );
    let offset32: Vec<Vec<u8>> = [
        &[240, 0, 31, 96, 0, 9][..],
        &[240, 1, 16, 0, 0],
        &[240, 4, 16],
    ]
    .map(<[u8]>::to_vec)
    .to_vec();
    assert_eq!(
        bs_utf_each("offset32", &offset32),
        [pair(0x1F600, &[9]), short.clone(), short]
    );
}

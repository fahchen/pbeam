//! The decode unit tests of `src/beam/code/instruction.rs` decode byte
//! sequences of compiled fixtures. Each test here checks that a fixture
//! still has these bytes, so a recompiled fixture that changes them fails
//! a test.

use std::fs;

/// Assert that each byte sequence of `sequences` occurs in the BEAM file at
/// `path`.
///
/// # Examples
///
/// ```rust,ignore
/// assert_in_fixture("fixtures/funs/fun_probe.beam", &[&[0x4B, 0x00]]); // call_fun 0
/// ```
fn assert_in_fixture(path: &str, sequences: &[&[u8]]) {
    let beam = fs::read(path).unwrap();
    for bytes in sequences {
        assert!(
            beam.windows(bytes.len()).any(|window| window == *bytes),
            "{bytes:02X?} is not in {path}"
        );
    }
}

#[test]
fn arith_probe_has_the_gc_bif1_and_gc_bif2_bytes() {
    assert_in_fixture(
        "fixtures/arith/arith_probe.beam",
        &[
            // up/1: gc_bif2 f58 live 1 '+'/2 {tr,x0,1} 1 x0
            &[0x7D, 0x0D, 0x3A, 0x10, 0x00, 0x57, 0x03, 0x10, 0x11, 0x03],
            // minus_overflow/0: gc_bif2 f0 live 1 '-'/2 {tr,y0,1} {tr,x0,1} x0
            &[
                0x7D, 0x05, 0x10, 0x10, 0x57, 0x04, 0x10, 0x57, 0x03, 0x10, 0x03,
            ],
            // neg/1: gc_bif1 f55 live 1 '-'/1 x0 x0
            &[0x7C, 0x0D, 0x37, 0x10, 0x50, 0x03, 0x03],
        ],
    );
}

#[test]
fn binary_part_probe_has_the_gc_bif3_bytes() {
    assert_in_fixture(
        "fixtures/binary_part/binary_part_probe.beam",
        &[
            // guard/3: gc_bif3 f13 live 3 binary_part/3 x0 x1 x2 x0
            &[0x98, 0xD5, 0x30, 0x00, 0x03, 0x13, 0x23, 0x03],
            // main/0: gc_bif3 f0 live 1 binary_part/3 y23 y22 x0 y22
            &[
                0x98, 0x05, 0x10, 0x00, 0x0C, 0x17, 0x0C, 0x16, 0x03, 0x0C, 0x16,
            ],
        ],
    );
}

#[test]
fn is_integer_probe_has_the_bif3_bytes() {
    assert_in_fixture(
        "fixtures/is_integer/is_integer_probe.beam",
        &[
            // guard/3: bif3 f12 is_integer/3 x0 x1 x2 x0
            &[0xB9, 0xC5, 0x10, 0x03, 0x13, 0x23, 0x03],
            // body/3: bif3 f0 is_integer/3 y0 y2 x0 x0
            &[0xB9, 0x05, 0x10, 0x04, 0x24, 0x03, 0x03],
        ],
    );
}

#[test]
fn binary_match_probe_has_the_binary_match_bytes() {
    assert_in_fixture(
        "fixtures/binary_match/binary_match_probe.beam",
        &[
            // kind/1: bs_start_match3 f5 x0 1 x0
            &[0xA6, 0x55, 0x03, 0x10, 0x03],
            // is_usascii/1, which has the instructions of
            // prim_tty:is_usascii/1: bs_start_match3 f17 x0 1 x0
            &[0xA6, 0x0D, 0x11, 0x03, 0x10, 0x03],
            // bs_get_position x0 x1 1
            &[0xA7, 0x03, 0x13, 0x10],
            // bs_get_utf8 f16 {tr,x0,1} 2 0 x2
            &[0x8A, 0x0D, 0x10, 0x57, 0x03, 0x10, 0x20, 0x00, 0x23],
            // bs_match f16 x0 {list,[ensure_at_least,0,8]}
            &[0xB6, 0x0D, 0x10, 0x03, 0x17, 0x30, 0x82, 0x00, 0x80],
            // bs_set_position x0 x1
            &[0xA8, 0x03, 0x13],
            // bs_match f17 x0 {list,[ensure_exactly,0]}
            &[0xB6, 0x0D, 0x11, 0x03, 0x17, 0x20, 0xF2, 0x00],
        ],
    );
}

#[test]
fn bs_match_probe_has_the_bs_match_command_bytes() {
    assert_in_fixture(
        "fixtures/binary_match/bs_match_probe.beam",
        &[
            // tail/1: bs_match f28 x0 {list,[ensure_at_least,8,8,skip,8,
            // get_tail,1,8,{x,0}]}
            &[
                0xB6, 0x0D, 0x1C, 0x03, 0x17, 0x90, 0x92, 0x80, 0x80, 0x0A, 0x11, 0x80, 0xB2, 0x10,
                0x80, 0x03,
            ],
            // prefix/1: bs_match f15 x0 {list,[ensure_at_least,32,1,
            // '=:=',nil,31,850802231,'=:=',nil,1,1]}
            &[
                0xB6, 0xF5, 0x03, 0x17, 0xB0, 0x92, 0x08, 0x20, 0x10, 0xA2, 0x02, 0x08, 0x1F, 0x58,
                0x32, 0xB6, 0x36, 0x37, 0xA2, 0x02, 0x10, 0x10,
            ],
            // flagged/1: {integer,1,{literal,[little]},16,1,{x,1}}
            &[0x42, 0x10, 0x47, 0x00, 0x08, 0x10, 0x10, 0x13],
            // binaries/1: {binary,1,{literal,[]},2,8,{x,1}}
            &[0x0A, 0x13, 0x10, 0x02, 0x20, 0x80, 0x13],
        ],
    );
}

#[test]
fn bs_get_probe_has_the_bs_start_match4_bs_get_binary2_and_bs_get_tail_bytes() {
    assert_in_fixture(
        "fixtures/binary_match/bs_get_probe.beam",
        &[
            // split/2: bs_start_match4 no_fail 2 x0 x0
            &[0xAA, 0x32, 0x20, 0x03, 0x03],
            // split/2: bs_get_binary2 f3 {tr,x0,1} 2 x1 8 0 x1
            &[0x77, 0x35, 0x57, 0x03, 0x10, 0x20, 0x13, 0x80, 0x00, 0x13],
            // split/2: bs_get_tail x0 x0 2
            &[0xA5, 0x03, 0x03, 0x20],
            // split_units/2: bs_get_binary2 f6 {tr,x0,1} 2 x1 16 0 x1
            &[
                0x77, 0x65, 0x57, 0x03, 0x10, 0x20, 0x13, 0x08, 0x10, 0x00, 0x13,
            ],
            // nibbles/2: bs_start_match4 resume 2 x0 x0
            &[0xAA, 0xB2, 0x20, 0x03, 0x03],
        ],
    );
}

#[test]
fn bitstr_probe_has_the_is_bitstr_bytes() {
    assert_in_fixture(
        "fixtures/bitstr/bitstr_probe.beam",
        &[
            // kind/1: is_bitstr f5 x0
            &[0x81, 0x55, 0x03],
        ],
    );
}

#[test]
fn raise_probe_has_the_raise_bytes() {
    assert_in_fixture(
        "fixtures/raise/raise_probe.beam",
        &[
            // rethrow/1: raise x2 x1
            &[0x6C, 0x23, 0x13],
        ],
    );
}

#[test]
fn error_path_probe_has_the_error_path_instruction_bytes() {
    assert_in_fixture(
        "fixtures/error_path/error_path_probe.beam",
        &[
            // badmatch/1: label 14, badmatch x0
            &[0x01, 0xE0, 0x48, 0x03],
            // badmatch_y/1: label 17, badmatch y0
            &[0x01, 0x08, 0x11, 0x48, 0x04],
            // case_end/1: label 20, case_end x0
            &[0x01, 0x08, 0x14, 0x4A, 0x03],
            // if_end/1: label 23, if_end
            &[0x01, 0x08, 0x17, 0x49],
            // try_case_end/1: label 27, try_case_end x0
            &[0x01, 0x08, 0x1B, 0x6B, 0x03],
            // badrecord/1: label 30, badrecord x0
            &[0x01, 0x08, 0x1E, 0xB4, 0x03],
            // raw_raise/1: move y0 x0, raw_raise
            &[0x40, 0x04, 0x03, 0xA1],
        ],
    );
}

#[test]
fn compare_probe_has_the_is_lt_is_eq_is_ne_is_integer_and_is_boolean_bytes() {
    assert_in_fixture(
        "fixtures/compare/compare_probe.beam",
        &[
            // next/1: is_lt f7 0 {tr,x1,1}
            &[0x27, 0x75, 0x01, 0x57, 0x13, 0x10],
            // width/1: is_lt f15 x0 0
            &[0x27, 0xF5, 0x03, 0x01],
            // kind/1: is_integer f11 x0
            &[0x2D, 0xB5, 0x03],
            // kind/1: is_boolean f12 x0
            &[0x72, 0xC5, 0x03],
            // eq/2: is_eq f25 x0 x1
            &[0x29, 0x0D, 0x19, 0x03, 0x13],
            // ne/2: is_ne f30 x0 x1
            &[0x2A, 0x0D, 0x1E, 0x03, 0x13],
        ],
    );
}

#[test]
fn float_test_probe_has_the_fconv_and_is_float_bytes() {
    assert_in_fixture(
        "fixtures/float_test/float_test_probe.beam",
        &[
            // half/1: fconv x0 fr0, with opcode 97
            &[0x61, 0x03, 0x27, 0x00],
            // kind/1: is_float f5 x0
            &[0x2E, 0x55, 0x03],
        ],
    );
}

#[test]
fn type_test_probe_has_the_is_number_bytes() {
    assert_in_fixture(
        "fixtures/type_test/type_test_probe.beam",
        &[
            // number/1: is_number f27 x0
            &[0x2F, 0x0D, 0x1B, 0x03],
        ],
    );
}

#[test]
fn fun_probe_has_the_fun_bytes() {
    assert_in_fixture(
        "fixtures/funs/fun_probe.beam",
        &[
            // make_pair/1: make_fun3 4 x0 [x0]
            &[0xAB, 0x40, 0x03, 0x17, 0x10, 0x03],
            // locals/0: make_fun3 1 y10 []
            &[0xAB, 0x10, 0xA4, 0x17, 0x00],
            // locals/0: call_fun 0
            &[0x4B, 0x00],
            // tail_call/1: call_fun2 0 1 x1
            &[0xB2, 0x00, 0x10, 0x57, 0x13, 0x20],
            // kind/1: is_function2 f100 x0 1
            &[0x73, 0x0D, 0x64, 0x57, 0x03, 0x40, 0x11],
            // kind/1: is_function f101 x0
            &[0x4D, 0x0D, 0x65, 0x57, 0x03, 0x40],
        ],
    );
}

#[test]
fn io_start_probe_has_the_get_tl_and_put_map_exact_bytes() {
    assert_in_fixture(
        "fixtures/io_start/io_start_probe.beam",
        &[
            // rest/1: {get_tl,{x,0},{x,0}}, with opcode 163
            &[0xA3, 0x03, 0x03],
            // update_guard/1: {put_map_exact,{f,38},{x,0},{x,0},1,
            // {list,[{atom,output},{atom,cooked}]}}, with opcode 155, and
            // output as atom 18 and cooked as atom 19 of the module.
            &[
                0x9B, 0x0D, 0x26, 0x03, 0x03, 0x10, 0x17, 0x20, 0x0A, 0x12, 0x0A, 0x13,
            ],
        ],
    );
}

#[test]
fn map_probe_has_the_has_map_fields_bytes() {
    assert_in_fixture(
        "fixtures/map/map_probe.beam",
        &[
            // pair/1: {test,has_map_fields,{f,17},{x,0},{list,[{atom,input},{atom,output}]}},
            // with opcode 157, and output as atom 4 and input as atom 17 of
            // the module.
            &[0x9D, 0x0D, 0x11, 0x03, 0x17, 0x20, 0x0A, 0x11, 0x42],
            // key/2: {test,has_map_fields,{f,20},{x,1},{list,[{x,0}]}}
            &[0x9D, 0x0D, 0x14, 0x13, 0x17, 0x10, 0x03],
        ],
    );
}

#[test]
fn bs_match_probe_has_the_bs_match_string_bs_skip_bits2_and_bs_get_integer2_bytes() {
    assert_in_fixture(
        "fixtures/binary_match/bs_match_probe.beam",
        &[
            // match_string/1: bs_match_string f37 x0 104 (offset 0)
            &[0x84, 0x0D, 0x25, 0x03, 0x08, 0x68, 0x00],
            // match_string_offset/1: bs_match_string f40 x0 104 (offset 0)
            &[0x84, 0x0D, 0x28, 0x03, 0x08, 0x68, 0x00],
            // match_string_bits/1: bs_match_string f43 x0 104 (offset 0)
            &[0x84, 0x0D, 0x2B, 0x03, 0x08, 0x68, 0x00],
            // skip_size/2: bs_skip_bits2 f46 {tr,x0,3} x1 8 0
            &[0x78, 0x0D, 0x2E, 0x57, 0x03, 0x30, 0x13, 0x80, 0x00],
            // skip_size_bits/2: bs_skip_bits2 f49 {tr,x0,3} x1 1 0
            &[0x78, 0x0D, 0x31, 0x57, 0x03, 0x30, 0x13, 0x10, 0x00],
            // get_size/2: bs_get_integer2 f52 2 {tr,x0,3} x1 1 0 x1
            &[
                0x75, 0x0D, 0x34, 0x57, 0x03, 0x30, 0x20, 0x13, 0x10, 0x00, 0x13,
            ],
            // get_size_flagged/2: bs_get_integer2 f55 2 {tr,x0,3} x1 1
            // {literal,[little,signed]} x1
            &[
                0x75, 0x0D, 0x37, 0x57, 0x03, 0x30, 0x20, 0x13, 0x10, 0x60, 0x13,
            ],
        ],
    );
}

#[test]
fn bs_utf_probe_has_the_bs_skip_utf8_bytes() {
    assert_in_fixture(
        "fixtures/binary_match/bs_utf_probe.beam",
        &[
            // skip8/1: bs_skip_utf8 f47 {tr,x0,1} 1 0
            &[0x8B, 0x0D, 0x2F, 0x57, 0x03, 0x10, 0x10, 0x00],
        ],
    );
}

#[test]
fn bs_create_bin_utf_probe_has_the_utf16_and_utf32_segment_bytes() {
    assert_in_fixture(
        "fixtures/bs_create_bin/bs_create_bin_utf_probe.beam",
        &[
            // utf16/1: bs_create_bin f0 0 1 16 x0 {list,[{atom,utf16},1,0,
            // nil,{x,0},{atom,undefined}]}
            &[
                0xB1, 0x05, 0x00, 0x10, 0x08, 0x10, 0x03, 0x17, 0x60, 0x22, 0x10, 0x00, 0x02, 0x03,
                0x32,
            ],
            // utf16_little/1: the flags are the literal list [little]
            &[
                0xB1, 0x05, 0x00, 0x10, 0x08, 0x10, 0x03, 0x17, 0x60, 0x22, 0x10, 0x00, 0x47, 0x00,
                0x03, 0x32,
            ],
            // utf32/1: the atom utf32 with the unit 0 and the size undefined
            &[
                0xB1, 0x05, 0x00, 0x10, 0x08, 0x20, 0x03, 0x17, 0x60, 0x62, 0x10, 0x00, 0x02, 0x03,
                0x32,
            ],
        ],
    );
}

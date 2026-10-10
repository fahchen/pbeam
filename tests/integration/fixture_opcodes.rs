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
fn compare_probe_has_the_is_lt_is_eq_is_integer_and_is_boolean_bytes() {
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

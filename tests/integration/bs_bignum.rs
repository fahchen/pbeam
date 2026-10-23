//! Integer segments of more than 64 bits that hold a bignum: `bs_create_bin`
//! writes them (`fmt_big_be` and `fmt_big_le`, erl_bits.c:989-1015 and
//! 1119-1145), and `bs_match` and `bs_get_integer2` read them
//! (`erts_bs_get_integer_2`, erl_bits.c:280-356).

use std::{process::Command, sync::Arc};

use num_bigint::BigInt;
use pbeam_runtime::{BeamProgram, Term};

const BS_BIGNUM_PROBE: &[u8] = include_bytes!("../../fixtures/binary_match/bs_bignum_probe.beam");

/// Return the bignum `sign * 2^shift + add`, which is outside the 64-bit
/// range.
fn power(sign: i64, shift: u32, add: i64) -> Term {
    Term::BigInt(Arc::new(
        BigInt::from(sign) * (BigInt::from(1) << shift) + add,
    ))
}

/// Render an integer or a binary as `io:format("~w")` does.
fn render(term: &Term) -> String {
    match term {
        Term::Integer(value) => value.to_string(),
        Term::BigInt(value) => value.to_string(),
        Term::Binary(binary) => {
            let bytes: Vec<String> = binary.bytes().unwrap().iter().map(u8::to_string).collect();
            format!("<<{}>>", bytes.join(","))
        }
        other => panic!("render has no form for {other:?}"),
    }
}

/// Run each `bs_bignum_probe` call on PBeam and on the pinned ERTS, and
/// return both `~w` texts.
fn run_both(cases: &[(&str, Vec<Term>)]) -> (String, String) {
    let program = BeamProgram::load(&[BS_BIGNUM_PROBE]).unwrap();
    let results: Vec<String> = cases
        .iter()
        .map(|(function, arguments)| {
            let arguments: Vec<_> = arguments
                .iter()
                .cloned()
                .map(|term| program.host_term(term))
                .collect();
            let result = program
                .execute("bs_bignum_probe", function, &arguments)
                .unwrap();
            render(&result.term)
        })
        .collect();
    let calls: Vec<String> = cases
        .iter()
        .map(|(function, arguments)| {
            let arguments: Vec<String> = arguments.iter().map(render).collect();
            format!("bs_bignum_probe:{function}({})", arguments.join(","))
        })
        .collect();
    let erts = Command::new("erl")
        .args([
            "-noshell",
            "-pa",
            "fixtures/binary_match",
            "-eval",
            &format!("io:format(\"~w~n\", [[{}]]), halt().", calls.join(",")),
        ])
        .output()
        .unwrap();
    assert!(erts.status.success());
    assert!(erts.stderr.is_empty());
    (
        format!("[{}]\n", results.join(",")),
        String::from_utf8(erts.stdout).unwrap(),
    )
}

#[test]
fn writes_bignum_integer_segments_as_pinned_erts() {
    // Big and little byte order, a signed segment, sizes of 0, 70, 72, 75,
    // 100, and 128 bits, a segment that does not start on a byte boundary,
    // negative values, and values that do not fit and keep their low bits.
    let cases = [
        ("put128", vec![power(1, 127, 5)]),
        ("put128", vec![power(1, 130, 7)]),
        ("put128", vec![power(-1, 70, 0)]),
        ("put100", vec![power(-1, 100, 3)]),
        ("put100", vec![power(1, 99, 0)]),
        ("put75", vec![power(1, 74, 1)]),
        ("put_sized", vec![power(1, 70, 1), Term::Integer(0)]),
        ("put_sized", vec![power(1, 70, 1), Term::Integer(72)]),
        ("put_little", vec![power(-1, 65, 0), Term::Integer(72)]),
        ("put_odd", vec![Term::Integer(5), power(1, 69, 9)]),
    ];
    let expected = "[<<128,0,0,0,0,0,0,0,0,0,0,0,0,0,0,5>>,<<0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,7>>,\
        <<255,255,255,255,255,255,255,192,0,0,0,0,0,0,0,0>>,<<3,0,0,0,0,0,0,0,0,0,0,0,0>>,\
        <<0,0,0,0,0,0,0,0,0,0,0,0,128>>,<<128,0,0,0,0,0,0,0,0,32>>,<<>>,\
        <<64,0,0,0,0,0,0,0,1>>,<<0,0,0,0,0,0,0,0,254>>,<<161,32,0,0,0,0,0,0,16,0>>]\n";
    let (pbeam, erts) = run_both(&cases);
    assert_eq!(pbeam, expected);
    assert_eq!(erts, expected);
}

#[test]
fn reads_bignum_integer_segments_as_pinned_erts() {
    // `get128`, `get100`, and `get_odd` read with `bs_match` commands;
    // `get_sized` and `get_little` read with `bs_get_integer2`. A segment
    // of more than 64 bits whose value fits 64 bits gives a small integer,
    // and a segment of 0 bits gives 0.
    let mut put128 = vec![128];
    put128.extend([0; 14]);
    put128.push(5);
    let mut put100 = vec![0; 12];
    put100.push(0x80);
    let mut negative = vec![255; 3];
    negative.push(240);
    negative.extend([0; 12]);
    let mut wide = vec![255];
    wide.extend([0; 8]);
    let cases = [
        ("get128", vec![Term::binary(put128)]),
        ("get100", vec![Term::binary(put100)]),
        (
            "get_odd",
            vec![Term::binary(vec![161, 32, 0, 0, 0, 0, 0, 0, 16, 0])],
        ),
        (
            "get_sized",
            vec![Term::binary(negative), Term::Integer(128)],
        ),
        ("get_sized", vec![Term::binary(wide), Term::Integer(65)]),
        (
            "get_sized",
            vec![Term::binary(vec![1, 2, 3]), Term::Integer(0)],
        ),
        (
            "get_little",
            vec![
                Term::binary(vec![0, 0, 0, 0, 0, 0, 0, 0, 254]),
                Term::Integer(72),
            ],
        ),
        (
            "get_little",
            vec![Term::binary((1..=13).collect()), Term::Integer(100)],
        ),
    ];
    let expected = "[170141183460469231731687303715884105733,-633825300114114700748351602688,\
        -590295810358705651703,-1267650600228229401496703205376,-144115188075855872,0,\
        4685472994722226110464,3727165692135864801209549313]\n";
    let (pbeam, erts) = run_both(&cases);
    assert_eq!(pbeam, expected);
    assert_eq!(erts, expected);
}

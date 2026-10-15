use std::{process::Command, sync::Arc};

use num_bigint::BigInt;
use pbeam_runtime::{BeamProgram, Term};

const BIGNUM_PROBE: &[u8] = include_bytes!("../../fixtures/bignum/bignum_probe.beam");

/// Run `bignum_probe:Function()` on the pinned ERTS and return its `~w`
/// text without the newline.
fn reference(function: &str) -> String {
    let output = Command::new("erl")
        .args([
            "-noshell",
            "-pa",
            "fixtures/bignum",
            "-eval",
            &format!("io:format(\"~w~n\", [bignum_probe:{function}()]), halt()."),
        ])
        .output()
        .unwrap();
    assert!(output.status.success(), "{output:?}");
    assert!(output.stderr.is_empty(), "{output:?}");
    String::from_utf8(output.stdout)
        .unwrap()
        .trim_end()
        .to_owned()
}

/// Read one term of `~w` text: an integer, a float, an atom, a list, a
/// tuple, or a binary. An integer outside the i64 range is a
/// `Term::BigInt`, the canonical form.
fn parse(text: &mut &str, program: &BeamProgram) -> Term {
    let elements = |text: &mut &str, close: &str| {
        let mut elements = Vec::new();
        while !text.starts_with(close) {
            elements.push(parse(text, program));
            *text = text.strip_prefix(',').unwrap_or(text);
        }
        *text = &text[close.len()..];
        elements
    };
    if let Some(rest) = text.strip_prefix('[') {
        *text = rest;
        return Term::list_from(elements(text, "]"));
    }
    if let Some(rest) = text.strip_prefix('{') {
        *text = rest;
        return Term::tuple(elements(text, "}"));
    }
    if let Some(rest) = text.strip_prefix("<<") {
        *text = rest;
        let bytes = elements(text, ">>").into_iter().map(|byte| match byte {
            Term::Integer(byte) => byte as u8,
            other => panic!("not a byte: {other:?}"),
        });
        return Term::binary(bytes.collect());
    }
    let end = text.find([',', ']', '}', '>']).unwrap_or(text.len());
    let (token, rest) = text.split_at(end);
    *text = rest;
    if token.starts_with(|c: char| c.is_ascii_lowercase()) {
        Term::Atom(program.atom(token).unwrap())
    } else if token.contains('.') {
        Term::Float(token.parse().unwrap())
    } else if let Ok(value) = token.parse::<i64>() {
        Term::Integer(value)
    } else {
        Term::BigInt(Arc::new(token.parse::<BigInt>().unwrap()))
    }
}

/// Assert that `bignum_probe:Function()` gives the same term on PBeam as on
/// the pinned ERTS, and return the ERTS text.
fn assert_matches_erts(function: &str) -> String {
    let program = BeamProgram::load(&[BIGNUM_PROBE]).unwrap();
    let erts = reference(function);
    let expected = parse(&mut erts.as_str(), &program);
    let actual = program.execute("bignum_probe", function, &[]).unwrap().term;
    assert_eq!(
        actual, expected,
        "bignum_probe:{function}() on ERTS: {erts}"
    );
    erts
}

#[test]
fn loads_bignum_literals_as_pinned_erts() {
    // The literal table holds `SMALL_BIG_EXT` and `LARGE_BIG_EXT` terms
    // (`external.c:4457-4497`), and the code holds integer operands of 9 or
    // more bytes (`marshal_integer`, `beam_file.c:2134-2215`).
    assert_eq!(
        assert_matches_erts("literals"),
        "[[18446744073709551616,-18446744073709551616,11400714819323198485,\
         -1606938044258990275541962092341162602522202993782792835301376],true,\
         18446744073709551621,-340282366920938463463374607431768211461,big]"
    );
}

#[test]
fn runs_bignum_arithmetic_as_pinned_erts() {
    // `erts_mixed_plus`, `erts_mixed_minus`, `erts_mixed_times`, and
    // `erts_int_div_rem` (`erl_arith.c:358-913`, `1172-1250`). A result in
    // the 64-bit range is a small integer term again, so `A - A =:= 0`.
    let erts = assert_matches_erts("arith");
    assert!(erts.starts_with("[18446744073709551617,"), "{erts}");
}

#[test]
fn compares_bignums_with_integers_and_floats_as_pinned_erts() {
    // `erts_cmp_compound` compares a bignum and a float by value
    // (`utils.c:2392-2430`), and the exact order puts each integer before
    // each float. `lists:keyfind/3` compares a bignum key with `==`
    // (`erl_bif_lists.c:1357-1374`).
    let erts = assert_matches_erts("compare");
    assert!(erts.starts_with("[true,false,true,true,"), "{erts}");
}

#[test]
fn converts_bignums_to_floats_as_pinned_erts() {
    // `big_to_double` (`big.c:2411-2529`) rounds to the nearest float, ties
    // to even, and fails above the float range: `float/1` raises `badarg`
    // and arithmetic raises `badarith`.
    let erts = assert_matches_erts("floats");
    assert!(erts.contains("badarg,badarith,badarith"), "{erts}");
}

#[test]
fn writes_the_text_of_bignums_as_pinned_erts() {
    assert_matches_erts("text");
}

#[test]
fn runs_bitwise_operations_and_shifts_of_bignums_as_pinned_erts() {
    // `erts_band`, `erts_bor`, `erts_bxor`, `bnot_1`, and `erts_shift`
    // (`erl_arith.c:164-349`, `1252-1359`).
    let erts = assert_matches_erts("bits");
    assert!(
        erts.contains("system_limit,system_limit,badarith"),
        "{erts}"
    );
}

#[test]
fn raises_system_limit_above_the_largest_bignum_as_pinned_erts() {
    // `BIG_ARITY_MAX` is 2^16 - 1 digits of 64 bits (`erl_term.h:465-466`).
    assert_eq!(
        assert_matches_erts("limits"),
        "[1,system_limit,system_limit,1,system_limit,system_limit,system_limit,system_limit]"
    );
}

#[test]
fn runs_the_rand_seed_arithmetic_as_pinned_erts() {
    // `splitmix64_next/1` and `exsss_next/1` of `rand.erl` use `+`, `*`,
    // `band`, `bor`, `bxor`, `bsl`, and `bsr` on bignums.
    assert_eq!(
        assert_matches_erts("splitmix"),
        "[9639867660769881563,11400714819323203196,5633216745478463924,\
         4354685564936850065,16490336266968443936,{73373040312303740,242017350429183843}]"
    );
}

#[test]
fn charges_bignum_operations_as_pinned_erts() {
    // `bump_reds_linear` and `bump_reds_quadratic` (`erl_arith.c:56-86`):
    // each value is the reductions of a fun call that runs one operation.
    assert_eq!(
        assert_matches_erts("costs"),
        "[3,4,3,4,4,5,4,4,11,3,5,3,4,4,4,4,4,4,3,4,4,4,4,3,6]"
    );
}

#[test]
fn writes_binary_segments_of_bignums_as_pinned_erts() {
    assert_eq!(
        assert_matches_erts("segments"),
        "[<<0,0,0,0,0,0,0,5>>,<<255>>,<<2,1>>]"
    );
}

#[test]
fn reports_a_wide_binary_segment_of_a_bignum() {
    // ERTS writes the low 72 bits of the two's complement value
    // (`erl_bits.c`). PBeam writes a bignum segment of at most 64 bits and
    // returns a runtime error for a wider one (row "Integer range" of
    // `docs/erts-differences.md`).
    let program = BeamProgram::load(&[BIGNUM_PROBE]).unwrap();
    let error = program
        .execute("bignum_probe", "wide_segment", &[])
        .unwrap_err();
    assert_eq!(
        error.to_string(),
        "unsupported bs_create_bin integer segment of 72 bits of a bignum"
    );
    assert_eq!(reference("wide_segment"), "<<1,0,0,0,0,0,0,0,1>>");
}

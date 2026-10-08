mod common;

use std::process::Command;

use common::pinned_beam;
use pbeam_runtime::{BeamProgram, Term};

const BINARY_MATCH_PROBE: &[u8] =
    include_bytes!("../fixtures/binary_match/binary_match_probe.beam");

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
        program.execute("binary_match_probe", "kinds", &[]).unwrap(),
        Term::List(vec![other; 11])
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
    let text = |text: &[u8]| {
        Term::List(
            text.iter()
                .map(|&byte| Term::Integer(byte.into()))
                .collect(),
        )
    };
    assert_eq!(
        program.execute("binary_match_probe", "upper", &[]).unwrap(),
        Term::List(vec![text(b"ERROR"), text(b"NOTICE"), text(b"R")])
    );
    assert_eq!(
        reference("binary_match_probe:upper()"),
        b"[[69,82,82,79,82],[78,79,84,73,67,69],[82]]\n"
    );
}

/// Run `binary_match_probe:Function/1` on each input and return the
/// results.
fn run_each(program: &BeamProgram, function: &str, inputs: &[&[u8]]) -> Vec<Term> {
    inputs
        .iter()
        .map(|input| {
            program
                .execute(
                    "binary_match_probe",
                    function,
                    &[Term::Binary(input.to_vec())],
                )
                .unwrap()
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
                &[Term::Binary(vec![195, 169])]
            )
            .unwrap(),
        Term::Tuple(vec![utf8, Term::Integer(233)])
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
fn stops_at_a_bs_match_command_that_is_not_implemented() {
    // `binary_match_probe:byte/1` runs `bs_match` with `ensure_at_least`
    // and then `integer`. ERTS gives the first byte, or `false` when no
    // byte remains. PBeam runs `ensure_at_least` and then stops at
    // `integer` (row "Binary matching").
    let program = BeamProgram::load(&[BINARY_MATCH_PROBE]).unwrap();
    let byte = |input: &[u8]| {
        program.execute(
            "binary_match_probe",
            "byte",
            &[Term::Binary(input.to_vec())],
        )
    };
    assert_eq!(
        byte(&[7, 8]).unwrap_err().to_string(),
        "unsupported bs_match command integer"
    );
    assert_eq!(
        byte(&[]).unwrap(),
        Term::Atom(program.atom("false").unwrap())
    );
    assert_eq!(
        reference("[binary_match_probe:byte(B) || B <- [<<7,8>>,<<>>]]"),
        b"[7,false]\n"
    );
}

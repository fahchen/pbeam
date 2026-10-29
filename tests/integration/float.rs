//! Runs float literals, arithmetic, comparison and text on PBeam and compares them with the pinned
//! ERTS.

#[path = "../common/mod.rs"]
mod common;

use std::process::Command;

use common::pinned_beam;
use pbeam_runtime::{BeamProgram, Term};

const FLOAT_PROBE: &[u8] = include_bytes!("../../fixtures/float/float_probe.beam");

/// The pinned OTP modules that the fixture calls: `io_lib:format/2` runs
/// `io_lib_format`, `io_lib`, and `io_lib_pretty`, which print a float with
/// `float_to_list/2` and `float_to_binary/2` and the `short` option
/// (`io_lib_format.erl:753-755`, `io_lib.erl:674-675`).
const MODULES: [&str; 10] = [
    "erlang",
    "erl_scan",
    "epp",
    "lists",
    "maps",
    "ets",
    "io_lib",
    "io_lib_format",
    "io_lib_pretty",
    "string",
];

/// Load the fixture with the pinned OTP modules that it calls.
fn program() -> BeamProgram {
    let modules: Vec<Vec<u8>> = MODULES.iter().map(|module| pinned_beam(module)).collect();
    let mut beams: Vec<&[u8]> = modules.iter().map(Vec::as_slice).collect();
    beams.push(FLOAT_PROBE);
    BeamProgram::load(&beams).unwrap()
}

/// Return the text of `float_probe:Function()` that `float_probe:show/1`
/// writes on the pinned ERTS.
fn reference(function: &str) -> Vec<u8> {
    let output = Command::new("erl")
        .args([
            "-noshell",
            "-pa",
            "fixtures/float",
            "-eval",
            &format!("io:format(\"~s\", [float_probe:show({function})]), halt()."),
        ])
        .output()
        .unwrap();
    assert!(output.status.success(), "{output:?}");
    assert!(output.stderr.is_empty(), "{output:?}");
    output.stdout
}

/// Return the text of `float_probe:Function()` that `float_probe:show/1`
/// writes on PBeam.
fn show(program: &BeamProgram, function: &str) -> Vec<u8> {
    let name = Term::Atom(program.atom(function).unwrap());
    let result = program
        .execute("float_probe", "show", &[program.host_term(name)])
        .unwrap()
        .term;
    let Term::Binary(text) = &result else {
        panic!("show/1 returns a binary, not {result:?}");
    };
    text.bytes().unwrap().to_vec()
}

/// Check that PBeam prints the result of a fixture function as the pinned
/// ERTS does, and that the text is `expected`.
fn assert_text(program: &BeamProgram, function: &str, expected: &str) {
    let text = show(program, function);
    assert_eq!(String::from_utf8_lossy(&text), expected);
    assert_eq!(String::from_utf8_lossy(&reference(function)), expected);
}

#[test]
fn loads_float_literals_as_pinned_erts() {
    // The literal table holds each float as `NEW_FLOAT_EXT`
    // (`external.c:4651-4673`), also in a tuple and a list.
    let program = program();
    let floats = [
        1.5,
        -0.0,
        0.0,
        0.1,
        1.0e300,
        5.0e-324,
        -2.5e-10,
        123_456_789.0,
    ]
    .map(Term::Float);
    let mut expected: Vec<Term> = floats.into();
    expected.push(Term::tuple(vec![
        Term::Float(2.0),
        Term::list_from([Term::Float(3.5)]),
    ]));
    let literals = program
        .execute("float_probe", "literals", &[])
        .unwrap()
        .term;
    assert_eq!(literals, Term::list_from(expected));
    assert_text(
        &program,
        "literals",
        "[1.5,-0.0,0.0,0.1,1.0e300,5.0e-324,-2.5e-10,123456789.0,{2.0,[3.5]}]",
    );
}

#[test]
fn runs_float_arithmetic_bifs_as_pinned_erts() {
    // `erts_mixed_plus`, `erts_mixed_minus`, `erts_mixed_times`, and
    // `erts_mixed_div` convert an integer to a float and raise badarith for
    // a result that is not finite (`erl_arith.c:487`, `erl_arith.c:1125`).
    // float/1 and abs/1 raise badarg (`erl_bif_guard.c:57-133`).
    let program = program();
    assert_text(
        &program,
        "arith",
        "[3.5,3.25,0.30000000000000004,4.5,6.0,3.5,0.25,-0.0,-2.5,-0.0,0.0,3.0,-0.5,\
         9.007199254740992e15,2.5,0.0,3,badarith,badarith,badarith,badarith,badarith,badarg,badarg,\
         3.5,true,false]",
    );
}

#[test]
fn runs_the_float_register_instructions_as_pinned_erts() {
    // `registers/2` runs `fmove`, `fadd`, `fsub`, `fmul`, `fdiv`, and
    // `fnegate`. A result that is not finite raises badarith
    // (`emit_check_float_error`, jit/arm/instr_float.cpp:33-53).
    let program = program();
    let result = program
        .execute(
            "float_probe",
            "registers",
            &[
                program.host_term(Term::Float(1.5)),
                program.host_term(Term::Float(0.25)),
            ],
        )
        .unwrap()
        .term;
    let expected = [1.75, 1.25, 0.375, 6.0, -1.5, -1.1].map(Term::Float);
    assert_eq!(result, Term::list_from(expected));
    assert_text(&program, "register_errors", "[badarith,badarith,badarith]");
    // Without a catch, the exception ends the root process.
    let huge = program.host_term(Term::Float(1.0e308));
    let error = program
        .execute("float_probe", "registers", &[huge.clone(), huge])
        .unwrap_err()
        .to_string();
    assert!(
        error.contains("root process exited with {badarith,"),
        "{error}"
    );
}

#[test]
fn compares_floats_and_integers_as_pinned_erts() {
    // Arithmetic comparison compares numbers by value, exact comparison
    // tells `1` from `1.0` and `0.0` from `-0.0` (`utils.c:1422-1433`,
    // `utils.c:2131-2137`, `utils.c:2366-2459`). `lists:keyfind/3` compares
    // a small key with a float element as C does (`erl_bif_lists.c:1311-1375`).
    let program = program();
    assert_text(
        &program,
        "compare",
        "[true,false,false,true,true,true,true,true,true,false,false,true,true,true,true,true,\
         false,true,true,false,true,[-3,-0.0,0.0,0,1.0,1,2.5,a],[0.0,1.0],{1.0,a},\
         {9.007199254740992e15,b},{1,c},false,[1.0]]",
    );
}

#[test]
fn keeps_float_map_and_ets_keys_as_pinned_erts() {
    // A float key hashes with `IHASH_TYPE_FLOAT` and its bits
    // (`erl_term_hashing.c:2193-2202`), so the order of the hashmap keys
    // and the ETS objects is the ERTS order.
    let program = program();
    let text = show(&program, "keys");
    assert_eq!(
        String::from_utf8_lossy(&text),
        String::from_utf8_lossy(&reference("keys"))
    );
}

#[test]
fn writes_float_text_as_pinned_erts() {
    // `float_to_list/1,2` and `float_to_binary/1,2` with each option
    // (`bif.c:3378-3494`), the shortest text of Ryu (`ryu/d2s.c`,
    // `ryu/to_chars.h`), `%.*e` (`sys/unix/sys_float.c:55-68`), and the fast
    // fixed text (`erl_sys_common_misc.c:161-273`).
    let program = program();
    let text = show(&program, "to_text");
    assert_eq!(
        String::from_utf8_lossy(&text),
        String::from_utf8_lossy(&reference("to_text"))
    );
    assert_text(
        &program,
        "text_errors",
        "[badarg,badarg,badarg,badarg,badarg,badarg,badarg,255,badarg,255,badarg,badarg,badarg,badarg]",
    );
}

#[test]
fn formats_floats_with_io_lib_as_pinned_erts() {
    // `~p` and `~w` print the shortest text, and `~f`, `~e`, and `~g` round
    // the digits of `float_to_list/1` (`io_lib_format.erl:690-790`).
    let program = program();
    let expected = "#{1.0 => 2.5} 1.5 0.1 [1.0e20,-0.0,0.6666666666666666] {5.0e-324} \
                    3.141590 -2.67 1.23450e+3 6.02e+23 0.500000 1.23e+4 1.00000e-10\n";
    let text = program.execute("float_probe", "format", &[]).unwrap().term;
    let expected_term = Term::list_from(expected.bytes().map(|byte| Term::Integer(byte.into())));
    assert_eq!(text, expected_term);
    let output = Command::new("erl")
        .args([
            "-noshell",
            "-pa",
            "fixtures/float",
            "-eval",
            "io:format(\"~s\", [float_probe:format()]), halt().",
        ])
        .output()
        .unwrap();
    assert!(output.status.success(), "{output:?}");
    assert_eq!(String::from_utf8_lossy(&output.stdout), expected);
}

#[test]
fn matches_large_select_val_choices_exactly_as_pinned_erts() {
    // A `select_val` of more than 32 choices takes the ordered form. ERTS
    // matches each choice exactly (`emu/select_instrs.tab:57-88`), so an
    // integer never matches a float choice, a float never matches an
    // integer choice, and `-0.0` does not match the `+0.0` choice.
    let program = program();
    assert_text(
        &program,
        "select",
        "[[nomatch,nomatch,i5,i0,nomatch,nomatch,nomatch],\
         [nomatch,nomatch,f7,nomatch,f0,nomatch,nomatch]]",
    );
}

//! Runs the `is_float` test and the float register instructions on PBeam and compares them with the
//! pinned ERTS.

#[path = "../common/mod.rs"]
mod common;

use std::process::Command;

use common::pinned_beam;
use pbeam_runtime::{BeamProgram, Term};

const FLOAT_TEST_PROBE: &[u8] = include_bytes!("../../fixtures/float_test/float_test_probe.beam");

#[test]
fn runs_the_is_float_test_as_pinned_erts() {
    // `float_test_probe:kinds/0` runs `is_float` on one term of each kind
    // that PBeam has. 2^60 is an ERTS bignum, so the JIT compares the
    // header of a boxed term that is not a float
    // (`jit/arm/instr_common.cpp:1229-1245`). `lists:concat/1` runs
    // `is_float` on each element in `lists:thing_to_list/1`
    // (`lists.erl:1283-1286`), as `logger_olp:reset_restart_flag/1` does.
    let erlang = pinned_beam("erlang");
    let lists = pinned_beam("lists");
    let program = BeamProgram::load(&[&erlang, &lists, FLOAT_TEST_PROBE]).unwrap();
    let other = Term::Atom(program.atom("other").unwrap());
    let restarting = Term::list_from(
        b"logger_std_h_default_restarting"
            .iter()
            .map(|&byte| Term::Integer(byte.into())),
    );
    let expected = Term::list_from(std::iter::repeat_n(other, 12).chain([restarting]));
    assert_eq!(
        program
            .execute("float_test_probe", "kinds", &[])
            .unwrap()
            .term,
        expected
    );
    let reference = Command::new("erl")
        .args([
            "-noshell",
            "-pa",
            "fixtures/float_test",
            "-eval",
            "io:format(\"~w~n\", [float_test_probe:kinds()]), halt().",
        ])
        .output()
        .unwrap();
    assert!(reference.status.success());
    assert_eq!(reference.stderr, [] as [u8; 0]);
    assert_eq!(
        reference.stdout,
        b"[other,other,other,other,other,other,other,other,other,other,other,other,\
          [108,111,103,103,101,114,95,115,116,100,95,104,95,100,101,102,97,117,108,116,95,\
          114,101,115,116,97,114,116,105,110,103]]\n"
    );
}

#[test]
fn runs_the_float_register_instructions_of_a_division() {
    // `float_test_probe:half/1` divides with `fconv`, `fmove`, and `fdiv`,
    // as `string` and `erl_scan` do. ERTS loads these instructions with
    // their float registers (emu ops.tab:1329-1343; JIT
    // jit/arm/ops.tab:1118-1130), and `half(8)` is `4.0`.
    let erlang = pinned_beam("erlang");
    let lists = pinned_beam("lists");
    let string = pinned_beam("string");
    let erl_scan = pinned_beam("erl_scan");
    let program =
        BeamProgram::load(&[&erlang, &lists, &string, &erl_scan, FLOAT_TEST_PROBE]).unwrap();
    assert_eq!(
        program
            .execute(
                "float_test_probe",
                "half",
                &[program.host_term(Term::Integer(8))],
            )
            .unwrap()
            .term,
        Term::Float(4.0)
    );
    let reference = Command::new("erl")
        .args([
            "-noshell",
            "-pa",
            "fixtures/float_test",
            "-eval",
            "io:format(\"~w~n\", [float_test_probe:half(8)]), halt().",
        ])
        .output()
        .unwrap();
    assert!(reference.status.success());
    assert_eq!(reference.stderr, [] as [u8; 0]);
    assert_eq!(reference.stdout, b"4.0\n");
}

#[test]
fn raises_badarith_when_fconv_gets_a_term_that_is_not_a_number() {
    // `float_test_probe:halves/0` runs `half/1` on one term of each kind
    // that PBeam has and that is not a number. ERTS `fconv` raises
    // `error:badarith` for each one (emu float_instrs.tab:38-52; JIT
    // `emit_fconv_shared` in jit/arm/instr_float.cpp:96-135).
    let erlang = pinned_beam("erlang");
    let program = BeamProgram::load(&[&erlang, FLOAT_TEST_PROBE]).unwrap();
    let badarith = Term::Atom(program.atom("badarith").unwrap());
    assert_eq!(
        program
            .execute("float_test_probe", "halves", &[])
            .unwrap()
            .term,
        Term::list_from(vec![badarith.clone(); 10])
    );
    // Without a catch, the exception ends the root process. PBeam gives each
    // `error` exception the stack trace `[]` (row "Exception stack trace" in
    // `docs/erts-differences.md`).
    let error = program
        .execute("float_test_probe", "half", &[program.host_term(badarith)])
        .unwrap_err()
        .to_string();
    assert!(
        error.contains("root process exited with {badarith,[]}"),
        "{error}"
    );
    let reference = Command::new("erl")
        .args([
            "-noshell",
            "-pa",
            "fixtures/float_test",
            "-eval",
            "io:format(\"~w~n\", [float_test_probe:halves()]), halt().",
        ])
        .output()
        .unwrap();
    assert!(reference.status.success());
    assert_eq!(reference.stderr, [] as [u8; 0]);
    assert_eq!(
        reference.stdout,
        b"[badarith,badarith,badarith,badarith,badarith,badarith,badarith,badarith,badarith,\
          badarith]\n"
    );
}

#[test]
fn raises_badarith_at_the_fconv_after_a_float_register_instruction() {
    // ERTS runs `fconv` of the integer 8 and `fmove` of the float literal
    // 2.0 to a float register, then raises `error:badarith` at the `fconv`
    // of `a` (emu float_instrs.tab:38-52; JIT `emit_fconv_shared` in
    // jit/arm/instr_float.cpp:96-135).
    let erlang = pinned_beam("erlang");
    let program = BeamProgram::load(&[&erlang, FLOAT_TEST_PROBE]).unwrap();
    let a = Term::Atom(program.atom("a").unwrap());
    assert_eq!(
        program
            .execute(
                "float_test_probe",
                "divide",
                &[
                    program.host_term(a.clone()),
                    program.host_term(Term::Integer(8)),
                ],
            )
            .unwrap()
            .term,
        Term::Atom(program.atom("badarith").unwrap())
    );
    assert_eq!(
        program
            .execute(
                "float_test_probe",
                "divide",
                &[
                    program.host_term(Term::Integer(8)),
                    program.host_term(a.clone()),
                ],
            )
            .unwrap()
            .term,
        Term::Atom(program.atom("badarith").unwrap())
    );
    assert_eq!(
        program
            .execute("float_test_probe", "two_over", &[program.host_term(a)])
            .unwrap()
            .term,
        Term::Atom(program.atom("badarith").unwrap())
    );
    let reference = Command::new("erl")
        .args([
            "-noshell",
            "-pa",
            "fixtures/float_test",
            "-eval",
            "io:format(\"~w~n\", [[float_test_probe:divide(a, 8), \
             float_test_probe:divide(8, a), float_test_probe:two_over(a)]]), halt().",
        ])
        .output()
        .unwrap();
    assert!(reference.status.success());
    assert_eq!(reference.stderr, [] as [u8; 0]);
    assert_eq!(reference.stdout, b"[badarith,badarith,badarith]\n");
}

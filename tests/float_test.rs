mod common;

use std::process::Command;

use common::pinned_beam;
use pbeam_runtime::{BeamProgram, Term};

const FLOAT_TEST_PROBE: &[u8] = include_bytes!("../fixtures/float_test/float_test_probe.beam");

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
    let restarting = Term::List(
        b"logger_std_h_default_restarting"
            .iter()
            .map(|&byte| Term::Integer(byte.into()))
            .collect(),
    );
    let expected = Term::List(std::iter::repeat_n(other, 12).chain([restarting]).collect());
    assert_eq!(
        program.execute("float_test_probe", "kinds", &[]).unwrap(),
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
    assert!(reference.stderr.is_empty());
    assert_eq!(
        reference.stdout,
        b"[other,other,other,other,other,other,other,other,other,other,other,other,\
          [108,111,103,103,101,114,95,115,116,100,95,104,95,100,101,102,97,117,108,116,95,\
          114,101,115,116,97,114,116,105,110,103]]\n"
    );
}

#[test]
fn loads_the_float_register_instructions_and_stops_when_one_runs() {
    // `float_test_probe:half/1` divides with `fconv`, `fmove`, and `fdiv`,
    // as `string` and `erl_scan` do. ERTS loads these instructions with
    // their float registers (emu ops.tab:1329-1343; JIT
    // jit/arm/ops.tab:1118-1130). PBeam has no float term, so the run stops
    // at the first one, `fconv` (opcode 97).
    let erlang = pinned_beam("erlang");
    let lists = pinned_beam("lists");
    let string = pinned_beam("string");
    let erl_scan = pinned_beam("erl_scan");
    let program =
        BeamProgram::load(&[&erlang, &lists, &string, &erl_scan, FLOAT_TEST_PROBE]).unwrap();
    assert_eq!(
        program
            .execute("float_test_probe", "half", &[Term::Integer(8)])
            .unwrap_err()
            .to_string(),
        "unsupported BEAM opcode 97"
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
    assert!(reference.stderr.is_empty());
    assert_eq!(reference.stdout, b"4.0\n");
}

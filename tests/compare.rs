use std::process::Command;

use pbeam_runtime::{BeamProgram, Term};

const COMPARE_PROBE: &[u8] = include_bytes!("../fixtures/compare/compare_probe.beam");

/// Run `compare_probe:Function()` on the pinned ERTS and return its `~w` output.
fn reference(function: &str) -> Vec<u8> {
    let reference = Command::new("erl")
        .args([
            "-noshell",
            "-pa",
            "fixtures/compare",
            "-eval",
            &format!("io:format(\"~w~n\", [compare_probe:{function}()]), halt()."),
        ])
        .output()
        .unwrap();
    assert!(reference.status.success());
    assert!(reference.stderr.is_empty());
    reference.stdout
}

#[test]
fn compares_terms_exactly_as_pinned_erts() {
    // `compare_probe:exact/0` runs the `bif` forms of `erlang:'=:='/2` and
    // `erlang:'=/='/2` with fail label 0, as `user_drv:init/1` does. Both
    // BIFs use `eq()` (`erl_bif_op.c:113-126`).
    let program = BeamProgram::load(&[COMPARE_PROBE]).unwrap();
    let exact = [
        true, false, false, true, false, true, false, true, false, true, false, true, false, true,
        true, false, true, false,
    ];
    let atoms = |negate: bool| {
        Term::List(
            exact
                .iter()
                .map(|&equal| {
                    Term::Atom(
                        program
                            .atom(if equal != negate { "true" } else { "false" })
                            .unwrap(),
                    )
                })
                .collect(),
        )
    };
    assert_eq!(
        program.execute("compare_probe", "exact", &[]).unwrap(),
        Term::List(vec![atoms(false), atoms(true)])
    );
    assert_eq!(
        reference("exact"),
        b"[[true,false,false,true,false,true,false,true,false,true,false,true,false,true,\
          true,false,true,false],[false,true,true,false,true,false,true,false,true,false,\
          true,false,true,false,false,true,false,true]]\n"
    );
}

#[test]
fn runs_the_is_eq_test_as_pinned_erts() {
    // `compare_probe:equal/0` runs the `is_eq` test of `compare_probe:eq/2`,
    // as the guard `CtrlSync==0` of `logger_h_common:handle_load/2` does, on
    // pairs of the same or of a different type. 2^59 is an ERTS bignum.
    let program = BeamProgram::load(&[COMPARE_PROBE]).unwrap();
    let equal = [
        true, false, false, true, false, true, false, true, false, true, false, true, false, true,
        true, false, true, false, true, false,
    ];
    assert_eq!(
        program.execute("compare_probe", "equal", &[]).unwrap(),
        Term::List(
            equal
                .iter()
                .map(|&equal| Term::Atom(program.atom(&equal.to_string()).unwrap()))
                .collect()
        )
    );
    assert_eq!(
        reference("equal"),
        b"[true,false,false,true,false,true,false,true,false,true,false,true,false,true,\
          true,false,true,false,true,false]\n"
    );
}

#[test]
fn runs_the_is_lt_is_integer_and_is_boolean_tests_as_pinned_erts() {
    // `compare_probe:guards/0` runs `is_lt` after a `gc_bif2` guard, as
    // `io_lib_format:build_limited_bin/5` does, on the atom `none` and 0, as
    // `io_lib_format:field_width/3` does, and on two registers in the ERTS
    // term order. It runs `is_integer` and `is_boolean`, as
    // `io_lib_format:field_value/2` and `standard_error:check_valid_opts/1`
    // do. 2^59 is an ERTS bignum and passes `is_integer`.
    let program = BeamProgram::load(&[COMPARE_PROBE]).unwrap();
    let atom = |name: &str| Term::Atom(program.atom(name).unwrap());
    let atoms = |names: &[&str]| Term::List(names.iter().map(|&name| atom(name)).collect());
    let pair = |tag, value| Term::Tuple(vec![atom(tag), Term::Integer(value)]);
    let expected = Term::List(vec![
        Term::List(vec![
            pair("pos", 2),
            pair("nonpos", 0),
            atom("other"),
            atom("other"),
        ]),
        atoms(&[
            "integer", "integer", "integer", "boolean", "boolean", "other", "other", "other",
            "other", "other", "other",
        ]),
        atoms(&["right", "left", "right", "right"]),
        atoms(&[
            "lt", "nlt", "nlt", "lt", "lt", "nlt", "lt", "nlt", "lt", "nlt", "lt", "nlt", "lt",
            "lt", "lt", "nlt",
        ]),
    ]);
    assert_eq!(
        program.execute("compare_probe", "guards", &[]).unwrap(),
        expected
    );
    assert_eq!(
        reference("guards"),
        b"[[{pos,2},{nonpos,0},other,other],[integer,integer,integer,boolean,boolean,\
          other,other,other,other,other,other],[right,left,right,right],[lt,nlt,nlt,lt,lt,\
          nlt,lt,nlt,lt,nlt,lt,nlt,lt,lt,lt,nlt]]\n"
    );
}

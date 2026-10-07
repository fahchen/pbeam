mod common;

use std::process::Command;

use common::pinned_beam;
use pbeam_runtime::{BeamProgram, Term};

const BINARY_MATCH_PROBE: &[u8] =
    include_bytes!("../fixtures/binary_match/binary_match_probe.beam");

/// Run `binary_match_probe:Call` on the pinned ERTS and return its `~w`
/// output.
fn reference(call: &str) -> Vec<u8> {
    let reference = Command::new("erl")
        .args([
            "-noshell",
            "-pa",
            "fixtures/binary_match",
            "-eval",
            &format!("io:format(\"~w~n\", [binary_match_probe:{call}]), halt()."),
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
        reference("kinds()"),
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
        reference("upper()"),
        b"[[69,82,82,79,82],[78,79,84,73,67,69],[82]]\n"
    );
}

#[test]
fn stops_the_run_at_bs_start_match3_of_a_binary() {
    // ERTS makes a match context for a binary (erl_bits.c:81) and
    // `binary_match_probe:first/1` gives `{utf8,233}`. PBeam has no match
    // context, so the run stops at `bs_start_match3`.
    let program = BeamProgram::load(&[BINARY_MATCH_PROBE]).unwrap();
    assert_eq!(
        program
            .execute(
                "binary_match_probe",
                "first",
                &[Term::Binary(vec![195, 169])]
            )
            .unwrap_err()
            .to_string(),
        "unsupported bs_start_match3 of a binary: needs a match context"
    );
    assert_eq!(reference("first(<<195,169>>)"), b"{utf8,233}\n");
}

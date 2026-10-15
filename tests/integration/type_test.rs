use std::process::Command;

use pbeam_runtime::{AtomTable, BeamProgram, Term};

#[path = "../common/mod.rs"]
mod common;

use common::pinned_beam;

const TYPE_TEST_PROBE: &[u8] = include_bytes!("../../fixtures/type_test/type_test_probe.beam");

/// The type-test results of `bif/0`, `funs/0`, and `applies/0`: for each
/// term of `terms/0`, the names of the type tests that give `true`.
const PASSED: &str = "[[is_atom],[is_atom,is_boolean],[is_number],[is_number],\
    [is_float,is_number],[is_binary,is_bitstring],[is_binary,is_bitstring],[is_list],[is_list],\
    [is_list],[is_tuple],[is_tuple],[is_map],[is_map],[is_pid],[is_port],[is_reference],\
    [is_function],[is_function]]";

/// Return a term as `io:format("~w")` prints it, for the atoms, lists, and
/// tuples of the probe results. Each atom name comes from `atoms`, the
/// table of the run that owns the term.
///
/// # Examples
///
/// ```rust,ignore
/// assert_eq!(text(&Term::nil(), &atoms), "[]");
/// ```
fn text(term: &Term, atoms: &AtomTable) -> String {
    let join = |terms: &[Term]| {
        terms
            .iter()
            .map(|term| text(term, atoms))
            .collect::<Vec<_>>()
            .join(",")
    };
    match term {
        Term::Atom(atom) => atoms.name_of(atom).to_owned(),
        _ if let Some(terms) = term.proper_list() => {
            format!("[{}]", join(&terms.cloned().collect::<Vec<_>>()))
        }
        Term::Tuple(terms) => format!("{{{}}}", join(terms)),
        other => panic!("unexpected term {other:?}"),
    }
}

/// Run `type_test_probe:Call` on the pinned ERTS and on PBeam, and check
/// that both give `expected` as `~w` text. PBeam loads the pinned `erlang`
/// module, whose `open_port/2` makes the port of `terms/0`.
///
/// # Examples
///
/// ```rust,ignore
/// assert_runs("guard()", "[[no,no,no]]");
/// ```
fn assert_runs(call: &str, expected: &str) {
    let reference = Command::new("erl")
        .args([
            "-noshell",
            "-pa",
            "fixtures/type_test",
            "-eval",
            &format!("io:format(\"~w~n\", [type_test_probe:{call}]), halt()."),
        ])
        .output()
        .unwrap();
    assert!(
        reference.status.success() && reference.stderr.is_empty(),
        "{reference:?}"
    );
    assert_eq!(
        String::from_utf8(reference.stdout).unwrap(),
        format!("{expected}\n"),
        "ERTS {call}"
    );
    let erlang = pinned_beam("erlang");
    let program = BeamProgram::load(&[TYPE_TEST_PROBE, &erlang]).unwrap();
    let function = call.strip_suffix("()").unwrap();
    let result = program.execute("type_test_probe", function, &[]).unwrap();
    assert_eq!(text(&result.term, result.atoms()), expected, "PBeam {call}");
}

#[test]
fn runs_each_type_test_bif_in_a_body_through_a_fun_and_through_apply_as_pinned_erts() {
    // `bif/0` runs each BIF as a `bif` instruction with the fail label 0,
    // `funs/0` calls `fun erlang:F/1`, and `applies/0` calls `apply/3`
    // (`erl_bif_op.c:133-357`). 2^59 is an ERTS bignum and a number. `[1|2]`
    // is a list for `is_list/1`, which tests the cons tag. No term is a
    // native record, so `erlang:is_record/1` gives `false` for each term.
    for call in ["bif()", "funs()", "applies()"] {
        assert_runs(call, PASSED);
    }
}

#[test]
fn runs_is_function_2_with_a_fail_label_in_a_guard_as_pinned_erts() {
    // A bad arity raises `badarg` (`erl_bif_op.c:244-256`), so the `bif`
    // instruction goes to its fail label. Only the arity-0 fun passes.
    let mut rows = vec!["[no,no,no]"; 19];
    rows[17] = "[yes,no,no]";
    assert_runs("guard()", &format!("[{}]", rows.join(",")));
}

#[test]
fn runs_is_function_2_in_a_body_through_a_fun_and_through_apply_as_pinned_erts() {
    // A negative arity or an arity that is not an integer raises `badarg`
    // (`erl_bif_op.c:244-256`), also for a term that is not a fun.
    assert_runs(
        "arity()",
        "[[true,true,true],[false,false,false],[true,true,true],[false,false,false],\
         [{error,badarg},{error,badarg},{error,badarg}],[{error,badarg},{error,badarg},{error,badarg}],\
         [{error,badarg},{error,badarg},{error,badarg}],[{error,badarg},{error,badarg},{error,badarg}]]",
    );
}

#[test]
fn runs_is_record_2_and_3_in_a_body_through_a_fun_and_through_apply_as_pinned_erts() {
    // A name that is not an atom raises `badarg` (`erl_bif_op.c:297-299`,
    // `328-330`). An atom size selects the native-record test, which gives
    // `false` (`erl_bif_op.c:332-338`). A float size and a size outside the small
    // integer range raise `badarg` (`erl_bif_op.c:347-348`). The compiler emits `erlang:is_record/3` in a
    // body as `call_ext`, which runs the BIF through its export.
    assert_runs(
        "record()",
        "[[true,true,true],[false,false,false],[false,false,false],[false,false,false],\
         [{error,badarg},{error,badarg},{error,badarg}],[true,true,true],[false,false,false],\
         [false,false,false],[{error,badarg},{error,badarg},{error,badarg}],\
         [{error,badarg},{error,badarg},{error,badarg}],[false,false,false],\
         [{error,badarg},{error,badarg},{error,badarg}],[false,false,false],\
         [false,false,false],[{error,badarg},{error,badarg},{error,badarg}]]",
    );
}

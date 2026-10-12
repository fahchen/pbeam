//! The combined instruction pairs of `docs/interpreter-dispatch-plan.md`,
//! step 9. The loader combines a comparison test with the `Bif` at the
//! target of its fail label, and a `Bif` with the `call_only` directly after
//! it. The tests drive each exit of each pair through the compiled fixtures
//! `fixtures/combined/combined_probe.beam` and
//! `fixtures/combined/combined_main.beam`, and compare the result with the
//! pinned ERTS.
//!
//! The fixtures keep the loops in a second module, so that the type analysis
//! of the compiler cannot fold the clause test of a loop away. A loop
//! without its clause test holds no pair.

use std::process::Command;

use pbeam_runtime::{BeamProgram, Term};

const COMBINED_PROBE: &[u8] = include_bytes!("../../fixtures/combined/combined_probe.beam");
const COMBINED_MAIN: &[u8] = include_bytes!("../../fixtures/combined/combined_main.beam");

/// The length of the list of `main/1`. `length/1` runs for `16 *` the
/// remaining reductions and then traps, so a list of 70000 elements traps
/// while the process has a full budget of 4000 reductions.
const TRAPPING_LENGTH: i64 = 70_000;

/// The printed result of `combined_main:main/1` on the pinned ERTS, from
/// `erl +S 1 -noshell -pa fixtures/combined -eval
/// 'io:format("~w~n", [combined_main:main(lists:seq(1, 70000))]), halt().'`.
///
/// The elements are: the value of each clause of `eq/1`, `ne/2`, `lt/1`,
/// and `ge/1` for the pass and the fail branch; the exception class and
/// reason of the raising `gc_bif` of pair 1; the three exits of
/// `guard_fail/1`; the value and the exception of the trapping `length/1` of
/// pair 1, for a short list, the trapping list, and a term that is not a
/// list; the value of pair 2 and its trapping `length/1`; the exception and
/// the value of the raising `gc_bif` of pair 2; the reduction charge of the
/// pairs; the mailbox order across the trap of `length/1`, which is
/// `{70000, first, second}`; and the signal observation of the trap, which
/// is `{70000, 1}`: the child process runs only while the pair is switched
/// out at the trap, so its message is queued when the pair completes.
const ERTS_RESULT: &str = "{0,0,nonzero,4,small,1,big,1,{error,badarith},\
zero,other,other,3,70000,{error,badarg},3,70000,{error,badarith},0,4399,\
{70000,first,second},{70000,1}}\n";

/// Return the result of `combined_main:main/1` as the fixture builds it.
///
/// # Examples
///
/// ```rust,ignore
/// let result = main(&program);
/// assert!(matches!(result, Term::Tuple(_)));
/// ```
fn main(program: &BeamProgram) -> Term {
    let long = Term::list_from((1..=TRAPPING_LENGTH).map(Term::Integer));
    program
        .execute("combined_main", "main", &[long])
        .expect("the fixture runs")
}

/// The exception class and reason of `{error, Reason}`.
///
/// # Examples
///
/// ```rust,ignore
/// assert_eq!(caught(&program, "badarith"), Term::tuple(vec![error, reason]));
/// ```
fn caught(program: &BeamProgram, reason: &str) -> Term {
    Term::tuple(vec![
        Term::Atom(program.atom("error").unwrap()),
        Term::Atom(program.atom(reason).unwrap()),
    ])
}

/// The expected result of `main/1`, as [`ERTS_RESULT`] gives it.
///
/// # Examples
///
/// ```rust,ignore
/// let expected = expected(&program);
/// assert!(matches!(expected, Term::Tuple(elements) if elements.len() == 22));
/// ```
fn expected(program: &BeamProgram) -> Term {
    let integer = Term::Integer;
    let atom = |name: &str| Term::Atom(program.atom(name).unwrap());
    Term::tuple(vec![
        integer(0),
        integer(0),
        atom("nonzero"),
        integer(4),
        atom("small"),
        integer(1),
        atom("big"),
        integer(1),
        caught(program, "badarith"),
        atom("zero"),
        atom("other"),
        atom("other"),
        integer(3),
        integer(TRAPPING_LENGTH),
        caught(program, "badarg"),
        integer(3),
        integer(TRAPPING_LENGTH),
        caught(program, "badarith"),
        integer(0),
        integer(4399),
        Term::tuple(vec![
            integer(TRAPPING_LENGTH),
            atom("first"),
            atom("second"),
        ]),
        Term::tuple(vec![integer(TRAPPING_LENGTH), integer(1)]),
    ])
}

/// Drive each exit of each combined pair and compare the result with the
/// pinned ERTS. The exits are: the pass and the fail branch of the test, the
/// value of the BIF, the raise of a BIF with fail label 0, the jump of a BIF
/// with another fail label, the trap of `length/1` and its completion, the
/// local call of the second pair, the reduction charge of the pairs, the
/// message order across the trap, and the signal observation of the trap.
#[test]
fn keeps_every_exit_of_the_combined_pairs_as_pinned_erts() {
    let program = BeamProgram::load(&[COMBINED_PROBE, COMBINED_MAIN]).unwrap();
    assert_eq!(main(&program), expected(&program));

    let output = Command::new("erl")
        .args([
            "+S",
            "1",
            "-noshell",
            "-pa",
            "fixtures/combined",
            "-eval",
            "io:format(\"~w~n\", [combined_main:main(lists:seq(1, 70000))]), halt().",
        ])
        .output()
        .unwrap();
    assert!(
        output.status.success() && output.stderr.is_empty(),
        "{output:?}"
    );
    assert_eq!(String::from_utf8(output.stdout).unwrap(), ERTS_RESULT);
}

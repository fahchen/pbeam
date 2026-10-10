use std::process::Command;

#[path = "../common/mod.rs"]
mod common;

use common::pinned_beam;
use pbeam_runtime::{BeamProgram, Term};

const EXCEPTION_PROBE: &[u8] = include_bytes!("../../fixtures/exception/exception_probe.beam");

fn atom(program: &BeamProgram, name: &str) -> Term {
    Term::Atom(program.atom(name).unwrap())
}

fn pair(first: Term, second: Term) -> Term {
    Term::Tuple(vec![first, second])
}

/// Run an expression under the pinned ERTS with the exception fixture on the
/// code path, and return its stdout. ERTS must exit normally with no stderr.
fn erl_reference(expression: &str) -> Vec<u8> {
    let reference = Command::new("erl")
        .args([
            "-noshell",
            "-pa",
            "fixtures/exception",
            "-eval",
            &format!("{expression}, halt()."),
        ])
        .output()
        .unwrap();
    assert!(reference.status.success());
    assert!(reference.stderr.is_empty());
    reference.stdout
}

#[test]
fn catches_error_and_exit_exceptions_with_try_and_catch() {
    let program = BeamProgram::load(&[EXCEPTION_PROBE]).unwrap();
    let atom = |name: &str| atom(&program, name);
    assert_eq!(
        program.execute("exception_probe", "main", &[]).unwrap(),
        Term::Tuple(vec![
            pair(atom("value"), atom("ok")),
            atom("ok"),
            pair(atom("exit"), atom("undef")),
            pair(atom("error"), atom("undef")),
            pair(atom("EXIT"), atom("bye")),
            pair(atom("exit"), atom("bye")),
            atom("true"),
            pair(atom("inner"), atom("a")),
            pair(atom("inner"), atom("b")),
            pair(atom("EXIT"), atom("b")),
            pair(atom("outer_caught"), atom("inner")),
        ])
    );
    assert_eq!(
        erl_reference("io:format(\"~w~n\", [exception_probe:main()])"),
        b"{{value,ok},ok,{exit,undef},{error,undef},{'EXIT',bye},{exit,bye},true,{inner,a},{inner,b},{'EXIT',b},{outer_caught,inner}}\n"
    );
}

#[test]
fn adds_the_stack_trace_to_a_caught_error() {
    // ERTS records the frames of `erlang:nif_error/1` and `stub/0` first.
    // Frames below these two depend on the caller, so the check compares
    // only these two.
    assert_eq!(
        erl_reference(
            "{'EXIT', {R, S}} = exception_probe:catch_stub(), \
             io:format(\"~p ~p~n\", [R, lists:sublist([{M, F, A} || {M, F, A, _} <- S], 2)])"
        ),
        b"undef [{erlang,nif_error,[undef]},{exception_probe,stub,0}]\n"
    );

    // PBeam records no stack frames, so the stack trace is empty. This is
    // the "Exception stack trace" row in `docs/erts-differences.md`.
    let program = BeamProgram::load(&[EXCEPTION_PROBE]).unwrap();
    let atom = |name: &str| atom(&program, name);
    assert_eq!(
        program
            .execute("exception_probe", "catch_stub", &[])
            .unwrap(),
        pair(atom("EXIT"), pair(atom("undef"), Term::List(Vec::new())))
    );
}

#[test]
fn raises_the_first_argument_of_error_3_as_an_error() {
    // ERTS puts the arguments and the `error_info` map of `erlang:error/3`
    // only into the stack trace. `erlang:group_leader/2` in `erlang.beam`
    // calls `erlang:error/3` when `erts_internal:group_leader/2` returns
    // `badarg`.
    assert_eq!(
        erl_reference(
            "io:format(\"~p~n~p~n\", \
             [exception_probe:error3(), exception_probe:group_leader_badarg()])"
        ),
        b"[{error,badarg},{error,{my,reason}},{error,x},{error,y},{error,z}]\n{error,badarg}\n"
    );

    let erlang = pinned_beam("erlang");
    let program = BeamProgram::load(&[&erlang, EXCEPTION_PROBE]).unwrap();
    let atom = |name: &str| atom(&program, name);
    let error = |reason| pair(atom("error"), reason);
    assert_eq!(
        program.execute("exception_probe", "error3", &[]).unwrap(),
        Term::List(vec![
            error(atom("badarg")),
            error(pair(atom("my"), atom("reason"))),
            error(atom("x")),
            error(atom("y")),
            error(atom("z")),
        ])
    );
    assert_eq!(
        program
            .execute("exception_probe", "group_leader_badarg", &[])
            .unwrap(),
        error(atom("badarg"))
    );
}

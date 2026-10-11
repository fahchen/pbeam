use std::process::Command;

use pbeam_runtime::{BeamProgram, Term};

#[path = "../common/mod.rs"]
mod common;

use common::pinned_beam;

const ERROR_PATH_PROBE: &[u8] = include_bytes!("../../fixtures/error_path/error_path_probe.beam");

/// Evaluate `error_path_probe:main()` on the pinned ERTS and return its `~w`
/// output. `-kernel logger_level none` stops the error report of the
/// process that `down/0` starts, as in the row "Undefined functions".
fn reference() -> String {
    let reference = Command::new("erl")
        .args([
            "-noshell",
            "-kernel",
            "logger_level",
            "none",
            "-pa",
            "fixtures/error_path",
            "-eval",
            "io:format(\"~w~n\", [error_path_probe:main()]), halt().",
        ])
        .output()
        .unwrap();
    assert!(
        reference.status.success() && reference.stderr.is_empty(),
        "{reference:?}"
    );
    String::from_utf8(reference.stdout).unwrap()
}

#[test]
fn raises_the_exceptions_of_the_error_path_instructions_as_pinned_erts() {
    // Each case of `main/0` raises an exception in a function that `try`
    // and `catch` call. `badmatch` (with an X and a Y register),
    // `case_end`, `try_case_end`, and `badrecord` raise `{Tag, Value}`, and
    // `if_end` raises `if_clause` (emu `instrs.tab:1137-1162`, `1229-1234`;
    // `beam_common.c:742-773`). `raw_raise` raises with the class in `x0`
    // (`beam_common.c:1148-1179`). `erlang:error/2` raises its first
    // argument (`bif.c:1448-1454`), and `erlang:throw/1` raises class
    // `throw` (`bif.c:4132-4136`). The fixture drops the stack traces,
    // which PBeam does not record (row "Exception stack trace"), except the
    // first frame of `erlang:error/2` without its location. The pinned
    // `erlang` module has `erlang:spawn_monitor/3`.
    let erlang = pinned_beam("erlang");
    let program = BeamProgram::load(&[ERROR_PATH_PROBE, &erlang]).unwrap();
    let atom = |name: &str| Term::Atom(program.atom(name).unwrap());
    let tuple = Term::tuple;
    let tagged = |tag: &str| tuple(vec![atom(tag), atom("b")]);
    let error = |reason: Term| {
        tuple(vec![
            tuple(vec![atom("error"), reason.clone()]),
            tuple(vec![atom("EXIT"), reason]),
        ])
    };
    let r = || atom("r");
    let frame = |function: &str, args: Term| {
        tuple(vec![r(), atom("error_path_probe"), atom(function), args])
    };
    assert_eq!(
        program.execute("error_path_probe", "main", &[]).unwrap(),
        tuple(vec![
            Term::list_from(vec![
                error(tagged("badmatch")),
                error(tagged("badmatch")),
                error(tagged("case_clause")),
                error(atom("if_clause")),
                error(tagged("try_clause")),
                error(tagged("badrecord")),
                // raw_raise with the classes error, exit, and throw.
                error(r()),
                tuple(vec![
                    tuple(vec![atom("exit"), r()]),
                    tuple(vec![atom("EXIT"), r()]),
                ]),
                tuple(vec![tuple(vec![atom("throw"), r()]), r()]),
                // erlang:error(r, [a]) and erlang:error(r, none).
                error(r()),
                error(r()),
                // erlang:throw(r).
                tuple(vec![tuple(vec![atom("throw"), r()]), r()]),
            ]),
            // raw_raise with the class foo writes badarg to x0.
            atom("badarg"),
            // A throw with no catch becomes the error `{nocatch, Reason}`
            // (`beam_common.c:509-516`).
            tuple(vec![atom("nocatch"), r()]),
            // The first frame of erlang:error/2 has the arguments, or the
            // arity of the caller for `none` (`utils.c:3448-3455`).
            Term::list_from(vec![
                frame("error_2_tail", Term::list_from(vec![atom("a")])),
                frame("error_2_tail", Term::Integer(1)),
                frame(
                    "error_2_tail",
                    Term::list_with_tail(vec![atom("a")], atom("b")),
                ),
                frame("error_2_tail", Term::nil()),
                frame("error_2_body", Term::list_from(vec![atom("a")])),
                // A tail call of a fun removes the frame first
                // (`instr_fun.cpp:425-455`), and a tail apply keeps it
                // (`beam_common.c:1345-1431`).
                frame("error_2_fun_caller", Term::list_from(vec![atom("b")])),
                frame("error_2_apply_tail", Term::list_from(vec![atom("c")])),
            ]),
            // With no caller frame, the first frame is the initial call with
            // its arity (`beam_common.c:1245-1248`).
            tuple(vec![
                r(),
                Term::list_from(vec![tuple(vec![
                    atom("error_path_probe"),
                    atom("error_2_fun_tail"),
                    Term::Integer(1),
                    Term::nil(),
                ])]),
            ]),
        ])
    );
    assert_eq!(
        reference(),
        "{[{{error,{badmatch,b}},{'EXIT',{badmatch,b}}},\
         {{error,{badmatch,b}},{'EXIT',{badmatch,b}}},\
         {{error,{case_clause,b}},{'EXIT',{case_clause,b}}},\
         {{error,if_clause},{'EXIT',if_clause}},\
         {{error,{try_clause,b}},{'EXIT',{try_clause,b}}},\
         {{error,{badrecord,b}},{'EXIT',{badrecord,b}}},\
         {{error,r},{'EXIT',r}},{{exit,r},{'EXIT',r}},{{throw,r},r},\
         {{error,r},{'EXIT',r}},{{error,r},{'EXIT',r}},{{throw,r},r}],\
         badarg,{nocatch,r},\
         [{r,error_path_probe,error_2_tail,[a]},\
         {r,error_path_probe,error_2_tail,1},\
         {r,error_path_probe,error_2_tail,[a|b]},\
         {r,error_path_probe,error_2_tail,[]},\
         {r,error_path_probe,error_2_body,[a]},\
         {r,error_path_probe,error_2_fun_caller,[b]},\
         {r,error_path_probe,error_2_apply_tail,[c]}],\
         {r,[{error_path_probe,error_2_fun_tail,1,[]}]}}\n"
    );
}

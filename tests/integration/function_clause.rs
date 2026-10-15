use std::process::Command;

use pbeam_runtime::{BeamProgram, Term};

#[path = "../common/mod.rs"]
mod common;

use common::pinned_beam;

const FUNCTION_CLAUSE_PROBE: &[u8] =
    include_bytes!("../../fixtures/function_clause/function_clause_probe.beam");

/// Evaluate `function_clause_probe:main()` on the pinned ERTS and return
/// its `~w` output.
fn reference() -> String {
    let reference = Command::new("erl")
        .args([
            "-noshell",
            "-pa",
            "fixtures/function_clause",
            "-eval",
            "io:format(\"~w~n\", [function_clause_probe:main()]), halt().",
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
fn raises_function_clause_with_the_arguments_and_the_caller_frames_as_pinned_erts() {
    // `i_func_info` raises `error:function_clause` and sets the current MFA
    // (`jit/arm/beam_asm_module.cpp:484-497`). The first frame holds the
    // argument registers (`beam_common.c:1011-1024`, `1186-1195`), and the
    // next frames are the callers. `lists:nth(5, [a])` fails in
    // `nth_1(4, [])`. The fixture drops the `file` and `line` location of
    // the `lists` frame, because PBeam keeps no line table.
    let erlang = pinned_beam("erlang");
    let lists = pinned_beam("lists");
    let program = BeamProgram::load(&[FUNCTION_CLAUSE_PROBE, &erlang, &lists]).unwrap();
    let atom = |name: &str| Term::Atom(program.atom(name).unwrap());
    let tuple = Term::tuple;
    let list = |elements: Vec<Term>| Term::list_from(elements);
    let nil = || list(Vec::new());
    let frame = |module: &str, function: &str, arguments: Term| {
        tuple(vec![atom(module), atom(function), arguments, nil()])
    };
    let probe =
        |function: &str, arity: i64| frame("function_clause_probe", function, Term::Integer(arity));
    let pick = |n: i64| {
        frame(
            "function_clause_probe",
            "pick",
            list(vec![atom("x"), Term::Integer(n)]),
        )
    };
    let nth = || frame("lists", "nth_1", list(vec![Term::Integer(4), nil()]));
    let stack = |first: Term, catcher: &str| list(vec![first, probe(catcher, 1), probe("main", 0)]);
    let caught = |first: Term| {
        tuple(vec![
            atom("error"),
            atom("function_clause"),
            stack(first, "try_stack"),
        ])
    };
    let exit = |first: Term| {
        tuple(vec![
            atom("EXIT"),
            tuple(vec![atom("function_clause"), stack(first, "catch_stack")]),
        ])
    };
    let body = list(vec![
        pick(3),
        probe("body", 1),
        probe("try_stack", 1),
        probe("main", 0),
    ]);
    assert_eq!(
        program
            .execute("function_clause_probe", "main", &[])
            .unwrap()
            .term,
        tuple(vec![
            list(vec![
                caught(nth()),
                caught(pick(1)),
                caught(pick(2)),
                tuple(vec![atom("error"), atom("function_clause"), body]),
            ]),
            list(vec![exit(nth()), exit(pick(1)), exit(pick(2))]),
            tuple(vec![atom("function_clause"), list(vec![pick(4)])]),
        ])
    );
    assert_eq!(
        reference(),
        "{[{error,function_clause,[{lists,nth_1,[4,[]],[]},\
         {function_clause_probe,try_stack,1,[]},{function_clause_probe,main,0,[]}]},\
         {error,function_clause,[{function_clause_probe,pick,[x,1],[]},\
         {function_clause_probe,try_stack,1,[]},{function_clause_probe,main,0,[]}]},\
         {error,function_clause,[{function_clause_probe,pick,[x,2],[]},\
         {function_clause_probe,try_stack,1,[]},{function_clause_probe,main,0,[]}]},\
         {error,function_clause,[{function_clause_probe,pick,[x,3],[]},\
         {function_clause_probe,body,1,[]},{function_clause_probe,try_stack,1,[]},\
         {function_clause_probe,main,0,[]}]}],\
         [{'EXIT',{function_clause,[{lists,nth_1,[4,[]],[]},\
         {function_clause_probe,catch_stack,1,[]},{function_clause_probe,main,0,[]}]}},\
         {'EXIT',{function_clause,[{function_clause_probe,pick,[x,1],[]},\
         {function_clause_probe,catch_stack,1,[]},{function_clause_probe,main,0,[]}]}},\
         {'EXIT',{function_clause,[{function_clause_probe,pick,[x,2],[]},\
         {function_clause_probe,catch_stack,1,[]},{function_clause_probe,main,0,[]}]}}],\
         {function_clause,[{function_clause_probe,pick,[x,4],[]}]}}\n"
    );
}

#[test]
fn exits_the_root_process_with_function_clause_and_its_stack_trace() {
    // An uncaught error exits the process with `{Reason, Stacktrace}`
    // (`beam_common.c:684-692`).
    let program = BeamProgram::load(&[FUNCTION_CLAUSE_PROBE]).unwrap();
    let error = program
        .execute("function_clause_probe", "uncaught", &[])
        .unwrap_err();
    assert_eq!(
        error.to_string(),
        "root process exited with {function_clause,[{function_clause_probe,pick,[x,4],[]}]}"
    );
}

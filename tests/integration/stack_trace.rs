use std::process::Command;

use pbeam_runtime::{BeamProgram, Term};

#[path = "../common/mod.rs"]
mod common;

use common::pinned_beam;

const STACK_TRACE_PROBE: &[u8] =
    include_bytes!("../../fixtures/stack_trace/stack_trace_probe.beam");

/// Evaluate `stack_trace_probe:main()` on the pinned ERTS and return its
/// `~w` output. `-kernel logger_level none` stops the error reports of the
/// processes that the probe starts.
fn reference() -> String {
    let reference = Command::new("erl")
        .args([
            "-noshell",
            "-kernel",
            "logger_level",
            "none",
            "-pa",
            "fixtures/stack_trace",
            "-eval",
            "io:format(\"~w~n\", [stack_trace_probe:main()]), halt().",
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
fn keeps_the_stack_trace_frames_of_raise_3_as_pinned_erts() {
    // `system_flag(backtrace_depth, Depth)` returns the old depth and sets
    // at most 64 (`bif.c:5108-5115`, `error.h:206`). `erlang:raise/3` keeps
    // at most `backtrace_depth` frames and one for the depth 0, and gives
    // the location `[]` to `{Fun, Args}` and `{M, F, A}`
    // (`bif.c:1626-1687`).
    let erlang = pinned_beam("erlang");
    let lists = pinned_beam("lists");
    let program = BeamProgram::load(&[STACK_TRACE_PROBE, &erlang, &lists]).unwrap();
    let atom = |name: &str| Term::Atom(program.atom(name).unwrap());
    let tuple = Term::Tuple;
    let list = Term::List;
    let int = Term::Integer;
    let nil = || list(Vec::new());
    let r = || atom("r");
    let badarg = || tuple(vec![atom("error"), atom("badarg")]);
    let caught = |class: &str, reason: Term, frames: Vec<Term>| {
        tuple(vec![atom(class), reason, list(frames)])
    };
    let mfa = |function: &str, arguments: Term, location: Term| {
        tuple(vec![atom("m"), atom(function), arguments, location])
    };
    let f = |arity: i64| mfa("f", int(arity), nil());
    let file = list("m.erl".bytes().map(|byte| int(byte.into())).collect());
    let located = mfa(
        "f",
        list(vec![atom("x")]),
        list(vec![
            tuple(vec![atom("file"), file]),
            tuple(vec![atom("line"), int(3)]),
        ]),
    );
    let all = vec![
        f(1),
        located.clone(),
        tuple(vec![atom("fun"), list(vec![atom("x")]), nil()]),
        tuple(vec![
            atom("fun"),
            list(vec![atom("x")]),
            list(vec![atom("x")]),
        ]),
        mfa("g", int(2), nil()),
    ];
    let raise = |old: i64, frames: Vec<Term>, long: Vec<Term>| {
        list(vec![
            int(old),
            caught("error", r(), frames.clone()),
            caught("exit", r(), frames.clone()),
            caught("throw", r(), frames),
            caught("error", r(), long),
            caught("error", r(), vec![f(1)]),
            caught("error", r(), Vec::new()),
        ])
    };
    let down = |reason: Term| tuple(vec![reason, list(vec![f(1), mfa("g", nil(), nil())])]);
    assert_eq!(
        program.execute("stack_trace_probe", "main", &[]).unwrap(),
        tuple(vec![
            list(vec![int(8), int(64), badarg(), badarg(), int(64)]),
            list(vec![
                raise(1, vec![f(1)], vec![f(1)]),
                raise(0, all[..2].to_vec(), vec![f(1), f(2)]),
                raise(2, all.clone(), (1..=8).map(f).collect()),
            ]),
            // raw_raise raises the exception again with its frames.
            caught("exit", r(), all),
            down(r()),
            down(tuple(vec![atom("nocatch"), r()])),
        ])
    );
    assert_eq!(
        reference(),
        "{[8,64,{error,badarg},{error,badarg},64],\
         [[1,{error,r,[{m,f,1,[]}]},{exit,r,[{m,f,1,[]}]},{throw,r,[{m,f,1,[]}]},\
         {error,r,[{m,f,1,[]}]},{error,r,[{m,f,1,[]}]},{error,r,[]}],\
         [0,{error,r,[{m,f,1,[]},{m,f,[x],[{file,[109,46,101,114,108]},{line,3}]}]},\
         {exit,r,[{m,f,1,[]},{m,f,[x],[{file,[109,46,101,114,108]},{line,3}]}]},\
         {throw,r,[{m,f,1,[]},{m,f,[x],[{file,[109,46,101,114,108]},{line,3}]}]},\
         {error,r,[{m,f,1,[]},{m,f,2,[]}]},{error,r,[{m,f,1,[]}]},{error,r,[]}],\
         [2,{error,r,[{m,f,1,[]},{m,f,[x],[{file,[109,46,101,114,108]},{line,3}]},\
         {'fun',[x],[]},{'fun',[x],[x]},{m,g,2,[]}]},\
         {exit,r,[{m,f,1,[]},{m,f,[x],[{file,[109,46,101,114,108]},{line,3}]},\
         {'fun',[x],[]},{'fun',[x],[x]},{m,g,2,[]}]},\
         {throw,r,[{m,f,1,[]},{m,f,[x],[{file,[109,46,101,114,108]},{line,3}]},\
         {'fun',[x],[]},{'fun',[x],[x]},{m,g,2,[]}]},\
         {error,r,[{m,f,1,[]},{m,f,2,[]},{m,f,3,[]},{m,f,4,[]},{m,f,5,[]},\
         {m,f,6,[]},{m,f,7,[]},{m,f,8,[]}]},\
         {error,r,[{m,f,1,[]}]},{error,r,[]}]],\
         {exit,r,[{m,f,1,[]},{m,f,[x],[{file,[109,46,101,114,108]},{line,3}]},\
         {'fun',[x],[]},{'fun',[x],[x]},{m,g,2,[]}]},\
         {r,[{m,f,1,[]},{m,g,[],[]}]},\
         {{nocatch,r},[{m,f,1,[]},{m,g,[],[]}]}}\n"
    );
}

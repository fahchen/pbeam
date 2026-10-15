use std::process::Command;

use pbeam_runtime::BeamProgram;

#[path = "../common/mod.rs"]
mod common;

use common::pinned_beam;

const THROW_EXIT_PROBE: &[u8] = include_bytes!("../../fixtures/throw_exit/throw_exit_probe.beam");

/// The `%T` text of the exit reason of `throw_exit_probe:uncaught()` on the pinned ERTS, as
/// `erts_internal:term_to_string/2` (`bif.c:4156-4190`) writes it. ERTS
/// `throw_1` and `exit_1` (`bif.c:1376-1430`) raise with a stack trace
/// whose first frame is the function that calls the BIF, with its arity
/// (`save_stacktrace`, `beam_common.c:880-935`). A tail call of
/// `fun erlang:throw/1` or `fun erlang:exit/1` removes the frame of the
/// caller first, a tail `apply/3` keeps it (`beam_common.c:1345-1431`), and
/// as the initial call the frame is the initial call with its arity
/// (`beam_common.c:1245-1248`). `catch` gives the bare reason of `exit/1`.
/// A process that starts at `erlang:apply/3` runs the apply BIF with no
/// frame, so `throw/1` and `error/1` give an empty stack trace
/// (`beam_common.c:1355-1366`).
const EXPECTED: &str = "[[{throw,r,[{throw_exit_probe,raise,2,[]},\
{throw_exit_probe,try_stack,1,[]},{throw_exit_probe,cases,1,[]},\
{throw_exit_probe,main,0,[]},{throw_exit_probe,uncaught,0,[]}]},\
{throw,t,[{throw_exit_probe,tail,1,[]},{throw_exit_probe,try_stack,1,[]},\
{throw_exit_probe,cases,1,[]},{throw_exit_probe,main,0,[]},{throw_exit_probe,uncaught,0,[]}]},\
{throw,f,[{throw_exit_probe,try_stack,1,[]},{throw_exit_probe,cases,1,[]},\
{throw_exit_probe,main,0,[]},{throw_exit_probe,uncaught,0,[]}]},\
{throw,a,[{throw_exit_probe,apply_tail,2,[]},{throw_exit_probe,try_stack,1,[]},\
{throw_exit_probe,cases,1,[]},{throw_exit_probe,main,0,[]},{throw_exit_probe,uncaught,0,[]}]},\
c,{{nocatch,i},[{throw_exit_probe,root,1,[]}]}],\
[{exit,r,[{throw_exit_probe,raise,2,[]},\
{throw_exit_probe,try_stack,1,[]},{throw_exit_probe,cases,1,[]},\
{throw_exit_probe,main,0,[]},{throw_exit_probe,uncaught,0,[]}]},\
{exit,t,[{throw_exit_probe,tail,1,[]},{throw_exit_probe,try_stack,1,[]},\
{throw_exit_probe,cases,1,[]},{throw_exit_probe,main,0,[]},{throw_exit_probe,uncaught,0,[]}]},\
{exit,f,[{throw_exit_probe,try_stack,1,[]},{throw_exit_probe,cases,1,[]},\
{throw_exit_probe,main,0,[]},{throw_exit_probe,uncaught,0,[]}]},\
{exit,a,[{throw_exit_probe,apply_tail,2,[]},{throw_exit_probe,try_stack,1,[]},\
{throw_exit_probe,cases,1,[]},{throw_exit_probe,main,0,[]},{throw_exit_probe,uncaught,0,[]}]},\
{'EXIT',c},i],\
[{{nocatch,x},[]},{{nocatch,x},[]},{x,[]}]]";

#[test]
fn builds_the_stack_trace_of_erlang_throw_1_and_exit_1_as_pinned_erts() {
    // The run removes the default logger handler, so the error report of
    // the exit of `root/1` does not go to the output.
    let reference = Command::new("erl")
        .args([
            "-noshell",
            "-pa",
            "fixtures/throw_exit",
            "-eval",
            "logger:remove_handler(default), {'EXIT', R} = (catch throw_exit_probe:uncaught()), io:format(\"~s~n\", [erts_internal:term_to_string(R, 100000)]), halt().",
        ])
        .output()
        .unwrap();
    assert!(
        reference.status.success() && reference.stderr.is_empty(),
        "{reference:?}"
    );
    assert_eq!(
        String::from_utf8(reference.stdout).unwrap(),
        format!("{EXPECTED}\n")
    );

    // The root process exits with the result (row "Run end" of
    // `docs/erts-differences.md`).
    let erlang = pinned_beam("erlang");
    let program = BeamProgram::load(&[THROW_EXIT_PROBE, &erlang]).unwrap();
    let error = program
        .execute("throw_exit_probe", "uncaught", &[])
        .unwrap_err();
    assert_eq!(
        error.to_string(),
        format!("root process exited with {EXPECTED}")
    );
}

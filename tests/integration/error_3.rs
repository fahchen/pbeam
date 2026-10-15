use std::process::Command;

use pbeam_runtime::BeamProgram;

#[path = "../common/mod.rs"]
mod common;

use common::pinned_beam;

const ERROR_3_PROBE: &[u8] = include_bytes!("../../fixtures/error_3/error_3_probe.beam");

/// The `%T` text of the exit reason of `error_3_probe:uncaught()` on the pinned ERTS, as
/// `erts_internal:term_to_string/2` (`bif.c:4156-4190`) writes it. ERTS
/// `error_3` puts `{error_info, Map}` of the last option in the location of
/// the first frame when each option is `{error_info, Map}`, and else gives
/// the stack trace of `erlang:error/2` (`bif.c:1462-1503`,
/// `beam_common.c:1030-1051`). The pinned `io:format/2` raises `badarg`
/// with the cause `{device, foo}` for a device that replies `{error, foo}`
/// (`io.erl:203-207`).
const EXPECTED: &str = "[{error,r,[{error_3_probe,'-main/0-fun-9-',[a],[]},\
{error_3_probe,try_stack,1,[]},{error_3_probe,main,0,[]},{error_3_probe,uncaught,0,[]}]},\
{error,r,[{error_3_probe,'-main/0-fun-8-',[a],[{error_info,#{cause=>c}}]},\
{error_3_probe,try_stack,1,[]},{error_3_probe,main,0,[]},{error_3_probe,uncaught,0,[]}]},\
{error,r,[{error_3_probe,'-main/0-fun-7-',0,[{error_info,#{}}]},\
{error_3_probe,try_stack,1,[]},{error_3_probe,main,0,[]},{error_3_probe,uncaught,0,[]}]},\
{error,r,[{error_3_probe,'-main/0-fun-6-',[a],[{error_info,#{b=>2}}]},\
{error_3_probe,try_stack,1,[]},{error_3_probe,main,0,[]},{error_3_probe,uncaught,0,[]}]},\
{error,r,[{error_3_probe,'-main/0-fun-5-',[a],[]},\
{error_3_probe,try_stack,1,[]},{error_3_probe,main,0,[]},{error_3_probe,uncaught,0,[]}]},\
{error,r,[{error_3_probe,'-main/0-fun-4-',[a],[]},\
{error_3_probe,try_stack,1,[]},{error_3_probe,main,0,[]},{error_3_probe,uncaught,0,[]}]},\
{error,r,[{error_3_probe,'-main/0-fun-3-',[a],[]},\
{error_3_probe,try_stack,1,[]},{error_3_probe,main,0,[]},{error_3_probe,uncaught,0,[]}]},\
{error,r,[{error_3_probe,'-main/0-fun-2-',[a],[]},\
{error_3_probe,try_stack,1,[]},{error_3_probe,main,0,[]},{error_3_probe,uncaught,0,[]}]},\
{error,r,[{error_3_probe,'-main/0-fun-1-',[a],[]},\
{error_3_probe,try_stack,1,[]},{error_3_probe,main,0,[]},{error_3_probe,uncaught,0,[]}]},\
{error,r,[{error_3_probe,body,[b],[{error_info,#{module=>error_3_probe}}]},\
{error_3_probe,try_stack,1,[]},{error_3_probe,main,0,[]},{error_3_probe,uncaught,0,[]}]},\
{error,badarg,[{io,format,[\"x~w\",[1]],\
[{error_info,#{cause=>{device,foo},module=>erl_stdlib_errors}}]},\
{error_3_probe,try_stack,1,[]},{error_3_probe,device_error,0,[]},\
{error_3_probe,main,0,[]},{error_3_probe,uncaught,0,[]}]}]";

#[test]
fn builds_the_stack_trace_of_erlang_error_3_as_pinned_erts() {
    let reference = Command::new("erl")
        .args([
            "-noshell",
            "-pa",
            "fixtures/error_3",
            "-eval",
            "{'EXIT', R} = (catch error_3_probe:uncaught()), io:format(\"~s~n\", [erts_internal:term_to_string(R, 100000)]), halt().",
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

    // The root process exits with the result, and the error has its `%T`
    // text (row "Run end" of `docs/erts-differences.md`).
    let erlang = pinned_beam("erlang");
    let io = pinned_beam("io");
    let program = BeamProgram::load(&[ERROR_3_PROBE, &erlang, &io]).unwrap();
    let error = program
        .execute("error_3_probe", "uncaught", &[])
        .unwrap_err();
    assert_eq!(
        error.to_string(),
        format!("root process exited with {EXPECTED}")
    );
}

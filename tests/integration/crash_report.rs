use std::process::Command;

use pbeam_runtime::{BeamProgram, debug_term};

#[path = "../common/mod.rs"]
mod common;

use common::pinned_beam;

const CRASH_REPORT_PROBE: &[u8] =
    include_bytes!("../../fixtures/crash_report/crash_report_probe.beam");

/// The debug text of the PBeam result of `crash_report_probe:routes/0`.
const DEBUG_RESULT: &str = "List([Atom(\"logger\"), Tuple([List([Integer(69), Integer(114), Integer(114), Integer(111), Integer(114), Integer(32), Integer(105), Integer(110), Integer(32), Integer(112), Integer(114), Integer(111), Integer(99), Integer(101), Integer(115), Integer(115), Integer(32), Integer(126), Integer(112), Integer(32), Integer(119), Integer(105), Integer(116), Integer(104), Integer(32), Integer(101), Integer(120), Integer(105), Integer(116), Integer(32), Integer(118), Integer(97), Integer(108), Integer(117), Integer(101), Integer(58), Integer(126), Integer(110), Integer(126), Integer(112), Integer(126), Integer(110)]), Tuple([Atom(\"boom\"), List([Tuple([Atom(\"crash_report_probe\"), Atom(\"-routes/0-fun-0-\"), Integer(0), List([])])])]), List([Atom(\"error_logger\"), Atom(\"pid\"), Atom(\"time\"), Atom(\"gl\")]), Map([(Atom(\"emulator\"), Atom(\"true\")), (Atom(\"tag\"), Atom(\"error\"))]), Atom(\"true\"), Atom(\"true\")]), Atom(\"none\"), Tuple([Atom(\"logger\"), Tuple([Tuple([Atom(\"nocatch\"), Atom(\"ball\")]), List([Tuple([Atom(\"crash_report_probe\"), Atom(\"-routes/0-fun-4-\"), Integer(0), List([])])])])]), Tuple([Atom(\"logger\"), Tuple([Atom(\"own\"), List([Tuple([Atom(\"crash_report_probe\"), Atom(\"-routes/0-fun-5-\"), Integer(0), List([])])])])]), Atom(\"none\")])";

#[test]
fn routes_the_error_report_as_the_system_logger_says() {
    // `crash_report_probe:routes/0` sets the system logger to a live pid, to
    // `undefined`, and to a pid that does not exist, and registers the
    // caller and then the crashing process as `logger`
    // (`erl_trace.c:2780-2824`). A live pid gets `{log, error, Format,
    // [Pid, Reason], Metadata}` (`beam_common.c:696-718`,
    // `utils.c:770-858`); `undefined` drops it; a pid that does not exist
    // makes `logger` the system logger again; an uncaught `exit` sends
    // nothing (`error.h:112-115`). The report of a sender that is itself the
    // logger process goes to stderr (`erl_trace.c:2564-2605`).
    let erts = Command::new("erl")
        .args([
            "-noshell",
            "-pa",
            "fixtures/crash_report",
            "-eval",
            "io:format(\"~w~n\", [crash_report_probe:routes()]), halt().",
        ])
        .output()
        .unwrap();
    let format = "[69,114,114,111,114,32,105,110,32,112,114,111,99,101,115,115,32,126,112,32,119,105,116,104,32,101,120,105,116,32,118,97,108,117,101,58,126,110,126,112,126,110]";
    let tail = |old: &str| {
        format!(
            "[{old},{{{format},{{boom,[{{crash_report_probe,'-routes/0-fun-0-',0,[]}}]}},[error_logger,pid,time,gl],#{{emulator => true,tag => error}},true,true}},none,{{logger,{{{{nocatch,ball}},[{{crash_report_probe,'-routes/0-fun-4-',0,[]}}]}}}},{{logger,{{own,[{{crash_report_probe,'-routes/0-fun-5-',0,[]}}]}}}},none]"
        )
    };
    let stdout = String::from_utf8(erts.stdout).unwrap();
    assert_eq!(stdout, format!("{}\n", tail("<0.72.0>")));
    let erlang = pinned_beam("erlang");
    let maps = pinned_beam("maps");
    let program = BeamProgram::load(&[&erlang, &maps, CRASH_REPORT_PROBE]).unwrap();
    let result = program
        .execute("crash_report_probe", "routes", &[])
        .unwrap();
    // The same value as the ERTS one, with `logger` as the system logger at
    // the start, because PBeam starts no `logger` process here.
    assert_eq!(debug_term(&result.term, result.atoms()), DEBUG_RESULT);
}

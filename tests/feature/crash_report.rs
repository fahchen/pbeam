use std::{
    process::{Command, Stdio},
    time::Duration,
};

#[path = "../common/boot.rs"]
mod boot;
#[expect(
    dead_code,
    reason = "this test crate gives BEAM paths, not bytes, to the binary"
)]
#[path = "../common/mod.rs"]
mod common;
#[expect(dead_code, reason = "this test crate does not use a broken pipe")]
#[path = "../common/run.rs"]
mod run;

use boot::{BOOT_MODULES, boot_beam, otp_root};
use common::pinned_beam_path;
use run::run_dumb_utf8;

const CRASH_REPORT_PROBE: &str = "fixtures/crash_report/crash_report_probe.beam";
const RUN_TIME_LIMIT: Duration = Duration::from_secs(10);

/// Replace the time of each `=ERROR REPORT==== <time> ===` line and each pid
/// `<0.N.0>` with fixed text, because they change from run to run.
fn mask(text: &str) -> String {
    let mut out = String::new();
    for line in text.split_inclusive('\n') {
        if line.starts_with("=ERROR REPORT==== ") {
            out.push_str("=ERROR REPORT==== TIME ===\n");
            continue;
        }
        let mut rest = line;
        while let Some(start) = rest.find("<0.") {
            out.push_str(&rest[..start]);
            let end = rest[start..]
                .find('>')
                .map_or(rest.len(), |end| start + end + 1);
            out.push_str("<PID>");
            rest = &rest[end..];
        }
        out.push_str(rest);
    }
    out
}

#[test]
fn writes_the_error_report_of_an_uncaught_error_or_throw_as_erts() {
    // A process that ends with an uncaught `error`, or with an uncaught
    // `throw` that becomes the error `{nocatch, Reason}`, sends the report
    // `Error in process ~p with exit value:~n~p~n` to `logger`
    // (beam_common.c:509-516, 696-718, utils.c:802-858), and the default
    // handler writes it to stdout. An uncaught `exit` sends no report:
    // `EXC_EXIT` has no `EXF_LOG` (error.h:112-115).
    let beams = BOOT_MODULES.map(pinned_beam_path);
    let report = |value: &str| {
        format!(
            "=ERROR REPORT==== TIME ===\nError in process <PID> with exit value:\n{value}\n\ndone\n"
        )
    };
    for (function, stdout) in [
        (
            "error",
            report("{{boom,fun lists:reverse/1},[{crash_report_probe,'-error/0-fun-0-',0,[]}]}"),
        ),
        (
            "throw",
            report("{{nocatch,ball},[{crash_report_probe,'-throw/0-fun-0-',0,[]}]}"),
        ),
        ("exit", "done\n".to_string()),
    ] {
        let erts = run_dumb_utf8(
            Command::new("erl")
                .args([
                    "-noshell",
                    "-pa",
                    "fixtures/crash_report",
                    "-eval",
                    &format!("crash_report_probe:{function}(), halt()."),
                ])
                .env("ERL_CRASH_DUMP_SECONDS", "0"),
            Stdio::piped(),
            RUN_TIME_LIMIT,
        );
        let pbeam = run_dumb_utf8(
            Command::new(env!("CARGO_BIN_EXE_pbeam"))
                .args([
                    "pbeam_boot",
                    "start",
                    "crash_report_probe",
                    function,
                    "--root",
                    otp_root().to_str().unwrap(),
                    "--",
                    CRASH_REPORT_PROBE,
                ])
                .args(&beams)
                .arg(boot_beam()),
            Stdio::piped(),
            RUN_TIME_LIMIT,
        );
        for (name, output) in [("ERTS", erts), ("PBeam", pbeam)] {
            assert_eq!(
                (
                    mask(&String::from_utf8_lossy(&output.stdout)),
                    String::from_utf8_lossy(&output.stderr).into_owned(),
                    output.status.code()
                ),
                (stdout.clone(), String::new(), Some(0)),
                "{name} {function}"
            );
        }
    }
}

#[test]
fn writes_the_report_to_stderr_when_no_logger_takes_it() {
    // `crash_report_probe:routes/0` sets the system logger to a live pid, to
    // `undefined`, and to a pid that does not exist, and registers the
    // caller and then the crashing process as `logger`
    // (erl_trace.c:2780-2824). The report of a sender that is itself the
    // logger process goes to stderr (erl_trace.c:2564-2605).
    let stderr = "(no logger present) unexpected logger message: {log,error,\"Error in process ~p with exit value:~n~p~n\",[<PID>,{boom,[{crash_report_probe,'-routes/0-fun-6-',0,[]}]}],#{error_logger=>#{emulator=>true,tag=>error},pid=><PID>,time=>TIME,gl=><PID>}}\r\n";
    let erts = run_dumb_utf8(
        Command::new("erl").args([
            "-noshell",
            "-pa",
            "fixtures/crash_report",
            "-eval",
            "io:format(\"~w~n\", [crash_report_probe:routes()]), halt().",
        ]),
        Stdio::piped(),
        RUN_TIME_LIMIT,
    );
    let pbeam = run_dumb_utf8(
        Command::new(env!("CARGO_BIN_EXE_pbeam"))
            .args(["crash_report_probe", "routes", "--", CRASH_REPORT_PROBE])
            .arg(pinned_beam_path("erlang"))
            .arg(pinned_beam_path("maps")),
        Stdio::piped(),
        RUN_TIME_LIMIT,
    );
    for (name, output) in [("ERTS", erts), ("PBeam", pbeam)] {
        let text = mask(&String::from_utf8_lossy(&output.stderr));
        // ERTS drops the report when its dispatcher runs before the exit
        // marks the sender as exiting (erl_trace.c:2817-2818, register.c:356).
        if name == "ERTS" && text.is_empty() {
            continue;
        }
        let start = text.find("time=>").map_or(text.len(), |start| start + 6);
        let end = text[start..].find(',').map_or(start, |end| start + end);
        let text = format!("{}TIME{}", &text[..start], &text[end..]);
        assert_eq!(text, stderr, "{name}");
    }
}

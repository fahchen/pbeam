use std::{process::Command, time::Duration};

#[path = "../common/boot.rs"]
mod boot;
#[expect(
    dead_code,
    reason = "this test crate gives BEAM paths, not bytes, to the binary"
)]
#[path = "../common/mod.rs"]
mod common;
#[path = "../common/log_event.rs"]
mod log_event;
#[path = "../common/run.rs"]
mod run;

use boot::{BOOT_MODULES, boot_beam};
use common::pinned_beam_path;
use log_event::{LOG_EVENT_MODULES, mask_time};
use run::{broken_pipe, run_dumb_utf8};

const IO_FAIL_PROBE: &str = "fixtures/io_fail/io_fail_probe.beam";
const RUN_TIME_LIMIT: Duration = Duration::from_secs(10);

#[test]
fn raises_terminated_after_a_failed_stdout_write_through_user_drv() {
    // `io:put_chars(standard_io, ...)` sends the request to `user`
    // (io.erl:1652-1655), and `user` and `user_drv` send it to the
    // `prim_tty` writer. `write_nif/2` gives `{error, epipe}`, and the
    // writer calls `exit(self(), epipe)` (prim_tty.erl:559-566). The link
    // exit signals go before the monitor signals (erl_process.c:14545-14560).
    // Thus `user_drv` gets `{'EXIT', Writer, epipe}`, logs
    // `Writer crashed (epipe)` with `?LOG_ERROR`, and stops
    // (user_drv.erl:603-615). `?LOG_ERROR` formats the event in `user_drv`
    // and casts it to the `default` handler before `user_drv` stops
    // (logger_h_common.erl:178-183; logger_olp.erl:80-87). `user` stops
    // after `user_drv`, the caller gets `DOWN`, and `io:put_chars/2` raises
    // `error:terminated` (io.erl:197-202, 782, 1679-1684). The file control
    // process of the handler fails to write the event to `user`, and writes
    // the first line and the event to `standard_error`
    // (logger_std_h.erl:680-688). `standard_error` converts each request
    // with `unicode:characters_to_list/2` (standard_error.erl:112-130,
    // 266-287), and `io:format(standard_error, ...)` calls
    // `net_kernel:dflag_unicode_io/1` (io.erl:1734-1735, 1752-1753).
    // `logger_std_h:filesync(default)` comes after the `DOWN` of `user`, and
    // the file control process replies after it writes the event
    // (logger_h_common.erl:262-266; logger_std_h.erl:601-623). Thus the
    // `result` line is the last line, also on PBeam, which ends the run at
    // the root exit.
    let beams = BOOT_MODULES.map(pinned_beam_path);
    let events = LOG_EVENT_MODULES.map(pinned_beam_path);
    let erts = run_dumb_utf8(
        Command::new("erl").args([
            "-noshell",
            "-pa",
            "fixtures/io_fail",
            "-eval",
            "io_fail_probe:main(), halt().",
        ]),
        broken_pipe(),
        RUN_TIME_LIMIT,
    );
    let pbeam = run_dumb_utf8(
        Command::new(env!("CARGO_BIN_EXE_pbeam"))
            .args([
                "pbeam_boot",
                "start",
                "io_fail_probe",
                "main",
                "--",
                IO_FAIL_PROBE,
            ])
            .args(&beams)
            .args(&events)
            .arg(boot_beam()),
        broken_pipe(),
        RUN_TIME_LIMIT,
    );
    for (name, output) in [("ERTS", erts), ("PBeam", pbeam)] {
        assert_eq!(
            (
                mask_time(&String::from_utf8_lossy(&output.stderr)).as_str(),
                output.status.code()
            ),
            (
                "Failed to write log message to stdout, trying stderr\n\
                 =ERROR REPORT==== <time> ===\n\
                 Writer crashed (epipe)\n\
                 result {error,terminated}\n",
                Some(0)
            ),
            "{name}"
        );
    }
}

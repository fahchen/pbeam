use std::{
    process::{Command, Stdio},
    time::Duration,
};

#[path = "common/boot.rs"]
mod boot;
#[expect(
    dead_code,
    reason = "this test crate gives BEAM paths, not bytes, to the binary"
)]
mod common;
#[path = "common/log_event.rs"]
mod log_event;
#[path = "common/run.rs"]
mod run;

use boot::{BOOT_BEAM, BOOT_MODULES};
use common::pinned_beam_path;
use log_event::{LOG_EVENT_MODULES, mask_time};
use run::run_dumb_utf8;

const LOG_PROBE: &str = "fixtures/log/log_probe.beam";
const RUN_TIME_LIMIT: Duration = Duration::from_secs(10);

#[test]
fn writes_a_log_event_through_the_default_handler_after_the_boot() {
    // `?LOG_ERROR` calls `logger:allow/2` and then `logger:macro_log/4`
    // (logger.hrl:68-74). After the default filters, `logger_backend` calls
    // `logger_std_h:log/2` (logger_std_h.erl:255-256). `logger_h_common:log/2`
    // formats the event in the caller process and gives the binary to
    // `logger_olp:load/2` (logger_h_common.erl:178-183), which casts
    // `{'$olp_load', Bin}` to the handler process in `async` mode
    // (logger_olp.erl:80-87). The handler sends `{log, Bin}` to the file
    // control process (logger_h_common.erl:246-260; logger_std_h.erl:432-434,
    // 544-546), which writes it to `user` with `io:put_chars/2`
    // (logger_std_h.erl:601-606, 680-688). `logger_std_h:filesync(default)`
    // calls the handler after the cast (logger_h_common.erl:64-65, 349-353;
    // logger_olp.erl:313-320). The handler then calls the file control
    // process, which replies after it writes the event
    // (logger_h_common.erl:262-266; logger_std_h.erl:427-429, 550-554,
    // 619-623). Thus the write is complete before `log_probe:main/0`
    // returns, and before `halt()` on ERTS or the root exit on PBeam ends
    // the run.
    let beams = BOOT_MODULES.map(pinned_beam_path);
    let events = LOG_EVENT_MODULES.map(pinned_beam_path);
    let erts = run_dumb_utf8(
        Command::new("erl").args([
            "-noshell",
            "-pa",
            "fixtures/log",
            "-eval",
            "ok = log_probe:main(), halt().",
        ]),
        Stdio::piped(),
        RUN_TIME_LIMIT,
    );
    let pbeam = run_dumb_utf8(
        Command::new(env!("CARGO_BIN_EXE_pbeam"))
            .args(["pbeam_boot", "start", "log_probe", "main", "--", LOG_PROBE])
            .args(&beams)
            .args(&events)
            .arg(BOOT_BEAM),
        Stdio::piped(),
        RUN_TIME_LIMIT,
    );
    for (name, output) in [("ERTS", erts), ("PBeam", pbeam)] {
        assert_eq!(
            (
                mask_time(&String::from_utf8_lossy(&output.stdout)).as_str(),
                String::from_utf8_lossy(&output.stderr).as_ref(),
                output.status.code()
            ),
            (
                "=ERROR REPORT==== <time> ===\nWriter crashed (epipe)\n",
                "",
                Some(0)
            ),
            "{name}"
        );
    }
}

#[test]
fn masks_only_a_time_of_the_legacy_header_form() {
    for (text, masked) in [
        (
            "=ERROR REPORT==== 6-Oct-2026::11:24:49.199467 ===\nx\n",
            "=ERROR REPORT==== <time> ===\nx\n",
        ),
        (
            "=ERROR REPORT==== 16-Dec-2026::01:02:03.000004 ===\n",
            "=ERROR REPORT==== <time> ===\n",
        ),
        (
            "=ERROR REPORT==== 116-Oct-2026::11:24:49.199467 ===\n",
            "=ERROR REPORT==== 116-Oct-2026::11:24:49.199467 ===\n",
        ),
        (
            "=ERROR REPORT==== 6-10-2026::11:24:49.199467 ===\n",
            "=ERROR REPORT==== 6-10-2026::11:24:49.199467 ===\n",
        ),
        (
            "=ERROR REPORT==== 6-Oct-26::11:24:49.199467 ===\n",
            "=ERROR REPORT==== 6-Oct-26::11:24:49.199467 ===\n",
        ),
        (
            "=ERROR REPORT==== 6-Oct-2026 11:24:49.199467 ===\n",
            "=ERROR REPORT==== 6-Oct-2026 11:24:49.199467 ===\n",
        ),
        (
            "=ERROR REPORT==== 6-Oct-2026::11:24:49 ===\n",
            "=ERROR REPORT==== 6-Oct-2026::11:24:49 ===\n",
        ),
        (
            "=ERROR REPORT==== 6-Oct-2026::11:4:49.199467 ===\n",
            "=ERROR REPORT==== 6-Oct-2026::11:4:49.199467 ===\n",
        ),
        (
            "=ERROR REPORT==== 6-Oct-2026::11:24:49.19946 ===\n",
            "=ERROR REPORT==== 6-Oct-2026::11:24:49.19946 ===\n",
        ),
        (
            "=ERROR REPORT==== 6-Oct-2026::11:24:49.199467 UTC ===\n",
            "=ERROR REPORT==== 6-Oct-2026::11:24:49.199467 UTC ===\n",
        ),
        ("=ERROR REPORT==== ", "=ERROR REPORT==== "),
    ] {
        assert_eq!(mask_time(text), masked, "{text:?}");
    }
}

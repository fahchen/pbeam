//! The pinned OTP modules that a log event of `?LOG_ERROR` calls after the
//! boot, and the time mask of a legacy `logger_formatter` header. A test
//! that runs a log event with the `pbeam` binary includes this file with
//! `#[path = "common/log_event.rs"] mod log_event;`. A constant in
//! `common/mod.rs` would be dead code in each other test crate.

use std::ops::RangeInclusive;

/// The pinned OTP modules that a log event of `?LOG_ERROR` calls after the
/// boot, in addition to `BOOT_MODULES`. `logger:macro_log/4` calls
/// `logger_backend:log_allowed/3` (logger.erl:1927). The default filters of
/// the `default` handler call `logger_filters:remote_gl/2`
/// (logger_internal.hrl:30-33). `logger_formatter` calls
/// `error_logger:get_format_depth/0` (logger_formatter.erl:667) and
/// `string:uppercase/1` for the title (logger_formatter.erl:616), which calls
/// `unicode_util:uppercase/1` (string.erl:1317). `~p` calls
/// `io_lib_pretty:print/2` (io_lib_format.erl:635), which calls
/// `epp:default_encoding/0` (io_lib_pretty.erl:102). `io_lib:quote_atom/2`
/// calls `erl_scan:reserved_word/1` (io_lib.erl:980), which calls
/// `erl_features:keywords/0` (erl_scan.erl:2282). Without one of them, the
/// run stops with status 1 and `pbeam: unresolved external MFA` of that
/// function (measured).
///
/// # Examples
///
/// ```rust,ignore
/// let beams = BOOT_MODULES.map(pinned_beam_path);
/// let events = LOG_EVENT_MODULES.map(pinned_beam_path);
/// Command::new(env!("CARGO_BIN_EXE_pbeam"))
///     .args(["pbeam_boot", "start", "log_probe", "main", "--", LOG_PROBE])
///     .args(&beams)
///     .args(&events)
///     .arg(boot_beam());
/// ```
pub const LOG_EVENT_MODULES: [&str; 9] = [
    "logger_backend",
    "logger_filters",
    "error_logger",
    "string",
    "unicode_util",
    "io_lib_pretty",
    "epp",
    "erl_scan",
    "erl_features",
];

/// The month names of `logger_formatter:month/1`
/// (logger_formatter.erl:620-631).
const MONTHS: [&str; 12] = [
    "Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec",
];

/// Replace each time field of a legacy `logger_formatter` header with
/// `<time>`. The header format is
/// `"=~ts==== ~w-~s-~4w::~2..0w:~2..0w:~2..0w.~6..0w ~s==="`
/// (logger_formatter.erl:596-597), and the UTC string is empty for the
/// local time. Thus a time field is the text between `==== ` and ` ===`
/// that has a day of 1 or 2 digits, a month name of `MONTHS`, a year of 4
/// digits, `::`, an hour, a minute, and a second of 2 digits, `.`, and 6
/// digits. A text of a different form stays the same, so a comparison with
/// the expected text fails.
///
/// # Examples
///
/// ```rust,ignore
/// assert_eq!(
///     mask_time("=ERROR REPORT==== 6-Oct-2026::11:24:49.199467 ===\n"),
///     "=ERROR REPORT==== <time> ===\n"
/// );
/// assert_eq!(
///     mask_time("=ERROR REPORT==== 6-Oct-2026::11:24:49 ===\n"),
///     "=ERROR REPORT==== 6-Oct-2026::11:24:49 ===\n"
/// );
/// ```
pub fn mask_time(text: &str) -> String {
    const START: &str = "==== ";
    let mut masked = String::new();
    let mut rest = text;
    while let Some(index) = rest.find(START) {
        let (head, tail) = rest.split_at(index + START.len());
        masked.push_str(head);
        rest = match tail.split_once(" ===") {
            Some((time, _)) if is_header_time(time) => {
                masked.push_str("<time>");
                &tail[time.len()..]
            }
            _ => tail,
        };
    }
    masked.push_str(rest);
    masked
}

/// Return `true` when `time` has the form `D-Mon-YYYY::hh:mm:ss.micro` of
/// the legacy header of `logger_formatter` (logger_formatter.erl:596-597).
///
/// # Examples
///
/// ```rust,ignore
/// assert!(is_header_time("16-Oct-2026::09:05:01.000042"));
/// assert!(!is_header_time("16-10-2026::09:05:01.000042"));
/// ```
fn is_header_time(time: &str) -> bool {
    let digits = |field: &str, count: RangeInclusive<usize>| {
        count.contains(&field.len()) && field.bytes().all(|byte| byte.is_ascii_digit())
    };
    let Some((date, clock)) = time.split_once("::") else {
        return false;
    };
    let Some((clock, micro)) = clock.split_once('.') else {
        return false;
    };
    let date: Vec<&str> = date.split('-').collect();
    let clock: Vec<&str> = clock.split(':').collect();
    matches!(date.as_slice(), [day, month, year]
        if digits(day, 1..=2) && MONTHS.contains(month) && digits(year, 4..=4))
        && matches!(clock.as_slice(), [hour, minute, second]
            if [hour, minute, second].iter().all(|field| digits(field, 2..=2)))
        && digits(micro, 6..=6)
}

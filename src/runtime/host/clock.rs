//! Read the host clocks that ERTS reads (`sys/unix/sys_time.c`), and the
//! local time of the host time zone (`sys/unix/erl_unix_sys.h`).

use std::{sync::LazyLock, time::Instant};

use crate::runtime::time::DateTime;

/// The start of the performance counter of the process, which the first
/// [`perf_counter`] call sets. ERTS counts from the boot of the host
/// (`erts_sys_hrtime`, `sys_time.c:1009-1014`), a base that its own
/// documentation allows to differ between runtime restarts.
static PERF_COUNTER_START: LazyLock<Instant> = LazyLock::new(Instant::now);

/// Return the OS system time: the host `CLOCK_REALTIME` in nanoseconds
/// since 1970, as the ERTS `erts_os_system_time` reads it with
/// `posix_clock_gettime` (`sys_time.c:469-497`). On the pinned ERTS,
/// `erlang:system_info(os_system_time_source)` gives
/// `{function,clock_gettime}` and `{clock_id,'CLOCK_REALTIME'}`.
///
/// ERTS aborts the emulator when `clock_gettime` fails, which needs a
/// clock id that the host does not have. `CLOCK_REALTIME` exists on each
/// Unix host, so the call does not fail.
///
/// # Examples
///
/// ```rust,ignore
/// // 2023-11-14 and later.
/// assert!(system_time() > 1_700_000_000_000_000_000);
/// ```
pub(crate) fn system_time() -> i64 {
    // SAFETY: all zeros is a valid `timespec`, and `clock_gettime` writes
    // only into the struct that the pointer gives.
    let time = unsafe {
        let mut time: libc::timespec = std::mem::zeroed();
        libc::clock_gettime(libc::CLOCK_REALTIME, &raw mut time);
        time
    };
    time.tv_sec * 1_000_000_000 + time.tv_nsec
}

unsafe extern "C" {
    /// POSIX `tzset`: read the time zone of the `TZ` environment variable
    /// into the C library. libc 0.2.190 has no binding for Unix.
    fn tzset();

    /// POSIX `daylight`: nonzero when the time zone that `tzset` read has
    /// a daylight saving time rule. ERTS reads it as `sys_daylight`
    /// (`sys/unix/erl_unix_sys.h:154-156`). libc 0.2.190 has no binding.
    static daylight: libc::c_int;
}

#[cfg(target_os = "macos")]
unsafe extern "C" {
    /// macOS `posix2time`: add the leap seconds of the time zone to a POSIX
    /// time. libc 0.2.190 has no binding.
    fn posix2time(time: libc::time_t) -> libc::time_t;

    /// macOS `time2posix`: subtract the leap seconds of the time zone from
    /// a time. libc 0.2.190 has no binding.
    fn time2posix(time: libc::time_t) -> libc::time_t;
}

/// Return the performance counter of the host in nanoseconds since
/// [`PERF_COUNTER_START`], in the unit that `erlang:system_info` calls
/// `perf_counter`: ERTS reads `erts_sys_hrtime`, which is the
/// `CLOCK_UPTIME_RAW` time of macOS and the `CLOCK_MONOTONIC` time of other
/// hosts (`sys_time.c:1009-1014`, `sys/unix/erl_unix_sys.h:218-219`).
/// `Instant` reads the same clock of the host as that function. ERTS counts
/// the value from the boot of the host and PBeam counts it from the first
/// call, a base that ERTS documents as the same only within a runtime
/// restart (`os:perf_counter/0`).
///
/// The counter is exclusive and non-negative, as `os_perf_counter_0`
/// (`erl_time_sup.c:2601-2605`) gives it.
///
/// # Examples
///
/// ```rust,ignore
/// assert!(perf_counter() <= perf_counter());
/// assert!(perf_counter() >= 0);
/// ```
pub(crate) fn perf_counter() -> i64 {
    i64::try_from(PERF_COUNTER_START.elapsed().as_nanos()).unwrap_or(i64::MAX)
}

/// Return the local date and time of `clock`, a POSIX time, in the host
/// time zone, as the end of ERTS `univ_to_local`
/// (`erl_time_sup.c:1822-1848`) does. On macOS, ERTS has `posix2time`
/// (`HAVE_POSIX2TIME`) and adds the leap seconds of the time zone first. A
/// glibc host has no `posix2time`. Then `sys_localtime_r` gives the local
/// time (see [`local_zone_time`]). Return `None` when `localtime_r` fails.
///
/// # Examples
///
/// ```rust,ignore
/// // TZ=EST5EDT,M3.2.0,M11.1.0
/// assert_eq!(local_time(0), Some((1969, 12, 31, 19, 0, 0)));
/// ```
pub(crate) fn local_time(clock: i64) -> Option<DateTime> {
    #[cfg(target_os = "macos")]
    // SAFETY: `posix2time` reads only its argument and the time zone data
    // of the C library.
    let clock = unsafe { posix2time(clock) };
    local_zone_time(clock)
}

/// Return the local date and time of `clock`, a POSIX time, in the host
/// time zone, as ERTS `sys_localtime_r` gives it to `get_localtime`,
/// `get_date`, and `get_time` (`erl_time_sup.c:1516-1557`). `sys_localtime_r`
/// calls `tzset`, so a change of `TZ` takes effect, and `localtime_r`
/// (`sys/unix/erl_unix_sys.h:331-343`). It adds no leap second of the time
/// zone, unlike the `univ_to_local` path of
/// `erlang:universaltime_to_localtime/1`, which calls `posix2time` first on
/// macOS. Return `None` when `localtime_r` fails.
///
/// # Examples
///
/// ```rust,ignore
/// // TZ=EST5EDT,M3.2.0,M11.1.0
/// assert_eq!(local_zone_time(0), Some((1969, 12, 31, 19, 0, 0)));
/// ```
pub(crate) fn local_zone_time(clock: i64) -> Option<DateTime> {
    // SAFETY: all zeros is a valid `tm`. `tzset` changes only the time zone
    // data of the C library, and `localtime_r` reads the time that the
    // first pointer gives and writes only into the struct that the second
    // pointer gives.
    let (result, tm) = unsafe {
        tzset();
        let mut tm: libc::tm = std::mem::zeroed();
        (libc::localtime_r(&raw const clock, &raw mut tm), tm)
    };
    (!result.is_null()).then(|| {
        (
            i64::from(tm.tm_year) + 1900,
            i64::from(tm.tm_mon) + 1,
            i64::from(tm.tm_mday),
            i64::from(tm.tm_hour),
            i64::from(tm.tm_min),
            i64::from(tm.tm_sec),
        )
    })
}

/// Return the UTC date and time of a local date and time in the host time
/// zone, as ERTS `local_to_univ` (`erl_time_sup.c:1756-1794`) finds it
/// after the range check. `isdst` is the `tm_isdst` of `mktime`: 1, 0, or
/// -1 for unknown. A time zone without daylight saving time (`sys_daylight`
/// is 0) uses 0 (`erl_time_sup.c:1764-1769`). `erl_mktime`
/// (`erl_time_sup.c:1607-1635`) calls `mktime`; when `mktime` gives -1, it
/// calls `mktime` again with one second less, and the result -2 means that
/// the time was -1. On macOS, ERTS has `time2posix` (`HAVE_TIME2POSIX`) and
/// subtracts the leap seconds of the time zone. Then `gmtime_r` gives the
/// UTC date and time; glibc adds the leap seconds of the time zone there.
/// Return `None` when `mktime` or `gmtime_r` fails.
///
/// The C library rules of `mktime` for `tm_isdst` and for a local time
/// that does not exist or occurs twice differ between macOS and glibc, and
/// no Rust time zone library takes `tm_isdst`. Thus PBeam calls the C
/// library as ERTS does.
///
/// # Examples
///
/// ```rust,ignore
/// // TZ=EST5EDT,M3.2.0,M11.1.0
/// assert_eq!(local_to_univ((1970, 1, 1, 0, 0, 0), -1), Some((1970, 1, 1, 5, 0, 0)));
/// ```
pub(crate) fn local_to_univ(
    (year, month, day, hour, minute, second): DateTime,
    isdst: libc::c_int,
) -> Option<DateTime> {
    // The caller checks the range (`erl_time_sup.c:1744-1754`), so each
    // field fits in an `int`.
    let field = |value: i64| libc::c_int::try_from(value).expect("a checked field fits in an int");
    // SAFETY: all zeros is a valid `tm`. `tzset` changes only the time zone
    // data of the C library, `daylight` is an `int` that only `tzset`
    // writes, and `mktime` reads the time zone data and normalizes only the
    // struct that the pointer gives.
    let clock = unsafe {
        let mut tm: libc::tm = std::mem::zeroed();
        tm.tm_year = field(year - 1900);
        tm.tm_mon = field(month - 1);
        tm.tm_mday = field(day);
        tm.tm_hour = field(hour);
        tm.tm_min = field(minute);
        tm.tm_sec = field(second);
        tzset();
        tm.tm_isdst = if daylight == 0 { 0 } else { isdst };
        match libc::mktime(&raw mut tm) {
            -1 => {
                tm.tm_sec -= 1;
                (libc::mktime(&raw mut tm) == -2).then_some(-1)
            }
            clock => Some(clock),
        }
    };
    let clock = clock?;
    #[cfg(target_os = "macos")]
    // SAFETY: `time2posix` reads only its argument and the time zone data
    // of the C library.
    let clock = unsafe { time2posix(clock) };
    // SAFETY: all zeros is a valid `tm`. `gmtime_r` reads the time that the
    // first pointer gives and writes only into the struct that the second
    // pointer gives.
    let (result, tm) = unsafe {
        let mut tm: libc::tm = std::mem::zeroed();
        (libc::gmtime_r(&raw const clock, &raw mut tm), tm)
    };
    (!result.is_null()).then(|| {
        (
            i64::from(tm.tm_year) + 1900,
            i64::from(tm.tm_mon) + 1,
            i64::from(tm.tm_mday),
            i64::from(tm.tm_hour),
            i64::from(tm.tm_min),
            i64::from(tm.tm_sec),
        )
    })
}

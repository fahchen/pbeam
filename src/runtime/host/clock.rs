//! Read the host clocks that ERTS reads (`sys/unix/sys_time.c`).

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

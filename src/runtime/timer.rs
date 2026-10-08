//! The timer service of the run, as the ERTS timer wheel (`time.c`) and the
//! high-level timers (`erl_hl_timer.c`) of one scheduler. All times are on
//! the ERTS internal monotonic time: nanoseconds, one second at the run
//! start. A timer end is a clock tick of 1 ms.

use std::{
    collections::{BTreeMap, HashMap},
    time::{Duration, Instant},
};

use super::time::MONOTONIC_BEGIN;
use crate::term::{Atom, ProcessId, Reference, Term};

/// Native time units in one clock tick of 1 ms, as
/// `ERTS_MONOTONIC_TO_CLKTCKS` divides with `SYS_CLOCK_RESOLUTION 1`
/// (`sys/unix/erl_unix_sys.h:325`, `erl_time.h:148`).
const UNITS_PER_TICK: i64 = 1_000_000;

/// The last valid timer end in clock ticks: `ERTS_MONOTONIC_END`, the
/// largest 64-bit multiple of one second (`erl_time.h:237-239`,
/// `sys/unix/erl_unix_sys.h:172-173`), in ticks.
const MONOTONIC_END_TICKS: i64 = i64::MAX / MONOTONIC_BEGIN * MONOTONIC_BEGIN / UNITS_PER_TICK;

/// The position of a timer in the queue: its end tick, then its start
/// number.
pub(super) type TimerKey = (i64, u64);

/// What a timer does when it expires.
#[derive(Debug, PartialEq)]
pub(super) enum Timer {
    /// The receive timer of a process, which `wait_timeout` starts.
    Receive(ProcessId),
    /// A BIF timer, which `erlang:start_timer/3` starts. It sends `message`
    /// to `receiver` when it expires (`bif_timer_timeout`,
    /// `erl_hl_timer.c:1146-1203`).
    Bif {
        /// The timer reference that the BIF returns.
        reference: Reference,
        /// The process that gets the message.
        receiver: Receiver,
        /// The message, `{timeout, Reference, Msg}`, which
        /// `setup_bif_timer` makes at the start (`erl_hl_timer.c:1825`).
        message: Term,
    },
}

/// The receiver of a BIF timer message.
#[derive(Debug, PartialEq)]
pub(super) enum Receiver {
    /// A pid. The exit of the process cancels the timer
    /// (`erl_process.c:14300-14303`).
    Process(ProcessId),
    /// A registered name. The timer finds the process with the name when it
    /// expires (`ERTS_TMR_ROFLG_REG_NAME`, `erl_hl_timer.c:1165-1170`).
    Name(Atom),
}

/// All active timers of the run, in expiry order. An ERTS timer wheel slot
/// is a FIFO list: `insert_timer_into_slot` adds a timer at the tail
/// (`time.c:647-668`), and `erts_bump_timers` expires slots in tick order
/// from the head (`time.c:784-1000`). The key (end tick, start number) gives
/// the same order, so timers with the same end expire in start order. ERTS
/// starts a timer of 16384 ms or more in the later wheel and later moves it
/// to the tail of its soon wheel slot (`time.c:1030-1130`), so it can expire
/// after a timer with the same end that started later. PBeam keeps the start
/// order for these timers too (`docs/erts-differences.md`, "Timers").
#[derive(Debug)]
pub(super) struct Timers {
    /// The run start, at the internal monotonic time `MONOTONIC_BEGIN`.
    start: Instant,
    /// Active timers by key.
    queue: BTreeMap<TimerKey, Timer>,
    /// The key of each active BIF timer by its reference, as the ERTS
    /// `btm_tree` of the timer service (`erl_hl_timer.c:312`).
    bif_timers: HashMap<Reference, TimerKey>,
    /// Start number of the next timer.
    next: u64,
}

impl Timers {
    /// Return a timer service with no timers. The run clock starts now.
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// let timers = Timers::new();
    /// assert!(timers.now() >= 1_000_000_000);
    /// ```
    pub(super) fn new() -> Self {
        Self {
            start: Instant::now(),
            queue: BTreeMap::new(),
            bif_timers: HashMap::new(),
            next: 0,
        }
    }

    /// Return the internal monotonic time of the run: `MONOTONIC_BEGIN`
    /// plus the nanoseconds since the run start. `Instant` reads
    /// `CLOCK_UPTIME_RAW` on macOS, which is the
    /// `os_monotonic_time_source` of the pinned ERTS, and
    /// `CLOCK_MONOTONIC` on other Unix hosts.
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// let timers = Timers::new();
    /// assert!(timers.now() <= timers.now());
    /// ```
    pub(super) fn now(&self) -> i64 {
        i64::try_from(self.start.elapsed().as_nanos())
            .unwrap_or(i64::MAX)
            .saturating_add(MONOTONIC_BEGIN)
    }

    /// Return the end tick of a timer of `msec` milliseconds that starts at
    /// the time `now`, as the relative branch of `parse_timeout_pos`
    /// (`erl_hl_timer.c:2494-2554`): `msec + (now - 1) div 1000000 + 1`.
    /// Return `None` for a negative `msec`, for an end after
    /// `ERTS_MONOTONIC_END`, and for a 64-bit overflow, which wraps in C to
    /// a value below `ERTS_MONOTONIC_BEGIN`.
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// assert_eq!(Timers::timeout_position(1_000_000_000, 100), Some(1100));
    /// assert_eq!(Timers::timeout_position(1_000_000_001, 100), Some(1101));
    /// assert_eq!(Timers::timeout_position(1_000_000_000, -1), None);
    /// assert_eq!(Timers::timeout_position(1_000_000_000, i64::MAX), None);
    /// ```
    pub(super) fn timeout_position(now: i64, msec: i64) -> Option<i64> {
        if msec < 0 {
            return None;
        }
        msec.checked_add((now - 1) / UNITS_PER_TICK)?
            .checked_add(1)
            .filter(|&end| end <= MONOTONIC_END_TICKS)
    }

    /// Return the milliseconds from the time `now` to the end tick `end`, as
    /// `get_time_left` (`erl_hl_timer.c:681-689`) does: `now` becomes the
    /// tick `(now - 1) div 1000000 + 1`, and an end at or before that tick
    /// gives 0.
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// assert_eq!(Timers::time_left(1_000_000_000, 1100), 100);
    /// assert_eq!(Timers::time_left(1_000_000_001, 1100), 99);
    /// assert_eq!(Timers::time_left(1_200_000_000, 1100), 0);
    /// ```
    pub(super) fn time_left(now: i64, end: i64) -> i64 {
        (end - ((now - 1) / UNITS_PER_TICK + 1)).max(0)
    }

    /// Add a timer that expires at the end tick `end`. Return its key.
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// let key = timers.start(1100, Timer::Receive(ProcessId(0)));
    /// assert_eq!(key, (1100, 0));
    /// ```
    pub(super) fn start(&mut self, end: i64, timer: Timer) -> TimerKey {
        let key = (end, self.next);
        self.next += 1;
        if let Timer::Bif { reference, .. } = &timer {
            self.bif_timers.insert(*reference, key);
        }
        self.queue.insert(key, timer);
        key
    }

    /// Remove the timer with the key `key`, as `erts_cancel_proc_timer`
    /// (`erl_hl_timer.c:2832-2846`) cancels a receive timer.
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// let key = timers.start(1100, Timer::Receive(ProcessId(0)));
    /// timers.cancel(key);
    /// assert_eq!(timers.wait_time(1_000_000_000), None);
    /// ```
    pub(super) fn cancel(&mut self, key: TimerKey) {
        self.remove(key);
    }

    /// Remove the BIF timer with the reference `reference`. Return its end
    /// tick and the timer, or `None` when no BIF timer with the reference is
    /// active, as `access_btm` (`erl_hl_timer.c:1922-1971`) finds no active
    /// timer.
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// timers.start(1100, Timer::Bif { reference, receiver, message });
    /// assert!(matches!(timers.cancel_bif(reference), Some((1100, Timer::Bif { .. }))));
    /// assert_eq!(timers.cancel_bif(reference), None);
    /// ```
    pub(super) fn cancel_bif(&mut self, reference: Reference) -> Option<(i64, Timer)> {
        let key = *self.bif_timers.get(&reference)?;
        self.remove(key).map(|timer| (key.0, timer))
    }

    /// Remove and return the timer with the key `key`. Keep the BIF timer
    /// index in sync.
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// let key = timers.start(1100, Timer::Receive(ProcessId(0)));
    /// assert_eq!(timers.remove(key), Some(Timer::Receive(ProcessId(0))));
    /// ```
    fn remove(&mut self, key: TimerKey) -> Option<Timer> {
        let timer = self.queue.remove(&key)?;
        if let Timer::Bif { reference, .. } = &timer {
            self.bif_timers.remove(reference);
        }
        Some(timer)
    }

    /// Remove and return the first timer when it is due at the time `now`:
    /// its end tick is at or before `now div 1000000`, as
    /// `erts_bump_timers` bumps to `ERTS_MONOTONIC_TO_CLKTCKS(curr_time)`
    /// (`time.c:820`).
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// timers.start(1100, Timer::Receive(ProcessId(0)));
    /// assert_eq!(timers.pop_due(1_099_999_999), None);
    /// assert_eq!(timers.pop_due(1_100_000_000), Some(Timer::Receive(ProcessId(0))));
    /// ```
    pub(super) fn pop_due(&mut self, now: i64) -> Option<Timer> {
        let (&key, _) = self.queue.first_key_value()?;
        if key.0 > now / UNITS_PER_TICK {
            return None;
        }
        self.remove(key)
    }

    /// Return the time from `now` until the first timer is due, or `None`
    /// when no timer is active. A timer that is due gives zero.
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// timers.start(1100, Timer::Receive(ProcessId(0)));
    /// assert_eq!(timers.wait_time(1_099_000_000), Some(Duration::from_millis(1)));
    /// assert_eq!(timers.wait_time(1_200_000_000), Some(Duration::ZERO));
    /// ```
    pub(super) fn wait_time(&self, now: i64) -> Option<Duration> {
        let (&(end, _), _) = self.queue.first_key_value()?;
        Some(Duration::from_nanos(
            u64::try_from(end * UNITS_PER_TICK - now).unwrap_or(0),
        ))
    }
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use super::{MONOTONIC_END_TICKS, Receiver, Timer, Timers};
    use crate::term::{ProcessId, Reference, test_atom};

    fn bif(reference: u64, receiver: Receiver) -> Timer {
        Timer::Bif {
            reference: Reference(reference),
            receiver,
            message: test_atom("x"),
        }
    }

    #[test]
    fn gives_the_end_tick_of_a_relative_timeout_as_parse_timeout_pos() {
        // At the run start, `now - 1` is in the tick before 1000 ms.
        assert_eq!(Timers::timeout_position(1_000_000_000, 100), Some(1100));
        assert_eq!(Timers::timeout_position(1_000_000_000, 0), Some(1000));
        // Past a tick boundary, the end moves to the next tick.
        assert_eq!(Timers::timeout_position(1_000_000_001, 100), Some(1101));
        assert_eq!(Timers::timeout_position(1_000_999_999, 100), Some(1101));
        assert_eq!(Timers::timeout_position(1_000_000_000, -1), None);
        // ERTS_MONOTONIC_END is 9223372036 seconds.
        assert_eq!(MONOTONIC_END_TICKS, 9_223_372_036_000);
        assert_eq!(
            Timers::timeout_position(1_000_000_000, 9_223_372_035_000),
            Some(MONOTONIC_END_TICKS)
        );
        assert_eq!(
            Timers::timeout_position(1_000_000_000, 9_223_372_035_001),
            None
        );
        assert_eq!(Timers::timeout_position(1_000_000_000, i64::MAX), None);
        assert_eq!(Timers::timeout_position(i64::MAX, i64::MAX - 1), None);
    }

    #[test]
    fn expires_timers_in_end_order_and_equal_ends_in_start_order() {
        let mut timers = Timers::new();
        assert_eq!(timers.wait_time(1_000_000_000), None);
        assert_eq!(timers.pop_due(i64::MAX), None);
        assert_eq!(timers.start(1101, Timer::Receive(ProcessId(0))), (1101, 0));
        timers.start(1100, Timer::Receive(ProcessId(1)));
        timers.start(1100, Timer::Receive(ProcessId(2)));
        let canceled = timers.start(1100, Timer::Receive(ProcessId(3)));
        timers.cancel(canceled);

        assert_eq!(
            timers.wait_time(1_099_000_001),
            Some(Duration::from_nanos(999_999))
        );
        assert_eq!(timers.pop_due(1_099_999_999), None);
        assert_eq!(
            timers.pop_due(1_100_999_999),
            Some(Timer::Receive(ProcessId(1)))
        );
        assert_eq!(
            timers.pop_due(1_100_999_999),
            Some(Timer::Receive(ProcessId(2)))
        );
        assert_eq!(timers.pop_due(1_100_999_999), None);
        assert_eq!(timers.wait_time(1_200_000_000), Some(Duration::ZERO));
        assert_eq!(
            timers.pop_due(1_101_000_000),
            Some(Timer::Receive(ProcessId(0)))
        );
        assert_eq!(timers.wait_time(1_200_000_000), None);
    }

    #[test]
    fn keeps_the_bif_timer_index_in_sync_with_the_queue() {
        let mut timers = Timers::new();
        timers.start(1100, bif(0, Receiver::Process(ProcessId(0))));
        timers.start(1100, bif(1, Receiver::Process(ProcessId(0))));
        let canceled = timers.start(1200, bif(2, Receiver::Process(ProcessId(0))));
        timers.cancel(canceled);
        assert_eq!(timers.cancel_bif(Reference(2)), None);
        assert_eq!(
            timers.cancel_bif(Reference(1)),
            Some((1100, bif(1, Receiver::Process(ProcessId(0)))))
        );
        assert_eq!(timers.cancel_bif(Reference(1)), None);
        assert_eq!(
            timers.pop_due(1_100_000_000),
            Some(bif(0, Receiver::Process(ProcessId(0))))
        );
        assert_eq!(timers.cancel_bif(Reference(0)), None);
        assert!(timers.bif_timers.is_empty());
        assert_eq!(timers.wait_time(1_000_000_000), None);
    }

    #[test]
    fn gives_the_time_left_as_get_time_left() {
        assert_eq!(Timers::time_left(1_000_000_000, 1100), 100);
        assert_eq!(Timers::time_left(1_000_000_001, 1100), 99);
        assert_eq!(Timers::time_left(1_099_000_000, 1100), 1);
        assert_eq!(Timers::time_left(1_099_000_001, 1100), 0);
        assert_eq!(Timers::time_left(1_200_000_000, 1100), 0);
    }

    #[test]
    fn reads_the_run_clock_from_one_second() {
        let timers = Timers::new();
        let first = timers.now();
        assert!(first >= 1_000_000_000);
        assert!(timers.now() >= first);
    }
}

//! The timer service of the run, as the ERTS timer wheel (`time.c`) and the
//! high-level timers (`erl_hl_timer.c`) of one scheduler. All times are on
//! the ERTS internal monotonic time: nanoseconds, one second at the run
//! start. A timer end is a clock tick of 1 ms.

use std::{
    collections::{BTreeMap, HashMap},
    sync::{Arc, RwLock, RwLockReadGuard},
    time::{Duration, Instant},
};

use super::time::MONOTONIC_BEGIN;
use std::cell::Cell;

/// The reductions after which a scheduler reads the clock again, as
/// `ERTS_CHECK_TIME_REDS` (`erl_process.c:59`).
const CHECK_TIME_REDS: i32 = 4000;
use crate::runtime::host::clock;

mod correction;

use crate::term::{Atom, LiveBytes, PortId, ProcessId, Reference, Term};
use correction::{Correction, HOST_CHECK};

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

/// A port timer: the scheduler whose timer wheel holds it, and its key
/// there. A port of any scheduler holds it, so the scheduler and the key
/// together name the timer in the node, as the `port->common.timer`
/// pointer of ERTS does (`erl_hl_timer.c:708-720`).
pub(super) type PortTimer = (usize, TimerKey);

/// What a timer does when it expires.
#[derive(Debug, PartialEq)]
pub(super) enum Timer {
    /// The receive timer of a process, which `wait_timeout` starts.
    Receive(ProcessId),
    /// A BIF timer, which `erlang:start_timer/3` or `erlang:send_after/3` starts. It sends `message`
    /// to `receiver` when it expires (`bif_timer_timeout`,
    /// `erl_hl_timer.c:1146-1203`).
    Bif {
        /// The timer reference that the BIF returns.
        reference: Reference,
        /// The process that gets the message.
        receiver: Receiver,
        /// The message, `{timeout, Reference, Msg}` of `start_timer/3` or
        /// `Msg` of `send_after/3`, which `setup_bif_timer` makes at the
        /// start (`erl_hl_timer.c:1824`).
        message: Term,
    },
    /// The timer of an accept of `caller` on the listen port `port`, which
    /// `INET_REQ_ACCEPT` starts with a timeout (`driver_set_timer` and
    /// `add_multi_timer`, inet_drv.c:12180-12230).
    Accept {
        /// The listen port.
        port: PortId,
        /// The process that waits in the accept queue.
        caller: ProcessId,
        /// The key of this timer, which the accept holds while it waits.
        key: TimerKey,
    },
    /// The timer of the connect of the port `port`, which `INET_REQ_CONNECT`
    /// starts with a timeout (`driver_set_timer`, inet_drv.c:12111-12112).
    Connect {
        /// The port that connects.
        port: PortId,
        /// The key of this timer, which the connect holds while it waits.
        key: TimerKey,
    },
    /// The timer of the receive on the connected port `port`, which
    /// `TCP_REQ_RECV` starts with a timeout (`add_multi_timer`,
    /// inet_drv.c:12313-12315).
    SocketReceive {
        /// The connected port.
        port: PortId,
        /// The key of this timer, which the receive holds while it waits.
        key: TimerKey,
    },
    /// The send timer of the port `port`, which a send that made the port
    /// busy starts for a `send_timeout` other than `infinity` and 0
    /// (`add_multi_timer`, inet_drv.c:13785-13792).
    SocketSend {
        /// The port with the queue that the send waits for.
        port: PortId,
        /// The key of this timer, which the send holds while it waits.
        key: TimerKey,
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

/// The time support of the node: the run start and one time correction
/// state that all schedulers share, as ERTS keeps one `time_sup` for the
/// node (`erl_time_sup.c:385-412`, `1305-1322`).
#[derive(Clone, Debug)]
pub(super) struct NodeClock {
    /// The run start, at the internal monotonic time `MONOTONIC_BEGIN`.
    start: Instant,
    shared: Arc<RwLock<Shared>>,
}

/// The time correction state of the node and the end tick of its one
/// check timer, the ERTS `parmon.timer` (`erl_time_sup.c:939-946`).
#[derive(Debug)]
struct Shared {
    correction: Correction,
    check: i64,
}

impl NodeClock {
    /// Return the time support of a node that starts now. The time offset
    /// and the time correction start as `erts_init_time_sup` sets them
    /// (`erl_time_sup.c:1240-1270`), and `late_init_time_correction` sets
    /// the one check timer (`erl_time_sup.c:913-950`, `1305-1322`).
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// let clock = NodeClock::new();
    /// assert!(clock.now() >= 1_000_000_000);
    /// ```
    pub(super) fn new() -> Self {
        let start = Instant::now();
        let correction = Correction::new(HOST_CHECK, clock::system_time());
        let now = correction.monotonic_time(os_monotonic_time(start));
        let check = Timers::check_position(now, correction.first_timeout());
        Self {
            start,
            shared: Arc::new(RwLock::new(Shared { correction, check })),
        }
    }

    fn read(&self) -> RwLockReadGuard<'_, Shared> {
        self.shared.read().expect("the clock lock is not poisoned")
    }

    /// Return the internal monotonic time. The OS time is read under the
    /// read lock of the correction state, as `read_corrected_time` does
    /// (`erl_time_sup.c:366-389`), so no check runs between the read and
    /// the selection of the correction instance.
    fn now(&self) -> i64 {
        let shared = self.read();
        shared
            .correction
            .monotonic_time(os_monotonic_time(self.start))
    }
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
    /// The time support of the node: the run start, the time offset, and
    /// the time correction.
    clock: NodeClock,
    /// The key of the time correction check in this timer service. ERTS
    /// has one check timer, `parmon.timer` (`erl_time_sup.c:772-776`,
    /// `827-831`, `944-950`). Each scheduler holds the end tick of that
    /// timer, and the first scheduler at that tick does the check. It is
    /// not in `queue`, so it does not keep a run alive.
    check: TimerKey,
    /// Active timers by key.
    queue: BTreeMap<TimerKey, Timer>,
    /// The key of each active BIF timer by its reference, as the ERTS
    /// `btm_tree` of the timer service (`erl_hl_timer.c:312`).
    bif_timers: HashMap<Reference, TimerKey>,
    /// Start number of the next timer.
    next: u64,
    /// The last time that this scheduler read, as
    /// `esdp->last_monotonic_time` (`erl_time_sup.c:279`).
    last: Cell<i64>,
    /// The reductions that processes used after the last read, as
    /// `esdp->check_time_reds` (`erl_process.h:535`, `erl_time_sup.c:280`).
    check_reds: Cell<i32>,
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
        Self::with_clock(NodeClock::new())
    }

    /// Return a timer service on the time support `clock` of the node.
    /// Each scheduler of a run with scheduler threads gets the same
    /// support, so `erlang:monotonic_time/0,1` and `erlang:time_offset/0,1`
    /// agree across the threads.
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// let timers = Timers::with_clock(node.clock());
    /// assert!(timers.now() >= 1_000_000_000);
    /// ```
    pub(super) fn with_clock(clock: NodeClock) -> Self {
        let check = clock.read().check;
        Self {
            clock,
            check: (check, 0),
            queue: BTreeMap::new(),
            bif_timers: HashMap::new(),
            next: 1,
            last: Cell::new(0),
            check_reds: Cell::new(0),
        }
    }

    /// Count the guest terms that the timer service holds: the message of
    /// each active BIF timer. A receive timer holds no term.
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// let mut timers = Timers::new();
    /// timers.start(1100, Timer::Bif {
    ///     reference: Reference(1),
    ///     receiver: Receiver::Process(ProcessId(0)),
    ///     message: Term::tuple(vec![Term::Integer(1)]),
    /// });
    /// let mut live = LiveBytes::new();
    /// timers.account_terms(&mut live);
    /// assert!(live.bytes() > 0);
    /// ```
    pub(super) fn account_terms(&self, live: &mut LiveBytes) {
        for timer in self.queue.values() {
            if let Timer::Bif { message, .. } = timer {
                live.term(message);
            }
        }
    }

    /// Return the internal monotonic time of the run: the Erlang monotonic
    /// time from the OS monotonic time with the time correction of the run,
    /// as `erts_get_monotonic_time` (`erl_time_sup.c:385-412`, `780-787`).
    /// `Instant` reads `CLOCK_UPTIME_RAW` on macOS, which is the
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
        let now = self.clock.now();
        self.last.set(now);
        self.check_reds.set(0);
        now
    }

    /// Add the actual reductions `reductions` of a turn, as
    /// `ERTS_PROC_REDUCTIONS_EXECUTED` does (`erl_process.h:529-536`).
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// timers.add_reductions(4000);
    /// ```
    pub(super) fn add_reductions(&self, reductions: i32) {
        self.check_reds
            .set(self.check_reds.get().saturating_add(reductions));
    }

    /// Return the time at which a scheduler expires its timers before it
    /// takes the next process. It reads the clock only when processes used
    /// `ERTS_CHECK_TIME_REDS` (`CONTEXT_REDS`, 4000) reductions after the
    /// last read; otherwise it gives the last time read
    /// (`erl_process.c:59`, `9803-9810`).
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// let now = timers.now();
    /// assert_eq!(timers.timer_time(), now);
    /// ```
    pub(super) fn timer_time(&self) -> i64 {
        if self.check_reds.get() >= CHECK_TIME_REDS {
            self.now()
        } else {
            self.last.get()
        }
    }

    /// Return the time offset in nanoseconds, as `erts_get_time_offset`
    /// reads it (`erl_time_sup.c`).
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// let timers = Timers::new();
    /// assert_eq!(timers.time_offset(), timers.time_offset());
    /// ```
    pub(super) fn time_offset(&self) -> i64 {
        self.clock.read().correction.time_offset()
    }

    /// Return the Erlang system time of the run in nanoseconds: the
    /// internal monotonic time plus the time offset, as `system_time_1`
    /// adds them (`erl_time_sup.c:2527-2535`). The call reads the offset
    /// and does not change it.
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// let timers = Timers::new();
    /// // 2023-11-14 and later.
    /// assert!(timers.system_time() > 1_700_000_000_000_000_000);
    /// ```
    pub(super) fn system_time(&self) -> i64 {
        let shared = self.clock.read();
        let now = shared
            .correction
            .monotonic_time(os_monotonic_time(self.clock.start));
        now.saturating_add(shared.correction.time_offset())
    }

    /// Do the time correction check with the OS clocks from `os_times`, and
    /// set the next check timer, as `check_time_offset` and
    /// `check_time_correction` do at their end (`erl_time_sup.c:772-776`,
    /// `824-831`). The check is done once for the node: when another
    /// scheduler did it first, the service takes the next check tick of
    /// the node. The next check starts after each timer that is active, so
    /// it has the next start number.
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// timers.check_time(|| (1_500_000_000, 1_500_000_000));
    /// ```
    fn check_time(&mut self, os_times: impl FnMut() -> (i64, i64)) {
        let mut shared = self
            .clock
            .shared
            .write()
            .expect("the clock lock is not poisoned");
        if shared.check <= self.check.0 {
            let (now, msec) = shared.correction.check(os_times);
            shared.check = Self::check_position(now, msec);
        }
        self.check = (shared.check, self.next);
        self.next += 1;
    }

    /// Return the end tick of the check timer `msec` milliseconds after the
    /// time `now`, as `get_timeout_pos` (`erl_time_sup.c:440-448`).
    fn check_position(now: i64, msec: i64) -> i64 {
        Self::timeout_position(now, msec).unwrap_or(MONOTONIC_END_TICKS)
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

    /// Return the end tick of an absolute timer of `msec` milliseconds in
    /// Erlang monotonic time, as the absolute branch of `parse_timeout_pos`
    /// (`erl_hl_timer.c:2494-2554`) does: the internal time is the Erlang
    /// time minus `ERTS_MONOTONIC_OFFSET_MSEC`, and a time before
    /// `ERTS_MONOTONIC_BEGIN` or after `ERTS_MONOTONIC_END` is `None`.
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// assert_eq!(Timers::absolute_position(-576_460_752_000_000), Some(1000));
    /// assert_eq!(Timers::absolute_position(-576_460_753_000_000), None);
    /// ```
    pub(super) fn absolute_position(msec: i64) -> Option<i64> {
        let offset = crate::runtime::time::MONOTONIC_OFFSET_NATIVE / UNITS_PER_TICK;
        let end = msec.checked_sub(offset)?;
        (MONOTONIC_BEGIN / UNITS_PER_TICK..=MONOTONIC_END_TICKS)
            .contains(&end)
            .then_some(end)
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

    /// Add the timer that `timer` makes from its own key, which expires at
    /// the end tick `end`. Return its key.
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// let key = timers.start_keyed(1100, |key| Timer::SocketReceive { port, key });
    /// ```
    pub(super) fn start_keyed(
        &mut self,
        end: i64,
        timer: impl FnOnce(TimerKey) -> Timer,
    ) -> TimerKey {
        self.start(end, timer((end, self.next)))
    }

    /// Remove the timer with the key `key`, as `erts_cancel_proc_timer`
    /// (`erl_hl_timer.c:2832-2846`) cancels a receive timer.
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// let key = timers.start(1100, Timer::Receive(ProcessId(0)));
    /// timers.cancel(key);
    /// assert!(!timers.active());
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
    /// (`time.c:820`). Before it, do each time correction check that is due
    /// and comes first in end and start order, with the OS clocks of now.
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// timers.start(1100, Timer::Receive(ProcessId(0)));
    /// assert_eq!(timers.pop_due(1_099_999_999), None);
    /// assert_eq!(timers.pop_due(1_100_000_000), Some(Timer::Receive(ProcessId(0))));
    /// ```
    pub(super) fn pop_due(&mut self, now: i64) -> Option<Timer> {
        let start = self.clock.start;
        self.pop_due_with(now, || (os_monotonic_time(start), clock::system_time()))
    }

    /// Do [`Self::pop_due`] with the OS clocks from `os_times`, which gives
    /// the OS monotonic time and the OS system time.
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// assert_eq!(timers.pop_due_with(2_500_000_000, || (1_500_000_000, os_stime)), None);
    /// ```
    fn pop_due_with(
        &mut self,
        now: i64,
        mut os_times: impl FnMut() -> (i64, i64),
    ) -> Option<Timer> {
        let tick = now / UNITS_PER_TICK;
        while self.check.0 <= tick
            && self
                .queue
                .first_key_value()
                .is_none_or(|(key, _)| self.check < *key)
        {
            self.check_time(&mut os_times);
        }
        let (&key, _) = self.queue.first_key_value()?;
        if key.0 > tick {
            return None;
        }
        self.remove(key)
    }

    /// Return true when a timer other than the time correction check is
    /// active. Only such a timer keeps an idle run alive.
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// assert!(!Timers::new().active());
    /// ```
    pub(super) fn active(&self) -> bool {
        !self.queue.is_empty()
    }

    /// Return the time from `now` until the first timer or the time
    /// correction check is due. A timer that is due gives zero.
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// timers.start(1100, Timer::Receive(ProcessId(0)));
    /// assert_eq!(timers.wait_time(1_099_000_000), Duration::from_millis(1));
    /// assert_eq!(timers.wait_time(1_200_000_000), Duration::ZERO);
    /// ```
    pub(super) fn wait_time(&self, now: i64) -> Duration {
        let (end, _) = self
            .queue
            .first_key_value()
            .map_or(self.check, |(&key, _)| key.min(self.check));
        Duration::from_nanos(u64::try_from(end * UNITS_PER_TICK - now).unwrap_or(0))
    }
}

/// Return the OS monotonic time of the run: the nanoseconds since the run
/// start `start`.
fn os_monotonic_time(start: Instant) -> i64 {
    i64::try_from(start.elapsed().as_nanos()).unwrap_or(i64::MAX)
}

#[cfg(test)]
#[path = "../../tests/runtime/timer.rs"]
mod tests;

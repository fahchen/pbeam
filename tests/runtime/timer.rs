use std::time::Duration;

use super::correction::{Check, Correction};
use super::{MONOTONIC_END_TICKS, NodeClock, Receiver, Timer, Timers};
use crate::term::{ProcessId, Reference, Term, test_atom};

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
    assert!(!timers.active());
    assert_eq!(timers.pop_due(1_000_000_000), None);
    assert_eq!(timers.start(1101, Timer::Receive(ProcessId(0))), (1101, 1));
    timers.start(1100, Timer::Receive(ProcessId(1)));
    timers.start(1100, Timer::Receive(ProcessId(2)));
    let canceled = timers.start(1100, Timer::Receive(ProcessId(3)));
    timers.cancel(canceled);

    assert_eq!(
        timers.wait_time(1_099_000_001),
        Duration::from_nanos(999_999)
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
    assert_eq!(timers.wait_time(1_200_000_000), Duration::ZERO);
    assert_eq!(
        timers.pop_due(1_101_000_000),
        Some(Timer::Receive(ProcessId(0)))
    );
    assert!(!timers.active());
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
    assert!(!timers.active());
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

#[test]
fn counts_the_message_of_an_active_bif_timer() {
    let mut timers = Timers::new();
    timers.start(
        1100,
        Timer::Bif {
            reference: Reference(1),
            receiver: Receiver::Process(ProcessId(0)),
            message: Term::tuple(vec![Term::Integer(1)]),
        },
    );
    let mut live = crate::term::LiveBytes::new();
    timers.account_terms(&mut live);
    assert!(live.bytes() > 0);
}

#[test]
fn sets_the_first_check_timer_at_the_start_as_late_init_time_correction() {
    let timers = Timers::new();
    let first = timers.clock.read().correction.first_timeout();
    // The check is after the first timeout from the start time, which is
    // 1000000000 or later. The upper bound allows the thread to be
    // preempted between the clock reads of `Timers::new` on a loaded host.
    assert!((1000 + first..=2000 + first).contains(&timers.check.0));
    assert_eq!(timers.check.1, 0);
    // The check timer alone is not active, so it keeps no run alive, but
    // a wait ends at it.
    assert!(!timers.active());
    assert_eq!(
        timers.wait_time(1_000_000_000),
        Duration::from_millis((timers.check.0 - 1000) as u64)
    );
}

#[test]
fn does_the_check_in_timer_order_and_reads_the_os_clocks_only_at_the_check() {
    const SINIT: i64 = 1_700_000_000_000_000_000;
    let hour = 3_600_000_000_000;
    let mut timers = Timers::new();
    timers.clock.shared.write().unwrap().correction = Correction::new(Check::Offset, SINIT);
    timers.check = (Timers::check_position(1_000_000_001, 1500), 0);
    timers.clock.shared.write().unwrap().check = timers.check.0;
    let offset = timers.time_offset();
    assert_eq!(timers.check, (2501, 0));
    timers.start(2000, Timer::Receive(ProcessId(0)));
    timers.start(2501, Timer::Receive(ProcessId(1)));
    // A timer before the check expires with no clock reading.
    assert_eq!(
        timers.pop_due_with(2_501_000_000, || unreachable!()),
        Some(Timer::Receive(ProcessId(0)))
    );
    // The check started before the timer with the same end, so it runs
    // first. It sees the step of one hour and sets the next check 60 s later.
    let m = 1_501_000_000;
    assert_eq!(
        timers.pop_due_with(2_501_000_000, || (m, SINIT + m + hour)),
        Some(Timer::Receive(ProcessId(1)))
    );
    assert_eq!(timers.time_offset(), offset + hour);
    assert_eq!(timers.check, (62_501, 3));
    assert_eq!(timers.wait_time(2_501_000_000), Duration::from_secs(60));
    // A step between checks is not seen before the next check.
    assert_eq!(timers.pop_due_with(62_500_999_999, || unreachable!()), None);
    assert_eq!(timers.time_offset(), offset + hour);
    let m = 61_501_000_000;
    assert_eq!(timers.pop_due_with(62_501_000_000, || (m, SINIT + m)), None);
    assert_eq!(timers.time_offset(), offset);
    assert_eq!(timers.check, (122_501, 4));
    assert!(!timers.active());
}

#[test]
fn does_the_check_once_for_the_schedulers_of_the_node() {
    const SINIT: i64 = 1_700_000_000_000_000_000;
    let hour = 3_600_000_000_000;
    let clock = NodeClock::new();
    clock.shared.write().unwrap().correction = Correction::new(Check::Offset, SINIT);
    clock.shared.write().unwrap().check = 2501;
    let mut first = Timers::with_clock(clock.clone());
    let mut second = Timers::with_clock(clock);
    assert_eq!((first.check, second.check), ((2501, 0), (2501, 0)));
    let offset = first.time_offset();
    // The first scheduler at the check tick does the check for the node.
    let m = 1_501_000_000;
    assert_eq!(
        first.pop_due_with(2_501_000_000, || (m, SINIT + m + hour)),
        None
    );
    assert_eq!(first.check, (62_501, 1));
    // The other scheduler sees the new offset and takes the next check
    // tick of the node with no clock reading.
    assert_eq!(second.time_offset(), offset + hour);
    assert_eq!(second.pop_due_with(2_501_000_000, || unreachable!()), None);
    assert_eq!(second.check, (62_501, 1));
}

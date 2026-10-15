use std::time::Duration;

use super::{MONOTONIC_END_TICKS, Receiver, Timer, Timers};
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
fn moves_the_time_offset_to_the_os_system_time_at_a_due_check_as_check_time_offset() {
    let mut timers = Timers::new();
    let offset = timers.time_offset;
    let hour = 3_600_000_000_000;
    // No check before 1.5 s: the call reads the existing offset.
    let early = 2_499_999_999;
    assert_eq!(
        timers.correct_time_offset(early, || unreachable!()),
        early + offset
    );
    // The first due check sees a step forward and sets the offset.
    let first = 2_500_000_000;
    assert_eq!(
        timers.correct_time_offset(first, || first + offset + hour),
        first + offset + hour
    );
    let offset = offset + hour;
    assert_eq!(timers.time_offset, offset);
    // After a change, the next check is 15 s later.
    let short = first + 15_000_000_000;
    assert_eq!(
        timers.correct_time_offset(short - 1, || unreachable!()),
        short - 1 + offset
    );
    // A drift of 2 * small_diff (100 us) keeps the offset.
    assert_eq!(
        timers.correct_time_offset(short, || short + offset + 100_000),
        short + offset
    );
    // After a check with no change, the next check is 60 s later.
    let long = short + 60_000_000_000;
    assert_eq!(
        timers.correct_time_offset(long - 1, || unreachable!()),
        long - 1 + offset
    );
    // A late call does one check: a step backward past the limit.
    let late = long + 10 * 60_000_000_000;
    assert_eq!(
        timers.correct_time_offset(late, || late + offset - 100_001),
        late + offset - 100_001
    );
    assert_eq!(timers.time_offset, offset - 100_001);
    assert_eq!(timers.next_check, late + 15_000_000_000);
}

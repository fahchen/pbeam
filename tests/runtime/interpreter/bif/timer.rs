//! Unit tests of `src/runtime/interpreter/bif/timer.rs` for the time
//! conversion of the absolute timer options and the timer lookup that a
//! compiled fixture also drives.

use super::*;
use crate::runtime::timer::Timers;
use crate::term::Reference;

#[test]
fn gives_the_absolute_position_of_pinned_erts() {
    // `parse_timeout_pos` takes an absolute time in Erlang monotonic
    // milliseconds and rejects a time before `ERTS_MONOTONIC_BEGIN`
    // (`erl_hl_timer.c:2494-2554`). The Erlang monotonic time of the run
    // start is -576460752000 ms, as `erlang:monotonic_time(millisecond)`
    // gives it, which is the internal time 1000 ms.
    assert_eq!(Timers::absolute_position(-576_460_752_000), Some(1000));
    assert_eq!(Timers::absolute_position(-576_460_753_000), None);
    assert_eq!(Timers::absolute_position(i64::MAX), None);
    assert_eq!(Timers::absolute_position(i64::MIN), None);
}

#[test]
fn reads_the_end_of_no_timer_of_a_new_service() {
    let timers = Timers::new();
    assert_eq!(timers.bif_timer_end(Reference::ordinary(0).unwrap()), None);
}

#[test]
fn parses_the_abs_option_as_pinned_erts() {
    let list = |terms: Vec<Term>| Term::list_from(terms);
    let abs = |value: &str| Term::tuple(vec![Term::static_atom("abs"), Term::static_atom(value)]);
    assert_eq!(absolute_option(&Term::nil()), Some(false));
    assert_eq!(absolute_option(&list(vec![abs("true")])), Some(true));
    assert_eq!(
        absolute_option(&list(vec![abs("true"), abs("false")])),
        Some(false)
    );
    assert_eq!(absolute_option(&Term::static_atom("abs")), None);
    assert_eq!(absolute_option(&list(vec![abs("yes")])), None);
    assert_eq!(absolute_option(&list(vec![abs("true"), Term::nil()])), None);
    // `start_timer/4` and `send_after/4` pass no pointer for `async` and
    // `info`, so those options are `badarg` (`erl_hl_timer.c:2369-2387`).
    assert_eq!(
        absolute_option(&list(vec![Term::tuple(vec![
            Term::static_atom("async"),
            Term::static_atom("true"),
        ])])),
        None
    );
}

mod common;

use std::{
    process::Command,
    time::{Duration, Instant},
};

use common::pinned_beam;
use pbeam_runtime::{BeamProgram, Term};

const TIMER_PROBE: &[u8] = include_bytes!("../fixtures/timer/timer_probe.beam");

#[test]
fn runs_receive_timeouts_as_pinned_erts() {
    // `receive_after/0` waits for a receive timer of 100 ms, then gets a
    // message before its timer of 1000 ms, then waits for a timer of 110 ms
    // that a message that does not match does not stop. `remove_message`
    // stops the timer of 1000 ms (`jit/beam_jit_common.cpp:1168`); else the
    // next `wait_timeout` waits until it expires, and the run takes more
    // than 1000 ms. ERTS takes about 255 ms.
    let erlang = pinned_beam("erlang");
    let program = BeamProgram::load(&[&erlang, TIMER_PROBE]).unwrap();
    let atom = |name| Term::Atom(program.atom(name).unwrap());
    let timeout_value = Term::Tuple(vec![atom("error"), atom("timeout_value")]);

    let start = Instant::now();
    let receive_after = program
        .execute("timer_probe", "receive_after", &[])
        .unwrap();
    let elapsed = start.elapsed();
    assert_eq!(
        receive_after,
        Term::List(vec![
            atom("timeout"),
            atom("early"),
            atom("timeout"),
            atom("other"),
            atom("late"),
            atom("none"),
        ])
    );
    assert!(
        elapsed >= Duration::from_millis(210) && elapsed < Duration::from_millis(1000),
        "receive_after/0 took {elapsed:?}"
    );

    // `wait_timeout` raises `error:timeout_value` for a negative integer, an
    // integer above 4294967295, and a term that is not an integer or
    // `infinity` (`erl_hl_timer.c:2786-2805`, `error.h:154`).
    let mut timeout_values = vec![atom("timeout")];
    timeout_values.extend(std::iter::repeat_n(timeout_value, 7));
    assert_eq!(
        program
            .execute("timer_probe", "timeout_values", &[])
            .unwrap(),
        Term::List(timeout_values)
    );

    // A timer of 4294967295 ms and `infinity` both wait for a message.
    assert_eq!(
        program
            .execute("timer_probe", "receive_limits", &[])
            .unwrap(),
        Term::List(vec![atom("ok"), atom("ok"), atom("other"), atom("none")])
    );

    let reference = Command::new("erl")
        .args([
            "-noshell",
            "-pa",
            "fixtures/timer",
            "-eval",
            "io:format(\"~w~n\", [[timer_probe:F() || F <- [receive_after, timeout_values, receive_limits]]]), halt().",
        ])
        .output()
        .unwrap();
    assert!(reference.status.success());
    assert!(reference.stderr.is_empty());
    assert_eq!(
        reference.stdout,
        b"[[timeout,early,timeout,other,late,none],[timeout,{error,timeout_value},{error,timeout_value},{error,timeout_value},{error,timeout_value},{error,timeout_value},{error,timeout_value},{error,timeout_value}],[ok,ok,other,none]]\n"
    );
}

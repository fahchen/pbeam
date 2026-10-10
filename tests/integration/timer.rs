#[path = "../common/mod.rs"]
mod common;

use std::{
    process::Command,
    time::{Duration, Instant},
};

use common::pinned_beam;
use pbeam_runtime::{BeamProgram, Term};

const TIMER_PROBE: &[u8] = include_bytes!("../../fixtures/timer/timer_probe.beam");

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

#[test]
fn runs_erlang_start_timer_and_cancel_timer_as_pinned_erts() {
    let erlang = pinned_beam("erlang");
    let program = BeamProgram::load(&[&erlang, TIMER_PROBE]).unwrap();
    let atom = |name| Term::Atom(program.atom(name).unwrap());
    let badargs = |count| {
        Term::List(vec![
            Term::Tuple(vec![atom("error"), atom("badarg")]);
            count
        ])
    };
    let run = |function| program.execute("timer_probe", function, &[]).unwrap();

    // The timer of 30 ms sends `{timeout, R2, b}` before the timer of 60 ms
    // sends `{timeout, R1, a}`. `start_timer_3` raises `badarg` for a time
    // that is not a non-negative integer and `setup_bif_timer` for a
    // receiver that is not a pid or an atom (`erl_hl_timer.c:2605-2620`,
    // `1814-1815`).
    assert_eq!(
        run("start_timer"),
        Term::List(vec![atom("true"), atom("true"), atom("none"), badargs(7)])
    );

    // `cancel_timer/2` gives the time left, `false`, or `ok`, and sends
    // `{cancel_timer, Ref, Result}` with `async` (`erl_hl_timer.c:2078-2147`).
    // A repeated option takes the last value (`erl_hl_timer.c:2364-2394`).
    // The canceled timers send no message. Each option that is not
    // `{async, Bool}` or `{info, Bool}` and each term that is not a reference
    // raise `badarg` (`erl_hl_timer.c:2237-2398`).
    assert_eq!(
        run("cancel_timer"),
        Term::List(vec![
            atom("true"),
            atom("false"),
            atom("ok"),
            atom("ok"),
            atom("ok"),
            atom("true"),
            atom("ok"),
            atom("false"),
            atom("ok"),
            atom("false"),
            atom("false"),
            atom("false"),
            atom("none"),
            badargs(11),
        ])
    );

    // A pid that is not alive gets no timer, and an exit cancels the timers
    // to its pid (`erl_hl_timer.c:1832-1858`, `erl_process.c:14300-14303`).
    // A name timer finds the process with the name when it expires, and a
    // name that no process has drops the message (`erl_hl_timer.c:1165-1171`).
    assert_eq!(
        run("receiver_exit"),
        Term::List(vec![
            atom("false"),
            atom("false"),
            atom("named"),
            atom("none")
        ])
    );

    // Timers with the same end expire in start order (`time.c:647-668`).
    let order = [1, 2, 3, 4, 40, 50, 60].map(Term::Integer).to_vec();
    assert_eq!(
        run("order"),
        Term::List(vec![Term::List(order), atom("recv"), atom("bif")])
    );

    // An end after `ERTS_MONOTONIC_END` and a 64-bit overflow raise `badarg`
    // (`erl_hl_timer.c:2549-2552`).
    let badarg = Term::Tuple(vec![atom("error"), atom("badarg")]);
    assert_eq!(
        run("timer_limits"),
        Term::List(vec![atom("ok"), atom("ok"), badarg.clone(), badarg])
    );

    let reference = Command::new("erl")
        .args([
            "-noshell",
            "-pa",
            "fixtures/timer",
            "-eval",
            "io:format(\"~w~n\", [[timer_probe:F() || F <- [start_timer, cancel_timer, receiver_exit, order, timer_limits]]]), halt().",
        ])
        .output()
        .unwrap();
    assert!(reference.status.success());
    assert!(reference.stderr.is_empty());
    assert_eq!(
        reference.stdout,
        b"[[true,true,none,[{error,badarg},{error,badarg},{error,badarg},{error,badarg},{error,badarg},{error,badarg},{error,badarg}]],[true,false,ok,ok,ok,true,ok,false,ok,false,false,false,none,[{error,badarg},{error,badarg},{error,badarg},{error,badarg},{error,badarg},{error,badarg},{error,badarg},{error,badarg},{error,badarg},{error,badarg},{error,badarg}]],[false,false,named,none],[[1,2,3,4,40,50,60],recv,bif],[ok,ok,{error,badarg},{error,badarg}]]\n"
    );
}

//! The dirty NIF calls of `fixtures/dirty_nif/dirty_nif_probe.erl`. Rust
//! native functions with the jobs `CpuBound` and `IoBound` run on the dirty
//! pools of the node, while their callers switch out. The expected values
//! of `signals_with/3` and `kill_with/3` are the values of the pinned ERTS,
//! which runs the same functions with `erts_debug:dirty_cpu(wait, 300)` as
//! the dirty call:
//!
//! ```text
//! mise run erts-eval --pa fixtures/dirty_nif 'C = fun() -> erts_debug:dirty_cpu(wait, 300) end,
//!   S = fun() -> timer:sleep(50) end, R = fun() -> ok end,
//!   {dirty_nif_probe:signals_with(C, S, R), dirty_nif_probe:kill_with(C, S, R)}'
//! {{ok,{message_queue_len,0},{message_queue_len,1},{status,running},true,ok},
//!  {killed,killed,no_late}}
//! ```

#[path = "../common/mod.rs"]
mod common;

use std::{
    collections::HashMap,
    process::Command,
    sync::{
        Condvar, Mutex, Once,
        atomic::{AtomicI64, Ordering},
    },
    thread,
    time::Duration,
};

use common::pinned_beam;
use pbeam_runtime::{
    Atom, BeamError, BeamProgram, Binding, Job, Library, NativeResult, NifContext, Term,
    register_library,
};

const DIRTY_NIF_PROBE: &[u8] = include_bytes!("../../fixtures/dirty_nif/dirty_nif_probe.beam");

/// The state of each key of `cpu_wait/1`: whether the call started, and
/// whether the key is released.
type Gates = Option<HashMap<i64, (bool, bool)>>;

/// The gates of the keys and the condition that wakes their waiters.
static GATES: (Mutex<Gates>, Condvar) = (Mutex::new(None), Condvar::new());

/// Change the state of `key` with `change`, wake each waiter, and return
/// the new state.
fn update(key: i64, change: impl FnOnce(&mut (bool, bool))) -> (bool, bool) {
    let mut gates = GATES.0.lock().unwrap();
    let gate = gates.get_or_insert_default().entry(key).or_default();
    change(gate);
    let state = *gate;
    GATES.1.notify_all();
    state
}

/// Return the static atom `name`.
fn static_atom(name: &str) -> Term {
    Term::Atom(Atom::from_static(name).unwrap())
}

/// Return the key of the first argument.
fn key(arguments: &[Term]) -> i64 {
    match arguments.first() {
        Some(Term::Integer(key)) => *key,
        other => panic!("the key is an integer: {other:?}"),
    }
}

/// Return the name of the current thread as a binary.
#[expect(
    clippy::unnecessary_wraps,
    reason = "NIF function pointer; every NIF returns `Result<NativeResult, BeamError>`"
)]
fn thread_name(_: &mut NifContext<'_>, _: &[Term]) -> Result<NativeResult, BeamError> {
    let name = thread::current()
        .name()
        .unwrap_or_default()
        .as_bytes()
        .to_vec();
    Ok(NativeResult::Value(Term::binary(name)))
}

/// Mark the key as started, wait until it is released, and return `ok`.
#[expect(
    clippy::unnecessary_wraps,
    reason = "NIF function pointer; every NIF returns `Result<NativeResult, BeamError>`"
)]
fn cpu_wait(_: &mut NifContext<'_>, arguments: &[Term]) -> Result<NativeResult, BeamError> {
    let key = key(arguments);
    update(key, |gate| gate.0 = true);
    let mut gates = GATES.0.lock().unwrap();
    while !gates.as_ref().unwrap()[&key].1 {
        gates = GATES.1.wait(gates).unwrap();
    }
    Ok(NativeResult::Value(static_atom("ok")))
}

/// Return `true` when the call of the key started.
#[expect(
    clippy::unnecessary_wraps,
    reason = "NIF function pointer; every NIF returns `Result<NativeResult, BeamError>`"
)]
fn started(_: &mut NifContext<'_>, arguments: &[Term]) -> Result<NativeResult, BeamError> {
    let started = update(key(arguments), |_| {}).0;
    Ok(NativeResult::Value(static_atom(if started {
        "true"
    } else {
        "false"
    })))
}

/// Release the key.
#[expect(
    clippy::unnecessary_wraps,
    reason = "NIF function pointer; every NIF returns `Result<NativeResult, BeamError>`"
)]
fn release(_: &mut NifContext<'_>, arguments: &[Term]) -> Result<NativeResult, BeamError> {
    update(key(arguments), |gate| gate.1 = true);
    Ok(NativeResult::Value(static_atom("ok")))
}

/// Raise `error:badarg`.
#[expect(
    clippy::unnecessary_wraps,
    reason = "NIF function pointer; every NIF returns `Result<NativeResult, BeamError>`"
)]
fn cpu_raise(context: &mut NifContext<'_>, _: &[Term]) -> Result<NativeResult, BeamError> {
    Ok(context.badarg())
}

/// Return the next number of a counter of the test binary.
#[expect(
    clippy::unnecessary_wraps,
    reason = "NIF function pointer; every NIF returns `Result<NativeResult, BeamError>`"
)]
fn cpu_sequence(_: &mut NifContext<'_>, _: &[Term]) -> Result<NativeResult, BeamError> {
    static NEXT: AtomicI64 = AtomicI64::new(0);
    Ok(NativeResult::Value(Term::Integer(
        NEXT.fetch_add(1, Ordering::SeqCst),
    )))
}

/// Return the binding of `name/arity` with `job` and `run`.
const fn bind(name: &'static str, arity: u32, job: Job, run: pbeam_runtime::NativeFn) -> Binding {
    Binding {
        name,
        arity,
        job,
        run,
    }
}

/// The library of the fixture: each function of its `-nifs` list has a
/// binding.
static DIRTY_NIF_LIBRARY: Library = Library {
    module: "dirty_nif_probe",
    functions: &[
        ("cpu_thread", 0),
        ("io_thread", 0),
        ("cpu_wait", 1),
        ("started", 1),
        ("release", 1),
        ("cpu_raise", 1),
        ("cpu_sequence", 0),
    ],
    bindings: &[
        bind("cpu_thread", 0, Job::CpuBound, thread_name),
        bind("io_thread", 0, Job::IoBound, thread_name),
        bind("cpu_wait", 1, Job::CpuBound, cpu_wait),
        bind("started", 1, Job::Normal, started),
        bind("release", 1, Job::Normal, release),
        bind("cpu_raise", 1, Job::CpuBound, cpu_raise),
        bind("cpu_sequence", 0, Job::CpuBound, cpu_sequence),
    ],
    load: None,
    unbound_fails: false,
};

/// Register the library of the fixture once for the test binary, and load
/// the fixture.
fn program() -> BeamProgram {
    static REGISTER: Once = Once::new();
    REGISTER.call_once(|| register_library(&DIRTY_NIF_LIBRARY).unwrap());
    BeamProgram::load(&[
        &pinned_beam("erlang"),
        &pinned_beam("erts_internal"),
        &pinned_beam("lists"),
        DIRTY_NIF_PROBE,
    ])
    .unwrap()
}

/// Run `dirty_nif_probe:Function(Argument)` and return its value.
fn run(program: &BeamProgram, function: &str, argument: Term) -> Term {
    program
        .execute("dirty_nif_probe", function, &[program.host_term(argument)])
        .unwrap()
        .term
}

/// Return the atom `name` of the program.
fn atom(program: &BeamProgram, name: &str) -> Term {
    Term::Atom(program.atom(name).unwrap())
}

#[test]
fn runs_cpu_bound_and_io_bound_calls_on_their_dirty_pools() {
    // ERTS names its dirty threads `erts_dcpus_N` and `erts_dios_N`
    // (erl_process.c:8958, 8966). A run of one scheduler has one dirty CPU
    // thread (erl_init.c:865) and 10 dirty IO threads (erl_init.c:867).
    let program = program();
    let value = program
        .execute("dirty_nif_probe", "threads", &[])
        .unwrap()
        .term;
    let Term::Tuple(names) = &value else {
        panic!("threads/0 gives a tuple: {value:?}");
    };
    let names: Vec<String> = names
        .iter()
        .map(|name| match name {
            Term::Binary(bytes) => String::from_utf8(bytes.bytes().unwrap().to_vec()).unwrap(),
            other => panic!("a thread name is a binary: {other:?}"),
        })
        .collect();
    assert_eq!(names[0], "erts_dcpus_1");
    let io: u32 = names[1]
        .strip_prefix("erts_dios_")
        .unwrap()
        .parse()
        .unwrap();
    assert!((1..=10).contains(&io), "{names:?}");
}

#[test]
fn handles_the_signals_of_a_caller_while_its_dirty_call_runs_as_erts() {
    // Another process runs while the call runs. A message does not end the
    // call, `exit(Caller, normal)` is ignored, and the status is `running`
    // (erts_process_state2status, erl_process.c:9319-9323).
    let program = program();
    let message_queue_len = |length| {
        Term::tuple(vec![
            atom(&program, "message_queue_len"),
            Term::Integer(length),
        ])
    };
    assert_eq!(
        run(&program, "signals", Term::Integer(1)),
        Term::tuple(vec![
            atom(&program, "ok"),
            message_queue_len(0),
            message_queue_len(1),
            Term::tuple(vec![atom(&program, "status"), atom(&program, "running")]),
            atom(&program, "true"),
            atom(&program, "ok"),
        ])
    );
}

#[test]
fn ends_a_killed_caller_at_once_and_drops_its_late_completion_as_erts() {
    // A kill ends the caller while the call runs (erl_process.c:10092-10110),
    // and the scheduler drops the result of the call (erl_nif.c:507-542).
    let program = program();
    let killed = atom(&program, "killed");
    assert_eq!(
        run(&program, "kill", Term::Integer(11)),
        Term::tuple(vec![killed.clone(), killed, atom(&program, "no_late")])
    );
}

#[test]
fn does_not_run_the_queued_call_of_a_killed_caller() {
    // The second call waits in the queue of the only dirty CPU thread, with
    // the status `runnable` (erl_process.c:9325-9328). After the kill of
    // its caller, the thread does not run it (erl_process.c:10252-10258).
    let program = program();
    assert_eq!(
        run(&program, "queued", Term::Integer(21)),
        Term::tuple(vec![
            Term::tuple(vec![atom(&program, "status"), atom(&program, "runnable")]),
            atom(&program, "false"),
            atom(&program, "ok"),
        ])
    );
}

#[test]
fn raises_the_exception_of_a_dirty_call_with_the_nif_entry_first() {
    // `dirty_nif_exception` restores the MFA of the NIF for the error
    // (erl_nif.c:3431-3433): on ERTS, `erts_debug:dirty_io(foo, bar)` gives
    // `{error,badarg,{erts_debug,dirty_io,[foo,bar],[]}}`.
    let program = program();
    assert_eq!(
        run(&program, "raise", Term::Integer(7)),
        Term::tuple(vec![
            atom(&program, "error"),
            atom(&program, "badarg"),
            Term::tuple(vec![
                atom(&program, "dirty_nif_probe"),
                atom(&program, "cpu_raise"),
                Term::list_from(vec![Term::Integer(7)]),
                Term::nil(),
            ]),
        ])
    );
}

#[test]
fn ends_the_run_without_the_end_of_a_dirty_call() {
    // ERTS `halt` does not wait for a dirty NIF (erl_nif.c:470-497).
    let program = program();
    assert_eq!(
        run(&program, "unfinished", Term::Integer(31)),
        atom(&program, "ok")
    );
    assert_eq!(update(31, |_| {}), (true, false));
    update(31, |gate| gate.1 = true);
}

#[test]
fn starts_the_queued_dirty_calls_by_the_priority_of_their_callers_as_erts() {
    // A process goes to the dirty run queue of its priority
    // (`select_enqueue_run_queue`, erl_process.c:6630-6660), and a dirty
    // scheduler picks `max`, then `high`, then the queue that `normal` and
    // `low` share, where a `low` process waits for 8 turns at the head
    // (erl_process.c:9993-10024, 4039-4052). The only dirty CPU thread is
    // busy while seven callers queue their calls: `+SDcpu 1:1` on ERTS, and
    // the one dirty CPU thread of a PBeam run with one scheduler.
    let erts = Command::new("erl")
        .args([
            "+S",
            "1",
            "+SDcpu",
            "1:1",
            "-noshell",
            "-pa",
            "fixtures/dirty_nif",
            "-eval",
            "io:format(\"~w~n\", [dirty_nif_probe:priorities_with(\
             fun() -> erts_debug:dirty_cpu(wait, 300) end, \
             fun() -> timer:sleep(50) end, \
             fun() -> erts_debug:dirty_cpu(wait, 20), erlang:monotonic_time() end, \
             fun() -> ok end)]), halt().",
        ])
        .output()
        .unwrap();
    assert!(erts.status.success(), "{erts:?}");
    assert_eq!(
        String::from_utf8_lossy(&erts.stdout),
        "[m1,m2,h1,n1,n2,l1,l2]\n"
    );
    let program = program();
    let tags = ["m1", "m2", "h1", "n1", "n2", "l1", "l2"].map(|tag| atom(&program, tag));
    assert_eq!(
        run(&program, "priorities", Term::Integer(41)),
        Term::list_from(tags)
    );
}

#[test]
fn a_dirty_call_of_another_scheduler_prevents_the_impossible_wait() {
    // The first root waits without a timer. While the dirty call of the
    // second root runs, the scheduler of the first root is idle, and only
    // the dirty call can lead to the message.
    let program = program();
    let run = thread::scope(|scope| {
        let run = scope.spawn(|| {
            program.execute_on_schedulers(&[
                ("dirty_nif_probe", "relay_wait"),
                ("dirty_nif_probe", "relay_send"),
            ])
        });
        // A run that ends before the call starts gives its error below.
        let mut gates = GATES.0.lock().unwrap();
        while !run.is_finished()
            && !gates
                .as_ref()
                .and_then(|gates| gates.get(&201))
                .is_some_and(|gate| gate.0)
        {
            gates = GATES
                .1
                .wait_timeout(gates, Duration::from_millis(10))
                .unwrap()
                .0;
        }
        drop(gates);
        thread::sleep(Duration::from_millis(100));
        update(201, |gate| gate.1 = true);
        run.join().unwrap()
    });
    let results: Vec<Term> = run.unwrap().into_iter().map(|result| result.term).collect();
    assert_eq!(results, [static_atom("ok"), static_atom("ok")]);
}

#[test]
fn shares_the_dirty_cpu_pool_of_the_node_between_two_schedulers() {
    // A node of 2 schedulers has 2 dirty CPU threads (erl_init.c:865). The
    // two roots start 3 calls, so 2 of them start and one waits in the
    // queue of the node until a thread is free.
    let program = program();
    let keys = [101, 102, 103];
    let run = thread::scope(|scope| {
        let run = scope.spawn(|| {
            program.execute_on_schedulers(&[
                ("dirty_nif_probe", "first_root"),
                ("dirty_nif_probe", "second_root"),
            ])
        });
        let count = |gates: &Gates| {
            let started = |key| gates.as_ref().and_then(|gates| gates.get(key));
            keys.iter()
                .filter(|key| started(key).is_some_and(|gate| gate.0))
                .count()
        };
        let mut gates = GATES.0.lock().unwrap();
        while count(&gates) < 2 {
            gates = GATES.1.wait(gates).unwrap();
        }
        drop(gates);
        // A third thread would start the third call in this time.
        thread::sleep(Duration::from_millis(100));
        assert_eq!(count(&GATES.0.lock().unwrap()), 2);
        for key in keys {
            update(key, |gate| gate.1 = true);
        }
        run.join().unwrap()
    });
    let results: Vec<Term> = run.unwrap().into_iter().map(|result| result.term).collect();
    let ok = static_atom("ok");
    assert_eq!(
        results,
        [
            Term::list_from(vec![ok.clone(), ok.clone()]),
            Term::list_from(vec![ok])
        ]
    );
}

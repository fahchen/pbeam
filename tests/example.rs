//! Run the examples of `examples/` with the mise task `example` on the
//! `pbeam` binary and on the pinned ERTS.

use std::{
    process::{Command, Output, Stdio},
    time::Duration,
};

#[expect(
    dead_code,
    reason = "this test crate keeps the locale and the stdout of the task"
)]
#[path = "common/run.rs"]
mod run;

use run::run_with_time_limit;

/// The task compiles the example and `pbeam_boot`, and starts `elixir`,
/// `erl`, and the runtime of the example.
const RUN_TIME_LIMIT: Duration = Duration::from_secs(60);

#[test]
fn runs_hello_world_with_one_command_as_pinned_erts() {
    // `mise run example hello_world` compiles `examples/hello_world/*.ex`
    // and `pbeam_boot.erl`, and runs `pbeam_boot:start('Elixir.HelloWorld',
    // main)` with the `pbeam` binary. With `--erts`, it runs
    // `'Elixir.HelloWorld':main(), halt().` with `erl +S 1 -noshell`. Each
    // runtime runs without a locale variable, which gives the C locale, and
    // with `LC_ALL=C.UTF-8`.
    for locale in [None, Some("C.UTF-8")] {
        let erts = run_example(&["hello_world", "--erts"], locale);
        let pbeam = run_example(&["hello_world"], locale);

        assert_eq!(
            parts(&erts),
            ("Hello, world!\\n".to_owned(), String::new(), Some(0)),
            "ERTS with LC_ALL {locale:?}"
        );
        assert_eq!(parts(&pbeam), parts(&erts), "PBeam with LC_ALL {locale:?}");
    }
}

#[test]
fn runs_fairness_as_pinned_erts_with_one_scheduler() {
    // `examples/fairness` spawns the busy processes `a` and `b`. Each one
    // counts to 20000 with no receive and reports each 10 steps. On one
    // scheduler, each turn of a process ends when it has used its 4000
    // reductions, and then the other process runs. The ERTS run uses
    // `erl +S 1`; with more schedulers, the runs change from one start to
    // the next.
    let expected: String = [
        "a 10..2660",
        "b 10..2660",
        "a 2670..5330",
        "b 2670..5330",
        "a 5340..8000",
        "b 5340..8000",
        "a 8010..10660",
        "b 8010..10660",
        "a 10670..13330",
        "b 10670..13330",
        "a 13340..16000",
        "b 13340..16000",
        "a 16010..18660",
        "b 16010..18660",
        "a 18670..20000",
        "b 18670..20000",
    ]
    .iter()
    .map(|line| format!("{line}\\n"))
    .collect();

    let erts = run_example(&["fairness", "--erts"], None);
    let pbeam = run_example(&["fairness"], None);

    assert_eq!(parts(&erts), (expected, String::new(), Some(0)), "ERTS");
    assert_eq!(parts(&pbeam), parts(&erts), "PBeam");
}

#[test]
fn runs_ping_pong_as_pinned_erts() {
    // `PingPong.main/0` spawns a ping and a pong process with `spawn/1`.
    // They exchange `{:ping, from}` and `{:pong, from}` three times, and the
    // root process prints the round-trip order and the count.
    let erts = run_example(&["ping_pong", "--erts"], None);
    let pbeam = run_example(&["ping_pong"], None);

    assert_eq!(
        parts(&erts),
        (
            "ping 1\\npong 1\\nping 2\\npong 2\\nping 3\\npong 3\\ncount: 3\\n".to_owned(),
            String::new(),
            Some(0)
        ),
        "ERTS"
    );
    assert_eq!(parts(&pbeam), parts(&erts), "PBeam");
}

/// Run `mise run example` with `arguments`, with no locale variable except
/// `LC_ALL=locale`. `PBEAM_BIN` gives the `pbeam` binary of this test
/// build, which `cargo llvm-cov` measures, so the task does not start
/// `cargo run`.
///
/// # Examples
///
/// ```rust,ignore
/// let output = run_example(&["hello_world", "--erts"], Some("C.UTF-8"));
/// assert_eq!(output.status.code(), Some(0));
/// ```
fn run_example(arguments: &[&str], locale: Option<&str>) -> Output {
    let mut command = Command::new("mise");
    command
        .args(["run", "example"])
        .args(arguments)
        .current_dir(env!("CARGO_MANIFEST_DIR"))
        .env("PBEAM_BIN", env!("CARGO_BIN_EXE_pbeam"))
        .env_remove("LANG")
        .env_remove("LC_CTYPE")
        .env_remove("LC_ALL");
    if let Some(locale) = locale {
        command.env("LC_ALL", locale);
    }
    run_with_time_limit(&mut command, Stdio::piped(), RUN_TIME_LIMIT)
}

/// Return the stdout, the stderr, and the exit status of a run. The output
/// is escaped as ASCII, so a failure message shows each byte.
///
/// # Examples
///
/// ```rust,ignore
/// assert_eq!(parts(&output), ("Hello, world!\\n".to_owned(), String::new(), Some(0)));
/// ```
fn parts(output: &Output) -> (String, String, Option<i32>) {
    (
        output.stdout.escape_ascii().to_string(),
        output.stderr.escape_ascii().to_string(),
        output.status.code(),
    )
}

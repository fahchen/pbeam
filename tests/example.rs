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

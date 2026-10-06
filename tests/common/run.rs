//! A time limit for a test that runs a child process. A test that runs a
//! binary includes this file with `#[path = "common/run.rs"] mod run;`. A
//! function in `common/mod.rs` would be dead code in each other test crate.

use std::{
    process::{Command, Output, Stdio},
    thread,
    time::{Duration, Instant},
};

/// Run a command with stdin `/dev/null` and stdout `stdout`, and return its
/// output. With `Stdio::piped()`, the output has the stdout of the command.
/// Kill the command and panic with its stdout when it does not end in
/// `limit`. The caller sets the arguments and the environment.
///
/// # Examples
///
/// ```rust,ignore
/// let output = run_with_time_limit(
///     Command::new("erl").args(["-noshell", "-eval", "halt()."]),
///     Stdio::piped(),
///     Duration::from_secs(10),
/// );
/// assert_eq!(output.status.code(), Some(0));
/// ```
pub fn run_with_time_limit(command: &mut Command, stdout: Stdio, limit: Duration) -> Output {
    let mut child = command
        .stdin(Stdio::null())
        .stdout(stdout)
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    let start = Instant::now();
    while child.try_wait().unwrap().is_none() {
        if start.elapsed() >= limit {
            child.kill().unwrap();
            let output = child.wait_with_output().unwrap();
            panic!(
                "the run did not end in {} s: {command:?}, stdout: {:?}",
                limit.as_secs(),
                String::from_utf8_lossy(&output.stdout)
            );
        }
        thread::sleep(Duration::from_millis(10));
    }
    child.wait_with_output().unwrap()
}

/// Run a command with `TERM=dumb` and `LC_ALL=C.UTF-8` through
/// `run_with_time_limit`, and return its output. The two variables give the
/// same terminal and encoding to ERTS and to PBeam.
///
/// # Examples
///
/// ```rust,ignore
/// let output = run_dumb_utf8(
///     Command::new("erl").args(["-noshell", "-eval", "halt()."]),
///     Stdio::piped(),
///     Duration::from_secs(10),
/// );
/// assert_eq!(output.status.code(), Some(0));
/// ```
pub fn run_dumb_utf8(command: &mut Command, stdout: Stdio, limit: Duration) -> Output {
    run_with_time_limit(
        command.env("TERM", "dumb").env("LC_ALL", "C.UTF-8"),
        stdout,
        limit,
    )
}

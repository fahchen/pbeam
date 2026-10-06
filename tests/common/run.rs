//! A time limit for a test that runs a child process. A test that runs a
//! binary includes this file with `#[path = "common/run.rs"] mod run;`. A
//! function in `common/mod.rs` would be dead code in each other test crate.

use std::{
    process::{Command, Output, Stdio},
    thread,
    time::{Duration, Instant},
};

/// Run a command with stdin `/dev/null` and return its output. Kill it and
/// panic with its stdout when it does not end in `limit`. The caller sets
/// the arguments and the environment.
///
/// # Examples
///
/// ```rust,ignore
/// let output = run_with_time_limit(
///     Command::new("erl").args(["-noshell", "-eval", "halt()."]),
///     Duration::from_secs(10),
/// );
/// assert_eq!(output.status.code(), Some(0));
/// ```
pub fn run_with_time_limit(command: &mut Command, limit: Duration) -> Output {
    let mut child = command
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
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

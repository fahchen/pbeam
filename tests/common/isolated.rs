//! Run an ignored test of this test binary in a child process.

use std::process::{Command, Stdio};

/// The environment variable that marks the child process of [`run`].
const ISOLATED: &str = "PBEAM_ISOLATED";

/// Run the ignored test `name` of this test binary in a child process, and
/// require that it passes.
///
/// A child process contains state that the whole process shares. A
/// disposition is the same for all threads, so a signal that one test
/// changes reaches every test. A closed fd number can be open again for the
/// next test that calls `open`, so a test of a closed fd races with the
/// other tests. The child inherits `LLVM_PROFILE_FILE`, so `cargo llvm-cov`
/// measures it.
pub(crate) fn run(name: &str) {
    let child = Command::new(std::env::current_exe().unwrap())
        .args(["--exact", name, "--include-ignored", "--quiet"])
        .env(ISOLATED, "1")
        .stdin(Stdio::null())
        .output()
        .unwrap();
    assert!(
        child.status.success(),
        "{name} exited with {}\n{}",
        child.status,
        String::from_utf8_lossy(&child.stdout)
    );
}

/// Require the child process of [`run`].
pub(crate) fn requires() {
    assert!(
        std::env::var_os(ISOLATED).is_some(),
        "run runs this test in a child process"
    );
}

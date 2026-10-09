use std::{
    io::Read,
    os::fd::{AsFd, AsRawFd},
    path::PathBuf,
    process::{Child, Command, Stdio},
    sync::LazyLock,
    thread,
    time::{Duration, Instant},
};

#[path = "../common/boot.rs"]
mod boot;
#[expect(
    dead_code,
    reason = "this test crate gives BEAM paths, not bytes, to the binary"
)]
#[path = "../common/mod.rs"]
mod common;

use boot::{BOOT_MODULES, boot_beam};
use common::pinned_beam_path;

const DIRTY_IO_PROBE: &str = "fixtures/boot/dirty_io_probe.beam";

/// The time that a run has to write each output that the test waits for,
/// and to end.
const RUN_TIME_LIMIT: Duration = Duration::from_secs(10);

/// The bytes that `dirty_io_probe:writer/1` writes to stdout: 1024 blocks of
/// 16 lines of 64 bytes.
static WRITTEN: LazyLock<Vec<u8>> = LazyLock::new(|| {
    b"0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcde\n".repeat(16 * 1024)
});

/// The paths of the pinned `BOOT_MODULES`. Each `pinned_beam_path` call
/// starts `erl`, so the tests of this crate find the paths only once.
static BOOT_PATHS: LazyLock<[PathBuf; BOOT_MODULES.len()]> =
    LazyLock::new(|| BOOT_MODULES.map(pinned_beam_path));

#[test]
fn runs_other_processes_while_a_write_waits_for_a_full_pipe() {
    // `dirty_io_probe:main/0` starts a writer that writes 1 MiB to stdout
    // through `io:put_chars/1`, `group`, `user_drv`, and the `prim_tty`
    // writer, which calls `write_nif/2` (prim_tty.erl:549-563). ERTS runs
    // `write_nif/2` on a dirty IO scheduler (prim_tty_nif.c:161), and its
    // `writev` waits while the pipe is full (prim_tty_nif.c:376-380). The
    // main process writes "tick" to stderr after each timer of 10 ms until
    // the writer is done.
    //
    // The test reads 1 byte of stdout, so the `writev` runs. Then it reads
    // all stderr bytes that are in the pipe, and it reads no more stdout
    // until a new "tick" arrives on stderr. If the `writev` stopped all
    // processes, each stderr byte came before the stdout byte, so no new
    // tick arrives, and the test fails after `RUN_TIME_LIMIT`. Only the
    // failure uses a time limit.
    for (name, command) in [("ERTS", erts()), ("PBeam", pbeam())] {
        let (stdout, stderr, status) = run_with_full_stdout(command, name);
        assert!(stdout == *WRITTEN, "{name} wrote {} bytes", stdout.len());
        let ticks = stderr
            .strip_suffix("done\n")
            .map(|ticks| ticks.split_terminator('\n'));
        assert!(
            ticks.is_some_and(|mut ticks| ticks.all(|tick| tick == "tick")),
            "{name} stderr: {stderr:?}"
        );
        assert_eq!(status, Some(0), "{name}");
    }
}

/// Run `command` with stdout and stderr pipes, and the protocol of
/// [`runs_other_processes_while_a_write_waits_for_a_full_pipe`]. Return
/// all stdout bytes, all stderr text, and the exit status code. Kill the
/// child and panic when an output or the exit does not come in
/// `RUN_TIME_LIMIT`.
///
/// # Examples
///
/// ```rust,ignore
/// let (stdout, stderr, status) = run_with_full_stdout(pbeam(), "PBeam");
/// ```
fn run_with_full_stdout(mut command: Command, name: &str) -> (Vec<u8>, String, Option<i32>) {
    let mut child = command
        .env("TERM", "dumb")
        .env("LC_ALL", "C.UTF-8")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    let mut stdout = child.stdout.take().unwrap();
    let mut stderr = child.stderr.take().unwrap();
    let mut first = [0];
    if !wait_readable(&stdout, RUN_TIME_LIMIT) {
        fail(child, name, "no stdout byte");
    }
    stdout.read_exact(&mut first).unwrap();
    let mut before = Vec::new();
    while wait_readable(&stderr, Duration::ZERO) && read_some(&mut stderr, &mut before) {}
    let mut after = Vec::new();
    let start = Instant::now();
    while !after.windows(5).any(|window| window == b"tick\n") {
        let left = RUN_TIME_LIMIT.saturating_sub(start.elapsed());
        if !(wait_readable(&stderr, left) && read_some(&mut stderr, &mut after)) {
            let (before, after) = (
                String::from_utf8_lossy(&before),
                String::from_utf8_lossy(&after),
            );
            let text = format!(
                "no tick after the stdout byte; stderr before it: {before:?}, after it: {after:?}"
            );
            fail(child, name, &text);
        }
    }
    // Threads read the rest of the output, so the time limit of the exit
    // also stops a run that stops writing. The kill in `fail` closes the
    // pipes, and the threads then end.
    let reading_stderr = thread::spawn(move || {
        let mut rest = Vec::new();
        stderr.read_to_end(&mut rest).unwrap();
        rest
    });
    let reading_stdout = thread::spawn(move || {
        let mut written = first.to_vec();
        stdout.read_to_end(&mut written).unwrap();
        written
    });
    let start = Instant::now();
    let status = loop {
        if let Some(status) = child.try_wait().unwrap() {
            break status;
        }
        if start.elapsed() >= RUN_TIME_LIMIT {
            fail(child, name, "the run did not end");
        }
        thread::sleep(Duration::from_millis(10));
    };
    let written = reading_stdout.join().unwrap();
    let rest = reading_stderr.join().unwrap();
    let stderr = String::from_utf8([before, after, rest].concat()).unwrap();
    (written, stderr, status.code())
}

/// Wait at most `timeout` until a read of `pipe` does not block. Return
/// false when the timeout ends first.
///
/// # Examples
///
/// ```rust,ignore
/// assert!(wait_readable(&stdout, RUN_TIME_LIMIT));
/// ```
fn wait_readable(pipe: &impl AsFd, timeout: Duration) -> bool {
    let mut poll = libc::pollfd {
        fd: pipe.as_fd().as_raw_fd(),
        events: libc::POLLIN,
        revents: 0,
    };
    let timeout = libc::c_int::try_from(timeout.as_millis()).unwrap();
    // SAFETY: `poll` reads and writes one `pollfd`, which `poll` points to.
    let count = unsafe { libc::poll(&raw mut poll, 1, timeout) };
    assert!(count >= 0, "poll failed");
    count == 1
}

/// Read the bytes that are in `pipe` into `bytes`, after [`wait_readable`]
/// returned true. Return false at the end of file.
///
/// # Examples
///
/// ```rust,ignore
/// while wait_readable(&stderr, Duration::ZERO) && read_some(&mut stderr, &mut bytes) {}
/// ```
fn read_some(pipe: &mut impl Read, bytes: &mut Vec<u8>) -> bool {
    let mut buffer = [0; 4096];
    let count = pipe.read(&mut buffer).unwrap();
    bytes.extend_from_slice(&buffer[..count]);
    count > 0
}

/// Kill `child` and panic with `text` and the exit status. `text` holds the
/// output that the test read.
///
/// # Examples
///
/// ```rust,ignore
/// fail(child, "PBeam", "no stdout byte");
/// ```
fn fail(mut child: Child, name: &str, text: &str) -> ! {
    child.kill().unwrap();
    let output = child.wait_with_output().unwrap();
    panic!("{name}: {text}; status: {}", output.status);
}

/// Return the pinned `erl` command that runs `dirty_io_probe:main()` on one
/// scheduler, then `halt()`.
///
/// # Examples
///
/// ```rust,ignore
/// let child = erts().spawn()?;
/// ```
fn erts() -> Command {
    let mut command = Command::new("erl");
    command.args([
        "+S",
        "1",
        "-noshell",
        "-pa",
        "fixtures/boot",
        "-eval",
        "dirty_io_probe:main(), halt().",
    ]);
    command
}

/// Return the `pbeam` command that boots with `pbeam_boot` and runs
/// `dirty_io_probe:main()`.
///
/// # Examples
///
/// ```rust,ignore
/// let child = pbeam().spawn()?;
/// ```
fn pbeam() -> Command {
    let mut command = Command::new(env!("CARGO_BIN_EXE_pbeam"));
    command
        .args([
            "pbeam_boot",
            "start",
            "dirty_io_probe",
            "main",
            "--",
            DIRTY_IO_PROBE,
        ])
        .args(BOOT_PATHS.iter())
        .arg(boot_beam());
    command
}

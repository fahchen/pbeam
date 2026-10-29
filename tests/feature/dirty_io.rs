//! Checks that other processes run while a write waits for a full pipe, and how the signals of a
//! waiting writer end the run.
#![expect(
    clippy::unwrap_used,
    reason = "test helpers report a failure by a panic"
)]

use std::{
    io::Read,
    os::fd::{AsFd, AsRawFd},
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

use boot::{BOOT_MODULES, boot_beam, otp_root};
use common::pinned_beam_path;

const DIRTY_IO_PROBE: &str = "fixtures/boot/dirty_io_probe.beam";
const DIRTY_EXIT_PROBE: &str = "fixtures/boot/dirty_exit_probe.beam";

/// The time that a run has to write each output that the test waits for,
/// and to end.
const RUN_TIME_LIMIT: Duration = Duration::from_secs(10);

/// The bytes that `dirty_io_probe:writer/1` writes to stdout: 1024 blocks of
/// 16 lines of 64 bytes.
static WRITTEN: LazyLock<Vec<u8>> = LazyLock::new(|| {
    b"0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcde\n".repeat(16 * 1024)
});

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
    let runs = [
        ("ERTS", erts("dirty_io_probe", "main")),
        ("PBeam", pbeam(DIRTY_IO_PROBE, "dirty_io_probe", "main")),
    ];
    for (name, command) in runs {
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

#[test]
fn handles_the_signals_of_a_writer_whose_write_waits_and_ends_the_run_without_the_write() {
    // `dirty_exit_probe:signals/0` starts a `prim_tty` writer that writes
    // 1 MiB to stdout, which the test does not read until the run ends. While
    // the write waits, the probe asks for the message queue length of the
    // writer, sends it a message, links a second process to it, sends it the
    // exit signal `normal`, asks whether it is alive, and kills it. ERTS
    // handles these signals with `erts_dirty_process_signal_handler`
    // (erts_dirty_process_signal_handler.erl:52-89), and a normal scheduler
    // runs the exit while the write waits (erl_process.c:10092-10110). The
    // link gives the exit reason `killed` to the second process. The run
    // then ends without the end of the write: ERTS `halt` does not wait for
    // a dirty NIF of `prim_tty` (erl_nif.c:470-497). If the run waited for
    // the write or for a signal reply, the test fails after
    // `RUN_TIME_LIMIT`.
    let runs = [
        ("ERTS", erts("dirty_exit_probe", "signals")),
        (
            "PBeam",
            pbeam(DIRTY_EXIT_PROBE, "dirty_exit_probe", "signals"),
        ),
    ];
    for (name, command) in runs {
        let mut child = spawn_with_pipes(command);
        let mut stderr = child.stderr.take().unwrap();
        let reading_stderr = thread::spawn(move || {
            let mut text = String::new();
            stderr.read_to_string(&mut text).unwrap();
            text
        });
        let status = wait_exit(&mut child, name);
        let mut stdout = Vec::new();
        child
            .stdout
            .take()
            .unwrap()
            .read_to_end(&mut stdout)
            .unwrap();
        assert_eq!(
            reading_stderr.join().unwrap(),
            "0 1 true killed killed\n",
            "{name}"
        );
        assert_eq!(status, Some(0), "{name}");
        assert!(
            stdout.len() < 1 << 20 && stdout.iter().all(|&byte| byte == b'a'),
            "{name} wrote {} bytes",
            stdout.len()
        );
    }
}

#[test]
fn does_not_run_the_queued_write_of_a_killed_writer() {
    // `dirty_exit_probe:cancel/0` starts 10 `prim_tty` writers, which write
    // 256 KiB of the bytes `0` to `9` to stdout, and an 11th writer, which
    // writes 256 KiB of `X`. The test does not read stdout, so the 10 writes
    // wait and use the 10 dirty IO schedulers (erl_init.c:867), and the 11th
    // write waits in the queue. The probe kills the 11th writer, and a shell
    // then writes "killed" to stderr without a dirty IO scheduler. The test
    // then reads stdout: the 10 writes complete, and the probe writes
    // "done". A dirty IO scheduler does not call the NIF of an exiting
    // process (erl_process.c:10252-10258), so no `X` arrives.
    let runs = [
        ("ERTS", erts("dirty_exit_probe", "cancel")),
        (
            "PBeam",
            pbeam(DIRTY_EXIT_PROBE, "dirty_exit_probe", "cancel"),
        ),
    ];
    for (name, command) in runs {
        let mut child = spawn_with_pipes(command);
        let mut stderr = child.stderr.take().unwrap();
        let mut killed = Vec::new();
        let start = Instant::now();
        while !killed.ends_with(b"killed\n") {
            let left = RUN_TIME_LIMIT.saturating_sub(start.elapsed());
            if !(wait_readable(&stderr, left) && read_some(&mut stderr, &mut killed)) {
                let text = String::from_utf8_lossy(&killed);
                fail(child, name, &format!("no killed line; stderr: {text:?}"));
            }
        }
        let mut stdout = child.stdout.take().unwrap();
        let reading_stdout = thread::spawn(move || {
            let mut written = Vec::new();
            stdout.read_to_end(&mut written).unwrap();
            written
        });
        let reading_stderr = thread::spawn(move || {
            let mut rest = Vec::new();
            stderr.read_to_end(&mut rest).unwrap();
            rest
        });
        let status = wait_exit(&mut child, name);
        let written = reading_stdout.join().unwrap();
        let stderr = [killed, reading_stderr.join().unwrap()].concat();
        assert_eq!(String::from_utf8_lossy(&stderr), "killed\ndone\n", "{name}");
        assert_eq!(status, Some(0), "{name}");
        let mut counts = [0; 256];
        for &byte in &written {
            counts[usize::from(byte)] += 1;
        }
        for byte in u8::MIN..=u8::MAX {
            let expected = if byte.is_ascii_digit() { 256 * 1024 } else { 0 };
            let count = counts[usize::from(byte)];
            assert_eq!(count, expected, "{name}: count of byte {byte}");
        }
    }
}

/// Spawn `command` with stdin `/dev/null`, and stdout and stderr pipes.
///
/// # Examples
///
/// ```rust,ignore
/// let child = spawn_with_pipes(pbeam(DIRTY_EXIT_PROBE, "dirty_exit_probe", "cancel"));
/// ```
fn spawn_with_pipes(mut command: Command) -> Child {
    command
        .env("TERM", "dumb")
        .env("LC_ALL", "C.UTF-8")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap()
}

/// Wait until `child` exits, and return its exit status code. Kill the
/// child and panic when it does not exit in `RUN_TIME_LIMIT`.
///
/// # Examples
///
/// ```rust,ignore
/// assert_eq!(wait_exit(&mut child, "PBeam"), Some(0));
/// ```
fn wait_exit(child: &mut Child, name: &str) -> Option<i32> {
    let start = Instant::now();
    loop {
        if let Some(status) = child.try_wait().unwrap() {
            return status.code();
        }
        if start.elapsed() >= RUN_TIME_LIMIT {
            child.kill().unwrap();
            let status = child.wait().unwrap();
            panic!("{name}: the run did not end; status: {status}");
        }
        thread::sleep(Duration::from_millis(10));
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

/// Return the pinned `erl` command that runs `module:function()` of
/// `fixtures/boot` on one scheduler, then `halt()`.
///
/// # Examples
///
/// ```rust,ignore
/// let child = erts("dirty_io_probe", "main").spawn()?;
/// ```
fn erts(module: &str, function: &str) -> Command {
    let mut command = Command::new("erl");
    command
        .args(["+S", "1", "-noshell", "-pa", "fixtures/boot", "-eval"])
        .arg(format!("{module}:{function}(), halt()."));
    command
}

/// Return the `pbeam` command that boots with `pbeam_boot` and runs
/// `module:function()` of `beam`.
///
/// # Examples
///
/// ```rust,ignore
/// let child = pbeam(DIRTY_IO_PROBE, "dirty_io_probe", "main").spawn()?;
/// ```
fn pbeam(beam: &str, module: &str, function: &str) -> Command {
    let mut command = Command::new(env!("CARGO_BIN_EXE_pbeam"));
    command
        .args([
            "pbeam_boot",
            "start",
            module,
            function,
            "--root",
            otp_root().to_str().unwrap(),
            "--",
            beam,
        ])
        .args(BOOT_MODULES.map(pinned_beam_path))
        .arg(boot_beam());
    command
}

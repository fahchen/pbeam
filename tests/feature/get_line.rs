//! Reads lines from a pipe and from a terminal through the `pbeam` binary and compares them with
//! the pinned ERTS.
#![expect(
    clippy::unwrap_used,
    reason = "test helpers report a failure by a panic"
)]

use std::{
    io::{self, Read, Write},
    os::fd::{FromRawFd, OwnedFd},
    process::{Child, Command, Stdio},
    sync::{Arc, Mutex},
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

const GET_LINE_PROBE: &str = "fixtures/boot/get_line_probe.beam";

/// The time that a run has to end, and that a terminal run has to write
/// each output that the test waits for.
const RUN_TIME_LIMIT: Duration = Duration::from_secs(10);

#[test]
fn reads_lines_from_a_pipe_as_pinned_erts() {
    // `get_line_probe:main/0` calls `io:get_line(standard_io, "> ")` until
    // `eof`, through `user`, `group`, and the cooked `prim_tty` reader of
    // `user_drv` (user_sup.erl:129-130). The reader selects fd 0 with
    // `tty_select/2` and reads it with `read_nif/3` after each
    // `{select, TTY, Ref, ready_input}` message (prim_tty.erl:436-490).
    // Stdin is a pipe that holds all input and has no writer. The last line
    // has no newline, so `group` writes the prompt again before the end of
    // file. A read of 0 bytes gives `{error, closed}`, which is `eof`
    // (prim_tty_nif.c:654-657). Without the select of fd 0, the reader gets
    // no input and the run waits without end.
    for (input, expected) in [
        (
            "hello\n\nw\u{f6}rld\nlast",
            "> > > > > > [104,101,108,108,111,10] [10] [119,246,114,108,100,10] [108,97,115,116] eof\n",
        ),
        ("", "> eof\n"),
    ] {
        for (name, mut command) in [("ERTS", erts("main")), ("PBeam", pbeam("main"))] {
            let (reader, mut writer) = io::pipe().unwrap();
            writer.write_all(input.as_bytes()).unwrap();
            drop(writer);
            let child = dumb_utf8(&mut command)
                .stdin(reader)
                .stdout(Stdio::piped())
                .stderr(Stdio::piped())
                .spawn()
                .unwrap();
            let (status, stdout, stderr) = wait_with_output(child);
            assert_eq!(
                (
                    String::from_utf8_lossy(&stdout).as_ref(),
                    String::from_utf8_lossy(&stderr).as_ref(),
                    status
                ),
                (expected, "", Some(0)),
                "{name} with the input {input:?}"
            );
        }
    }
}

#[test]
fn reads_lines_from_a_terminal_in_cooked_mode_as_pinned_erts() {
    // Stdin, stdout, and stderr are one pseudo-terminal in its first mode,
    // with `ECHO` and `ICANON`. The test types each line after the prompt
    // of its `io:get_line/2` call, and the terminal driver gives each line
    // to one `read`. `^D` at the start of a line makes the next `read` give
    // 0 bytes, which is `eof`. The terminal echoes the input; on macOS, it
    // also echoes `^D` as "^D" and two backspaces.
    let typed: [&[u8]; 4] = [b"hello\n", b"\n", "w\u{f6}rld\n".as_bytes(), b"\x04"];
    let script = typed
        .iter()
        .enumerate()
        .map(|(index, input)| (Wait::Prompts(index + 1), *input))
        .collect::<Vec<_>>();
    let erts = run_in_terminal(&mut erts("main"), &script);
    let pbeam = run_in_terminal(&mut pbeam("main"), &script);
    assert_eq!(
        String::from_utf8_lossy(&pbeam),
        String::from_utf8_lossy(&erts)
    );
    assert!(
        pbeam.starts_with("> hello\r\n> \r\n> w\u{f6}rld\r\n> ".as_bytes()),
        "{pbeam:?}"
    );
    assert!(
        pbeam.ends_with(b"[104,101,108,108,111,10] [10] [119,246,114,108,100,10] eof\r\n"),
        "{pbeam:?}"
    );
}

#[test]
fn runs_other_processes_while_a_reader_waits_for_input() {
    // `get_line_probe:wait/0` spawns a process that waits in
    // `io:get_line/2`, and writes "tick" after a timer of 100 ms. The test
    // types the line only after "tick". While no process can run, the
    // scheduler waits for fd 0 only until the timer end
    // (erl_process.c:3536-3567), so the timer still ends while the reader
    // waits.
    let script = [(Wait::Text(b"tick\r\n"), &b"hello\n"[..])];
    for (name, mut command) in [("ERTS", erts("wait")), ("PBeam", pbeam("wait"))] {
        assert_eq!(
            String::from_utf8_lossy(&run_in_terminal(&mut command, &script)),
            "tick\r\nhello\r\nline [104,101,108,108,111,10]\r\n",
            "{name}"
        );
    }
}

/// Return the `erl -noshell` command that evaluates
/// `get_line_probe:Function()` and `halt()`.
///
/// # Examples
///
/// ```rust,ignore
/// let command = erts("main");
/// ```
fn erts(function: &str) -> Command {
    let mut command = Command::new("erl");
    command.args([
        "-noshell",
        "-pa",
        "fixtures/boot",
        "-eval",
        &format!("get_line_probe:{function}(), halt()."),
    ]);
    command
}

/// Return the `pbeam` command that boots with `pbeam_boot` and runs
/// `get_line_probe:Function()`.
///
/// # Examples
///
/// ```rust,ignore
/// let command = pbeam("main");
/// ```
fn pbeam(function: &str) -> Command {
    let mut command = Command::new(env!("CARGO_BIN_EXE_pbeam"));
    command
        .args([
            "pbeam_boot",
            "start",
            "get_line_probe",
            function,
            "--root",
            otp_root().to_str().unwrap(),
            "--",
            GET_LINE_PROBE,
        ])
        .args(BOOT_MODULES.map(pinned_beam_path))
        .arg(boot_beam());
    command
}

/// Set `TERM=dumb` and `LC_ALL=C.UTF-8`, which give the same terminal and
/// encoding to ERTS and to PBeam.
///
/// # Examples
///
/// ```rust,ignore
/// dumb_utf8(&mut erts("main")).spawn()?;
/// ```
fn dumb_utf8(command: &mut Command) -> &mut Command {
    command.env("TERM", "dumb").env("LC_ALL", "C.UTF-8")
}

/// Wait until `child` exits, and return its exit status code, its stdout,
/// and its stderr. Kill it and panic when it does not end in
/// `RUN_TIME_LIMIT`.
///
/// # Examples
///
/// ```rust,ignore
/// let (status, stdout, stderr) = wait_with_output(child);
/// assert_eq!(status, Some(0));
/// ```
fn wait_with_output(mut child: Child) -> (Option<i32>, Vec<u8>, Vec<u8>) {
    let start = Instant::now();
    while child.try_wait().unwrap().is_none() {
        if start.elapsed() >= RUN_TIME_LIMIT {
            child.kill().unwrap();
            let output = child.wait_with_output().unwrap();
            panic!(
                "the run did not end in {} s, stdout: {:?}, stderr: {:?}",
                RUN_TIME_LIMIT.as_secs(),
                String::from_utf8_lossy(&output.stdout),
                String::from_utf8_lossy(&output.stderr)
            );
        }
        thread::sleep(Duration::from_millis(10));
    }
    let output = child.wait_with_output().unwrap();
    (output.status.code(), output.stdout, output.stderr)
}

/// What a terminal run waits for before the test types its next input.
#[derive(Clone, Copy)]
enum Wait {
    /// This number of "> " prompts in the output.
    Prompts(usize),
    /// This text in the output.
    Text(&'static [u8]),
}

/// Run `command` with a new pseudo-terminal as its stdin, stdout, and
/// stderr. For each step of `script`, wait for its output, then type its
/// input on the terminal. Return all output of the terminal after the
/// command exits with status 0. Panic when an output or the exit does not
/// come in `RUN_TIME_LIMIT`.
///
/// # Examples
///
/// ```rust,ignore
/// let output = run_in_terminal(&mut erts("wait"), &[(Wait::Text(b"tick\r\n"), b"hello\n")]);
/// ```
fn run_in_terminal(command: &mut Command, script: &[(Wait, &[u8])]) -> Vec<u8> {
    let (master, slave) = open_terminal();
    let mut child = dumb_utf8(command)
        .stdin(slave.try_clone().unwrap())
        .stdout(slave.try_clone().unwrap())
        .stderr(slave)
        .spawn()
        .unwrap();
    // The command keeps its standard streams; replace them, so that the
    // child holds the only slave fds, and the master reads the end of the
    // output when the child exits.
    command
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    let output = Arc::new(Mutex::new(Vec::new()));
    let mut terminal = std::fs::File::from(master);
    let reading = {
        let mut terminal = terminal.try_clone().unwrap();
        let output = Arc::clone(&output);
        // A read of the master fails with `EIO` on Linux, or gives 0 bytes
        // on macOS, after the child and the test closed each slave fd.
        thread::spawn(move || {
            let mut buffer = [0; 4096];
            while let Ok(count @ 1..) = terminal.read(&mut buffer) {
                output.lock().unwrap().extend_from_slice(&buffer[..count]);
            }
        })
    };
    for &(wait, input) in script {
        let start = Instant::now();
        while !match wait {
            Wait::Prompts(count) => {
                output
                    .lock()
                    .unwrap()
                    .windows(2)
                    .filter(|w| w == b"> ")
                    .count()
                    >= count
            }
            Wait::Text(text) => output
                .lock()
                .unwrap()
                .windows(text.len())
                .any(|w| w == text),
        } {
            if start.elapsed() >= RUN_TIME_LIMIT {
                child.kill().unwrap();
                panic!(
                    "no output for the next input {input:?}: {:?}",
                    String::from_utf8_lossy(&output.lock().unwrap())
                );
            }
            thread::sleep(Duration::from_millis(10));
        }
        terminal.write_all(input).unwrap();
    }
    let start = Instant::now();
    let status = loop {
        if let Some(status) = child.try_wait().unwrap() {
            break status;
        }
        if start.elapsed() >= RUN_TIME_LIMIT {
            child.kill().unwrap();
            panic!(
                "the run did not end: {:?}",
                String::from_utf8_lossy(&output.lock().unwrap())
            );
        }
        thread::sleep(Duration::from_millis(10));
    };
    reading.join().unwrap();
    let output = output.lock().unwrap().clone();
    assert!(
        status.success(),
        "{status}: {:?}",
        String::from_utf8_lossy(&output)
    );
    output
}

/// Open a pseudo-terminal in the first mode of the host, and return its
/// master and its slave. Both are close-on-exec, so a child of a parallel
/// test does not keep them open.
///
/// # Examples
///
/// ```rust,ignore
/// let (master, slave) = open_terminal();
/// ```
fn open_terminal() -> (OwnedFd, OwnedFd) {
    let (mut master, mut slave) = (-1, -1);
    // SAFETY: `openpty` writes two new fds to `master` and `slave`, and
    // accepts a null name, mode, and window size. `fcntl` changes only the
    // fd flags. Each `OwnedFd` takes one new fd.
    unsafe {
        let result = libc::openpty(
            &raw mut master,
            &raw mut slave,
            std::ptr::null_mut(),
            std::ptr::null_mut(),
            std::ptr::null_mut(),
        );
        assert_eq!(result, 0, "openpty failed");
        for fd in [master, slave] {
            assert_eq!(libc::fcntl(fd, libc::F_SETFD, libc::FD_CLOEXEC), 0);
        }
        (OwnedFd::from_raw_fd(master), OwnedFd::from_raw_fd(slave))
    }
}

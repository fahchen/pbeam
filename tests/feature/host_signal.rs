use std::{
    ffi::c_int,
    io::{self, Read, Write},
    os::{
        fd::{FromRawFd, OwnedFd},
        unix::process::CommandExt,
    },
    process::{Child, Command, Stdio},
    sync::{Arc, Mutex},
    thread::{self, JoinHandle},
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
#[path = "../common/log_event.rs"]
mod log_event;

use boot::{BOOT_MODULES, boot_beam, otp_root};
use common::pinned_beam_path;
use log_event::{LOG_EVENT_MODULES, mask_time};

const NOTIFY_PROBE: &str = "fixtures/boot/notify_probe.beam";

/// The time that a run has to end, and that a run has to write each output
/// that the test waits for.
const RUN_TIME_LIMIT: Duration = Duration::from_secs(30);

/// The exit status code, the stdout, and the stderr of a run.
type Output = (Option<i32>, String, String);

#[test]
fn notifies_erl_signal_server_of_each_handled_signal_as_pinned_erts() {
    // `notify_probe:notify/0` handles `sighup` and `sigusr2` with
    // `os:set_signal/2`, adds a handler to `erl_signal_server`, writes
    // "ready", and then waits in `receive` with no timer and no input
    // select. The test sends SIGHUP, waits for its event, and sends SIGUSR2.
    // Each signal must wake the idle scheduler: without the wait for the
    // notify pipe, the PBeam run ends at once with `root process waits for a
    // message that cannot arrive`. ERTS sends `{notify, Signal}` to
    // `erl_signal_server` (sys.c:589-607, 1163-1176).
    let steps = [("ready\n", libc::SIGHUP), ("sighup\n", libc::SIGUSR2)];
    for (name, mut command) in [("ERTS", erts("notify")), ("PBeam", pbeam("notify"))] {
        assert_eq!(
            run_with_signals(&mut command, &steps),
            (
                Some(0),
                String::from("ready\nsighup\nsigusr2\n"),
                String::new()
            ),
            "{name}"
        );
    }
}

#[test]
fn notifies_erl_signal_server_while_a_process_can_always_run() {
    // `notify_probe:busy/0` handles `sighup` and starts a process that never
    // waits, so the scheduler is never idle. ERTS sends the message from its
    // signal dispatcher thread at once (sys.c:1122-1180). PBeam reads the
    // notify flag before each scheduling step.
    for (name, mut command) in [("ERTS", erts("busy")), ("PBeam", pbeam("busy"))] {
        assert_eq!(
            run_with_signals(&mut command, &[("ready\n", libc::SIGHUP)]),
            (Some(0), String::from("ready\nsighup\n"), String::new()),
            "{name}"
        );
    }
}

#[test]
fn runs_the_default_handler_of_a_start_signal_as_pinned_erts() {
    // `notify_probe:wait/0` writes "ready" and waits with no timer. ERTS
    // handles SIGQUIT and SIGTERM from the start (sys.c:799-823), and the
    // default `erl_signal_handler` calls `erlang:halt/0` for `sigquit`, and
    // writes an info report and calls `init:stop/0` for `sigterm`
    // (erl_signal_handler.erl:39-48). `init` stops the kernel processes and
    // halts with status 0 (init.erl:924-945). The `pbeam_boot` init request
    // process does the same. Its kernel process shuts down `logger_sup`
    // first, so the `default` handler writes the report before the halt.
    for (signal, expected) in [
        (libc::SIGQUIT, "ready\n"),
        (
            libc::SIGTERM,
            "ready\n=INFO REPORT==== <time> ===\nSIGTERM received - shutting down\n\n",
        ),
    ] {
        for (name, mut command) in [("ERTS", erts("wait")), ("PBeam", pbeam("wait"))] {
            let (status, stdout, stderr) = run_with_signals(&mut command, &[("ready\n", signal)]);
            assert_eq!(
                (status, mask_time(&stdout), stderr),
                (Some(0), String::from(expected), String::new()),
                "{name} {signal}"
            );
        }
    }
}

#[test]
fn gives_a_runtime_error_for_a_crash_dump_and_a_break() {
    // For SIGUSR1, `erl_signal_handler` calls `erlang:halt/1` with a slogan
    // (erl_signal_handler.erl:39-41), which writes a crash dump. For SIGINT,
    // ERTS runs its break handler (sys.c:611-629, 1165-1167), which writes
    // the break menu and reads a key from fd 0 (break.c:571-605). PBeam has
    // neither, so both give a runtime error.
    let breaks = "ready\n\r\nBREAK: (a)bort (A)bort with dump (c)ontinue (p)roc info (i)nfo\r\n       (l)oaded (v)ersion (k)ill (D)b-tables (d)istribution\r\n";
    for (signal, erts_output, pbeam_stderr) in [
        (
            libc::SIGUSR1,
            (Some(1), "ready\n", "Received SIGUSR1\r\n"),
            "pbeam: unsupported erlang:halt/2 with a crash dump slogan: PBeam writes no crash dump\n",
        ),
        (
            libc::SIGINT,
            (Some(0), breaks, ""),
            "pbeam: unsupported break of SIGINT: PBeam has no break handler\n",
        ),
    ] {
        let (status, stdout, stderr) = erts_output;
        assert_eq!(
            run_with_signals(&mut erts("wait"), &[("ready\n", signal)]),
            (status, String::from(stdout), String::from(stderr)),
            "ERTS {signal}"
        );
        assert_eq!(
            run_with_signals(&mut pbeam("wait"), &[("ready\n", signal)]),
            (Some(1), String::from("ready\n"), String::from(pbeam_stderr)),
            "PBeam {signal}"
        );
    }
}

#[test]
fn stops_at_a_quit_typed_on_the_terminal_as_pinned_erts() {
    // The run has a new pseudo-terminal as its controlling terminal and its
    // stdin, stdout, and stderr. `^\` on the terminal sends SIGQUIT to the
    // foreground process group, and the default `erl_signal_handler` calls
    // `erlang:halt/0`. The terminal echoes `^\`.
    let erts = run_in_terminal(&mut erts("wait"));
    let pbeam = run_in_terminal(&mut pbeam("wait"));
    assert_eq!(
        String::from_utf8_lossy(&pbeam),
        String::from_utf8_lossy(&erts)
    );
    assert!(pbeam.starts_with(b"ready\r\n"), "{pbeam:?}");
}

/// Return the `erl -noshell` command that evaluates
/// `notify_probe:Function()` and `halt()`. `ERL_CRASH_DUMP_SECONDS=0`
/// stops the crash dump of SIGUSR1.
///
/// # Examples
///
/// ```rust,ignore
/// let command = erts("wait");
/// ```
fn erts(function: &str) -> Command {
    let mut command = Command::new("erl");
    command
        .args([
            "-noshell",
            "-pa",
            "fixtures/boot",
            "-eval",
            &format!("notify_probe:{function}(), halt()."),
        ])
        .env("ERL_CRASH_DUMP_SECONDS", "0");
    dumb_utf8(&mut command);
    command
}

/// Return the `pbeam` command that boots with `pbeam_boot` and runs
/// `notify_probe:Function()`. The modules of a log event write the
/// SIGTERM report.
///
/// # Examples
///
/// ```rust,ignore
/// let command = pbeam("wait");
/// ```
fn pbeam(function: &str) -> Command {
    let mut command = Command::new(env!("CARGO_BIN_EXE_pbeam"));
    command
        .args([
            "pbeam_boot",
            "start",
            "notify_probe",
            function,
            "--root",
            otp_root().to_str().unwrap(),
            "--",
            NOTIFY_PROBE,
        ])
        .args(BOOT_MODULES.map(pinned_beam_path))
        .args(LOG_EVENT_MODULES.map(pinned_beam_path))
        .arg(boot_beam());
    dumb_utf8(&mut command);
    command
}

/// Set `TERM=dumb` and `LC_ALL=C.UTF-8`, which give the same terminal and
/// encoding to ERTS and to PBeam.
///
/// # Examples
///
/// ```rust,ignore
/// dumb_utf8(&mut command);
/// ```
fn dumb_utf8(command: &mut Command) {
    command.env("TERM", "dumb").env("LC_ALL", "C.UTF-8");
}

/// Run `command` with stdin `/dev/null`. For each step, wait until stdout
/// holds the text of the step, then send the signal of the step to the
/// run. Return the output when the run ends. Kill the run and panic when a
/// text or the end does not come in `RUN_TIME_LIMIT`.
///
/// # Examples
///
/// ```rust,ignore
/// let output = run_with_signals(&mut pbeam("wait"), &[("ready\n", libc::SIGQUIT)]);
/// assert_eq!(output, (Some(0), String::from("ready\n"), String::new()));
/// ```
fn run_with_signals(command: &mut Command, steps: &[(&str, c_int)]) -> Output {
    let mut child = command
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    let stdout = Arc::new(Mutex::new(Vec::new()));
    let stdout_reader = collect(child.stdout.take().unwrap(), Arc::clone(&stdout));
    let stderr = Arc::new(Mutex::new(Vec::new()));
    let stderr_reader = collect(child.stderr.take().unwrap(), Arc::clone(&stderr));
    for &(text, signal) in steps {
        wait_for(&mut child, &stdout, text.as_bytes());
        send(&child, signal);
    }
    let status = wait_for_exit(&mut child, &stdout);
    stdout_reader.join().unwrap();
    stderr_reader.join().unwrap();
    let text =
        |bytes: &Arc<Mutex<Vec<u8>>>| String::from_utf8_lossy(&bytes.lock().unwrap()).into_owned();
    (status, text(&stdout), text(&stderr))
}

/// Read `source` until its end into `output` in a new thread.
///
/// # Examples
///
/// ```rust,ignore
/// let reader = collect(child.stdout.take().unwrap(), Arc::clone(&stdout));
/// reader.join().unwrap();
/// ```
fn collect(mut source: impl Read + Send + 'static, output: Arc<Mutex<Vec<u8>>>) -> JoinHandle<()> {
    thread::spawn(move || {
        let mut buffer = [0; 4096];
        // A read of a pseudo-terminal master fails with `EIO` on Linux, or
        // gives 0 bytes on macOS, after each slave fd is closed.
        while let Ok(count @ 1..) = source.read(&mut buffer) {
            output.lock().unwrap().extend_from_slice(&buffer[..count]);
        }
    })
}

/// Wait until `output` holds `text`. Kill `child` and panic when it does
/// not come in `RUN_TIME_LIMIT`.
///
/// # Examples
///
/// ```rust,ignore
/// wait_for(&mut child, &stdout, b"ready\n");
/// ```
fn wait_for(child: &mut Child, output: &Mutex<Vec<u8>>, text: &[u8]) {
    let start = Instant::now();
    while !output
        .lock()
        .unwrap()
        .windows(text.len())
        .any(|window| window == text)
    {
        if start.elapsed() >= RUN_TIME_LIMIT {
            child.kill().unwrap();
            panic!(
                "no {:?} in the output {:?}",
                String::from_utf8_lossy(text),
                String::from_utf8_lossy(&output.lock().unwrap())
            );
        }
        thread::sleep(Duration::from_millis(10));
    }
}

/// Wait until `child` exits, and return its exit status code. Kill it and
/// panic with its output when it does not end in `RUN_TIME_LIMIT`.
///
/// # Examples
///
/// ```rust,ignore
/// assert_eq!(wait_for_exit(&mut child, &stdout), Some(0));
/// ```
fn wait_for_exit(child: &mut Child, output: &Mutex<Vec<u8>>) -> Option<i32> {
    let start = Instant::now();
    loop {
        if let Some(status) = child.try_wait().unwrap() {
            return status.code();
        }
        if start.elapsed() >= RUN_TIME_LIMIT {
            child.kill().unwrap();
            panic!(
                "the run did not end: {:?}",
                String::from_utf8_lossy(&output.lock().unwrap())
            );
        }
        thread::sleep(Duration::from_millis(10));
    }
}

/// Send `signal` to `child`.
///
/// # Examples
///
/// ```rust,ignore
/// send(&child, libc::SIGTERM);
/// ```
fn send(child: &Child, signal: c_int) {
    let pid = libc::pid_t::try_from(child.id()).unwrap();
    // SAFETY: `kill` sends a signal to the child process, which the test
    // has not waited for, so its pid is not reused.
    assert_eq!(unsafe { libc::kill(pid, signal) }, 0);
}

/// Run `command` in a new session whose controlling terminal is a new
/// pseudo-terminal, which is also its stdin, stdout, and stderr. Wait for
/// "ready", type `^\`, and return all output of the terminal after the
/// command exits with status 0.
///
/// # Examples
///
/// ```rust,ignore
/// let output = run_in_terminal(&mut pbeam("wait"));
/// ```
fn run_in_terminal(command: &mut Command) -> Vec<u8> {
    let (master, slave) = open_terminal();
    // SAFETY: the closure runs in the child before `exec`, and calls only
    // the async-signal-safe `setsid` and `ioctl`. `setsid` makes the child a
    // session leader with no controlling terminal, and `TIOCSCTTY` makes
    // its stdin, the slave, the controlling terminal.
    unsafe {
        command.pre_exec(|| {
            if libc::setsid() == -1 || set_controlling_terminal() == -1 {
                return Err(io::Error::last_os_error());
            }
            Ok(())
        });
    }
    let mut child = command
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
    let mut terminal = std::fs::File::from(master);
    let output = Arc::new(Mutex::new(Vec::new()));
    let reader = collect(terminal.try_clone().unwrap(), Arc::clone(&output));
    wait_for(&mut child, &output, b"ready\r\n");
    terminal.write_all(b"\x1c").unwrap();
    let status = wait_for_exit(&mut child, &output);
    reader.join().unwrap();
    let output = output.lock().unwrap().clone();
    assert_eq!(status, Some(0), "{:?}", String::from_utf8_lossy(&output));
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

/// Make fd 0, a terminal, the controlling terminal of the calling session
/// leader with `TIOCSCTTY`, and return the result of `ioctl`.
///
/// # Examples
///
/// ```rust,ignore
/// assert_eq!(set_controlling_terminal(), 0);
/// ```
#[cfg_attr(
    not(target_os = "macos"),
    allow(
        clippy::useless_conversion,
        reason = "the request type is c_ulong, and TIOCSCTTY is c_uint on macOS"
    )
)]
fn set_controlling_terminal() -> c_int {
    // SAFETY: `TIOCSCTTY` reads only its integer argument.
    unsafe { libc::ioctl(0, libc::TIOCSCTTY.into(), 0) }
}

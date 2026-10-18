use std::{
    ffi::c_int,
    io::{self, Read, Write},
    os::{
        fd::{AsRawFd, FromRawFd, OwnedFd},
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
            run_with_signals(&mut command, b"", &steps),
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
            run_with_signals(&mut command, b"", &[("ready\n", libc::SIGHUP)]),
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
            let (status, stdout, stderr) =
                run_with_signals(&mut command, b"", &[("ready\n", signal)]);
            assert_eq!(
                (status, mask_time(&stdout), stderr),
                (Some(0), String::from(expected), String::new()),
                "{name} {signal}"
            );
        }
    }
}

/// The break menu of ERTS on stdout, with the line endings of the console
/// writer (`erts_printf`, `break.c:574-575`, `600`).
const BREAKS: &str = "ready\n\r\nBREAK: (a)bort (A)bort with dump (c)ontinue (p)roc info (i)nfo\r\n       (l)oaded (v)ersion (k)ill (D)b-tables (d)istribution\r\n";

#[test]
fn gives_a_runtime_error_for_a_crash_dump() {
    // `erl_signal_handler` calls `erlang:halt/1` with a slogan for SIGUSR1
    // (erl_signal_handler.erl:39-41), which writes a crash dump. PBeam
    // writes no crash dump, so the run gives a runtime error.
    assert_eq!(
        run_with_signals(&mut erts("wait"), b"", &[("ready\n", libc::SIGUSR1)]),
        (
            Some(1),
            String::from("ready\n"),
            String::from("Received SIGUSR1\r\n")
        ),
        "ERTS"
    );
    assert_eq!(
        run_with_signals(&mut pbeam("wait"), b"", &[("ready\n", libc::SIGUSR1)]),
        (
            Some(1),
            String::from("ready\n"),
            String::from(
                "pbeam: unsupported erlang:halt/2 with a crash dump slogan: PBeam writes no crash dump\n"
            )
        ),
        "PBeam"
    );
}

#[test]
fn runs_the_break_menu_of_a_sigint_as_pinned_erts() {
    // ERTS runs the break handler for SIGINT (sys.c:611-629, 1165-1167): it
    // writes the menu and reads a key from fd 0 (break.c:571-618). The end
    // of the input aborts the run with the status 0.
    for (name, mut command) in [("ERTS", erts("wait")), ("PBeam", pbeam("wait"))] {
        assert_eq!(
            run_with_signals(&mut command, b"", &[("ready\n", libc::SIGINT)]),
            (Some(0), String::from(BREAKS), String::new()),
            "{name}"
        );
    }
}

#[test]
fn aborts_the_break_menu_of_a_sigint_as_pinned_erts() {
    // The keys `a`, `q`, and `*` abort the run with the status 0
    // (break.c:606-614).
    for (name, mut command) in [("ERTS", erts("wait")), ("PBeam", pbeam("wait"))] {
        assert_eq!(
            run_with_signals(&mut command, b"a\n", &[("ready\n", libc::SIGINT)]),
            (Some(0), String::from(BREAKS), String::new()),
            "{name}"
        );
    }
}

#[test]
fn continues_after_an_invalid_break_key_as_pinned_erts() {
    // An invalid key writes its message and the menu, and reads the next
    // key (break.c:681-684); the key `a` then aborts the run
    // (break.c:606-614). Each key comes in its own write, as a key from a
    // terminal does, because `sys_get_key` reads up to 64 bytes and uses
    // only the first (`sys.c:1051-1071`).
    let steps = [
        ("ready\n", Action::Signal(libc::SIGINT)),
        ("BREAK:", Action::Write(b"x")),
        ("Invalid option", Action::Write(b"a")),
    ];
    let erts = run_with_actions(&mut erts("wait"), b"", &steps);
    let pbeam = run_with_actions(&mut pbeam("wait"), b"", &steps);
    assert_eq!(erts, pbeam, "ERTS and PBeam");
    let (status, stdout, stderr) = pbeam;
    assert_eq!(status, Some(0));
    assert_eq!(stderr, "");
    assert!(stdout.starts_with(BREAKS), "{stdout:?}");
    assert!(stdout.contains("Invalid option 'x'."), "{stdout:?}");
}

#[test]
fn writes_an_invalid_break_key_as_its_byte_as_pinned_erts() {
    // The invalid-key message writes the key byte itself, as the `%c` of
    // `break.c:682-684` does, also for a byte above 0x7f.
    let steps = [
        ("ready\n", Action::Signal(libc::SIGINT)),
        ("BREAK:", Action::Write(b"\xff")),
    ];
    let erts = run_with_actions_raw(&mut erts("wait"), b"", &steps);
    let pbeam = run_with_actions_raw(&mut pbeam("wait"), b"", &steps);
    assert_eq!(erts, pbeam, "ERTS and PBeam");
    let (status, stdout, stderr) = pbeam;
    assert_eq!(status, Some(0));
    assert!(stderr.is_empty(), "{stderr:?}");
    let message = b"Invalid option '\xff'.";
    assert!(
        stdout
            .windows(message.len())
            .any(|window| window == message),
        "{stdout:?}"
    );
}

#[test]
fn continues_the_break_menu_on_c_as_pinned_erts() {
    // The key `c` continues the run (break.c:617-618), and the timer of the
    // probe then writes `done` and ends the run with the status 0.
    let steps = [
        ("ready\n", Action::Signal(libc::SIGINT)),
        ("BREAK:", Action::Write(b"c")),
    ];
    let erts = run_with_actions(&mut erts("done"), b"", &steps);
    let pbeam = run_with_actions(&mut pbeam("done"), b"", &steps);
    assert_eq!(erts, pbeam, "ERTS and PBeam");
    let (status, stdout, stderr) = pbeam;
    assert_eq!(status, Some(0));
    assert_eq!(stderr, "");
    assert!(stdout.starts_with(BREAKS), "{stdout:?}");
    assert!(stdout.ends_with("done\n"), "{stdout:?}");
}

#[test]
fn continues_the_break_menu_after_a_read_of_64_bytes_as_pinned_erts() {
    // `sys_get_key` reads up to 64 bytes and uses the first
    // (`sys.c:1051-1071`): the key `c` continues the run, and the last byte
    // of the write is the next key of the menu.
    let mut input = vec![b'c'];
    input.extend_from_slice(&[b'x'; 64]);
    let steps = [
        ("ready\n", Action::Signal(libc::SIGINT)),
        ("done\n", Action::Signal(libc::SIGINT)),
    ];
    let erts = run_with_actions(&mut erts("hold"), &input, &steps);
    let pbeam = run_with_actions(&mut pbeam("hold"), &input, &steps);
    assert_eq!(erts, pbeam, "ERTS and PBeam");
    let (status, stdout, stderr) = pbeam;
    assert_eq!(status, Some(0));
    assert_eq!(stderr, "");
    assert!(stdout.contains("done\n"), "{stdout:?}");
    assert!(stdout.contains("Invalid option 'x'."), "{stdout:?}");
}
#[test]
fn aborts_the_break_menu_at_a_nul_key_as_pinned_erts() {
    // `sys_get_key` returns 0 for the NUL byte, and `do_break` aborts the
    // run with the status 0 (`break.c:602-604`).
    let steps = [
        ("ready\n", Action::Signal(libc::SIGINT)),
        ("BREAK:", Action::Write(b"\0")),
    ];
    for (name, mut command) in [("ERTS", erts("wait")), ("PBeam", pbeam("wait"))] {
        assert_eq!(
            run_with_actions(&mut command, b"", &steps),
            (Some(0), String::from(BREAKS), String::new()),
            "{name}"
        );
    }
}

#[test]
fn keeps_the_break_menu_open_for_another_signal_as_pinned_erts() {
    // A handled signal writes to the notify pipe without stopping the wait
    // of `sys_get_key`, so ERTS keeps the menu open (`sys.c:611-624`,
    // `break.c:602-604`). The key `a` then aborts the run.
    let erts = break_after_signal(&mut erts("wait"));
    let pbeam = break_after_signal(&mut pbeam("wait"));
    assert_eq!(erts, pbeam, "ERTS and PBeam");
    let (status, output) = pbeam;
    assert_eq!(status, Some(0));
    assert!(
        output
            .as_bytes()
            .windows(7)
            .any(|window| window == b"BREAK: "),
        "{output:?}"
    );
}

/// Run `command`, send `SIGINT`, send `SIGTERM` while the menu waits, check
/// that the run is alive, and then send the key `a`. Return the exit status
/// and the output. A run that ended on `SIGTERM` fails the check.
fn break_after_signal(command: &mut Command) -> (Option<i32>, String) {
    let mut child = command
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    let mut stdin = child.stdin.take().expect("the child has a stdin");
    let stdout = Arc::new(Mutex::new(Vec::new()));
    let stdout_reader = collect(child.stdout.take().unwrap(), Arc::clone(&stdout));
    let stderr = Arc::new(Mutex::new(Vec::new()));
    let stderr_reader = collect(child.stderr.take().unwrap(), Arc::clone(&stderr));
    wait_for(&mut child, &stdout, b"ready\n");
    send(&child, libc::SIGINT);
    wait_for(&mut child, &stdout, b"BREAK: ");
    send(&child, libc::SIGTERM);
    thread::sleep(Duration::from_millis(200));
    assert!(
        child.try_wait().unwrap().is_none(),
        "the run ended on SIGTERM in the break menu"
    );
    stdin
        .write_all(b"a")
        .expect("the input pipe takes the bytes");
    drop(stdin);
    let status = wait_for_exit(&mut child, &stdout);
    stdout_reader.join().unwrap();
    stderr_reader.join().unwrap();
    let output = String::from_utf8_lossy(&stdout.lock().unwrap()).into_owned();
    (status, output)
}

#[test]
fn stops_at_two_sigints_as_pinned_erts() {
    // The second break request stops the run (`break_requested`,
    // `sys.c:611-624`). Send it after the menu is open, so that the two
    // signals do not coalesce, and keep stdin open, so that the run must
    // not depend on its end.
    for (name, mut command) in [("ERTS", erts("wait")), ("PBeam", pbeam("wait"))] {
        let mut child = command
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap();
        let stdin = child.stdin.take().expect("the child has a stdin");
        let stdout = Arc::new(Mutex::new(Vec::new()));
        let stdout_reader = collect(child.stdout.take().unwrap(), Arc::clone(&stdout));
        let stderr = Arc::new(Mutex::new(Vec::new()));
        let stderr_reader = collect(child.stderr.take().unwrap(), Arc::clone(&stderr));
        wait_for(&mut child, &stdout, b"ready\n");
        send(&child, libc::SIGINT);
        wait_for(&mut child, &stdout, b"BREAK: ");
        send(&child, libc::SIGINT);
        let status = wait_for_exit(&mut child, &stdout);
        drop(stdin);
        stdout_reader.join().unwrap();
        stderr_reader.join().unwrap();
        assert_eq!(status, Some(0), "{name}");
    }
}

#[test]
fn delivers_a_signal_that_arrived_during_the_menu_as_pinned_erts() {
    // SIGTERM during the menu waits in the notify pipe. The key `c`
    // continues the run, and the scheduler then sends the signal to
    // `erl_signal_server`, whose default handler writes the report and stops
    // the node (`erl_signal_handler.erl:39-48`).
    let erts = continue_after_signal(&mut erts("hold"));
    let pbeam = continue_after_signal(&mut pbeam("hold"));
    assert_eq!(erts, pbeam, "ERTS and PBeam");
    let (status, stdout, stderr) = pbeam;
    assert_eq!(status, Some(0));
    assert_eq!(stderr, "");
    assert!(
        stdout.contains("SIGTERM received - shutting down"),
        "{stdout:?}"
    );
}

/// Run `command`, send `SIGINT`, send `SIGTERM` while the menu waits, and
/// send the key `c` after a pause. Return the exit status and the output,
/// with the time of a log report masked.
fn continue_after_signal(command: &mut Command) -> (Option<i32>, String, String) {
    let mut child = command
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    let mut stdin = child.stdin.take().expect("the child has a stdin");
    let stdout = Arc::new(Mutex::new(Vec::new()));
    let stdout_reader = collect(child.stdout.take().unwrap(), Arc::clone(&stdout));
    let stderr = Arc::new(Mutex::new(Vec::new()));
    let stderr_reader = collect(child.stderr.take().unwrap(), Arc::clone(&stderr));
    wait_for(&mut child, &stdout, b"ready\n");
    send(&child, libc::SIGINT);
    wait_for(&mut child, &stdout, b"BREAK: ");
    send(&child, libc::SIGTERM);
    thread::sleep(Duration::from_millis(200));
    stdin
        .write_all(b"c")
        .expect("the input pipe takes the bytes");
    drop(stdin);
    let status = wait_for_exit(&mut child, &stdout);
    stdout_reader.join().unwrap();
    stderr_reader.join().unwrap();
    let stdout = mask_time(&String::from_utf8_lossy(&stdout.lock().unwrap()));
    let stderr = String::from_utf8_lossy(&stderr.lock().unwrap()).into_owned();
    (status, stdout, stderr)
}

#[test]
fn stops_the_break_menu_at_a_closed_stdin_as_pinned_erts() {
    // A `select` on a closed stdin fails, `sys_get_key` returns 0, and the
    // run stops with status 0 (`sys.c:1051-1071`, `break.c:602-604`).
    let steps = [("ready\n", libc::SIGINT)];
    let erts = run_with_signals(closes_stdin(&mut erts("wait")), b"", &steps);
    let pbeam = run_with_signals(closes_stdin(&mut pbeam("wait")), b"", &steps);
    assert_eq!(erts, pbeam, "ERTS and PBeam");
    assert_eq!(pbeam.0, Some(0));
    assert!(
        pbeam
            .1
            .as_bytes()
            .windows(7)
            .any(|window| window == b"BREAK: "),
        "{:?}",
        pbeam.1
    );
}

/// Close fd 0 of the child before `exec`.
fn closes_stdin(command: &mut Command) -> &mut Command {
    // SAFETY: the closure runs in the child before `exec` and calls only the
    // async-signal-safe `close`.
    unsafe {
        command.pre_exec(|| {
            libc::close(0);
            Ok(())
        });
    }
    command
}

#[test]
fn stops_the_break_menu_at_a_second_sigint_as_pinned_erts() {
    // A second SIGINT while the menu waits stops the run with status 0
    // (`sys.c:611-624`, `break.c:602-604`), while stdin stays open.
    let erts = break_twice(&mut erts("wait"));
    let pbeam = break_twice(&mut pbeam("wait"));
    assert_eq!(erts, pbeam, "ERTS and PBeam");
    let (status, output) = pbeam;
    assert_eq!(status, Some(0));
    assert!(
        output
            .as_bytes()
            .windows(7)
            .any(|window| window == b"BREAK: "),
        "{output:?}"
    );
}

/// Run `command` with stdin open, send `SIGINT`, wait for the menu, send a
/// second `SIGINT`, and return the exit status and the output.
fn break_twice(command: &mut Command) -> (Option<i32>, String) {
    let mut child = command
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    let stdin = child.stdin.take().expect("the child has a stdin");
    let stdout = Arc::new(Mutex::new(Vec::new()));
    let stdout_reader = collect(child.stdout.take().unwrap(), Arc::clone(&stdout));
    let stderr = Arc::new(Mutex::new(Vec::new()));
    let stderr_reader = collect(child.stderr.take().unwrap(), Arc::clone(&stderr));
    wait_for(&mut child, &stdout, b"ready\n");
    send(&child, libc::SIGINT);
    wait_for(&mut child, &stdout, b"BREAK: ");
    send(&child, libc::SIGINT);
    let status = wait_for_exit(&mut child, &stdout);
    drop(stdin);
    stdout_reader.join().unwrap();
    stderr_reader.join().unwrap();
    let output = String::from_utf8_lossy(&stdout.lock().unwrap()).into_owned();
    (status, output)
}

#[test]
fn gives_a_runtime_error_for_a_break_command_that_pbeam_does_not_run() {
    // The crash dump of `A` (break.c:615-616) and the information commands
    // (break.c:619-646) are not implemented, so PBeam names the command in a
    // runtime error. The "OS signals" row of `docs/erts-differences.md`
    // records the difference.
    for (key, detail) in [
        (b'A', "unsupported break command A"),
        (b'p', "unsupported break command p"),
    ] {
        let input = [key, b'\n'];
        let (status, stdout, stderr) =
            run_with_signals(&mut pbeam("wait"), &input, &[("ready\n", libc::SIGINT)]);
        assert_eq!(status, Some(1), "{key}");
        assert_eq!(stdout, BREAKS, "{key}");
        assert!(stderr.contains(detail), "expected {detail} in {stderr:?}");
    }
}

#[test]
fn reads_a_break_key_in_the_initial_terminal_mode_as_pinned_erts() {
    // ERTS sets fd 0 to the mode of the start of the run during the menu
    // and restores the application mode after it (`sys.c:899-923`). A key
    // that the application reads in raw mode then waits in the line
    // discipline for the newline, as in the initial mode.
    for (name, mut command) in [("ERTS", erts("wait")), ("PBeam", pbeam("wait"))] {
        let (status, output) = break_in_raw_terminal(&mut command);
        assert_eq!(status, Some(0), "{name}");
        assert!(
            output.windows(7).any(|window| window == b"BREAK: "),
            "{name}: {:?}",
            String::from_utf8_lossy(&output)
        );
    }
}

/// Run `command` in a pseudo-terminal, set the terminal to raw mode, send
/// `SIGINT`, and write the key `a` without a newline. Return the exit status
/// and the output. Assert that the run waits for the newline, which gives
/// the key in the initial terminal mode.
fn break_in_raw_terminal(command: &mut Command) -> (Option<i32>, Vec<u8>) {
    let (master, slave) = open_terminal();
    let raw = slave.try_clone().unwrap();
    // SAFETY: the closure runs in the child before `exec`, and calls only
    // the async-signal-safe `setsid` and `ioctl`, as `run_in_terminal` does.
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
    command
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    let mut terminal = std::fs::File::from(master);
    let output = Arc::new(Mutex::new(Vec::new()));
    let reader = collect(terminal.try_clone().unwrap(), Arc::clone(&output));
    wait_for(&mut child, &output, b"ready\r\n");
    set_raw(&raw);
    // Drop the slave: while the test holds it, the master gives no end of
    // the output after the run exits.
    drop(raw);
    send(&child, libc::SIGINT);
    wait_for(&mut child, &output, b"BREAK: ");
    terminal.write_all(b"a").unwrap();
    thread::sleep(Duration::from_millis(200));
    assert!(
        child.try_wait().unwrap().is_none(),
        "the run read the key before the newline"
    );
    terminal.write_all(b"\n").unwrap();
    let status = wait_for_exit(&mut child, &output);
    reader.join().unwrap();
    let output = output.lock().unwrap().clone();
    (status, output)
}

/// Set `fd`, a pseudo-terminal, to raw mode, as an application does with
/// `cfmakeraw`.
fn set_raw(fd: &OwnedFd) {
    // SAFETY: `tcgetattr` writes one `termios` and `tcsetattr` reads one
    // from the terminal fd, which the test owns.
    unsafe {
        let mut mode = std::mem::zeroed();
        assert_eq!(libc::tcgetattr(fd.as_raw_fd(), &raw mut mode), 0);
        libc::cfmakeraw(&raw mut mode);
        assert_eq!(libc::tcsetattr(fd.as_raw_fd(), libc::TCSANOW, &mode), 0);
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

/// Run `command` with the bytes `input` on stdin, which then ends. For each
/// step, wait until stdout holds the text of the step, then send the signal
/// of the step to the run. Return the output when the run ends. Kill the run
/// and panic when a text or the end does not come in `RUN_TIME_LIMIT`.
///
/// # Examples
///
/// ```rust,ignore
/// let output = run_with_signals(&mut pbeam("wait"), b"", &[("ready\n", libc::SIGQUIT)]);
/// assert_eq!(output, (Some(0), String::from("ready\n"), String::new()));
/// ```
fn run_with_signals(command: &mut Command, input: &[u8], steps: &[(&str, c_int)]) -> Output {
    let steps: Vec<_> = steps
        .iter()
        .map(|&(text, signal)| (text, Action::Signal(signal)))
        .collect();
    run_with_actions(command, input, &steps)
}

/// An action of [`run_with_actions`] after its step text.
#[derive(Clone, Copy)]
enum Action {
    /// Write these bytes to stdin.
    Write(&'static [u8]),
    /// Send this signal to the run.
    Signal(c_int),
}

/// Run `command` with the bytes `input` on stdin and the steps of `steps`:
/// wait until stdout holds the text of a step, then do its action. Return
/// the output when the run ends, and give the run the end of the input after
/// the steps. Kill the run and panic when a text or the end does not come in
/// `RUN_TIME_LIMIT`.
///
/// # Examples
///
/// ```rust,ignore
/// let steps = [("ready\n", Action::Signal(libc::SIGTERM))];
/// let output = run_with_actions(&mut pbeam("wait"), b"", &steps);
/// assert_eq!(output.0, Some(0));
/// ```
fn run_with_actions(command: &mut Command, input: &[u8], steps: &[(&str, Action)]) -> Output {
    let (status, stdout, stderr) = run_with_actions_raw(command, input, steps);
    let text = |bytes: &[u8]| String::from_utf8_lossy(bytes).into_owned();
    (status, text(&stdout), text(&stderr))
}

/// Run the steps of [`run_with_actions`] and return the raw bytes of the
/// output, so that a comparison does not lose a byte that is not UTF-8.
fn run_with_actions_raw(
    command: &mut Command,
    input: &[u8],
    steps: &[(&str, Action)],
) -> (Option<i32>, Vec<u8>, Vec<u8>) {
    let mut child = command
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    let mut stdin = child.stdin.take().expect("the child has a stdin");
    stdin
        .write_all(input)
        .expect("the input pipe takes the bytes");
    let stdout = Arc::new(Mutex::new(Vec::new()));
    let stdout_reader = collect(child.stdout.take().unwrap(), Arc::clone(&stdout));
    let stderr = Arc::new(Mutex::new(Vec::new()));
    let stderr_reader = collect(child.stderr.take().unwrap(), Arc::clone(&stderr));
    for &(text, action) in steps {
        wait_for(&mut child, &stdout, text.as_bytes());
        match action {
            Action::Write(bytes) => stdin
                .write_all(bytes)
                .expect("the input pipe takes the bytes"),
            Action::Signal(signal) => send(&child, signal),
        }
    }
    drop(stdin);
    let status = wait_for_exit(&mut child, &stdout);
    stdout_reader.join().unwrap();
    stderr_reader.join().unwrap();
    let stdout = stdout.lock().unwrap().clone();
    let stderr = stderr.lock().unwrap().clone();
    (status, stdout, stderr)
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

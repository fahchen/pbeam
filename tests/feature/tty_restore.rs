use std::{
    io::{Read, Write},
    os::fd::{FromRawFd, OwnedFd},
    path::PathBuf,
    process::Command,
    sync::{Arc, LazyLock, Mutex},
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

const TTY_RESTORE_PROBE: &str = "fixtures/tty_restore/tty_restore_probe.beam";

/// The time that a run has to write its prompt, and to end after the test
/// types its line.
const RUN_TIME_LIMIT: Duration = Duration::from_secs(10);

/// The paths of the pinned `BOOT_MODULES`. Each `pinned_beam_path` call
/// starts `erl`, so the tests of this crate find the paths only once.
static BOOT_PATHS: LazyLock<[PathBuf; BOOT_MODULES.len()]> =
    LazyLock::new(|| BOOT_MODULES.map(pinned_beam_path));

/// The terminal modes of one run, as `stty -g` writes them, and the exit
/// status and the output of the run.
#[derive(Debug)]
struct Modes {
    /// The mode before the run starts.
    first: String,
    /// The mode after the test sets raw mode during the run.
    raw: String,
    /// The mode after the run ends.
    last: String,
    /// The exit status code of the run.
    status: Option<i32>,
    /// The stdout and stderr of the run, for the failure messages.
    output: String,
}

#[test]
fn sets_fd_0_to_its_first_mode_at_the_run_end_as_pinned_erts() {
    // Stdin, stdout, and stderr are one pseudo-terminal. Each function of
    // `tty_restore_probe` waits in `io:get_line/2` for a line. While it
    // waits, the test sets the terminal to raw mode with `stty raw -echo`
    // and types the line. Then `main/0` returns, `halt/0` calls
    // `erlang:halt/0`, and `crash/0` exits with the reason `boom`. At the
    // emulator exit, ERTS sets fd 0 to its mode at the emulator start
    // (`sys_tty_reset`, sys.c:182-190; erl_child_setup.c:563-576). PBeam
    // does the same at the end of the run, also when the run ends with a
    // runtime error: PBeam has no `erlang:halt/2` (see "Run end" in
    // docs/erts-differences.md). The reader of `prim_tty` has a read select
    // of fd 0 at the end. On macOS, the kernel check of that select clears
    // `PENDIN`, which the change back to `ICANON` sets.
    for (function, erts_status, pbeam_status) in [("main", 0, 0), ("halt", 0, 1), ("crash", 1, 1)] {
        for (name, command, status) in [
            ("ERTS", erts(function), erts_status),
            ("PBeam", pbeam(function), pbeam_status),
        ] {
            let modes = run_in_terminal(command, true);
            assert_ne!(modes.raw, modes.first, "{name} {function}: {modes:?}");
            assert_eq!(
                (modes.last.as_str(), modes.status),
                (modes.first.as_str(), Some(status)),
                "{name} {function}: {:?}",
                modes.output
            );
        }
    }
}

#[test]
fn keeps_the_mode_of_fd_0_when_stdout_is_not_a_terminal() {
    // Stdin is a pseudo-terminal, and stdout and stderr are a pipe. ERTS
    // sets fd 0 to its first mode only when fds 0 and 1 are terminals
    // (sys.c:187, erl_child_setup.c:565, 574), so the raw mode stays.
    for (name, command) in [("ERTS", erts("main")), ("PBeam", pbeam("main"))] {
        let modes = run_in_terminal(command, false);
        assert_ne!(modes.raw, modes.first, "{name}: {modes:?}");
        assert_eq!(
            (modes.last.as_str(), modes.status),
            (modes.raw.as_str(), Some(0)),
            "{name}: {:?}",
            modes.output
        );
    }
}

/// Return the `erl -noshell` command that evaluates
/// `tty_restore_probe:Function()` and `halt()`.
///
/// # Examples
///
/// ```rust,ignore
/// let command = erts("main");
/// ```
fn erts(function: &str) -> Command {
    let mut command = Command::new("erl");
    command
        .args([
            "-noshell",
            "-pa",
            "fixtures/tty_restore",
            "-eval",
            &format!("tty_restore_probe:{function}(), halt()."),
        ])
        // Without it, ERTS writes `erl_crash.dump` for `crash/0`.
        .env("ERL_CRASH_DUMP_SECONDS", "0");
    command
}

/// Return the `pbeam` command that boots with `pbeam_boot` and runs
/// `tty_restore_probe:Function()`.
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
            "tty_restore_probe",
            function,
            "--",
            TTY_RESTORE_PROBE,
        ])
        .args(BOOT_PATHS.iter())
        .arg(boot_beam());
    command
}

/// Run `command` with `TERM=dumb` and `LC_ALL=C.UTF-8`, and with a new
/// pseudo-terminal as its stdin. Its stdout and stderr are the terminal
/// when `terminal_output` is `true`, or else one pipe. When the output has
/// the prompt "ready> ", set the terminal to raw mode and type "go\n".
/// Return the modes before the run, after the change to raw mode, and after
/// the run, with the exit status. Panic when the prompt or the exit does
/// not come in `RUN_TIME_LIMIT`.
///
/// # Examples
///
/// ```rust,ignore
/// let modes = run_in_terminal(erts("main"), true);
/// assert_eq!(modes.last, modes.first);
/// ```
fn run_in_terminal(mut command: Command, terminal_output: bool) -> Modes {
    let (master, slave) = open_terminal();
    let first = stty(&slave, &["-g"]);
    let (output_reader, output_writer) = if terminal_output {
        (master.try_clone().unwrap(), slave.try_clone().unwrap())
    } else {
        let (reader, writer) = std::io::pipe().unwrap();
        (OwnedFd::from(reader), OwnedFd::from(writer))
    };
    let mut child = command
        .env("TERM", "dumb")
        .env("LC_ALL", "C.UTF-8")
        .stdin(slave.try_clone().unwrap())
        .stdout(output_writer.try_clone().unwrap())
        .stderr(output_writer)
        .spawn()
        .unwrap();
    // The command keeps its standard streams. Drop it, so that the reader
    // gets the end of the output when the child and the test close their
    // fds.
    drop(command);
    let output = Arc::new(Mutex::new(Vec::new()));
    let reading = {
        let output = Arc::clone(&output);
        let mut source = std::fs::File::from(output_reader);
        // A read of the master fails with `EIO` on Linux, or gives 0 bytes
        // on macOS, after each slave fd is closed.
        thread::spawn(move || {
            let mut buffer = [0; 4096];
            while let Ok(count @ 1..) = source.read(&mut buffer) {
                output.lock().unwrap().extend_from_slice(&buffer[..count]);
            }
        })
    };
    let start = Instant::now();
    while !output
        .lock()
        .unwrap()
        .windows(b"ready> ".len())
        .any(|window| window == b"ready> ")
    {
        if start.elapsed() >= RUN_TIME_LIMIT {
            child.kill().unwrap();
            panic!(
                "no prompt: {:?}",
                String::from_utf8_lossy(&output.lock().unwrap())
            );
        }
        thread::sleep(Duration::from_millis(10));
    }
    stty(&slave, &["raw", "-echo"]);
    let raw = stty(&slave, &["-g"]);
    // The master stays open until the end of the run. Its close would hang
    // up the terminal.
    std::fs::File::from(master.try_clone().unwrap())
        .write_all(b"go\n")
        .unwrap();
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
    let last = stty(&slave, &["-g"]);
    drop((master, slave));
    reading.join().unwrap();
    let output = String::from_utf8_lossy(&output.lock().unwrap()).into_owned();
    Modes {
        first,
        raw,
        last,
        status: status.code(),
        output,
    }
}

/// Run `stty` with `arguments` on the terminal `slave`, and return its
/// stdout without the line end.
///
/// # Examples
///
/// ```rust,ignore
/// let mode = stty(&slave, &["-g"]);
/// stty(&slave, &["raw", "-echo"]);
/// ```
fn stty(slave: &OwnedFd, arguments: &[&str]) -> String {
    let output = Command::new("stty")
        .args(arguments)
        .stdin(slave.try_clone().unwrap())
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "stty {arguments:?}: {}: {}",
        output.status,
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8(output.stdout)
        .unwrap()
        .trim_end()
        .to_owned()
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

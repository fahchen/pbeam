//! Sets the terminal mode with `stty` through a port and compares the result and the exit status
//! with the pinned ERTS.

use std::{
    io::{self, Read},
    os::{
        fd::{FromRawFd, OwnedFd},
        unix::process::CommandExt,
    },
    process::{Child, Command, Stdio},
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

const PORT_PROBE: &str = "fixtures/port/port_probe.beam";

/// The time that a run has to end.
const RUN_TIME_LIMIT: Duration = Duration::from_secs(10);

#[test]
fn sets_the_terminal_mode_with_stty_through_a_port_as_pinned_erts() {
    // `port_probe:terminal/0` runs the stty path of the Terra TTY backend
    // (lib/terra/terminal/tty.ex:88-123): `sh -c 'stty -isig 2>/dev/null'`
    // through a `nouse_stdio` port with `exit_status`, then `stty isig`.
    // With `nouse_stdio`, the child keeps fd 0 of the runtime, which is the
    // terminal, and its output pipe is fd 4 (erl_child_setup.c:315-336).
    // After each step, a second program writes the ISIG state of the
    // terminal to fd 4. The child starts a new session
    // (erl_child_setup.c:347-352), and `stty` sets the mode of fd 0 also
    // when the terminal is not its controlling terminal.
    let expected = "ok off ok on\r\n";
    for (name, mut command) in [("ERTS", erts()), ("PBeam", pbeam())] {
        assert_eq!(
            String::from_utf8_lossy(&run_in_terminal(&mut command)),
            expected,
            "{name}"
        );
    }
}

#[test]
fn reports_the_exit_status_of_stty_without_a_terminal_as_pinned_erts() {
    // With stdin `/dev/null`, `stty` fails with status 1, and the ISIG state
    // reads `on`, because `stty -a` fails too.
    let expected = "stty_exit 1 on stty_exit 1 on\n";
    for (name, mut command) in [("ERTS", erts()), ("PBeam", pbeam())] {
        let child = command
            .stdin(Stdio::null())
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
            "{name}"
        );
    }
}

/// Return the `erl -noshell` command that evaluates
/// `port_probe:terminal()` and `halt()`.
///
/// # Examples
///
/// ```rust,ignore
/// let command = erts();
/// ```
fn erts() -> Command {
    let mut command = Command::new("erl");
    command.args([
        "-noshell",
        "-pa",
        "fixtures/port",
        "-eval",
        "port_probe:terminal(), halt().",
    ]);
    command
}

/// Return the `pbeam` command that boots with `pbeam_boot` and runs
/// `port_probe:terminal()`.
///
/// # Examples
///
/// ```rust,ignore
/// let command = pbeam();
/// ```
fn pbeam() -> Command {
    let mut command = Command::new(env!("CARGO_BIN_EXE_pbeam"));
    command
        .args([
            "pbeam_boot",
            "start",
            "port_probe",
            "terminal",
            "--root",
            otp_root().to_str().unwrap(),
            "--",
            PORT_PROBE,
        ])
        .args(BOOT_MODULES.map(pinned_beam_path))
        .arg(boot_beam());
    command
}

/// Wait until `child` exits, and return its exit status code, its stdout,
/// and its stderr. Kill it and panic when it does not end in
/// `RUN_TIME_LIMIT`.
///
/// # Examples
///
/// ```rust,ignore
/// let (status, stdout, stderr) = wait_with_output(child);
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

/// Run `command` with a new pseudo-terminal in its first mode as its
/// stdin, stdout, and stderr, and as the controlling terminal of a new
/// session, with `TERM=dumb`, as a terminal emulator starts a program.
/// Return all output of the terminal after the command exits with status
/// 0. Panic when the run does not end in `RUN_TIME_LIMIT`.
///
/// The controlling terminal is necessary for ERTS: when fd 0 is a terminal
/// that no session controls, the pinned `erl -noshell` exits with status 1
/// and no output after a `nouse_stdio` port program exits (measured on
/// macOS).
///
/// # Examples
///
/// ```rust,ignore
/// let output = run_in_terminal(&mut erts());
/// ```
fn run_in_terminal(command: &mut Command) -> Vec<u8> {
    let (master, slave) = open_terminal();
    // SAFETY: the hook runs in the child before `exec` and calls only
    // `setsid` and `ioctl`, which are async-signal-safe.
    unsafe {
        command.pre_exec(|| {
            // `TIOCSCTTY` is a `c_uint` on macOS and has the request type
            // of `ioctl` on Linux, so the conversion does nothing there.
            #[allow(clippy::useless_conversion)]
            let request = libc::TIOCSCTTY.into();
            if libc::setsid() < 0 || libc::ioctl(0, request, 0) < 0 {
                return Err(io::Error::last_os_error());
            }
            Ok(())
        });
    }
    let mut child = command
        .env("TERM", "dumb")
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
    // A read of the master fails with `EIO` on Linux, or gives 0 bytes on
    // macOS, after the child closed each slave fd.
    let reading = thread::spawn(move || {
        let mut output = Vec::new();
        let mut buffer = [0; 4096];
        while let Ok(count @ 1..) = terminal.read(&mut buffer) {
            output.extend_from_slice(&buffer[..count]);
        }
        output
    });
    let status = wait_status(&mut child);
    let output = reading.join().unwrap();
    assert_eq!(status, Some(0), "{:?}", String::from_utf8_lossy(&output));
    output
}

/// Wait until `child` exits and return its exit status code. Kill it and
/// panic when it does not end in `RUN_TIME_LIMIT`.
///
/// # Examples
///
/// ```rust,ignore
/// assert_eq!(wait_status(&mut child), Some(0));
/// ```
fn wait_status(child: &mut Child) -> Option<i32> {
    let start = Instant::now();
    loop {
        if let Some(status) = child.try_wait().unwrap() {
            return status.code();
        }
        if start.elapsed() >= RUN_TIME_LIMIT {
            child.kill().unwrap();
            panic!("the run did not end in {} s", RUN_TIME_LIMIT.as_secs());
        }
        thread::sleep(Duration::from_millis(10));
    }
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

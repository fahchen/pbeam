//! Reads and writes an IO device from a pipe and from a terminal in raw mode, and compares the
//! result with the pinned ERTS.
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

const IO_DEVICE_PROBE: &str = "fixtures/boot/io_device_probe.beam";

/// The pinned OTP modules that the probe calls in addition to the boot
/// modules: `file` for `file:read/2` and `file:write/2`, and `shell` for
/// `shell:start_interactive/1`.
const PROBE_MODULES: [&str; 2] = ["file", "shell"];

/// The time that a run has to end, and that a terminal run has to write
/// each output that the test waits for.
const RUN_TIME_LIMIT: Duration = Duration::from_secs(10);

#[test]
fn reads_and_writes_an_io_device_from_a_pipe_as_pinned_erts() {
    // `io_device_probe:main/0` calls `file:read/2` and `file:write/2` with
    // `standard_io` and with the pid of the group leader. Each call sends an
    // I/O request to `user` (file.erl:1618-1624, 1782-1788;
    // io.erl:1650-1685), which is `group` with the cooked `prim_tty` reader
    // of `user_drv`. Stdin is a pipe that holds all input and has no writer.
    //
    // A read in binary mode splits the input that `group` holds with
    // `erlang:split_binary/2` (io_lib.erl `collect_chars/4`). With the
    // latin1 encoding, `group` converts the UTF-8 input with
    // `unicode:characters_to_binary/3`, which calls `unicode:bin_is_7bit/1`
    // and `erlang:list_to_binary/1` (unicode.erl:345-368, 1101-1102), and a
    // write gives each byte as one character to the terminal. With the
    // unicode encoding, `user_drv` converts the bytes of a latin1 request
    // with `unicode:characters_to_binary(Bytes, latin1)`
    // (user_drv.erl:924-939), so the bytes 195 and 169 come out as 4 bytes.
    let input = b"abcdefgh\xc3\xa9xy\xc3\xa9z";
    let expected: &[u8] = b"atom device\npid device\n\xe9\n\xc3\x83\xc2\xa9\n\
        [ok,ok,{ok,[97,98,99]},{ok,[100,101]},{ok,<<102,103>>},{ok,<<104,233,120>>},ok,\
        {ok,<<121,233>>},ok,[{error,badarg},{error,badarg},{error,arguments},\
        {error,arguments},{error,calling_self},{error,calling_self},{error,terminated},\
        {error,terminated},{error,badarg},{error,badarg}],{ok,<<122>>},eof]\n";
    for (name, mut command) in [("ERTS", erts("main")), ("PBeam", pbeam("main"))] {
        let (reader, mut writer) = io::pipe().unwrap();
        writer.write_all(input).unwrap();
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
            (String::from_utf8_lossy(expected).as_ref(), "", Some(0)),
            "{name}"
        );
        assert_eq!(stdout, expected, "{name}");
    }
}

#[test]
fn reads_keys_from_a_terminal_in_raw_mode_as_pinned_erts() {
    // `io_device_probe:raw/0` reads keys as Terra does: raw mode, binary
    // mode, the latin1 encoding, and `file:read(standard_io, 1)`. The
    // `prim_tty` reader converts each input with
    // `unicode:characters_to_binary([Acc, Bytes], latin1, utf8)`
    // (prim_tty.erl:498), which calls the BIF
    // `unicode:characters_to_binary/2` with `latin1`. `group` converts the
    // characters back to latin1 bytes (group.erl:1324-1325). The key "é"
    // gives two bytes in one input, so the second read takes the second
    // byte from the buffer of `group` with `erlang:split_binary/2`. The test
    // types each key after the output of the key before it.
    let script: [(&[u8], &[u8]); 4] = [
        (b"ready\r\n", b"j"),
        (b"key <<106>>\r\n", "\u{e9}".as_bytes()),
        (b"key <<169>>\r\n", b"k"),
        (b"key <<107>>\r\n", b"q"),
    ];
    for (name, mut command) in [("ERTS", erts("raw")), ("PBeam", pbeam("raw"))] {
        assert_eq!(
            String::from_utf8_lossy(&run_in_terminal(&mut command, &script)),
            "ready\r\nkey <<106>>\r\nkey <<195>>\r\nkey <<169>>\r\nkey <<107>>\r\n",
            "{name}"
        );
    }
}

/// Return the `erl -noshell` command that evaluates
/// `io_device_probe:Function()` and `halt()`.
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
        &format!("io_device_probe:{function}(), halt()."),
    ]);
    command
}

/// Return the `pbeam` command that boots with `pbeam_boot` and runs
/// `io_device_probe:Function()`.
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
            "io_device_probe",
            function,
            "--root",
            otp_root().to_str().unwrap(),
            "--",
            IO_DEVICE_PROBE,
        ])
        .args(BOOT_MODULES.map(pinned_beam_path))
        .args(PROBE_MODULES.map(pinned_beam_path))
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

/// Run `command` with a new pseudo-terminal as its stdin, stdout, and
/// stderr. For each step of `script`, wait for its text in the output, then
/// type its input on the terminal. Return all output of the terminal after
/// the command exits with status 0. Panic when an output or the exit does
/// not come in `RUN_TIME_LIMIT`.
///
/// # Examples
///
/// ```rust,ignore
/// let output = run_in_terminal(&mut erts("raw"), &[(b"ready\r\n", b"q")]);
/// ```
fn run_in_terminal(command: &mut Command, script: &[(&[u8], &[u8])]) -> Vec<u8> {
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
    for &(text, input) in script {
        let start = Instant::now();
        while !output
            .lock()
            .unwrap()
            .windows(text.len())
            .any(|w| w == text)
        {
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

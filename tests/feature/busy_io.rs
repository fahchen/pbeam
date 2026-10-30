//! Checks that socket and port input and output progress while a process always can run.
#![expect(
    clippy::unwrap_used,
    reason = "test helpers report a failure by a panic"
)]

use std::{
    io::{BufRead, BufReader, Read, Write},
    net::TcpStream,
    process::{Command, Stdio},
    thread,
    time::Duration,
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

const BUSY_IO: &str = "fixtures/poll/busy_io.beam";

#[test]
fn gets_socket_and_port_input_while_a_process_always_can_run() {
    // `busy_io:main/0` spawns a process that always can run, and then waits
    // for an accept, a passive receive, and an active receive on a
    // `tcp_inet` socket, and for the data and the exit status of a spawn
    // port. The ERTS poll thread waits for the fds while processes run
    // (`erl_process.c:3339-3372`), and the scheduler handles them at a
    // switch point (`erl_process.c:9946-9960`). Without the PBeam node poll
    // thread, the one scheduler never waits for the fds, and each receive
    // gives `timeout`.
    let expected = "[[{ok,<<97,98>>},<<99,100>>],[{data,<<104,105,10>>},{exit_status,0}]]\n";
    let mut erts = Command::new("erl");
    erts.args([
        "+S",
        "1",
        "-noshell",
        "-boot",
        "start_clean",
        "-pa",
        "fixtures/poll",
        "-eval",
        "busy_io:main(), halt().",
    ]);
    let mut pbeam = Command::new(env!("CARGO_BIN_EXE_pbeam"));
    pbeam
        .args(["pbeam_boot", "start", "busy_io", "main", "--root"])
        .arg(otp_root())
        .args(["--", BUSY_IO])
        .args(BOOT_MODULES.map(pinned_beam_path))
        .arg(boot_beam());
    for (name, command) in [("ERTS", &mut erts), ("PBeam", &mut pbeam)] {
        assert_eq!(with_client(command), expected, "{name}");
    }
}

#[test]
fn flushes_a_full_socket_while_a_process_always_can_run() {
    // `busy_io:flush/0` fills the send buffer of a socket and then runs a
    // busy loop of 3 s while the client drains the socket. The ERTS poll
    // thread reports the write fd while the process runs
    // (`erl_process.c:3339-3372`), so the send ends before the loop ends.
    let mut erts = Command::new("erl");
    erts.args([
        "+S",
        "1",
        "-noshell",
        "-boot",
        "start_clean",
        "-pa",
        "fixtures/poll",
        "-eval",
        "busy_io:flush(), halt().",
    ]);
    let mut pbeam = Command::new(env!("CARGO_BIN_EXE_pbeam"));
    pbeam
        .args(["pbeam_boot", "start", "busy_io", "flush", "--root"])
        .arg(otp_root())
        .args(["--", BUSY_IO])
        .args(BOOT_MODULES.map(pinned_beam_path))
        .arg(boot_beam());
    for (name, command) in [("ERTS", &mut erts), ("PBeam", &mut pbeam)] {
        assert_eq!(with_drain(command), "sent_while_busy\n", "{name}");
    }
}

/// Run `command`, read the listen port from the first line of its standard
/// output, drain 8 MiB with a small receive buffer, and send one byte.
/// Return the rest of the standard output.
fn with_drain(command: &mut Command) -> String {
    let mut child = command
        .env("TERM", "dumb")
        .env("LC_ALL", "C.UTF-8")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::inherit())
        .spawn()
        .unwrap();
    let mut stdout = BufReader::new(child.stdout.take().unwrap());
    let mut first = String::new();
    stdout.read_line(&mut first).unwrap();
    let port: u16 = first.trim_end().parse().unwrap();
    let mut stream = TcpStream::connect(("127.0.0.1", port)).unwrap();
    stream
        .set_read_timeout(Some(Duration::from_secs(30)))
        .unwrap();
    let mut bytes = vec![0; 1 << 16];
    let mut count = 0;
    while count < 8 << 20 {
        match stream.read(&mut bytes) {
            Ok(0) | Err(_) => break,
            Ok(read) => count += read,
        }
    }
    stream.write_all(b"d").unwrap();
    let mut output = String::new();
    stdout.read_to_string(&mut output).unwrap();
    assert!(child.wait().unwrap().success());
    output
}

/// Run `command`, read the listen port from the first line of its standard
/// output, send "ab", wait, send "cd", and read until the probe closes.
/// Return the rest of the standard output.
fn with_client(command: &mut Command) -> String {
    let mut child = command
        .env("TERM", "dumb")
        .env("LC_ALL", "C.UTF-8")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::inherit())
        .spawn()
        .unwrap();
    let mut stdout = BufReader::new(child.stdout.take().unwrap());
    let mut first = String::new();
    stdout.read_line(&mut first).unwrap();
    let port: u16 = first.trim_end().parse().unwrap();
    let mut stream = TcpStream::connect(("127.0.0.1", port)).unwrap();
    stream.write_all(b"ab").unwrap();
    thread::sleep(Duration::from_millis(300));
    stream.write_all(b"cd").unwrap();
    stream
        .set_read_timeout(Some(Duration::from_secs(30)))
        .unwrap();
    let mut rest = Vec::new();
    let _ = stream.read_to_end(&mut rest);
    let mut output = String::new();
    stdout.read_to_string(&mut output).unwrap();
    assert!(child.wait().unwrap().success());
    output
}

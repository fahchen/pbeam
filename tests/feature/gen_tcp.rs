use std::{
    io::{BufRead, BufReader, Read, Write},
    net::TcpStream,
    process::{Command, Stdio},
    sync::mpsc,
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

const GEN_TCP_PROBE: &str = "fixtures/gen_tcp/gen_tcp_probe.beam";

const GEN_TCP_CONNECT: &str = "fixtures/gen_tcp/gen_tcp_connect.beam";

const GEN_TCP_LOCAL: &str = "fixtures/gen_tcp/gen_tcp_local.beam";

#[test]
fn runs_the_gen_tcp_calls_of_thousand_island_as_pinned_erts() {
    // `gen_tcp_probe:main/0` listens with the options of Thousand Island
    // (transports/tcp.ex:44-75) and `inet`, reads the socket name and
    // options, listens on the same port again, accepts two connections of
    // the client with `gen_tcp:accept/1,2`, reads with `gen_tcp:recv/2,3`
    // and `{active, once}`, moves the socket with
    // `gen_tcp:controlling_process/2`, and closes each socket with
    // `gen_tcp:close/1`. Both runs give the same line. The ERTS run gives
    // `-boot start_clean`, as `pbeam_boot` starts `kernel` and `stdlib`
    // only, and `+S 1`, as the `pbeam` binary runs one scheduler.
    let expected = "[[true,{0,0,0,0},true,{error,eaddrinuse},\
                    {ok,[{mode,binary},{active,false},{nodelay,true},{reuseaddr,true},\
                    {send_timeout,30000},{send_timeout_close,true}]},inet_tcp,{error,timeout}],\
                    [[ok,{ok,<<97,98,99>>},ok,{ok,<<100,101,102,103>>},{error,timeout}],\
                    [ok,ok,{tcp,<<104,105,106>>},{ok,[{active,false}]}],ok,\
                    [ok,ok,{tcp,<<107,108>>},ok,tcp_closed],ok],\
                    [{ok,[{mode,binary},{active,false},{nodelay,true},{send_timeout,30000},\
                    {send_timeout_close,true}]},ok,{ok,<<122>>},ok,undefined],\
                    [ok,{error,einval},{error,einval},undefined],[]]\n";
    let mut erts = Command::new("erl");
    erts.args([
        "+S",
        "1",
        "-noshell",
        "-boot",
        "start_clean",
        "-pa",
        "fixtures/gen_tcp",
        "-eval",
        "gen_tcp_probe:main(), halt().",
    ]);
    let mut pbeam = Command::new(env!("CARGO_BIN_EXE_pbeam"));
    pbeam
        .args(["pbeam_boot", "start", "gen_tcp_probe", "main", "--root"])
        .arg(otp_root())
        .args(["--", GEN_TCP_PROBE])
        .args(BOOT_MODULES.map(pinned_beam_path))
        .arg(boot_beam());
    for (name, command) in [("ERTS", &mut erts), ("PBeam", &mut pbeam)] {
        assert_eq!(
            with_client(name, command),
            (expected.into(), "".into(), Some(0)),
            "{name}"
        );
    }
}

/// Run `command`, read the listen port from the first line of its standard
/// output, and run the client on it. Return the rest of the standard
/// output, the standard error, and the status after the client has ended.
fn with_client(name: &str, command: &mut Command) -> (String, String, Option<i32>) {
    let mut child = command
        .env("TERM", "dumb")
        .env("LC_ALL", "C.UTF-8")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    // A run that does not end, or a client that fails while the probe waits
    // in `gen_tcp:accept/1`, kills the child, so the test reports the
    // failure and does not hang.
    let pid = child.id();
    let (done, finished) = mpsc::channel::<()>();
    let watchdog = thread::spawn(move || {
        if finished.recv_timeout(Duration::from_secs(60)).is_err() {
            kill(pid);
        }
    });
    let mut stdout = BufReader::new(child.stdout.take().unwrap());
    let mut first = String::new();
    stdout.read_line(&mut first).unwrap();
    let client = first.trim_end().parse::<u16>().map(|port| {
        thread::spawn(move || {
            let result = client(port);
            if result.is_err() {
                kill(pid);
            }
            result
        })
    });
    if client.is_err() {
        kill(pid);
    }
    let mut rest = String::new();
    stdout.read_to_string(&mut rest).unwrap();
    let _ = done.send(());
    watchdog.join().unwrap();
    let output = child.wait_with_output().unwrap();
    let stderr = String::from_utf8_lossy(&output.stderr).into_owned();
    let result = match client {
        Ok(client) => client.join().unwrap(),
        Err(_) => Err(format!("the first line {first:?} is not a port")),
    };
    if let Err(message) = result {
        panic!(
            "{name} client: {message}\nstatus: {}\nstdout: {first}{rest}\nstderr: {stderr}",
            output.status
        );
    }
    (rest, stderr, output.status.code())
}

/// Kill the process `pid` with `SIGKILL`.
fn kill(pid: u32) {
    let _ = Command::new("kill").args(["-9", &pid.to_string()]).status();
}

/// Connect twice to `port`. On the first connection, read each text of the
/// probe and send the next reply, then close. On the second connection,
/// read `bye`, send `z`, and read until the probe closes.
fn client(port: u16) -> Result<(), String> {
    let connect = || -> Result<TcpStream, String> {
        let stream = TcpStream::connect(("127.0.0.1", port)).map_err(|error| error.to_string())?;
        stream
            .set_read_timeout(Some(Duration::from_secs(10)))
            .map_err(|error| error.to_string())?;
        Ok(stream)
    };
    let mut first = connect()?;
    for (text, reply) in [
        (&b"ready"[..], &b"abc"[..]),
        (b"next", b"defg"),
        (b"once", b"hij"),
        (b"last", b"kl"),
    ] {
        exchange(&mut first, text, reply)?;
    }
    drop(first);
    let mut second = connect()?;
    exchange(&mut second, b"bye", b"z")?;
    let mut rest = Vec::new();
    second
        .read_to_end(&mut rest)
        .map_err(|error| error.to_string())?;
    if rest.is_empty() {
        Ok(())
    } else {
        Err(format!("data after the close: {rest:?}"))
    }
}

/// Read `text` from `stream`, then write `reply`.
fn exchange(stream: &mut TcpStream, text: &[u8], reply: &[u8]) -> Result<(), String> {
    let mut received = vec![0; text.len()];
    stream
        .read_exact(&mut received)
        .map_err(|error| format!("read of {text:?}: {error}"))?;
    if received != text {
        return Err(format!("read {received:?}, not {text:?}"));
    }
    stream.write_all(reply).map_err(|error| error.to_string())
}

#[test]
fn connects_to_a_local_listen_socket_as_pinned_erts() {
    // `gen_tcp:connect/3,4` to a listen port, the names of the connected
    // socket, a send and a receive on each side, a connect of a connected
    // socket (`eisconn`) and with a bad timeout (`einval`), a connect in
    // active mode, and a connect to a closed port (`econnrefused`)
    // (inet_drv.c:12064-12141, 14242-14288; prim_inet.erl:368-431).
    assert_connect_runs(
        "main",
        "[[{127,0,0,1},true,{127,0,0,1},true],[ok,{ok,<<104,105>>},ok,{ok,<<121,111>>}],\
         [{error,eisconn},{error,einval}],<<97,99,116>>,\
         [{error,econnrefused},{error,econnrefused}],[]]\n",
    );
}

#[test]
fn ends_a_connect_at_its_timeout_and_at_a_close_as_pinned_erts() {
    // A connect to TEST-NET-1 waits until its timer ends it
    // (inet_drv.c:12569-12580); a second connect gives `einval`, and a
    // close ends the connect with `closed` (inet_drv.c:11883-11885).
    assert_connect_runs(
        "timeout",
        "[{error,timeout},{error,einval},{error,closed},[{'EXIT',true,normal}]]\n",
    );
}

#[test]
fn connects_ipv6_sockets_and_binds_the_address_tags_as_pinned_erts() {
    // `inet6` listen, connect, and accept on `::1`, the names of each
    // socket, `ipv6_v6only` on an IPv6 and an IPv4 socket, and a bind to
    // `any` and `loopback` of both families (inet_drv.c:4706-4913,
    // 7806-7823, 9538-9552, 11969-11984). Neither OTP nor PBeam sets
    // `IPV6_V6ONLY` on a socket that does not ask for it; both read the
    // kernel value. Linux sets it on a bind to a specific IPv6 address
    // such as `::1` (net/ipv6/af_inet6.c, `__inet6_bind`), and the
    // accepted socket inherits it. macOS leaves it off.
    let only = cfg!(target_os = "linux");
    let expected = "[[true,{0,0,0,0,0,0,0,1},true,{0,0,0,0,0,0,0,1},true,{0,0,0,0,0,0,0,1}],\
         [ok,{ok,<<104,105>>}],[{ok,[{ipv6_v6only,ONLY}]},{ok,[{ipv6_v6only,ONLY}]}],\
         [{ok,[{ipv6_v6only,true}]},{ok,[]},{error,einval}],\
         [{0,0,0,0,0,0,0,0},{0,0,0,0,0,0,0,0},{0,0,0,0,0,0,0,1},{0,0,0,0},{127,0,0,1}],\
         [[0,101,105,110,118,97,108],[0,101,105,110,118,97,108]],\
         [{0,0,0,0},{127,0,0,1},{0,0,0,0,0,0,0,0},{0,0,0,0,0,0,0,1}],\
         [ok,{ok,[{tclass,8},{recvtclass,true}]}],[]]\n"
        .replace("ONLY", &only.to_string());
    assert_connect_runs("ipv6", &expected);
}

/// Run `gen_tcp_connect:Function()` on the pinned ERTS and on PBeam, as
/// [`runs_the_gen_tcp_calls_of_thousand_island_as_pinned_erts`] runs its
/// probe, and check that each run writes `expected`.
fn assert_connect_runs(function: &str, expected: &str) {
    let mut erts = Command::new("erl");
    erts.args([
        "+S",
        "1",
        "-noshell",
        "-boot",
        "start_clean",
        "-pa",
        "fixtures/gen_tcp",
        "-eval",
    ])
    .arg(format!("gen_tcp_connect:{function}(), halt()."));
    let mut pbeam = Command::new(env!("CARGO_BIN_EXE_pbeam"));
    pbeam
        .args(["pbeam_boot", "start", "gen_tcp_connect", function, "--root"])
        .arg(otp_root())
        .args(["--", GEN_TCP_CONNECT])
        .args(BOOT_MODULES.map(pinned_beam_path))
        .arg(boot_beam());
    for (name, command) in [("ERTS", &mut erts), ("PBeam", &mut pbeam)] {
        let output = command
            .env("TERM", "dumb")
            .env("LC_ALL", "C.UTF-8")
            .stdin(Stdio::null())
            .output()
            .unwrap();
        let stdout = String::from_utf8_lossy(&output.stdout);
        let stderr = String::from_utf8_lossy(&output.stderr);
        assert_eq!(
            (stdout.as_ref(), stderr.as_ref(), output.status.code()),
            (expected, "", Some(0)),
            "{name}"
        );
    }
}

#[test]
fn runs_the_local_family_as_pinned_erts() {
    // `gen_tcp_local:main/0` listens, connects, accepts, sends, and
    // receives at a `local` file name, connects to a name with no socket,
    // and on Linux listens on an abstract address. `inet_set_address` and
    // `inet_get_address` of `AF_UNIX` (inet_drv.c:4748-4778, 4915-4936)
    // give the address bytes: the accepted and the connected socket have
    // no name, and an abstract name keeps its first zero byte. Each run
    // removes the file name before and after it.
    let abstract_name = if cfg!(target_os = "linux") {
        let name = "{local,<<0,112,98,101,97,109,45,97,98,115,116,114,97,99,116,45,49,49,57,56>>}";
        format!("[{{ok,{name}}},{{ok,{name}}}]")
    } else {
        "linux_only".to_owned()
    };
    let expected = format!(
        "[[<<112,105,110,103>>,<<112,111,110,103>>,true,{{ok,{{local,<<>>}}}},true,\
         {{ok,{{local,<<>>}}}},{{error,eaddrinuse}}],{{error,enoent}},{abstract_name}]\n"
    );
    let path = std::env::temp_dir().join(format!("pbeam-local-{}", std::process::id()));
    let mut erts = Command::new("erl");
    erts.args([
        "+S",
        "1",
        "-noshell",
        "-boot",
        "start_clean",
        "-pa",
        "fixtures/gen_tcp",
        "-eval",
        "gen_tcp_local:main(), halt().",
    ]);
    let mut pbeam = Command::new(env!("CARGO_BIN_EXE_pbeam"));
    pbeam
        .args(["pbeam_boot", "start", "gen_tcp_local", "main", "--root"])
        .arg(otp_root())
        .args(["--", GEN_TCP_LOCAL])
        .args(BOOT_MODULES.map(pinned_beam_path))
        .arg(boot_beam());
    for (name, command) in [("ERTS", &mut erts), ("PBeam", &mut pbeam)] {
        let _ = std::fs::remove_file(&path);
        let output = command
            .env("PBEAM_LOCAL_PATH", &path)
            .stdin(Stdio::null())
            .output()
            .unwrap();
        let _ = std::fs::remove_file(&path);
        assert_eq!(
            (
                String::from_utf8_lossy(&output.stdout).into_owned(),
                String::from_utf8_lossy(&output.stderr).into_owned(),
                output.status.code()
            ),
            (expected.clone(), "".into(), Some(0)),
            "{name}"
        );
    }
}

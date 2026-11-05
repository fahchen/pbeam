//! Connects from PBeam to an `ssh:daemon/2` on the pinned ERTS with a
//! public key and opens a TCP/IP tunnel with
//! `ssh:tcpip_tunnel_to_server/6`, as the Redbug CLI does
//! (`server/lib/server/ssh_tunnel.ex:185`, `:195`).
#![expect(
    clippy::unwrap_used,
    reason = "test helpers report a failure by a panic"
)]

use std::{
    path::{Path, PathBuf},
    process::{Command, Stdio},
};

#[expect(
    dead_code,
    reason = "this test crate gives BEAM paths, not bytes, to the binary"
)]
#[path = "../common/mod.rs"]
mod common;
#[path = "../common/peer.rs"]
mod peer;

use common::pinned_beam_path;
use peer::{Epmd, ErtsNode};

/// The OTP applications that the client run puts on its code path with
/// `-pa`, in the order of `application:ensure_all_started(ssh)`:
/// `crypto`, `asn1`, `public_key`, and `ssh`. The kernel start puts the
/// kernel and stdlib directories on the path, and `code_server` finds the
/// other applications under `Root/lib` on demand.
fn app_directories() -> Vec<String> {
    let output = Command::new("erl")
        .args([
            "-noshell",
            "-eval",
            "[io:format(\"~ts~n\", [filename:join(code:lib_dir(App), \"ebin\")]) || App <- \
             [crypto, asn1, public_key, ssh]], halt().",
        ])
        .output()
        .unwrap();
    assert!(output.status.success(), "{output:?}");
    String::from_utf8(output.stdout)
        .unwrap()
        .lines()
        .map(str::to_owned)
        .collect()
}

/// Return the root and the bin directory of the pinned install, as the
/// distribution tests do.
fn emulator_flags() -> (String, PathBuf) {
    let output = Command::new("erl")
        .args([
            "-noshell",
            "-eval",
            "{ok, [[R]]} = init:get_argument(root), {ok, [[B]]} = init:get_argument(bindir), \
             io:put_chars([R, $\\n, B]), halt().",
        ])
        .output()
        .unwrap();
    assert!(output.status.success(), "{output:?}");
    let text = String::from_utf8(output.stdout).unwrap();
    let (root, bindir) = text.split_once('\n').unwrap();
    (root.to_owned(), PathBuf::from(bindir))
}

/// The directory of the preloaded modules of the pinned install.
fn pinned_erlang_dir() -> PathBuf {
    pinned_beam_path("erlang").parent().unwrap().to_owned()
}

/// Generate the host key, the client key, and the `authorized_keys` file of
/// the daemon into `dir`, with the pinned ERTS.
fn generate_keys(dir: &Path) {
    let _ = std::fs::remove_dir_all(dir);
    std::fs::create_dir_all(dir).unwrap();
    let expression = format!("Dir = \"{}\", ssh_keys:write(Dir), halt().", dir.display());
    let output = Command::new("erl")
        .args([
            "-noshell",
            "-pa",
            "fixtures/ssh_tunnel",
            "-eval",
            &expression,
        ])
        .stdin(Stdio::null())
        .output()
        .unwrap();
    assert!(output.status.success(), "{output:?}");
}

/// Start the SSH daemon and an echo server on the pinned ERTS with the key
/// directory `dir`, and return the node (which the test holds until the
/// probe ends), the SSH port, and the echo port. The node writes the ports
/// on its first two lines.
fn start_peer(epmd: &Epmd, dir: &Path) -> (ErtsNode, u16, u16) {
    let expression = format!(
        "{{ok, _}} = application:ensure_all_started(ssh), \
         Dir = \"{dir}\", \
         {{ok, Daemon}} = ssh:daemon(0, [{{system_dir, Dir}}, {{user_dir, Dir}}, \
                                      {{auth_methods, \"publickey\"}}, \
                                      {{tcpip_tunnel_in, true}}, \
                                      {{ip, {{127, 0, 0, 1}}}}]), \
         {{ok, Info}} = ssh:daemon_info(Daemon), \
         io:format(\"~w~n\", [proplists:get_value(port, Info)]), \
         {{ok, Echo}} = gen_tcp:listen(0, [binary, {{active, false}}, \
                                         {{ip, {{127, 0, 0, 1}}}}, {{reuseaddr, true}}]), \
         {{ok, EchoPort}} = inet:port(Echo), \
         io:format(\"~w~n\", [EchoPort]), \
         {{ok, Sock}} = gen_tcp:accept(Echo), \
         case gen_tcp:recv(Sock, 0) of \
             {{ok, Data}} -> ok = gen_tcp:send(Sock, Data), io:format(\"~p~n\", [Data]); \
             Other -> io:format(\"~p~n\", [Other]) \
         end, \
         halt().",
        dir = dir.display()
    );
    let node = ErtsNode::start(epmd, &["-noshell"], &expression);
    let ssh_port = node.line().parse().unwrap();
    let echo_port = node.line().parse().unwrap();
    (node, ssh_port, echo_port)
}

/// Run the probe on PBeam with the key directory `dir`, the SSH port
/// `ssh_port`, and the echo port `echo_port`.
fn run_probe(epmd: &Epmd, dir: &Path, ssh_port: u16, echo_port: u16) -> ErtsNode {
    let expression = format!(
        "os:putenv(\"PBEAM_SSH_DIR\", \"{dir}\"), \
         os:putenv(\"PBEAM_SSH_PORT\", \"{ssh_port}\"), \
         os:putenv(\"PBEAM_ECHO_PORT\", \"{echo_port}\"), \
         ssh_tunnel_probe:main(), halt().",
        dir = dir.display()
    );
    let mut arguments = vec![
        "-noshell".to_owned(),
        "-pa".to_owned(),
        "fixtures/ssh_tunnel".to_owned(),
    ];
    for app in app_directories() {
        arguments.push("-pa".to_owned());
        arguments.push(app);
    }
    let (root, bindir) = emulator_flags();
    let mut command = Command::new(env!("CARGO_BIN_EXE_pbeam"));
    command
        .args(["erl", "--root", &root, "--bindir"])
        .arg(&bindir)
        .arg("--preloaded")
        .arg(pinned_erlang_dir())
        .arg("--")
        .args(["-start_epmd", "false"])
        .args(&arguments)
        .env("ERL_EPMD_PORT", epmd.port())
        .env("TERM", "dumb")
        .env("LC_ALL", "C.UTF-8");
    let arguments: Vec<&str> = arguments.iter().map(String::as_str).collect();
    ErtsNode::start_command(command, epmd, &arguments, &expression)
}

#[test]
fn connects_to_an_ssh_daemon_of_the_pinned_erts_and_opens_a_tunnel() {
    // The client runs on PBeam and the daemon on the pinned ERTS. The
    // pinned crypto, asn1, public_key, and ssh applications start on the
    // code path of the pinned install, and the keys are generated in the
    // test. ssh:tcpip_tunnel_to_server/6 asks the daemon to listen on a
    // free port, and the client sends `hello` through the tunnel; the
    // daemon's echo server replies `hello`, which the probe and the peer
    // each write.
    let epmd = Epmd::start();
    let dir = std::env::temp_dir().join(format!("pbeam-ssh-{}", std::process::id()));
    generate_keys(&dir);
    let (peer, ssh_port, echo_port) = start_peer(&epmd, &dir);
    let node = run_probe(&epmd, &dir, ssh_port, echo_port);
    assert_eq!(node.line(), "{ok,<<\"hello\">>}");
    assert_eq!(peer.line(), "<<\"hello\">>");
}

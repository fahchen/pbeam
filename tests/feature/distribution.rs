#![expect(
    clippy::unwrap_used,
    reason = "test helpers report a failure by a panic"
)]

//! Distribution connections of a PBeam node with the pinned ERTS.

use std::{
    ffi::OsStr,
    path::PathBuf,
    process::{Command, Output, Stdio},
};

#[expect(
    dead_code,
    reason = "this test crate uses only the path of the preloaded modules"
)]
#[path = "../common/mod.rs"]
mod common;
#[path = "../common/peer.rs"]
mod peer;

use common::pinned_beam_path;
use peer::{Epmd, ErtsNode};

/// The arguments of a node with the `prim_socket` distribution carrier.
const SOCKET_CARRIER: [&str; 4] = ["-proto_dist", "inet_epmd", "-inet_epmd", "socket"];

/// Return the root and the bin directory of the pinned install.
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

/// Run `pbeam erl` with the `epmd` of `epmd` and the `erl` arguments
/// `arguments`. The test starts `epmd`, so the run does not.
fn pbeam_erl<S: AsRef<OsStr>>(epmd: &Epmd, arguments: &[S]) -> Output {
    let (root, bindir) = emulator_flags();
    Command::new(env!("CARGO_BIN_EXE_pbeam"))
        .args(["erl", "--root", &root, "--bindir"])
        .arg(&bindir)
        .arg("--preloaded")
        .arg(pinned_beam_path("erlang").parent().unwrap())
        .arg("--")
        .args(["-start_epmd", "false"])
        .args(arguments)
        .env("ERL_EPMD_PORT", epmd.port())
        .stdin(Stdio::null())
        .output()
        .unwrap()
}

/// Run `erl` of the pinned ERTS with the `epmd` of `epmd` and the
/// arguments `arguments`.
fn erts_erl<S: AsRef<OsStr>>(epmd: &Epmd, arguments: &[S]) -> Output {
    Command::new("erl")
        .args(["-start_epmd", "false"])
        .args(arguments)
        .env("ERL_EPMD_PORT", epmd.port())
        .stdin(Stdio::null())
        .output()
        .unwrap()
}

/// Return the standard output of a run that exits with status 0 and writes
/// nothing to the standard error.
fn stdout(output: &Output) -> String {
    assert!(
        output.status.success() && output.stderr.is_empty(),
        "{output:?}\nstderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8(output.stdout.clone()).unwrap()
}

/// Return the text of the node name `name` as `~p` writes the atom
/// (`io_lib:write_atom/1`): plain when it starts with a lowercase letter
/// and has only letters, digits, `_`, and `@`, else quoted. A host name
/// with a `-` gives a quoted name, and a host name without one does not.
fn atom_text(name: &str) -> String {
    let plain = name.starts_with(|c: char| c.is_ascii_lowercase())
        && name
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '@');
    if plain {
        name.to_owned()
    } else {
        format!("'{name}'")
    }
}

/// Return the arguments that run `dist_channel_probe:Function()` and halt,
/// after the arguments `before`.
fn probe<'a>(before: &[&'a str], expression: &'a str) -> Vec<&'a str> {
    let mut arguments = before.to_vec();
    arguments.extend(["-noshell", "-pa", "fixtures/dist", "-eval", expression]);
    arguments
}

#[test]
fn runs_the_bifs_of_a_process_controller_as_pinned_erts() {
    // new_connection/1, abort_pending_connection/2, create_dist_channel/3,
    // the dist_ctrl_* BIFs, dist_get_stat/1, and nodes/1,2 of a connection
    // with a process controller and no other node (dist.c:4272-4720,
    // 5127-5711, 6250-6470). The exit of net_kernel kills the controller
    // and makes the pending connection idle (dist.c:926-1001).
    let epmd = Epmd::start();
    let name = format!("dchan{}", std::process::id());
    let main = probe(&["-sname", &name], "dist_channel_probe:main(), halt().");
    let erts = stdout(&erts_erl(&epmd, &main));
    assert!(erts.contains("controller: [false,false,true,"), "{erts}");
    assert_eq!(stdout(&pbeam_erl(&epmd, &main)), erts);
    // A tcp_inet port controls a connection, and the exit of the port ends
    // it (dist.c:5269-5336, io.c:3910-3916).
    let port = probe(
        &["-sname", &name],
        "dist_channel_probe:port_channel(), halt().",
    );
    let erts = stdout(&erts_erl(&epmd, &port));
    assert!(
        erts.contains("port_errors: [badarg,badarg,badarg,badarg,"),
        "{erts}"
    );
    assert_eq!(stdout(&pbeam_erl(&epmd, &port)), erts);
    // Packets with data on a process and a port controller go to a pid
    // and a name, or are dropped (dist.c:2463-2553).
    let input = probe(&["-sname", &name], "dist_channel_probe:input(), halt().");
    let erts = stdout(&erts_erl(&epmd, &input));
    assert!(erts.contains("{process,send_sender},"), "{erts}");
    assert_eq!(stdout(&pbeam_erl(&epmd, &input)), erts);
    let exit = probe(&[], "dist_channel_probe:net_kernel_exit(), halt().");
    let erts = stdout(&erts_erl(&epmd, &exit));
    assert!(
        erts.contains("stopped: [killed,net_kernel_terminated,nonode@nohost,"),
        "{erts}"
    );
    assert_eq!(stdout(&pbeam_erl(&epmd, &exit)), erts);
}

#[test]
fn offers_the_mandatory_distribution_flags() {
    // PBeam offers only DFLAG_DIST_MANDATORY (dist.h:80-98), with the
    // rejectable and the strict order flags of ERTS (dist.h:129-135).
    let epmd = Epmd::start();
    let output = pbeam_erl(
        &epmd,
        &[
            "-noshell",
            "-eval",
            "io:format(\"~p~n\", [erts_internal:get_dflags()]), halt().",
        ],
    );
    assert_eq!(
        stdout(&output),
        "{erts_dflags,17230663572,17230663572,17230663572,8396866,8192}\n"
    );
}

#[test]
fn stops_at_a_part_of_a_channel_that_pbeam_does_not_have() {
    // Flags without a mandatory flag, where ERTS sends an error report
    // (dist.c:5176-5187), a packet that ERTS rejects on a process or a port
    // controller, where ERTS ends the connection (dist.c:3144-3165,
    // external.c:922-977), a control message with a trace token, another
    // operation, and `{deliver, port}` data of a port that is not a
    // controller, where ERTS sends `{Port, {data, Data}}`
    // (inet_drv.c:2753-2784), stop the run with a runtime error.
    let epmd = Epmd::start();
    let name = format!("dstop{}", std::process::id());
    let rejected = "unsupported distribution message that ERTS rejects";
    let deliver = "unsupported data of a port with the option {deliver, port}";
    let mut cases = vec![
        (
            "no_mandatory()".to_owned(),
            "unsupported erts_internal:create_dist_channel/3 with flags that miss a mandatory flag",
        ),
        ("message()".to_owned(), rejected),
        ("port_message()".to_owned(), rejected),
        (
            "trace_token()".to_owned(),
            "unsupported distribution message with a trace token",
        ),
        (
            "link_op()".to_owned(),
            "unsupported distribution operation 1",
        ),
        ("deliver_port()".to_owned(), deliver),
        ("deliver_udp()".to_owned(), deliver),
    ];
    cases.extend([1, 2, 3, 4, 5, 6, 7, 8, 10].map(|n| (format!("reject({n})"), rejected)));
    cases.push((
        "atom_cache()".to_owned(),
        "unsupported distribution message on a connection with the atom cache or fragments",
    ));
    cases.push((
        "reject(9)".to_owned(),
        "unsupported distribution message of a pid reference",
    ));
    for (call, error) in cases {
        let expression = format!("dist_channel_probe:{call}, halt().");
        let output = pbeam_erl(&epmd, &probe(&["-sname", &name], &expression));
        let stderr = String::from_utf8_lossy(&output.stderr);
        assert!(
            !output.status.success() && stderr.contains(error),
            "{call}: {output:?}\nstderr: {stderr}"
        );
    }
}

#[test]
fn connects_to_an_erts_node_as_a_hidden_node_as_pinned_erts() {
    // net_kernel, dist_util, inet_epmd_dist, and inet_epmd_socket of the
    // pinned OTP set up the connection with the prim_socket carrier and
    // the cookie handshake. The ERTS node writes each change of its hidden
    // nodes. The connection stays up for more than two net_ticktime of 2 s,
    // so each side gets the ticks of the other (dist_util.erl:628-670,
    // 1174-1200).
    // A wrong cookie gives false, and the ERTS node does not connect.
    let epmd = Epmd::start();
    let id = std::process::id();
    let server = format!("dsrv{id}");
    let node = ErtsNode::start(
        &epmd,
        &[
            "-sname",
            &server,
            "-setcookie",
            "ck",
            "-kernel",
            "net_ticktime",
            "2",
            "-kernel",
            "logger_level",
            "none",
        ],
        "io:format(\"~s~n\", [node()]), \
         L = fun L(Old) -> case nodes(hidden) of Old -> timer:sleep(20), L(Old); \
         New -> io:format(\"~p~n\", [New]), L(New) end end, L([]).",
    );
    let server = node.line();
    let host = server.split_once('@').unwrap().1.to_owned();
    let client = |name: &str, cookie: &str| {
        let expression = format!(
            "io:format(\"~p~n~p~n\", [net_kernel:connect_node('{server}'), nodes(hidden)]), \
             timer:sleep(5000), io:format(\"~p~n\", [nodes(hidden)]), halt()."
        );
        let mut arguments = vec!["-sname", name, "-setcookie", cookie, "-hidden"];
        arguments.extend(SOCKET_CARRIER);
        arguments.extend(["-kernel", "net_ticktime", "2", "-noshell", "-eval"]);
        let mut arguments: Vec<String> = arguments.into_iter().map(str::to_owned).collect();
        arguments.push(expression);
        arguments
    };
    let (pbeam, erts) = (format!("dpb{id}"), format!("derts{id}"));
    let pbeam_output = stdout(&pbeam_erl(&epmd, &client(&pbeam, "ck")));
    assert_eq!(
        node.line(),
        format!("[{}]", atom_text(&format!("{pbeam}@{host}")))
    );
    assert_eq!(node.line(), "[]");
    let erts_output = stdout(&erts_erl(&epmd, &client(&erts, "ck")));
    assert_eq!(
        node.line(),
        format!("[{}]", atom_text(&format!("{erts}@{host}")))
    );
    assert_eq!(node.line(), "[]");
    let connected = format!("true\n[{0}]\n[{0}]\n", atom_text(&server));
    assert_eq!(erts_output, connected);
    assert_eq!(pbeam_output, connected);
    let refused = "false\n[]\n[]\n";
    assert_eq!(stdout(&erts_erl(&epmd, &client(&erts, "bad"))), refused);
    assert_eq!(stdout(&pbeam_erl(&epmd, &client(&pbeam, "bad"))), refused);
}

#[test]
fn connects_over_a_tcp_inet_port_as_pinned_erts() {
    // inet_tcp_dist, the default carrier, makes the tcp_inet port the
    // controller of the connection (dist.c:5269-5336) and sets
    // `{deliver, port}` and `{packet, 4}` on it (inet_tcp_dist.erl
    // f_setopts_post_nodeup). The connection stays up for more than two
    // net_ticktime of 2 s, so each side gets the ticks of the other
    // (dist_util.erl:628-670). The custom `-epmd_module` dist_epmd gives
    // the fixed listen port of the ERTS node from address_please/3 as
    // `{ok, Ip, Port, Version}` (net_kernel.erl, inet_tcp_dist.erl).
    let epmd = Epmd::start();
    let id = std::process::id();
    let listen = std::net::TcpListener::bind("127.0.0.1:0")
        .unwrap()
        .local_addr()
        .unwrap()
        .port()
        .to_string();
    let server = format!("dtsrv{id}");
    let node = ErtsNode::start(
        &epmd,
        &[
            "-sname",
            &server,
            "-setcookie",
            "ck",
            "-kernel",
            "net_ticktime",
            "2",
            "-kernel",
            "logger_level",
            "none",
            "-kernel",
            "inet_dist_listen_min",
            &listen,
            "-kernel",
            "inet_dist_listen_max",
            &listen,
        ],
        "io:format(\"~s~n\", [node()]), \
         L = fun L(Old) -> case nodes(hidden) of Old -> timer:sleep(20), L(Old); \
         New -> io:format(\"~p~n\", [New]), L(New) end end, L([]).",
    );
    let server = node.line();
    let host = server.split_once('@').unwrap().1.to_owned();
    let client = |name: &str, custom: bool| {
        let expression = format!(
            "io:format(\"~p~n~p~n\", [net_kernel:connect_node('{server}'), nodes(hidden)]), \
             timer:sleep(5000), io:format(\"~p~n\", [nodes(hidden)]), halt()."
        );
        let mut arguments = vec!["-sname", name, "-setcookie", "ck", "-hidden"];
        if custom {
            arguments.extend(["-pa", "fixtures/dist", "-epmd_module", "dist_epmd"]);
            arguments.extend(["-dist_epmd_port", &listen]);
        }
        arguments.extend(["-kernel", "net_ticktime", "2", "-noshell", "-eval"]);
        let mut arguments: Vec<String> = arguments.into_iter().map(str::to_owned).collect();
        arguments.push(expression);
        arguments
    };
    let connected = format!("true\n[{0}]\n[{0}]\n", atom_text(&server));
    for custom in [false, true] {
        let (pbeam, erts) = (format!("dtpb{custom}{id}"), format!("dterts{custom}{id}"));
        assert_eq!(stdout(&erts_erl(&epmd, &client(&erts, custom))), connected);
        assert_eq!(
            node.line(),
            format!("[{}]", atom_text(&format!("{erts}@{host}")))
        );
        assert_eq!(node.line(), "[]");
        assert_eq!(
            stdout(&pbeam_erl(&epmd, &client(&pbeam, custom))),
            connected
        );
        assert_eq!(
            node.line(),
            format!("[{}]", atom_text(&format!("{pbeam}@{host}")))
        );
        assert_eq!(node.line(), "[]");
    }
}

#[test]
fn receives_a_message_from_an_erts_node_as_pinned_erts() {
    // The ERTS node sends REG_SEND to the name dreg of each new hidden node
    // (dist.c:1668-1675), over the prim_socket carrier, where the
    // controller gives the packet to erlang:dist_ctrl_put_data/2, and over
    // the tcp_inet port controller (dist.c:2468-2507).
    let epmd = Epmd::start();
    let id = std::process::id();
    let server = format!("drsrv{id}");
    let node = ErtsNode::start(
        &epmd,
        &[
            "-sname",
            &server,
            "-setcookie",
            "ck",
            "-kernel",
            "logger_level",
            "none",
        ],
        "io:format(\"~s~n\", [node()]), \
         L = fun L(Old) -> New = nodes(hidden), \
         [{dreg, N} ! {hello, node()} || N <- New -- Old], timer:sleep(20), L(New) end, L([]).",
    );
    let server = node.line();
    let client = |name: &str, socket: bool| {
        let expression = format!(
            "register(dreg, self()), true = net_kernel:connect_node('{server}'), \
             receive M -> io:format(\"~p~n\", [M]) after 5000 -> io:format(\"timeout~n\") end, \
             halt()."
        );
        let mut arguments = vec!["-sname", name, "-setcookie", "ck", "-hidden"];
        if socket {
            arguments.extend(SOCKET_CARRIER);
        }
        arguments.extend(["-noshell", "-eval"]);
        let mut arguments: Vec<String> = arguments.into_iter().map(str::to_owned).collect();
        arguments.push(expression);
        arguments
    };
    let hello = format!("{{hello,{}}}\n", atom_text(&server));
    for socket in [true, false] {
        let erts = format!("drerts{socket}{id}");
        assert_eq!(stdout(&erts_erl(&epmd, &client(&erts, socket))), hello);
        let pbeam = format!("drpb{socket}{id}");
        assert_eq!(stdout(&pbeam_erl(&epmd, &client(&pbeam, socket))), hello);
    }
}

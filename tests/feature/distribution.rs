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

/// Start a node of `pbeam erl` with the `epmd` of `epmd`, the arguments
/// `arguments`, and the expression `expression`.
fn pbeam_node(epmd: &Epmd, arguments: &[&str], expression: &str) -> ErtsNode {
    let (root, bindir) = emulator_flags();
    let mut command = Command::new(env!("CARGO_BIN_EXE_pbeam"));
    command
        .args(["erl", "--root", &root, "--bindir"])
        .arg(&bindir)
        .arg("--preloaded")
        .arg(pinned_beam_path("erlang").parent().unwrap())
        .arg("--")
        .args(["-start_epmd", "false"]);
    ErtsNode::start_command(command, epmd, arguments, expression)
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
    // Packets with data on a process and a port controller go to a pid,
    // a name, and an active alias, or are dropped (dist.c:2463-2578,
    // erl_proc_sig_queue.c:2314-2331).
    let input = probe(&["-sname", &name], "dist_channel_probe:input(), halt().");
    let erts = stdout(&erts_erl(&epmd, &input));
    assert!(erts.contains("[{process,alias},"), "{erts}");
    assert_eq!(stdout(&pbeam_erl(&epmd, &input)), erts);
    // EXIT2 and PAYLOAD_EXIT2 packets give an exit signal to a local
    // process, or are dropped (dist.c:2806-2870).
    let exit_input = probe(
        &["-sname", &name],
        "dist_channel_probe:exit_input(), halt().",
    );
    let erts = stdout(&erts_erl(&epmd, &exit_input));
    assert!(erts.contains("[ok,{down,killed}]"), "{erts}");
    assert_eq!(stdout(&pbeam_erl(&epmd, &exit_input)), erts);
    // LINK, UNLINK_ID, UNLINK_ID_ACK, EXIT, and PAYLOAD_EXIT packets link
    // local processes to a pid of the node, and local processes send them
    // (bif.c:263-394, 1295-1348, dist.c:2224-2339, 2745-2805).
    let link_input = probe(
        &["-sname", &name],
        "dist_channel_probe:link_input(), halt().",
    );
    let erts = stdout(&erts_erl(&epmd, &link_input));
    assert!(
        erts.contains("[[[from],[{'EXIT',from,boom},alive],[]],"),
        "{erts}"
    );
    assert_eq!(stdout(&pbeam_erl(&epmd, &link_input)), erts);
    // A node that is not alive has no connection (bif.c:323-340).
    let noconnection = probe(&[], "dist_channel_probe:link_noconnection(), halt().");
    let erts = stdout(&erts_erl(&epmd, &noconnection));
    assert_eq!(
        erts,
        "noconnection: [true,noconnection,true,{links,[<0.0.0>]}]\n"
    );
    assert_eq!(stdout(&pbeam_erl(&epmd, &noconnection)), erts);
    let exit = probe(&[], "dist_channel_probe:net_kernel_exit(), halt().");
    let erts = stdout(&erts_erl(&epmd, &exit));
    assert!(
        erts.contains("stopped: [killed,net_kernel_terminated,nonode@nohost,"),
        "{erts}"
    );
    assert_eq!(stdout(&pbeam_erl(&epmd, &exit)), erts);
}

#[test]
fn queues_and_writes_the_messages_to_a_connection_as_pinned_erts() {
    // A send to a pid, a name, and an alias of a connected node puts the
    // packet of erts_dsig_send_msg in the output queue of a process
    // controller (dist.c:1640-1702, 3646-3683, 4272-4746), and a tcp_inet
    // port controller writes it with no inet_reply (dist.c:3844-3921,
    // inet_drv.c:12660-12698).
    let epmd = Epmd::start();
    let name = format!("dout{}", std::process::id());
    for function in ["output", "port_output"] {
        let expression = format!("dist_channel_probe:{function}(), halt().");
        let arguments = probe(&["-sname", &name], &expression);
        let erts = stdout(&erts_erl(&epmd, &arguments));
        assert!(erts.contains("[{ok,0,"), "{erts}");
        assert_eq!(stdout(&pbeam_erl(&epmd, &arguments)), erts, "{function}");
    }
}

#[test]
fn offers_the_mandatory_distribution_flags_exit_payload_and_alias() {
    // PBeam offers DFLAG_DIST_MANDATORY (dist.h:80-98), DFLAG_EXIT_PAYLOAD,
    // and DFLAG_ALIAS (dist.h:52, 67, 113-123), with the rejectable and the
    // strict order flags of ERTS (dist.h:129-135).
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
        "{erts_dflags,51594596244,17230663572,51594596244,8396866,8192}\n"
    );
}

#[test]
fn encodes_an_alias_as_a_pid_reference_as_pinned_erts() {
    // An alias has five reference numbers: the pid marker bit in the
    // second, and the pid word of its owner in the last two
    // (external.c:3642-3668, 4821-4856, erl_bif_unique.h:259-277).
    let epmd = Epmd::start();
    let name = format!("dalext{}", std::process::id());
    let arguments = probe(
        &["-sname", &name],
        "dist_channel_probe:alias_ext(), halt().",
    );
    let erts = stdout(&erts_erl(&epmd, &arguments));
    assert!(
        erts.starts_with("alias: [5,65536,true,0,true,true,"),
        "{erts}"
    );
    assert_eq!(stdout(&pbeam_erl(&epmd, &arguments)), erts);
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
            "monitor_op()".to_owned(),
            "unsupported distribution operation 19",
        ),
        ("deliver_port()".to_owned(), deliver),
        ("deliver_udp()".to_owned(), deliver),
    ];
    cases.extend([12, 13, 25].map(|op| {
        (
            format!("trace_token({op})"),
            "unsupported distribution message with a trace token",
        )
    }));
    cases.extend(
        [
            1, 2, 3, 4, 5, 6, 7, 8, 10, 11, 12, 13, 14, 15, 16, 17, 18, 19, 20, 21, 22, 23, 24, 25,
            26,
        ]
        .map(|n| (format!("reject({n})"), rejected)),
    );
    cases.push((
        "atom_cache()".to_owned(),
        "unsupported distribution message on a connection with the atom cache or fragments",
    ));
    cases.push((
        "reject(9)".to_owned(),
        "unsupported distribution message of a pid reference that is not of five numbers",
    ));
    for use_ in ["binary", "size", "send"] {
        cases.push((
            format!("dead_alias({use_})"),
            "unsupported external term of a pid reference whose owner exited",
        ));
    }
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
fn sends_messages_between_erts_and_pbeam_nodes_in_order() {
    // A client sends 100 REG_SEND messages to the name decho of a server
    // node, and then 100 SEND messages to the pid of the server, which
    // replies to each with SEND (dist.c:1668-1700). The echoes come in the
    // order of the sends. Each pair of ERTS and PBeam nodes runs over the
    // prim_socket carrier and over the tcp_inet port controller.
    let epmd = Epmd::start();
    let id = std::process::id();
    let server = "io:format(\"~s~n\", [node()]), register(decho, self()), \
                  L = fun L() -> receive {From, Tag, N} -> From ! {echo, self(), Tag, N}, L() end end, L().";
    for (server_pbeam, socket) in [(false, true), (false, false), (true, true), (true, false)] {
        let name = format!("dssrv{server_pbeam}{socket}{id}");
        let mut arguments = vec!["-sname", &name, "-setcookie", "ck"];
        if socket {
            arguments.extend(SOCKET_CARRIER);
        }
        let node = if server_pbeam {
            pbeam_node(&epmd, &arguments, server)
        } else {
            ErtsNode::start(&epmd, &arguments, server)
        };
        let server = node.line();
        let expression = format!(
            "true = net_kernel:connect_node('{server}'), Seq = lists:seq(1, 100), \
             Echo = fun() -> receive {{echo, P, T, N}} -> {{P, T, N}} end end, \
             [{{decho, '{server}'}} ! {{self(), name, N}} || N <- Seq], \
             Names = [Echo() || _ <- Seq], [{{Pid, _, _}} | _] = Names, \
             [Pid ! {{self(), pid, N}} || N <- Seq], Pids = [Echo() || _ <- Seq], \
             io:format(\"~p~n\", [{{node(Pid), [{{T, N}} || {{_, T, N}} <- Names ++ Pids] =:= \
             [{{name, N}} || N <- Seq] ++ [{{pid, N}} || N <- Seq]}}]), halt()."
        );
        let client = |name: &str| {
            let mut arguments = vec!["-sname", name, "-setcookie", "ck", "-hidden"];
            if socket {
                arguments.extend(SOCKET_CARRIER);
            }
            arguments.extend(["-noshell", "-eval", &expression]);
            arguments.into_iter().map(str::to_owned).collect::<Vec<_>>()
        };
        let echoed = format!("{{{},true}}\n", atom_text(&server));
        let erts = format!("dserts{server_pbeam}{socket}{id}");
        assert_eq!(stdout(&erts_erl(&epmd, &client(&erts))), echoed);
        let pbeam = format!("dspb{server_pbeam}{socket}{id}");
        assert_eq!(stdout(&pbeam_erl(&epmd, &client(&pbeam))), echoed);
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

#[test]
fn sends_messages_to_an_alias_between_erts_and_pbeam_nodes() {
    // A client sends an alias to a server node, which replies to it with
    // ALIAS_SEND (dist.c:1640-1666, 2556-2578). After unalias/1 the owner
    // drops the next reply (erl_proc_sig_queue.c:2314-2331), and the SEND
    // after it comes. Each pair of ERTS and PBeam nodes runs over the
    // prim_socket carrier and over the tcp_inet port controller. An ERTS
    // client also gets the reply of gen_server:call/2 to net_kernel of the
    // server through an alias (gen.erl, net_kernel.erl).
    let epmd = Epmd::start();
    let id = std::process::id();
    let server = "io:format(\"~s~n\", [node()]), register(dalias, self()), \
                  L = fun L() -> receive {A, From} -> A ! {reply, node()}, From ! sync, L(); \
                  {late, A, From} -> A ! late, From ! done, L() end end, L().";
    for (server_pbeam, socket) in [(false, true), (false, false), (true, true), (true, false)] {
        let name = format!("dasrv{server_pbeam}{socket}{id}");
        let mut arguments = vec!["-sname", &name, "-setcookie", "ck"];
        if socket {
            arguments.extend(SOCKET_CARRIER);
        }
        let node = if server_pbeam {
            pbeam_node(&epmd, &arguments, server)
        } else {
            ErtsNode::start(&epmd, &arguments, server)
        };
        let server = node.line();
        let alias = format!(
            "true = net_kernel:connect_node('{server}'), A = alias(), \
             {{dalias, '{server}'}} ! {{A, self()}}, \
             R = receive {{reply, N}} -> N after 5000 -> timeout end, \
             receive sync -> ok end, unalias(A), {{dalias, '{server}'}} ! {{late, A, self()}}, \
             receive done -> ok end, Late = receive late -> late after 0 -> none end"
        );
        let client = |name: &str, expression: &str| {
            let mut arguments = vec!["-sname", name, "-setcookie", "ck", "-hidden"];
            if socket {
                arguments.extend(SOCKET_CARRIER);
            }
            arguments.extend(["-noshell", "-eval", expression]);
            arguments.into_iter().map(str::to_owned).collect::<Vec<_>>()
        };
        let erts = format!(
            "{alias}, Yes = gen_server:call({{net_kernel, '{server}'}}, {{is_auth, node()}}), \
             io:format(\"~p~n\", [{{R, Late, Yes}}]), halt()."
        );
        let replied = format!("{{{},none,yes}}\n", atom_text(&server));
        let erts_name = format!("daerts{server_pbeam}{socket}{id}");
        assert_eq!(
            stdout(&erts_erl(&epmd, &client(&erts_name, &erts))),
            replied
        );
        let pbeam = format!("{alias}, io:format(\"~p~n\", [{{R, Late}}]), halt().");
        let replied = format!("{{{},none}}\n", atom_text(&server));
        let pbeam_name = format!("dapb{server_pbeam}{socket}{id}");
        assert_eq!(
            stdout(&pbeam_erl(&epmd, &client(&pbeam_name, &pbeam))),
            replied
        );
    }
}

#[test]
fn links_processes_between_erts_and_pbeam_nodes() {
    // A client process links to a server process and the server process
    // links to a client process, with and without trap_exit, and the
    // linked process exits with normal, kill, or a reason that is not
    // immediate (bif.c:263-394, dist.c:2224-2275, 2745-2805,
    // erl_process.c:14101-14147, 13981-14040); an unlink sends UNLINK_ID
    // and UNLINK_ID_ACK (bif.c:1295-1348, dist.c:2284-2339). Each pair of
    // ERTS and PBeam nodes runs over the prim_socket carrier and over the
    // tcp_inet port controller.
    let epmd = Epmd::start();
    let id = std::process::id();
    let expected = "[{to,false,normal,alive},\n {to,false,kill,{down,kill}},\n \
                    {to,false,{boom},{down,{boom}}},\n {to,true,normal,{'EXIT',normal}},\n \
                    {to,true,kill,{'EXIT',kill}},\n {to,true,{boom},{'EXIT',{boom}}},\n \
                    {from,false,normal,alive},\n {from,false,kill,{down,kill}},\n \
                    {from,false,{boom},{down,{boom}}},\n {from,true,normal,{'EXIT',normal}},\n \
                    {from,true,kill,{'EXIT',kill}},\n {from,true,{boom},{'EXIT',{boom}}},\n \
                    {unlink_to,boom,[]},\n {unlink_from,[],boom,alive}]\n";
    for (server_pbeam, socket) in [(false, true), (false, false), (true, true), (true, false)] {
        let name = format!("dlsrv{server_pbeam}{socket}{id}");
        let mut arguments = vec!["-sname", &name, "-setcookie", "ck", "-pa", "fixtures/dist"];
        if socket {
            arguments.extend(SOCKET_CARRIER);
        }
        let server = "dist_link_probe:server().";
        let node = if server_pbeam {
            pbeam_node(&epmd, &arguments, server)
        } else {
            ErtsNode::start(&epmd, &arguments, server)
        };
        let expression = format!("dist_link_probe:client('{}'), halt().", node.line());
        let client = |name: &str| {
            let mut arguments = vec!["-sname", name, "-setcookie", "ck", "-hidden"];
            if socket {
                arguments.extend(SOCKET_CARRIER);
            }
            arguments.extend(["-noshell", "-pa", "fixtures/dist", "-eval", &expression]);
            arguments.into_iter().map(str::to_owned).collect::<Vec<_>>()
        };
        let erts = format!("dlerts{server_pbeam}{socket}{id}");
        assert_eq!(stdout(&erts_erl(&epmd, &client(&erts))), expected);
        let pbeam = format!("dlpb{server_pbeam}{socket}{id}");
        assert_eq!(stdout(&pbeam_erl(&epmd, &client(&pbeam))), expected);
    }
}

#[test]
fn sends_exit_signals_between_erts_and_pbeam_nodes() {
    // A client sends exit/2 with normal, kill, and another reason to a
    // process of a server node that traps exits or not, as EXIT2 or
    // PAYLOAD_EXIT2 (dist.c:1788-1828, 2806-2870). The process forwards
    // its messages, and a local monitor of the server reports its exit.
    // Each pair of ERTS and PBeam nodes runs over the prim_socket carrier
    // and over the tcp_inet port controller.
    let epmd = Epmd::start();
    let id = std::process::id();
    let server = "io:format(\"~s~n\", [node()]), register(dexit, self()), \
                  L = fun L() -> receive {Trap, From} -> \
                  V = spawn(fun() -> process_flag(trap_exit, Trap), receive go -> ok end, \
                  From ! {victim, self()}, F = fun F() -> receive M -> From ! {got, M}, F() end end, F() end), \
                  spawn(fun() -> R = monitor(process, V), V ! go, \
                  receive {'DOWN', R, _, _, W} -> From ! {down, W} end end), L() end end, L().";
    let cases = "[{T, R} || T <- [false, true], R <- [normal, kill, boom]]";
    for (server_pbeam, socket) in [(false, true), (false, false), (true, true), (true, false)] {
        let name = format!("desrv{server_pbeam}{socket}{id}");
        let mut arguments = vec!["-sname", &name, "-setcookie", "ck"];
        if socket {
            arguments.extend(SOCKET_CARRIER);
        }
        let node = if server_pbeam {
            pbeam_node(&epmd, &arguments, server)
        } else {
            ErtsNode::start(&epmd, &arguments, server)
        };
        let server = node.line();
        let expression = format!(
            "true = net_kernel:connect_node('{server}'), \
             Collect = fun C() -> receive {{got, ping}} -> [alive]; {{down, W}} -> [{{down, W}}]; \
             {{got, {{'EXIT', P, W}}}} -> [{{'EXIT', P =:= self(), W}} | C()] after 5000 -> [timeout] end end, \
             Run = fun({{T, R}}) -> {{dexit, '{server}'}} ! {{T, self()}}, \
             V = receive {{victim, Pid}} -> Pid end, true = exit(V, R), V ! ping, {{T, R, Collect()}} end, \
             io:format(\"~p~n\", [[Run(C) || C <- {cases}]]), halt()."
        );
        let client = |name: &str| {
            let mut arguments = vec!["-sname", name, "-setcookie", "ck", "-hidden"];
            if socket {
                arguments.extend(SOCKET_CARRIER);
            }
            arguments.extend(["-noshell", "-eval", &expression]);
            arguments.into_iter().map(str::to_owned).collect::<Vec<_>>()
        };
        let expected = "[{false,normal,[alive]},\n {false,kill,[{down,killed}]},\n {false,boom,[{down,boom}]},\n \
                        {true,normal,[{'EXIT',true,normal},alive]},\n {true,kill,[{down,killed}]},\n \
                        {true,boom,[{'EXIT',true,boom},alive]}]\n";
        let erts = format!("deerts{server_pbeam}{socket}{id}");
        assert_eq!(stdout(&erts_erl(&epmd, &client(&erts))), expected);
        let pbeam = format!("depb{server_pbeam}{socket}{id}");
        assert_eq!(stdout(&pbeam_erl(&epmd, &client(&pbeam))), expected);
    }
}

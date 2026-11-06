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
    // A DEMONITOR_P for a name finds no monitor of the connection and
    // removes nothing (dist.c:2441-2457).
    let demonitor = probe(
        &["-sname", &name],
        "dist_channel_probe:demonitor_name(), halt().",
    );
    let erts = stdout(&erts_erl(&epmd, &demonitor));
    assert_eq!(erts, "demonitor_name: [ok,[zz@nohost]]\n");
    assert_eq!(stdout(&pbeam_erl(&epmd, &demonitor)), erts);
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
fn offers_the_mandatory_distribution_flags_the_monitor_flags_exit_payload_and_alias() {
    // PBeam offers DFLAG_DIST_MANDATORY (dist.h:80-98), DFLAG_DIST_MONITOR,
    // DFLAG_DIST_MONITOR_NAME, DFLAG_DIST_HDR_ATOM_CACHE, DFLAG_EXIT_PAYLOAD,
    // and DFLAG_ALIAS (dist.h:33, 34, 43, 52, 67, 113-123), with the rejectable and the strict
    // order flags of ERTS (dist.h:129-135).
    let epmd = Epmd::start();
    let erts = erts_erl(
        &epmd,
        &[
            "-noshell",
            "-eval",
            "io:format(\"~p~n\", [erts_internal:get_dflags()]), halt().",
        ],
    );
    assert_eq!(
        stdout(&erts),
        "{erts_dflags,468283523004,17230663572,468283523004,8396866,8192}\n"
    );
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
        "{erts_dflags,55889571772,17230663572,55889571772,8396866,8192}\n"
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
        ("monitor_op()".to_owned(), rejected),
        (
            "altact_op()".to_owned(),
            "unsupported distribution operation 37",
        ),
        ("demonitor_ref()".to_owned(), rejected),
        ("deliver_port()".to_owned(), deliver),
        ("deliver_udp()".to_owned(), deliver),
    ];
    // Each part of a spawn request, a spawn reply, and a group leader
    // message that ERTS rejects ends the connection (dist.c:2889-3130).
    cases.extend(
        [
            "spawn_bad_ref()",
            "spawn_bad_from()",
            "spawn_bad_from_local()",
            "spawn_bad_leader()",
            "spawn_bad_mfa()",
            "spawn_bad_arity()",
            "spawn_bad_arguments()",
            "reply_bad_to()",
            "reply_bad_ref()",
            "reply_bad_flags()",
            "reply_bad_result()",
            "group_leader_bad()",
            "group_leader_bad_to()",
            "spawn_local_ref()",
            "spawn_bad_local_leader()",
            "spawn_bad_mfa_not_tuple()",
            "spawn_bad_mfa_tuple()",
            "spawn_bad_mfa_fields()",
            "spawn_bad_arguments_list()",
        ]
        .map(|call| (call.to_owned(), rejected)),
    );
    // PBeam does not implement the priority option of a monitor or a link and
    // the alias option of a monitor (`bif.c:675-780`).
    cases.extend([
        (
            "spawn_monitor_priority()".to_owned(),
            "unsupported monitor option priority",
        ),
        (
            "spawn_monitor_alias()".to_owned(),
            "unsupported spawn monitor option alias",
        ),
        (
            "spawn_link_priority()".to_owned(),
            "unsupported link option priority",
        ),
        (
            "spawn_scheduler()".to_owned(),
            "unsupported spawn option Tuple([Atom(\"scheduler\"), Integer(1)])",
        ),
    ]);
    cases.extend([12, 13, 18, 25, 27].map(|op| {
        (
            format!("trace_token({op})"),
            "unsupported distribution message with a trace token",
        )
    }));
    cases.extend(
        [
            1, 2, 3, 4, 5, 6, 7, 8, 10, 11, 12, 13, 14, 15, 16, 17, 18, 19, 20, 21, 22, 23, 24, 25,
            26, 27,
        ]
        .map(|n| (format!("reject({n})"), rejected)),
    );
    cases.push(("atom_cache()".to_owned(), rejected));
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
fn exchanges_many_cached_atoms_with_pinned_erts() {
    let epmd = Epmd::start();
    let id = std::process::id();
    let server_name = format!("dacache{id}");
    let server = ErtsNode::start(
        &epmd,
        &["-sname", &server_name, "-setcookie", "ck"],
        "register(cache_echo, self()), io:format(\"~s~n\", [node()]), \
         F = fun F() -> receive {From, Atoms} -> From ! {atoms, Atoms}, F() \
         after 30000 -> halt(2) end end, F().",
    );
    let node = server.line();
    let expression = format!(
        "true = net_kernel:connect_node('{node}'), \
         Atoms = [list_to_atom(\"cache_atom_\" ++ integer_to_list(N)) || N <- lists:seq(1,300)], \
         Exchange = fun() -> {{cache_echo, '{node}'}} ! {{self(), Atoms}}, \
         receive {{atoms, Result}} -> Result =:= Atoms after 5000 -> timeout end end, \
         A = Exchange(), B = Exchange(), io:format(\"~p~n\", [{{A,B,length(Atoms)}}]), halt()."
    );
    let run = |name: &str| {
        vec![
            "-sname".to_owned(),
            name.to_owned(),
            "-setcookie".to_owned(),
            "ck".to_owned(),
            "-noshell".to_owned(),
            "-eval".to_owned(),
            expression.clone(),
        ]
    };
    assert_eq!(
        stdout(&erts_erl(&epmd, &run(&format!("dacerts{id}")))),
        "{true,true,300}\n"
    );
    assert_eq!(
        stdout(&pbeam_erl(&epmd, &run(&format!("dacpbeam{id}")))),
        "{true,true,300}\n"
    );
}

#[test]
fn exchanges_cached_identifier_nodes_with_pinned_erts() {
    let epmd = Epmd::start();
    let id = std::process::id();
    let name = format!("dcids{id}");
    let server = ErtsNode::start(
        &epmd,
        &["-sname", &name, "-setcookie", "ck"],
        "register(cache_ids, self()), io:format(\"~s~n\", [node()]), \
         Name = <<\"cache_foreign_identifier_node@host\">>, N = byte_size(Name), \
         Node = <<119,N,Name/binary>>, \
         Pid = binary_to_term(<<131,88,Node/binary,1:32,0:32,4:32>>), \
         Port = binary_to_term(<<131,120,Node/binary,2:64,4:32>>), \
         Ref = binary_to_term(<<131,90,1:16,Node/binary,4:32,3:32>>), \
         Ids = {Pid,Port,Ref}, \
         F = fun F() -> receive {From, get_ids} -> From ! {ids,Ids}, F(); \
         {From, {check,Got}} -> From ! {same,Got =:= Ids}, F() \
         after 30000 -> halt(2) end end, F().",
    );
    let node = server.line();
    let expression = format!(
        "true = net_kernel:connect_node('{node}'), \
         {{cache_ids, '{node}'}} ! {{self(), get_ids}}, \
         receive {{ids, {{Pid,Port,Ref}} = Ids}} -> \
         {{cache_ids, '{node}'}} ! {{self(), {{check,Ids}}}}, \
         receive {{same,Same}} -> io:format(\"~0p~n\", \
         [{{is_pid(Pid),is_port(Port),is_reference(Ref), \
         node(Pid),node(Port),node(Ref),Same}}]) after 5000 -> halt(2) end \
         after 5000 -> halt(3) end, halt()."
    );
    let run = |name: &str| {
        vec![
            "-sname".to_owned(),
            name.to_owned(),
            "-setcookie".to_owned(),
            "ck".to_owned(),
            "-noshell".to_owned(),
            "-eval".to_owned(),
            expression.clone(),
        ]
    };
    let expected = "{true,true,true,cache_foreign_identifier_node@host,cache_foreign_identifier_node@host,cache_foreign_identifier_node@host,true}\n";
    assert_eq!(
        stdout(&erts_erl(&epmd, &run(&format!("dcierts{id}")))),
        expected
    );
    assert_eq!(
        stdout(&pbeam_erl(&epmd, &run(&format!("dcipbeam{id}")))),
        expected
    );
}

#[test]
fn exchanges_cached_record_headers_with_pinned_erts() {
    let epmd = Epmd::start();
    let id = std::process::id();
    let name = format!("dcrecord{id}");
    let server = ErtsNode::start(
        &epmd,
        &["-sname", &name, "-setcookie", "ck"],
        "register(cache_record, self()), io:format(\"~s~n\", [node()]), \
         Record = records:create(cache_record_module, cache_record_name, \
         [{cache_field_one,17},{cache_field_two,23}], #{is_exported => true}), \
         F = fun F() -> receive {From, get_record} -> From ! {record,Record}, F(); \
         {From, {check,Got}} -> From ! {same,Got =:= Record}, F() \
         after 30000 -> halt(2) end end, F().",
    );
    let node = server.line();
    let expression = format!(
        "true = net_kernel:connect_node('{node}'), \
         {{cache_record, '{node}'}} ! {{self(), get_record}}, \
         receive {{record, Record}} -> \
         {{cache_record, '{node}'}} ! {{self(), {{check,Record}}}}, \
         receive {{same,Same}} -> io:format(\"~0p~n\", \
         [{{records:get_module(Record), records:get_name(Record), \
         records:get_field_names(Record),Same}}]) after 5000 -> halt(2) end \
         after 5000 -> halt(3) end, halt()."
    );
    let run = |name: &str| {
        vec![
            "-sname".to_owned(),
            name.to_owned(),
            "-setcookie".to_owned(),
            "ck".to_owned(),
            "-noshell".to_owned(),
            "-eval".to_owned(),
            expression.clone(),
        ]
    };
    let expected =
        "{cache_record_module,cache_record_name,[cache_field_one,cache_field_two],true}\n";
    assert_eq!(
        stdout(&erts_erl(&epmd, &run(&format!("dcrerts{id}")))),
        expected
    );
    assert_eq!(
        stdout(&pbeam_erl(&epmd, &run(&format!("dcrpbeam{id}")))),
        expected
    );
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
fn monitors_a_registered_name_of_another_node_as_pinned_erts() {
    // A `MONITOR_P` packet can ask for a registered name, and the node looks
    // the name up when it gets the packet: the process that the name has
    // then gets the monitor, and a name that no process has answers
    // `MONITOR_P_EXIT` with `noproc` (`dist.c:2364-2379`, `2398-2410`,
    // `erl_process.c:13844-13847`). The monitor exit packet names the name
    // of the packet, so the `DOWN` item of the watcher is `{Name, Node}`.
    // The removal of a monitor whose target already exited still gives
    // `true` and sends its `DEMONITOR_P` (`bif.c:525-570`), and the name
    // then names the target of no monitor of the connection, so the node
    // removes nothing (`dist.c:2441-2457`). The end of the connection gives
    // `noconnection` to the last name monitor (`dist.c:305-321`). Each pair
    // of ERTS and PBeam nodes runs over the prim_socket carrier and over the
    // tcp_inet port controller. The client stops the server node at the end,
    // because a node that the test kills writes no profile of its own runs.
    let epmd = Epmd::start();
    let id = std::process::id();
    let expected = "[{down,boom},{removed,none},{late,true},{noproc,noproc},{lost,noconnection}]\n";
    for (server_pbeam, socket) in [(false, true), (false, false), (true, true), (true, false)] {
        for client_pbeam in [false, true] {
            // The `lost` case stops the server node, so each client needs a
            // server of its own.
            let name = format!("dnsrv{server_pbeam}{socket}{client_pbeam}{id}");
            let mut arguments = vec!["-sname", &name, "-setcookie", "ck", "-pa", "fixtures/dist"];
            if socket {
                arguments.extend(SOCKET_CARRIER);
            }
            let server = "dist_name_probe:server().";
            let node = if server_pbeam {
                pbeam_node(&epmd, &arguments, server)
            } else {
                ErtsNode::start(&epmd, &arguments, server)
            };
            let expression = format!("dist_name_probe:client('{}'), halt().", node.line());
            let client = |name: &str| {
                let mut arguments = vec!["-sname", name, "-setcookie", "ck", "-hidden"];
                if socket {
                    arguments.extend(SOCKET_CARRIER);
                }
                arguments.extend(["-noshell", "-pa", "fixtures/dist", "-eval", &expression]);
                arguments.into_iter().map(str::to_owned).collect::<Vec<_>>()
            };
            let client_name = format!("dncl{server_pbeam}{socket}{client_pbeam}{id}");
            let output = if client_pbeam {
                pbeam_erl(&epmd, &client(&client_name))
            } else {
                erts_erl(&epmd, &client(&client_name))
            };
            assert_eq!(
                stdout(&output),
                expected,
                "server pbeam {server_pbeam}, socket {socket}, client pbeam {client_pbeam}"
            );
        }
    }
}

#[test]
fn spawns_processes_on_another_node_as_pinned_erts() {
    // `spawn/4`, `spawn_link/4`, `spawn_monitor/4`, `spawn_opt/5`, and
    // `spawn_request/5` start a process on another node; a link and a monitor
    // join it, a spawn request answers `{spawn_reply, ReqId, ok, Pid}` and a
    // `DOWN` at the exit, a node with no connection answers `noconnection`,
    // and `io:format/2` of the remote process writes to the group leader of
    // the caller (bif.c:1027-1075, erl_process.c:12440-12530,
    // dist.c:5850-6260). Each pair of ERTS and PBeam nodes runs over the
    // prim_socket carrier and over the tcp_inet port controller. The client
    // stops the server node at the end, because a node that the test kills
    // writes no profile of its own runs.
    let epmd = Epmd::start();
    let id = std::process::id();
    let expected = "[{spawn,true},{monitor,normal},{link,boom},{request,ok},\
                    {request_down,boom},{noconnection,noconnection},\
                    {lost,noconnection},{badopt,{badopt,badopt,badarg}},\
                    {unknown,{badopt,badarg}},\
                    {reply,[{no,{none,normal}},{error_only,{none,normal}},\
                            {success_only,none},{no_error,none}]}]\n";
    for (server_pbeam, socket) in [(false, true), (false, false), (true, true), (true, false)] {
        for client_pbeam in [false, true] {
            let name = format!("dssrv{server_pbeam}{socket}{client_pbeam}{id}");
            let mut arguments = vec!["-sname", &name, "-setcookie", "ck", "-pa", "fixtures/dist"];
            if socket {
                arguments.extend(SOCKET_CARRIER);
            }
            let server = "dist_spawn_probe:server().";
            let node = if server_pbeam {
                pbeam_node(&epmd, &arguments, server)
            } else {
                ErtsNode::start(&epmd, &arguments, server)
            };
            let expression = format!("dist_spawn_probe:client('{}'), halt().", node.line());
            let client = |name: &str| {
                let mut arguments = vec!["-sname", name, "-setcookie", "ck", "-hidden"];
                if socket {
                    arguments.extend(SOCKET_CARRIER);
                }
                arguments.extend(["-noshell", "-pa", "fixtures/dist", "-eval", &expression]);
                arguments.into_iter().map(str::to_owned).collect::<Vec<_>>()
            };
            let client_name = format!("dscl{server_pbeam}{socket}{client_pbeam}{id}");
            let output = if client_pbeam {
                pbeam_erl(&epmd, &client(&client_name))
            } else {
                erts_erl(&epmd, &client(&client_name))
            };
            assert_eq!(
                stdout(&output),
                expected,
                "server pbeam {server_pbeam}, socket {socket}, client pbeam {client_pbeam}"
            );
        }
    }
}

#[test]
fn gives_a_spawned_process_the_group_leader_of_the_caller_as_pinned_erts() {
    // A spawn request carries the group leader of the caller, so the new
    // process writes `io:format/2` to the caller's group leader
    // (`dist.c:6129-6131`, `erl_process.c:12440-12530`). PBeam keeps that
    // pid, also when it belongs to another node (`bif.c:4914-4944`). The
    // probe prints `gl` on the group leader of the new process, then whether
    // that leader is the leader of the caller and whether it is a pid of the
    // caller's node.
    let epmd = Epmd::start();
    let id = std::process::id();
    for (server_pbeam, socket) in [(false, true), (false, false), (true, true), (true, false)] {
        for client_pbeam in [false, true] {
            let name = format!("dlsrv{server_pbeam}{socket}{client_pbeam}{id}");
            let mut arguments = vec!["-sname", &name, "-setcookie", "ck", "-pa", "fixtures/dist"];
            if socket {
                arguments.extend(SOCKET_CARRIER);
            }
            let server = "dist_spawn_probe:server().";
            let node = if server_pbeam {
                pbeam_node(&epmd, &arguments, server)
            } else {
                ErtsNode::start(&epmd, &arguments, server)
            };
            let expression = format!("dist_spawn_probe:leader('{}'), halt().", node.line());
            let client = |name: &str| {
                let mut arguments = vec!["-sname", name, "-setcookie", "ck", "-hidden"];
                if socket {
                    arguments.extend(SOCKET_CARRIER);
                }
                arguments.extend(["-noshell", "-pa", "fixtures/dist", "-eval", &expression]);
                arguments.into_iter().map(str::to_owned).collect::<Vec<_>>()
            };
            let client_name = format!("dlcl{server_pbeam}{socket}{client_pbeam}{id}");
            let output = if client_pbeam {
                pbeam_erl(&epmd, &client(&client_name))
            } else {
                erts_erl(&epmd, &client(&client_name))
            };
            let expected = "gl\nleader: [{true,true}]\n";
            assert_eq!(
                stdout(&output),
                expected,
                "server pbeam {server_pbeam}, socket {socket}, client pbeam {client_pbeam}"
            );
        }
    }
}

#[test]
fn shows_the_options_of_a_spawn_as_pinned_erts() {
    // Each argument check of `erts_internal:dist_spawn_request/4` returns
    // `badarg`, and the option list gives the reply policy, the tag, and the
    // link and monitor options (`dist.c:5860-5975`). The probe sends each
    // option list to a server of the pinned ERTS, so only the caller differs
    // between the two runs.
    let epmd = Epmd::start();
    let id = std::process::id();
    let name = format!("dsopt{id}");
    let arguments = vec!["-sname", &name, "-setcookie", "ck", "-pa", "fixtures/dist"];
    let server = ErtsNode::start(&epmd, &arguments, "dist_spawn_probe:server().");
    let expression = format!("dist_spawn_probe:options('{}'), halt().", server.line());
    let client = [
        "-sname",
        &format!("dsoptc{id}"),
        "-setcookie",
        "ck",
        "-hidden",
        "-noshell",
        "-pa",
        "fixtures/dist",
        "-eval",
        &expression,
    ];
    let erts = stdout(&erts_erl(&epmd, &client));
    assert!(
        erts.starts_with("options: [{bad_node,badarg},{local,badarg},"),
        "{erts}"
    );
    assert!(
        erts.ends_with("{no_connection,none},{no_receiver,none}]\n"),
        "{erts}"
    );
    assert_eq!(stdout(&pbeam_erl(&epmd, &client)), erts);
}

#[test]
fn spawns_a_request_over_a_channel_as_pinned_erts() {
    // A connection whose flags have no `DFLAG_SPAWN` answers a spawn request
    // with `notsup`, the end of a connection answers a spawn request that
    // waits with `noconnection` or with no message for a suppressed error,
    // and a reply for a reference that no monitor holds is ignored
    // (`dist.c:3086-3130`, `6081-6093`, `6197-6202`,
    // `erl_proc_sig_queue.c:4864-4900`).
    let epmd = Epmd::start();
    let name = format!("dspawnc{}", std::process::id());
    for (function, expected) in [
        ("spawn_notsup", "spawn_notsup: notsup\n"),
        ("spawn_lost", "spawn_lost: noconnection\n"),
        ("spawn_lost_suppressed", "spawn_lost_suppressed: none\n"),
        ("spawn_reply_unknown", "spawn_reply_unknown: true\n"),
        (
            "spawn_reply_ordinary_monitor",
            "spawn_reply_ordinary_monitor: [{normal,true}]\n",
        ),
    ] {
        let expression = format!("dist_channel_probe:{function}(), halt().");
        let arguments = probe(&["-sname", &name], &expression);
        let erts = stdout(&erts_erl(&epmd, &arguments));
        assert_eq!(erts, expected, "{function}");
        assert_eq!(stdout(&pbeam_erl(&epmd, &arguments)), erts, "{function}");
    }
}

#[test]
fn sets_the_group_leader_of_a_local_process_as_pinned_erts() {
    // A spawn request that names a local pid as the group leader uses it, a
    // `DOP_GROUP_LEADER` packet sets the group leader of a local process,
    // and a monitor down for a reference that no monitor holds is ignored
    // (`dist.c:2889-2901`, `3086-3130`). The probes print the reply of the
    // first packet, the new group leader, and whether the connection stays
    // open.
    let epmd = Epmd::start();
    let name = format!("dsgl{}", std::process::id());
    for function in [
        "spawn_local_leader",
        "group_leader_packet",
        "group_leader_external_to",
        "monitor_down_unknown",
    ] {
        let expression = format!("dist_channel_probe:{function}(), halt().");
        let arguments = probe(&["-sname", &name], &expression);
        let erts = stdout(&erts_erl(&epmd, &arguments));
        assert!(!erts.is_empty(), "{function}");
        assert_eq!(stdout(&pbeam_erl(&epmd, &arguments)), erts, "{function}");
    }
}

#[test]
fn gives_the_unicode_io_flag_of_a_remote_group_leader_as_pinned_erts() {
    // `net_kernel:dflag_unicode_io/1` gives the flag of the connection of a
    // pid of another node, `true` for a pid of this node also of an old
    // incarnation, and `false` for a node with no connection
    // (`dist.c:6693-6711`). `io:format/2` of a process whose group leader is
    // a pid of another node asks this flag with that pid.
    let epmd = Epmd::start();
    let name = format!("dsflag{}", std::process::id());
    let arguments = probe(
        &["-sname", &name],
        "dist_channel_probe:dflag_unicode_io(), halt().",
    );
    let erts = stdout(&erts_erl(&epmd, &arguments));
    assert!(
        erts.contains("{not_unicode,false}") && erts.contains("{unicode,true}"),
        "{erts}"
    );
    assert_eq!(stdout(&pbeam_erl(&epmd, &arguments)), erts);
}

#[test]
fn applies_the_options_of_a_spawn_request_as_pinned_erts() {
    // The options of a spawn request set the start values of the priority,
    // the message queue data, and the `async_dist` flag of the new process,
    // as `erlang:spawn_opt/4` does (`erl_process.c:12275-12320`,
    // `12552-12558`). The probe reads each start value and prints it, and
    // `{scheduler, 1}` stops the run as the local spawn does, because PBeam
    // does not bind a process to a scheduler (`erl_process.c:12500-12507`).
    let epmd = Epmd::start();
    let id = std::process::id();
    for server_pbeam in [false, true] {
        let name = format!("dsfl{server_pbeam}{id}");
        let arguments = vec!["-sname", &name, "-setcookie", "ck", "-pa", "fixtures/dist"];
        let server = "dist_spawn_probe:server().";
        let node = if server_pbeam {
            pbeam_node(&epmd, &arguments, server)
        } else {
            ErtsNode::start(&epmd, &arguments, server)
        };
        let expression = format!("dist_spawn_probe:flags('{}'), halt().", node.line());
        let client = [
            "-sname",
            &format!("dsfc{server_pbeam}{id}"),
            "-setcookie",
            "ck",
            "-hidden",
            "-noshell",
            "-pa",
            "fixtures/dist",
            "-eval",
            &expression,
        ];
        let output = if server_pbeam {
            pbeam_erl(&epmd, &client)
        } else {
            erts_erl(&epmd, &client)
        };
        assert_eq!(
            stdout(&output),
            "flags: [{spawn,true},{info,{{priority,high},off_heap,true}}]\n",
            "server pbeam {server_pbeam}"
        );
    }
}

#[test]
fn demonitors_a_process_of_another_node_as_pinned_erts() {
    // A monitor that `spawn_monitor/4` started keeps the pid of the new
    // process and its connection, so `demonitor/1` sends `DEMONITOR_P` and
    // removes the monitor of the connection (`bif.c:436-541`,
    // `dist.c:2434-2459`). The probe asks the target for the monitors of the
    // process after the demonitor.
    let epmd = Epmd::start();
    let id = std::process::id();
    let name = format!("dsdm{id}");
    let arguments = vec!["-sname", &name, "-setcookie", "ck", "-pa", "fixtures/dist"];
    let server = ErtsNode::start(&epmd, &arguments, "dist_spawn_probe:server().");
    let expression = format!("dist_spawn_probe:demonitor('{}'), halt().", server.line());
    let client = [
        "-sname",
        &format!("dsdmc{id}"),
        "-setcookie",
        "ck",
        "-hidden",
        "-noshell",
        "-pa",
        "fixtures/dist",
        "-eval",
        &expression,
    ];
    assert_eq!(
        stdout(&pbeam_erl(&epmd, &client)),
        "demonitor: {monitored_by,[]}\n"
    );
}

#[test]
fn reads_the_options_of_a_spawn_request_as_pinned_erts() {
    // A spawn request that this node receives reads its option list as
    // `erts_parse_spawn_opts` does (`erl_process.c:12222-12428`,
    // `dist.c:2949-2984`): a valid option spawns the process with the reply
    // flags of the link and the monitor, and each rejected option, also an
    // improper list, answers `badopt` with no process. The reply of the
    // first packet names the reference, the pid of the other node, the
    // flags, and the new pid or `badopt`.
    let epmd = Epmd::start();
    let name = format!("dsopts{}", std::process::id());
    let arguments = probe(
        &["-sname", &name],
        "dist_channel_probe:spawn_options(), halt().",
    );
    let erts = stdout(&erts_erl(&epmd, &arguments));
    // `~p` wraps a term at 80 columns and the host name of the node changes
    // where it wraps, so the check names the case and not the reply text.
    for tag in [
        "link: ",
        "atom: ",
        "improper_list: ",
        "monitor_name_not_atom: ",
    ] {
        assert!(
            erts.lines().any(|line| line.starts_with(tag)),
            "{tag}\n{erts}"
        );
    }
    let pbeam = stdout(&pbeam_erl(&epmd, &arguments));
    assert_eq!(pbeam, erts);
}

#[test]
fn calls_a_server_of_another_node_by_name_as_pinned_erts() {
    // `gen_server:call/2` to `{Name, Node}` monitors the name with the
    // `demonitor` alias option and waits for the reply through the alias
    // (`gen.erl`, `gen_server.erl`), and `net_adm:ping/1` makes a `gen:call`
    // to `{net_kernel, Node}` and answers `pong` or `pang`
    // (`net_adm.erl:101-110`). The client stops the server node at the end,
    // because a node that the test kills writes no profile of its own runs.
    // Each pair of ERTS and PBeam nodes runs over the prim_socket carrier and
    // over the tcp_inet port controller.
    let epmd = Epmd::start();
    let id = std::process::id();
    let expected = "[{ping,pong},{pang,pang},{reply,42},{noproc,noproc},{lost,noconnection}]\n";
    for (server_pbeam, socket) in [(false, true), (false, false), (true, true), (true, false)] {
        for client_pbeam in [false, true] {
            // The client stops the server node, so each client needs a server
            // of its own.
            let name = format!("dcsrv{server_pbeam}{socket}{client_pbeam}{id}");
            let mut arguments = vec!["-sname", &name, "-setcookie", "ck", "-pa", "fixtures/dist"];
            if socket {
                arguments.extend(SOCKET_CARRIER);
            }
            let server = "dist_call_probe:server().";
            let node = if server_pbeam {
                pbeam_node(&epmd, &arguments, server)
            } else {
                ErtsNode::start(&epmd, &arguments, server)
            };
            let expression = format!("dist_call_probe:client('{}'), halt().", node.line());
            let client = |name: &str| {
                let mut arguments = vec!["-sname", name, "-setcookie", "ck", "-hidden"];
                if socket {
                    arguments.extend(SOCKET_CARRIER);
                }
                arguments.extend(["-noshell", "-pa", "fixtures/dist", "-eval", &expression]);
                arguments.into_iter().map(str::to_owned).collect::<Vec<_>>()
            };
            let client_name = format!("dccl{server_pbeam}{socket}{client_pbeam}{id}");
            let output = if client_pbeam {
                pbeam_erl(&epmd, &client(&client_name))
            } else {
                erts_erl(&epmd, &client(&client_name))
            };
            assert_eq!(
                stdout(&output),
                expected,
                "server pbeam {server_pbeam}, socket {socket}, client pbeam {client_pbeam}"
            );
        }
    }
}

#[test]
fn pings_a_live_node_with_no_connection_as_pinned_erts() {
    // `net_adm:ping/1` makes a `gen:call` to `{net_kernel, Node}`, so it
    // monitors the name and sends to it. ERTS starts a connection on that
    // call and answers `pong`. PBeam ends the monitor of a node with no
    // connection at once with `noconnection`, so it answers `pang`; #1412
    // adds the connection on the first signal (row "Distribution").
    let epmd = Epmd::start();
    let id = std::process::id();
    for (server_pbeam, socket) in [(false, true), (false, false), (true, true), (true, false)] {
        let name = format!("dpsrv{server_pbeam}{socket}{id}");
        let mut arguments = vec!["-sname", &name, "-setcookie", "ck", "-pa", "fixtures/dist"];
        if socket {
            arguments.extend(SOCKET_CARRIER);
        }
        let server = "dist_call_probe:server().";
        let node = if server_pbeam {
            pbeam_node(&epmd, &arguments, server)
        } else {
            ErtsNode::start(&epmd, &arguments, server)
        };
        let expression = format!("dist_call_probe:unconnected('{}'), halt().", node.line());
        let client = |name: &str| {
            let mut arguments = vec!["-sname", name, "-setcookie", "ck", "-hidden"];
            if socket {
                arguments.extend(SOCKET_CARRIER);
            }
            arguments.extend(["-noshell", "-pa", "fixtures/dist", "-eval", &expression]);
            arguments.into_iter().map(str::to_owned).collect::<Vec<_>>()
        };
        let erts = format!("dperts{server_pbeam}{socket}{id}");
        assert_eq!(
            stdout(&erts_erl(&epmd, &client(&erts))),
            "[{unconnected,pong}]\n",
            "server pbeam {server_pbeam}, socket {socket}"
        );
        let pbeam = format!("dppb{server_pbeam}{socket}{id}");
        assert_eq!(
            stdout(&pbeam_erl(&epmd, &client(&pbeam))),
            "[{unconnected,pang}]\n",
            "server pbeam {server_pbeam}, socket {socket}"
        );
    }
}

#[test]
fn monitors_processes_between_erts_and_pbeam_nodes() {
    // A client process monitors a process of a server node and a process of
    // the server node monitors a client process, and the watched process
    // exits with normal, kill, or a reason that is not immediate; the
    // removal of a monitor gives no DOWN message, a pid that is not alive
    // gives noproc, and the end of the connection gives noconnection from
    // the oldest monitor to the newest (bif.c:758-981, 436-541;
    // dist.c:1496-1600, 2341-2470, 2688-2746, 1021-1117, 305-321;
    // erl_process.c:14101-14147). Each pair of ERTS and PBeam nodes runs
    // over the prim_socket carrier and over the tcp_inet port controller.
    let epmd = Epmd::start();
    let id = std::process::id();
    let expected = "[{to,normal,normal},\n {to,kill,kill},\n {to,{boom},{boom}},\n \
                    {from,normal,normal},\n {from,kill,kill},\n {from,{boom},{boom}},\n \
                    {demonitor,none},\n {noproc,noproc},\n \
                    {lost,[{oldest,noconnection},{newest,noconnection}]}]\n";
    for (server_pbeam, socket) in [(false, true), (false, false), (true, true), (true, false)] {
        for client_pbeam in [false, true] {
            // The `lost` case stops the server node, so each client needs a
            // server of its own.
            let name = format!("dmsrv{server_pbeam}{socket}{client_pbeam}{id}");
            let mut arguments = vec!["-sname", &name, "-setcookie", "ck", "-pa", "fixtures/dist"];
            if socket {
                arguments.extend(SOCKET_CARRIER);
            }
            let server = "dist_monitor_probe:server().";
            let node = if server_pbeam {
                pbeam_node(&epmd, &arguments, server)
            } else {
                ErtsNode::start(&epmd, &arguments, server)
            };
            let expression = format!("dist_monitor_probe:client('{}'), halt().", node.line());
            let client = |name: &str| {
                let mut arguments = vec!["-sname", name, "-setcookie", "ck", "-hidden"];
                if socket {
                    arguments.extend(SOCKET_CARRIER);
                }
                arguments.extend(["-noshell", "-pa", "fixtures/dist", "-eval", &expression]);
                arguments.into_iter().map(str::to_owned).collect::<Vec<_>>()
            };
            let client_name = format!("dmcl{server_pbeam}{socket}{client_pbeam}{id}");
            let output = if client_pbeam {
                pbeam_erl(&epmd, &client(&client_name))
            } else {
                erts_erl(&epmd, &client(&client_name))
            };
            let text = stdout(&output);
            assert_eq!(
                text, expected,
                "server pbeam {server_pbeam}, socket {socket}, client pbeam {client_pbeam}"
            );
        }
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

#[test]
fn ends_the_links_of_a_lost_connection_as_pinned_erts() {
    // The server node halts, and each process of the client node that has a
    // link to its process takes the exit signal `noconnection`: the process
    // that traps exits gets the message `{'EXIT', Pid, noconnection}`, and
    // the process that does not trap exits ends with the reason
    // `noconnection` (erts_do_net_exits, dist.c:1021-1117,
    // erl_proc_sig_queue.c:4632-4815). Each pair of ERTS and PBeam nodes
    // runs over the prim_socket carrier and over the tcp_inet port
    // controller. A server node serves one client node, because the client
    // stops it.
    let epmd = Epmd::start();
    let id = std::process::id();
    let expected = "[{linked,noconnection,noconnection}]\n";
    for (server_pbeam, socket) in [(false, true), (false, false), (true, true), (true, false)] {
        for client_pbeam in [false, true] {
            let name = format!("dlo{server_pbeam}{socket}{client_pbeam}{id}");
            let mut arguments = vec!["-sname", &name, "-setcookie", "ck", "-pa", "fixtures/dist"];
            if socket {
                arguments.extend(SOCKET_CARRIER);
            }
            let server = "dist_lost_probe:server().";
            let node = if server_pbeam {
                pbeam_node(&epmd, &arguments, server)
            } else {
                ErtsNode::start(&epmd, &arguments, server)
            };
            let expression = format!("dist_lost_probe:client('{}'), halt().", node.line());
            let client = |name: &str| {
                let mut arguments = vec!["-sname", name, "-setcookie", "ck", "-hidden"];
                if socket {
                    arguments.extend(SOCKET_CARRIER);
                }
                arguments.extend(["-noshell", "-pa", "fixtures/dist", "-eval", &expression]);
                arguments.into_iter().map(str::to_owned).collect::<Vec<_>>()
            };
            let client_name = format!("dlc{server_pbeam}{socket}{client_pbeam}{id}");
            let output = if client_pbeam {
                pbeam_erl(&epmd, &client(&client_name))
            } else {
                erts_erl(&epmd, &client(&client_name))
            };
            assert_eq!(stdout(&output), expected);
        }
    }
}

#[test]
fn runs_the_local_spawn_request_forms_as_pinned_erts() {
    // `erts_internal:spawn_request/4` and `erlang:spawn_request_abandon/1`
    // of a local node give the pinned ERTS results (bif.c:1078-1208,
    // erl_process.c:12440-12530, 12949-12976): the request id is a
    // reference, a bad option raises `badarg` with the `badopt` cause, the
    // reply keeps the `spawn_reply` or `reply_tag` form, and an abandoned
    // request that links sends the child the exit signal `abandoned` while
    // each other abandoned request gives the caller no message.
    let epmd = Epmd::start();
    let name = format!("dreq{}", std::process::id());
    let expression = "dist_request_probe:local(), halt().";
    let arguments = probe(&["-sname", &name], expression);
    let erts = stdout(&erts_erl(&epmd, &arguments));
    assert!(!erts.is_empty(), "{erts}");
    assert_eq!(stdout(&pbeam_erl(&epmd, &arguments)), erts);
    // The unsupported `{scheduler, 1}` option stops the run, so a separate
    // invocation asserts its runtime error text.
    let stop_name = format!("dreqs{name}");
    let expression = "dist_request_probe:scheduler_stop(), halt().";
    let arguments = probe(&["-sname", &stop_name], expression);
    let output = pbeam_erl(&epmd, &arguments);
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        !output.status.success()
            && stderr.contains("unsupported spawn option Tuple([Atom(\"scheduler\"), Integer(1)])"),
        "{output:?}"
    );
    // `abandon/1` sends a spawn request to a server node and abandons it, so
    // it covers the external reference of a request of another node
    // (`bif.c:1171-1174`). Each client needs a server of its own, so the
    // server and the client pair over both carriers.
    for socket in [true, false] {
        let server_name = format!("dreqsv{name}{socket}");
        let mut server_args = vec![
            "-sname",
            &server_name,
            "-setcookie",
            "ck",
            "-pa",
            "fixtures/dist",
        ];
        if socket {
            server_args.extend(SOCKET_CARRIER);
        }
        let node = ErtsNode::start(
            &epmd,
            &server_args,
            "io:format(\"~s~n\", [node()]), timer:sleep(5000).",
        );
        let expression = format!("dist_request_probe:abandon('{}'), halt().", node.line());
        let client_name = format!("dreqac{name}{socket}");
        let mut client_args = vec!["-sname", &client_name, "-setcookie", "ck", "-hidden"];
        if socket {
            client_args.extend(SOCKET_CARRIER);
        }
        client_args.extend(["-noshell", "-pa", "fixtures/dist", "-eval", &expression]);
        let client_args: Vec<String> = client_args.into_iter().map(str::to_owned).collect();
        let expected = stdout(&erts_erl(&epmd, &client_args));
        assert!(!expected.is_empty(), "{expected}");
        assert_eq!(
            stdout(&pbeam_erl(&epmd, &client_args)),
            expected,
            "socket {socket}"
        );
    }
}

#[test]
fn runs_erpc_and_rpc_between_erts_and_pbeam_nodes_as_pinned_erts() {
    // `erpc:call/4,5`, `erpc:multicall/4,5`, `rpc:call/4,5`,
    // `rpc:multicall/4,5`, `erlang:spawn/4` (the `Node.spawn/4` of Elixir),
    // and a remote `code:load_binary/3` through `rpc:call/4` give the
    // pinned ERTS results between a PBeam node and an ERTS node
    // (lib/kernel/src/erpc.erl:277-323, 418-429, 1045-1144,
    // lib/kernel/src/rpc.erl:441-493, 878-893). The client loads
    // `dist_erpc_load` on the server, and PBeam cannot replace a loaded
    // module (code_table.rs), so each client run gets a fresh server. Each
    // pair of ERTS and PBeam nodes runs over the prim_socket carrier and
    // over the tcp_inet port controller.
    let epmd = Epmd::start();
    let id = std::process::id();
    for (server_pbeam, socket) in [(false, true), (false, false), (true, true), (true, false)] {
        // The client loads `dist_erpc_load` on the server, and PBeam cannot
        // replace a loaded module (code_table.rs), so each client run gets a
        // fresh server. The probe normalizes the server atom to `server`,
        // so the two runtimes give the same text.
        let run_client = |client_pbeam: bool| {
            let name = format!("desrv{server_pbeam}{socket}{client_pbeam}{id}");
            let mut arguments = vec!["-sname", &name, "-setcookie", "ck", "-pa", "fixtures/dist"];
            if socket {
                arguments.extend(SOCKET_CARRIER);
            }
            let node = if server_pbeam {
                pbeam_node(&epmd, &arguments, "dist_erpc_probe:server().")
            } else {
                ErtsNode::start(&epmd, &arguments, "dist_erpc_probe:server().")
            };
            let expression = format!("dist_erpc_probe:client('{}'), halt().", node.line());
            let client = |name: &str| {
                let mut arguments = vec!["-sname", name, "-setcookie", "ck", "-hidden"];
                if socket {
                    arguments.extend(SOCKET_CARRIER);
                }
                arguments.extend(["-noshell", "-pa", "fixtures/dist", "-eval", &expression]);
                arguments.into_iter().map(str::to_owned).collect::<Vec<_>>()
            };
            let client_name = format!("decl{server_pbeam}{socket}{client_pbeam}{id}");
            let output = if client_pbeam {
                pbeam_erl(&epmd, &client(&client_name))
            } else {
                erts_erl(&epmd, &client(&client_name))
            };
            stdout(&output)
        };
        // The ERTS client run gives the expected text of the pair, and the
        // PBeam client run must equal it, so a wrong timeout, exception, or
        // nodedown result fails the test.
        let expected = run_client(false);
        assert!(
            expected.starts_with("client: [")
                && expected.contains("{load,{{module,dist_erpc_load},loaded,7,true}}"),
            "server pbeam {server_pbeam}, socket {socket}: {expected}"
        );
        assert_eq!(
            run_client(true),
            expected,
            "server pbeam {server_pbeam}, socket {socket}"
        );
    }
}

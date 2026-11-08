#![expect(
    clippy::unwrap_used,
    reason = "test helpers report a failure by a panic"
)]

//! Distribution connections of a PBeam node with the pinned ERTS.

use std::{
    ffi::OsStr,
    io::{BufRead, BufReader, Read, Write},
    net::{TcpListener, TcpStream},
    path::PathBuf,
    process::{Child, Command, Output, Stdio},
    sync::{Arc, Mutex, mpsc},
    thread,
    time::{Duration, Instant},
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
fn orders_immediate_noconnection_priority_down_as_pinned_erts() {
    let epmd = Epmd::start();
    let name = format!("priomon{}", std::process::id());
    let arguments = probe(
        &["-sname", &name, "-pa", "fixtures/alias"],
        "io:format(\"~w~n\", [alias_probe:priority_immediate_remote()]), halt().",
    );
    let expected = "{[down,first,second],[first,second,down]}\n";
    assert_eq!(stdout(&erts_erl(&epmd, &arguments)), expected);
    assert_eq!(stdout(&pbeam_erl(&epmd, &arguments)), expected);
}

#[test]
fn aborts_pending_connections_as_pinned_erts() {
    // dist.c:5627-5711: abort clears only the matching pending connection.
    let epmd = Epmd::start();
    let name = format!("dabort{}", std::process::id());
    let expression = "Node = abort_peer@nohost, \
        Old = erts_internal:new_connection(Node), \
        Same = Old =:= erts_internal:new_connection(Node), \
        A = erts_internal:abort_pending_connection(Node, Old), \
        B = erts_internal:abort_pending_connection(Node, Old), \
        New = erts_internal:new_connection(Node), \
        C = erts_internal:abort_pending_connection(Node, Old), \
        StillPending = New =:= erts_internal:new_connection(Node), \
        D = erts_internal:abort_pending_connection(Node, New), \
        Error = try erts_internal:abort_pending_connection(other@nohost, New) \
                catch error:badarg -> badarg end, \
        io:format(\"~w~n\", [{Same,A,B,Old =/= New,C,StillPending,D,Error}]), halt().";
    let arguments = probe(&["-sname", &name], expression);
    let expected = "{true,true,true,true,true,true,true,badarg}\n";
    assert_eq!(stdout(&erts_erl(&epmd, &arguments)), expected);
    assert_eq!(stdout(&pbeam_erl(&epmd, &arguments)), expected);
}

#[test]
fn delivers_name_sends_queued_on_a_pending_connection_as_pinned_erts() {
    // bif.c:2391-2417 accepts noconnect on a pending connection.
    let epmd = Epmd::start();
    let id = std::process::id();
    for socket in [false, true] {
        for pbeam in [false, true] {
            let server_name = format!("dpendserver{socket}{pbeam}{id}");
            let mut server_args = vec!["-sname", server_name.as_str(), "-setcookie", "ck"];
            if socket {
                server_args.extend(SOCKET_CARRIER);
            }
            let server = ErtsNode::start(
                &epmd,
                &server_args,
                "register(pending_receiver,self()), io:format(\"~s~n\",[node()]), \
                 receive {one,From} -> receive two -> io:format(\"~w~n\",[{one,two}]), From ! ack end end, \
                 receive stop -> ok after 5000 -> ok end, halt().",
            );
            let expression = format!(
                "N='{}', erts_internal:new_connection(N), \
                 ok=erlang:send({{pending_receiver,N}},{{one,self()}},[noconnect]), \
                 ok=erlang:send({{pending_receiver,N}},two,[noconnect]), \
                 true=net_kernel:connect_node(N), \
                 receive ack -> io:format(\"true~n\") after 5000 -> io:format(\"timeout~n\") end, halt().",
                server.line()
            );
            let client_name = format!("dpendclient{socket}{pbeam}{id}");
            let mut args = vec![
                "-sname",
                client_name.as_str(),
                "-setcookie",
                "ck",
                "-kernel",
                "logger_level",
                "none",
            ];
            if socket {
                args.extend(SOCKET_CARRIER);
            }
            args.extend(["-noshell", "-eval", expression.as_str()]);
            let output = if pbeam {
                pbeam_erl(&epmd, &args)
            } else {
                erts_erl(&epmd, &args)
            };
            let text = stdout(&output);
            assert_eq!(text, "true\n", "socket={socket} pbeam={pbeam}");
            assert_eq!(server.line(), "{one,two}");
        }
    }
    let name = format!("dpendabort{id}");
    let expression = "N=pending_abort@nohost, H=erts_internal:new_connection(N), \
        ok=erlang:send({pending_receiver,N},discard,[noconnect]), \
        true=erts_internal:abort_pending_connection(N,H), \
        H2=erts_internal:new_connection(N), Flags=element(3,erts_internal:get_dflags()), \
        {ok,H2}=erts_internal:create_dist_channel(N,self(),{Flags,1}), \
        io:format(\"~w~n\",[erlang:dist_ctrl_get_data(H2)]), halt().";
    let args = [
        "-sname",
        &name,
        "-kernel",
        "logger_level",
        "none",
        "-noshell",
        "-eval",
        expression,
    ];
    assert_eq!(stdout(&erts_erl(&epmd, &args)), "none\n");
    assert_eq!(stdout(&pbeam_erl(&epmd, &args)), "none\n");
}

#[test]
fn sends_to_a_name_on_the_first_signal_as_pinned_erts() {
    // dist.c:5713-5759 sends auto_connect to net_kernel for an idle entry.
    let epmd = Epmd::start();
    let id = std::process::id();
    for socket in [false, true] {
        for pbeam in [false, true] {
            let server_name = format!("dfirstserver{socket}{pbeam}{id}");
            let mut server_args = vec!["-sname", server_name.as_str(), "-setcookie", "ck"];
            if socket {
                server_args.extend(SOCKET_CARRIER);
            }
            let server = ErtsNode::start(
                &epmd,
                &server_args,
                "register(pending_receiver,self()), io:format(\"~s~n\",[node()]), \
                 receive {one,From} -> receive two -> io:format(\"~w~n\",[{one,two}]), From ! ack end end, \
                 receive stop -> ok after 5000 -> ok end, halt().",
            );
            let expression = format!(
                "N='{}', noconnect=erlang:send({{pending_receiver,N}},discard,[noconnect]), \
                 {{one,_}}=erlang:send({{pending_receiver,N}},{{one,self()}}), \
                 ok=erlang:send({{pending_receiver,N}},two,[noconnect]), \
                 receive ack -> io:format(\"true~n\") after 5000 -> io:format(\"timeout~n\") end, halt().",
                server.line()
            );
            let client_name = format!("dfirstclient{socket}{pbeam}{id}");
            let mut args = vec![
                "-sname",
                client_name.as_str(),
                "-setcookie",
                "ck",
                "-kernel",
                "logger_level",
                "none",
            ];
            if socket {
                args.extend(SOCKET_CARRIER);
            }
            args.extend(["-noshell", "-eval", expression.as_str()]);
            let output = if pbeam {
                pbeam_erl(&epmd, &args)
            } else {
                erts_erl(&epmd, &args)
            };
            assert_eq!(stdout(&output), "true\n", "socket={socket} pbeam={pbeam}");
            assert_eq!(server.line(), "{one,two}");
        }
    }
    let name = format!("dfirstmissing{id}");
    let expression = "ok=erlang:send({pending_receiver,missing@localhost},lost,[]), \
        false=net_kernel:connect_node(missing@localhost), io:format(\"ok~n\"), halt().";
    let args = [
        "-sname",
        &name,
        "-kernel",
        "logger_level",
        "none",
        "-noshell",
        "-eval",
        expression,
    ];
    assert_eq!(stdout(&erts_erl(&epmd, &args)), "ok\n");
    assert_eq!(stdout(&pbeam_erl(&epmd, &args)), "ok\n");
}

#[test]
fn applies_auto_connect_policy_to_name_sends_as_pinned_erts() {
    // net_kernel.erl:1047-1100 aborts a barred or disabled auto-connect.
    let epmd = Epmd::start();
    let id = std::process::id();
    for policy in ["never", "once"] {
        for pbeam in [false, true] {
            let server_name = format!("dpolicyserver{policy}{pbeam}{id}");
            let server = ErtsNode::start(
                &epmd,
                &["-sname", &server_name, "-setcookie", "ck"],
                "register(pending_receiver,self()), io:format(\"~s~n\",[node()]), \
                 receive {initial,P} -> P ! initial_ack; {final,P} -> P ! final_ack; _ -> halt(3) end, \
                 receive {final,P2} -> P2 ! final_ack; Unexpected -> io:format(standard_error, \"policy server unexpected: ~p~n\", [Unexpected]), halt(4) after 5000 -> io:format(standard_error, \"policy server timed out~n\"), ok end, halt().",
            );
            let first = if policy == "once" {
                "{pending_receiver,N} ! {initial,self()}, \
                 receive initial_ack -> ok after 5000 -> halt(5) end, \
                 case erlang:disconnect_node(N) of true -> ok; Disconnect -> error({disconnect_failed,Disconnect}) end, timer:sleep(200),"
            } else {
                ""
            };
            let expression = format!(
                "N='{}', {first} {{pending_receiver,N}} ! leaked, \
                 R=monitor(process,{{pending_receiver,N}}), \
                 receive {{'DOWN',R,process,_,noconnection}} -> ok after 5000 -> halt(7) end, \
                 B=atom_to_binary(N), P=binary_to_term(<<131,88,118,(byte_size(B)):16,B/binary,1:32,0:32,1:32>>), \
                 process_flag(trap_exit,true), true=link(P), \
                 receive {{'EXIT',P,noconnection}} -> ok after 5000 -> halt(8) end, \
                 Req=spawn_request(N,erlang,node,[],[]), \
                 receive {{spawn_reply,Req,error,noconnection}} -> ok after 5000 -> halt(9) end, \
                 sys:get_state(net_kernel), \
                 false=lists:member(N,nodes()), case net_kernel:connect_node(N) of true -> ok; Connect -> error({{connect_failed,Connect}}) end, \
                 {{pending_receiver,N}} ! {{final,self()}}, \
                 receive final_ack -> io:format(\"ok~n\") after 5000 -> halt(6) end, halt().",
                server.line()
            );
            let client_name = format!("dpolicyclient{policy}{pbeam}{id}");
            let args = [
                "-sname",
                &client_name,
                "-setcookie",
                "ck",
                "-kernel",
                "logger_level",
                "none",
                "dist_auto_connect",
                policy,
                "-noshell",
                "-eval",
                &expression,
            ];
            let output = if pbeam {
                pbeam_erl(&epmd, &args)
            } else {
                erts_erl(&epmd, &args)
            };
            assert_eq!(stdout(&output), "ok\n", "policy={policy} pbeam={pbeam}");
        }
    }
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
fn offers_the_mandatory_distribution_flags_the_monitor_flags_exit_payload_fragments_and_alias() {
    // PBeam offers DFLAG_DIST_MANDATORY (dist.h:80-98), DFLAG_DIST_MONITOR,
    // DFLAG_DIST_MONITOR_NAME, DFLAG_DIST_HDR_ATOM_CACHE, DFLAG_EXIT_PAYLOAD,
    // DFLAG_FRAGMENTS, and DFLAG_ALIAS (dist.h:33, 34, 43, 52, 53, 67, 113-123), with the rejectable and the strict
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
        "{erts_dflags,55897960380,17230663572,55897960380,8396866,8192}\n"
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
    cases.extend([(
        "spawn_scheduler()".to_owned(),
        "unsupported spawn option Tuple([Atom(\"scheduler\"), Integer(1)])",
    )]);
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

/// A TLS probe node with captured output and a bounded wait.
struct TlsProbeNode {
    child: Child,
    first_line: mpsc::Receiver<Vec<u8>>,
    stdout: thread::JoinHandle<Vec<u8>>,
    stderr: thread::JoinHandle<Vec<u8>>,
}

impl TlsProbeNode {
    /// Start a node and drain both output streams while it runs.
    #[expect(
        clippy::fn_params_excessive_bools,
        reason = "the probe selects one of two runtimes"
    )]
    fn start(pbeam: bool, epmd: &Epmd, arguments: &[&str], expression: &str) -> Self {
        let mut command = if pbeam {
            let (root, bindir) = emulator_flags();
            let mut command = Command::new(env!("CARGO_BIN_EXE_pbeam"));
            command
                .args(["erl", "--root", &root, "--bindir"])
                .arg(bindir)
                .arg("--preloaded")
                .arg(pinned_beam_path("erlang").parent().unwrap())
                .args(["--", "-start_epmd", "false"]);
            command
        } else {
            Command::new("erl")
        };
        let mut child = command
            .arg("-noshell")
            .args(arguments)
            .args(["-eval", expression])
            .env("ERL_EPMD_PORT", epmd.port())
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap();
        let stdout = child.stdout.take().unwrap();
        let mut stderr = child.stderr.take().unwrap();
        let (sender, first_line) = mpsc::channel();
        let stdout = thread::spawn(move || {
            let mut reader = BufReader::new(stdout);
            let mut bytes = Vec::new();
            if reader.read_until(b'\n', &mut bytes).is_ok() && !bytes.is_empty() {
                let _ = sender.send(bytes.clone());
            }
            reader.read_to_end(&mut bytes).unwrap();
            bytes
        });
        let stderr = thread::spawn(move || {
            let mut bytes = Vec::new();
            stderr.read_to_end(&mut bytes).unwrap();
            bytes
        });
        Self {
            child,
            first_line,
            stdout,
            stderr,
        }
    }

    /// Wait for exit, or kill the node after the timeout, and collect its output.
    #[expect(
        clippy::fn_params_excessive_bools,
        reason = "a failed peer requires immediate termination"
    )]
    fn finish(mut self, kill: bool) -> (Output, bool) {
        let deadline = Instant::now() + Duration::from_secs(60);
        let mut timed_out = false;
        if kill {
            let _ = self.child.kill();
        }
        let status = loop {
            if let Some(status) = self.child.try_wait().unwrap() {
                break status;
            }
            if Instant::now() >= deadline {
                timed_out = true;
                let _ = self.child.kill();
                break self.child.wait().unwrap();
            }
            thread::sleep(Duration::from_millis(20));
        };
        (
            Output {
                status,
                stdout: self.stdout.join().unwrap(),
                stderr: self.stderr.join().unwrap(),
            },
            timed_out,
        )
    }
}

#[test]
fn connects_over_tls_in_both_directions_as_pinned_erts() {
    let epmd = Epmd::start();
    let tls = std::fs::canonicalize("fixtures/gen_tcp/tls").unwrap();
    let cert = tls.join("ca.pem");
    let key = tls.join("key.pem");
    let node_cert = tls.join("node.pem");
    let options =
        std::env::temp_dir().join(format!("pbeam-dist-tls-{}.config", std::process::id()));
    // Both nodes verify the test CA. Node names use the host name, so the
    // client disables hostname checking for this certificate fixture.
    std::fs::write(
        &options,
        format!(
            "[{{server,[{{certfile,{2:?}}},{{keyfile,{1:?}}},{{cacertfile,{0:?}}},{{verify,verify_peer}},{{fail_if_no_peer_cert,true}}]}},{{client,[{{certfile,{2:?}}},{{keyfile,{1:?}}},{{cacertfile,{0:?}}},{{verify,verify_peer}},{{server_name_indication,disable}}]}}].\n",
            cert.to_str().unwrap(),
            key.to_str().unwrap(),
            node_cert.to_str().unwrap()
        ),
    )
    .unwrap();
    let options_path = options.to_str().unwrap();
    let id = std::process::id();
    let run = |server_pbeam: bool, client_pbeam: bool| {
        let server_name = format!("tlssrv{server_pbeam}{client_pbeam}{id}");
        let server_args = [
            "+S",
            "1",
            "-sname",
            &server_name,
            "-setcookie",
            "ck",
            "-proto_dist",
            "inet_tls",
            "-ssl_dist_optfile",
            options_path,
        ];
        let server_expression = "register(tls_probe, self()), io:format(\"~s~n\", [node()]), receive stop -> io:format(\"stopped~n\"), halt() after 60000 -> halt(2) end.";
        let server = TlsProbeNode::start(server_pbeam, &epmd, &server_args, server_expression);
        let server_line = server.first_line.recv_timeout(Duration::from_secs(60));
        let node = match server_line {
            Ok(line) => String::from_utf8_lossy(&line).trim_end().to_owned(),
            Err(error) => {
                let (output, timed_out) = server.finish(true);
                panic!(
                    "server startup: {error}; timed out: {timed_out}; status: {}; stdout: {:?}; stderr: {:?}; raw output: {output:?}",
                    output.status,
                    String::from_utf8_lossy(&output.stdout),
                    String::from_utf8_lossy(&output.stderr)
                );
            }
        };
        let expression = format!(
            "N = '{node}', R = {{net_adm:ping(N), erpc:call(N, erlang, length, [[a,b,c]])}}, {{tls_probe,N}} ! stop, io:format(\"~w~n\", [R]), halt()."
        );
        let client_name = format!("tlscli{server_pbeam}{client_pbeam}{id}");
        let client_args = [
            "+S",
            "1",
            "-sname",
            &client_name,
            "-setcookie",
            "ck",
            "-proto_dist",
            "inet_tls",
            "-ssl_dist_optfile",
            options_path,
        ];
        let client = TlsProbeNode::start(client_pbeam, &epmd, &client_args, &expression);
        let (client_output, client_timeout) = client.finish(false);
        let client_ok = !client_timeout
            && client_output.status.success()
            && client_output.stdout == b"{pong,3}\n"
            && client_output.stderr.is_empty();
        let (server_output, server_timeout) = server.finish(!client_ok);
        assert!(
            client_ok
                && !server_timeout
                && server_output.status.success()
                && server_output.stdout == format!("{node}\nstopped\n").as_bytes()
                && server_output.stderr.is_empty(),
            "server_pbeam={server_pbeam}, client_pbeam={client_pbeam}; server timed out: {server_timeout}; status: {}; stdout: {:?}; stderr: {:?}; client timed out: {client_timeout}; status: {}; stdout: {:?}; stderr: {:?}; raw server: {server_output:?}; raw client: {client_output:?}",
            server_output.status,
            String::from_utf8_lossy(&server_output.stdout),
            String::from_utf8_lossy(&server_output.stderr),
            client_output.status,
            String::from_utf8_lossy(&client_output.stdout),
            String::from_utf8_lossy(&client_output.stderr)
        );
        String::from_utf8(client_output.stdout)
            .unwrap()
            .trim_end()
            .to_owned()
    };
    let expected = run(false, false);
    assert_eq!(expected, "{pong,3}");
    assert_eq!(run(false, true), expected, "PBeam to ERTS");
    assert_eq!(run(true, false), expected, "ERTS to PBeam");
    std::fs::remove_file(options).unwrap();
}

#[test]
fn accepts_connections_over_both_carriers() {
    // The OTP carrier opens the listen socket and registers its port with
    // epmd (inet_tcp_dist.erl:175-194, inet_epmd_socket.erl:49-86).
    let epmd = Epmd::start();
    let id = std::process::id();
    for socket in [false, true] {
        for pbeam_server in [false, true] {
            let name = format!("inbound{socket}{pbeam_server}{id}");
            let mut arguments = vec![
                "-sname",
                name.as_str(),
                "-setcookie",
                "ck",
                "-kernel",
                "net_ticktime",
                "2",
            ];
            if socket {
                arguments.extend(SOCKET_CARRIER);
            }
            let server_expression = "register(inbound_probe, self()), io:format(\"~s~n\", [node()]), \
             F = fun F() -> receive {From, Peer} -> \
             From ! {net_adm:ping(Peer), lists:member(Peer, nodes(hidden)), \
             rpc:call(Peer, erlang, node, []), erpc:call(Peer, erlang, node, [])}, F() \
             after 30000 -> ok end end, F().";
            let server = if pbeam_server {
                pbeam_node(&epmd, &arguments, server_expression)
            } else {
                ErtsNode::start(&epmd, &arguments, server_expression)
            };
            let node = server.line();
            let expression = format!(
                "spawn(fun() -> timer:sleep(12000), halt(2) end), \
             N = '{node}', true = net_kernel:connect_node(N), \
             timer:sleep(5000), Ping = net_adm:ping(N), \
             {{inbound_probe, N}} ! {{self(), node()}}, \
             receive Reply -> io:format(\"~w~n\", [{{Ping, lists:member(N, nodes(hidden)), \
             rpc:call(N, erlang, node, []), erpc:call(N, erlang, node, []), Reply}}]) end, halt()."
            );
            let client = format!("inclient{socket}{id}");
            let mut client_arguments = vec![
                "-sname",
                client.as_str(),
                "-setcookie",
                "ck",
                "-hidden",
                "-kernel",
                "net_ticktime",
                "2",
            ];
            if socket {
                client_arguments.extend(SOCKET_CARRIER);
            }
            let peer = format!("{}@{}", client, node.split_once('@').unwrap().1);
            let expected = format!(
                "{{pong,true,{0},{0},{{pong,true,{1},{1}}}}}",
                atom_text(&node),
                atom_text(&peer)
            );
            let erts = ErtsNode::start(&epmd, &client_arguments, &expression);
            assert_eq!(erts.line(), expected);
            drop(erts);
            let pbeam = pbeam_node(&epmd, &client_arguments, &expression);
            assert_eq!(pbeam.line(), expected);
        }
    }
}

#[test]
fn registers_the_listen_range_and_honours_dist_listen_false() {
    // inet_tcp_dist.erl:175-207 registers the selected port and reads the
    // configured range. net_kernel owns startup when listening is disabled.
    let epmd = Epmd::start();
    let port = TcpListener::bind("127.0.0.1:0")
        .unwrap()
        .local_addr()
        .unwrap()
        .port()
        .to_string();
    let name = format!("inrange{}", std::process::id());
    let expression = "[Name, Host] = string:split(atom_to_list(node()), \"@\"), \
                      io:format(\"~p~n\", [erl_epmd:port_please(Name, Host)]), halt().";
    for socket in [false, true] {
        for listen in [true, false] {
            let mut arguments = vec![
                "-sname",
                name.as_str(),
                "-kernel",
                "inet_dist_listen_min",
                port.as_str(),
                "-kernel",
                "inet_dist_listen_max",
                port.as_str(),
            ];
            if socket {
                arguments.extend(SOCKET_CARRIER);
            }
            if !listen {
                arguments.extend(["-dist_listen", "false"]);
            }
            arguments.extend(["-noshell", "-eval", expression]);
            let expected = if listen {
                format!("{{port,{port},6}}\n")
            } else {
                "noport\n".to_owned()
            };
            assert_eq!(stdout(&erts_erl(&epmd, &arguments)), expected);
            assert_eq!(stdout(&pbeam_erl(&epmd, &arguments)), expected);
        }
    }
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
         Atoms = [list_to_atom(lists:duplicate(150, 228)) | \
         [list_to_atom(\"cache_atom_\" ++ integer_to_list(N)) || N <- lists:seq(1,300)]], \
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
        "{true,true,301}\n"
    );
    assert_eq!(
        stdout(&pbeam_erl(&epmd, &run(&format!("dacpbeam{id}")))),
        "{true,true,301}\n"
    );
}

#[test]
fn exchanges_large_messages_with_pinned_erts() {
    let epmd = Epmd::start();
    let id = std::process::id();
    let server_name = format!("dfrag{id}");
    let server = ErtsNode::start(
        &epmd,
        &["+S", "4", "-sname", &server_name, "-setcookie", "ck"],
        "register(fragment_echo, self()), io:format(\"~s~n\", [node()]), \
         F = fun F() -> receive {From, Data} -> spawn(fun() -> From ! {echo, Data} end), F() \
         after 30000 -> halt(2) end end, F().",
    );
    let node = server.line();
    let expression = format!(
        "true = net_kernel:connect_node('{node}'), Parent = self(), \
         Data = binary:copy(<<42>>, 131073), \
         Exchange = fun() -> {{fragment_echo, '{node}'}} ! {{self(), Data}}, \
         receive {{echo, Result}} -> Result =:= Data after 10000 -> timeout end end, \
         Single = Exchange(), \
         Atoms = [list_to_atom(lists:duplicate(200, 2048) ++ integer_to_list(N)) || N <- lists:seq(1,160)], \
         Wide = {{Atoms, Data}}, {{fragment_echo, '{node}'}} ! {{self(), Wide}}, \
         WideResult = receive {{echo, Reply}} -> Reply =:= Wide after 10000 -> timeout end, \
         Ps = [spawn(fun() -> receive go -> Parent ! {{done, Exchange()}} end end) || _ <- lists:seq(1,4)], \
         [P ! go || P <- Ps], \
         Results = [receive {{done, R}} -> R after 10000 -> timeout end || _ <- Ps], \
         io:format(\"~p~n\", [{{Single, WideResult, Results}}]), halt()."
    );
    let run = |name: &str| {
        vec![
            "+S".to_owned(),
            "4".to_owned(),
            "-sname".to_owned(),
            name.to_owned(),
            "-setcookie".to_owned(),
            "ck".to_owned(),
            "-noshell".to_owned(),
            "-eval".to_owned(),
            expression.clone(),
        ]
    };
    for (label, output) in [
        ("ERTS", erts_erl(&epmd, &run(&format!("dfrerts{id}")))),
        ("PBeam", pbeam_erl(&epmd, &run(&format!("dfrpbeam{id}")))),
    ] {
        assert_eq!(
            stdout(&output),
            "{true,true,[true,true,true,true]}\n",
            "{label}: {output:?}"
        );
    }
}

#[test]
fn kills_the_fragment_controller_on_sequence_errors_as_pinned_erts() {
    let epmd = Epmd::start();
    let id = std::process::id();
    let expression = "F = fun(N, Case) -> Parent=self(), \
        {Pid,Ref}=spawn_monitor(fun() -> \
        Node=list_to_atom(\"frag_peer_\"++integer_to_list(N)++\"@nohost\"), \
        erts_internal:new_connection(Node), Flags=element(3,erts_internal:get_dflags()) bor 16#800000, \
        {ok,H}=erts_internal:create_dist_channel(Node,self(),{Flags,1}), \
        <<131,C/binary>>=term_to_binary({2,empty,self()}), \
        First= <<131,69,3:64,3:64,0,C/binary>>, Last= <<131,70,3:64,1:64>>, \
        case Case of missing -> erlang:dist_ctrl_put_data(H,Last); \
        duplicate -> erlang:dist_ctrl_put_data(H,First), erlang:dist_ctrl_put_data(H,First); \
        wrong -> erlang:dist_ctrl_put_data(H,First), erlang:dist_ctrl_put_data(H,Last); \
        split -> erlang:dist_ctrl_put_data(H,<<131,69,3:64,2:64,0,97>>); \
        short -> erlang:dist_ctrl_put_data(H,<<131,70>>); \
        version -> erlang:dist_ctrl_put_data(H,<<130,70,3:64,1:64>>) end, \
        Parent ! survived, receive after 1000 -> ok end end), \
        receive {'DOWN',Ref,process,Pid,R} -> {Case,R,nodes(connected)} after 5000 -> timeout end end, \
        io:format(\"~w~n\",[[F(1,missing),F(2,duplicate),F(3,wrong),F(4,split),F(5,short),F(6,version)]]), halt().";
    let run = |name: &str| {
        vec![
            "-sname".to_owned(),
            name.to_owned(),
            "-kernel".to_owned(),
            "logger_level".to_owned(),
            "none".to_owned(),
            "-noshell".to_owned(),
            "-eval".to_owned(),
            expression.to_owned(),
        ]
    };
    for (label, output) in [
        ("ERTS", erts_erl(&epmd, &run(&format!("fser{id}")))),
        ("PBeam", pbeam_erl(&epmd, &run(&format!("fspb{id}")))),
    ] {
        assert_eq!(
            stdout(&output),
            "[{missing,killed,[]},{duplicate,killed,[]},{wrong,killed,[]},{split,killed,[]},{short,killed,[]},{version,killed,[]}]\n",
            "{label}: {output:?}"
        );
    }
}

#[test]
fn reassembles_interleaved_fragments_with_their_own_atom_tables_as_pinned_erts() {
    let epmd = Epmd::start();
    let id = std::process::id();
    let erts_name = format!("dfintererts{id}");
    let pbeam_name = format!("dfinterpb{id}");
    let expression = "dist_channel_probe:fragment_interleaving(), halt().";
    let erts = probe(
        &["-sname", &erts_name, "-kernel", "logger_level", "none"],
        expression,
    );
    let pbeam = probe(
        &["-sname", &pbeam_name, "-kernel", "logger_level", "none"],
        expression,
    );
    let expected = "fragment_interleaving: [[alpha,beta],[beta,alpha]]\n";
    assert_eq!(stdout(&erts_erl(&epmd, &erts)), expected);
    assert_eq!(stdout(&pbeam_erl(&epmd, &pbeam)), expected);
}

#[test]
fn clears_fragment_sequences_when_a_connection_is_replaced_as_pinned_erts() {
    let epmd = Epmd::start();
    let id = std::process::id();
    let erts_name = format!("dfreconnecterts{id}");
    let pbeam_name = format!("dfreconnectpb{id}");
    let expression = "dist_channel_probe:fragment_reconnect(), halt().";
    let erts = probe(
        &["-sname", &erts_name, "-kernel", "logger_level", "none"],
        expression,
    );
    let pbeam = probe(
        &["-sname", &pbeam_name, "-kernel", "logger_level", "none"],
        expression,
    );
    let expected = "fragment_reconnect: [killed,noconnection,[],42]\n";
    assert_eq!(stdout(&erts_erl(&epmd, &erts)), expected);
    assert_eq!(stdout(&pbeam_erl(&epmd, &pbeam)), expected);
}

#[test]
fn writes_fragment_headers_and_a_large_atom_cache_as_pinned_erts() {
    let epmd = Epmd::start();
    let id = std::process::id();
    let erts_name = format!("dfoutputerts{id}");
    let pbeam_name = format!("dfoutputpb{id}");
    let expression = "dist_channel_probe:fragment_output(), halt().";
    let erts = probe(
        &["-sname", &erts_name, "-kernel", "logger_level", "none"],
        expression,
    );
    let pbeam = probe(
        &["-sname", &pbeam_name, "-kernel", "logger_level", "none"],
        expression,
    );
    let expected = "fragment_output: [3,none,true,none]\n";
    assert_eq!(stdout(&erts_erl(&epmd, &erts)), expected);
    assert_eq!(stdout(&pbeam_erl(&epmd, &pbeam)), expected);
}

#[test]
fn keeps_a_large_control_term_whole_until_1562_as_the_row_records() {
    // Pinned ERTS sends an E fragment with id 2. PBeam keeps the D packet
    // whole. Issue #1562 adds support for a split control term.
    let epmd = Epmd::start();
    let id = std::process::id();
    let expression = "dist_channel_probe:fragment_large_control(), halt().";
    let erts_name = format!("dflargeerts{id}");
    let pbeam_name = format!("dflargepb{id}");
    let erts = probe(&["-sname", &erts_name], expression);
    let pbeam = probe(&["-sname", &pbeam_name], expression);
    assert_eq!(
        stdout(&erts_erl(&epmd, &erts)),
        "fragment_large_control: [69,false]\n"
    );
    assert_eq!(
        stdout(&pbeam_erl(&epmd, &pbeam)),
        "fragment_large_control: [68,true]\n"
    );
}

#[test]
fn closes_a_port_controller_for_a_fragment_with_an_unknown_sequence() {
    // The accepted tcp_inet port receives the packet under its port lock.
    // A distribution payload that does not follow the documented layout
    // makes ERTS close the controller with killed
    // (dist.c:2147-2150, 3161-3165, 4015-4018).
    let epmd = Epmd::start();
    let id = std::process::id();
    let expression = "process_flag(trap_exit,true), \
        {ok,L}=gen_tcp:listen(0,[binary,{active,false}]), \
        {ok,Port}=inet:port(L), \
        {ok,C}=gen_tcp:connect({127,0,0,1},Port,[binary,{active,false},{packet,4}]), \
        {ok,A}=gen_tcp:accept(L), Ref=erlang:monitor(port,A), \
        Node=fragment_port_peer@nohost, erts_internal:new_connection(Node), \
        Flags=element(3,erts_internal:get_dflags()) bor 16#800000, \
        {ok,_}=erts_internal:create_dist_channel(Node,A,{Flags,1}), \
        ok=inet:setopts(A,[{active,true},{deliver,port},{packet,4}]), \
        ok=gen_tcp:send(C,<<131,70,3:64,1:64>>), \
        Reason=receive {'DOWN',Ref,port,A,R} -> R after 5000 -> timeout end, \
        io:format(\"~p~n\",[{Reason,erlang:port_info(A),nodes(connected),gen_tcp:recv(C,0,5000)}]), halt().";
    let run = |name: &str| {
        vec![
            "-sname".to_owned(),
            name.to_owned(),
            "-kernel".to_owned(),
            "logger_level".to_owned(),
            "none".to_owned(),
            "-noshell".to_owned(),
            "-eval".to_owned(),
            expression.to_owned(),
        ]
    };
    let expected = "{killed,undefined,[],{error,closed}}\n";
    assert_eq!(
        stdout(&erts_erl(&epmd, &run(&format!("dfporterts{id}")))),
        expected
    );
    assert_eq!(
        stdout(&pbeam_erl(&epmd, &run(&format!("dfportpb{id}")))),
        expected
    );
}

#[test]
fn checks_fragment_layout_from_a_process_input_handler_for_a_port_controller() {
    // A fragment sequence that does not follow the documented layout closes
    // the port controller when its input handler is a process.
    let epmd = Epmd::start();
    let id = std::process::id();
    let expression = "process_flag(trap_exit,true), \
        {ok,L}=gen_tcp:listen(0,[binary,{active,false}]), \
        {ok,Port}=inet:port(L), \
        {ok,C}=gen_tcp:connect({127,0,0,1},Port,[binary,{active,false},{packet,4}]), \
        {ok,A}=gen_tcp:accept(L), Ref=erlang:monitor(port,A), \
        Node=fragment_handler_peer@nohost, erts_internal:new_connection(Node), \
        Flags=element(3,erts_internal:get_dflags()) bor 16#800000, \
        {ok,Handle}=erts_internal:create_dist_channel(Node,A,{Flags,1}), \
        ok=erlang:dist_ctrl_input_handler(Handle,self()), \
        ok=erlang:dist_ctrl_put_data(Handle,<<131,70,3:64,1:64>>), \
        Reason=receive {'DOWN',Ref,port,A,R} -> R after 5000 -> timeout end, \
        io:format(\"~p~n\",[{Reason,erlang:port_info(A),nodes(connected),gen_tcp:recv(C,0,5000)}]), halt().";
    let erts_name = format!("dfhandlererts{id}");
    let pbeam_name = format!("dfhandlerpb{id}");
    let run = |name: &str| {
        vec![
            "-sname".to_owned(),
            name.to_owned(),
            "-kernel".to_owned(),
            "logger_level".to_owned(),
            "none".to_owned(),
            "-noshell".to_owned(),
            "-eval".to_owned(),
            expression.to_owned(),
        ]
    };
    let expected = "{killed,undefined,[],{error,closed}}\n";
    assert_eq!(stdout(&erts_erl(&epmd, &run(&erts_name))), expected);
    assert_eq!(stdout(&pbeam_erl(&epmd, &run(&pbeam_name))), expected);
}

#[test]
fn checks_fragment_layout_when_a_port_is_activated() {
    // The first packet makes active-once passive. The second packet is
    // delivered by the control that activates the port again.
    let epmd = Epmd::start();
    let id = std::process::id();
    let expression = "process_flag(trap_exit,true), \
        {ok,L}=gen_tcp:listen(0,[binary,{active,false}]), \
        {ok,Port}=inet:port(L), \
        {ok,C}=gen_tcp:connect({127,0,0,1},Port,[binary,{active,false}]), \
        {ok,A}=gen_tcp:accept(L), Ref=erlang:monitor(port,A), \
        Node=activation_fragment_peer@nohost, erts_internal:new_connection(Node), \
        Flags=element(3,erts_internal:get_dflags()) bor 16#800000, \
        {ok,_}=erts_internal:create_dist_channel(Node,A,{Flags,1}), \
        ok=inet:setopts(A,[{active,once},{deliver,port},{packet,4}]), \
        <<131,Control/binary>>=term_to_binary({2,empty,self()}), \
        <<131,Message/binary>>=term_to_binary(ready), \
        First= <<131,68,0,Control/binary,Message/binary>>, \
        Second= <<131,70,3:64,1:64>>, \
        ok=gen_tcp:send(C,<<(byte_size(First)):32,First/binary,(byte_size(Second)):32,Second/binary>>), \
        receive ready -> ok after 5000 -> exit(first_timeout) end, \
        ok=inet:setopts(A,[{active,true}]), \
        Reason=receive {'DOWN',Ref,port,A,R} -> R after 5000 -> timeout end, \
        io:format(\"~p~n\",[{Reason,erlang:port_info(A),nodes(connected),gen_tcp:recv(C,0,5000)}]), halt().";
    let erts_name = format!("dfactiveerts{id}");
    let pbeam_name = format!("dfactivepb{id}");
    let erts = ErtsNode::start(
        &epmd,
        &["-sname", &erts_name, "-kernel", "logger_level", "none"],
        expression,
    );
    assert_eq!(erts.line(), "{killed,undefined,[],{error,closed}}");
    let pbeam = pbeam_node(
        &epmd,
        &["-sname", &pbeam_name, "-kernel", "logger_level", "none"],
        expression,
    );
    assert_eq!(pbeam.line(), "{killed,undefined,[],{error,closed}}");
}

// Pinned OTP 29.1.1 exits 139 for sequence 0, fragment 2 with complete
// control and message. PBeam follows the non-fragmented source path instead.
#[test]
fn treats_zero_sequence_as_a_nonfragmented_message() {
    let epmd = Epmd::start();
    let id = std::process::id();
    let run = |name: &str, sequence: u8, fragment: u8| {
        let expression = format!(
            "Node=zero_fragment_peer@nohost, erts_internal:new_connection(Node), \
            Flags=element(3,erts_internal:get_dflags()) bor 16#800000, \
            {{ok,H}}=erts_internal:create_dist_channel(Node,self(),{{Flags,1}}), \
            <<131,C/binary>>=term_to_binary({{2,empty,self()}}), \
            <<131,M/binary>>=term_to_binary(ok), \
            erlang:dist_ctrl_put_data(H,<<131,69,{sequence}:64,{fragment}:64,0,C/binary,M/binary>>), \
            receive ok -> io:format(\"delivered~n\") after 5000 -> io:format(\"timeout~n\") end, halt()."
        );
        vec![
            "-sname".to_owned(),
            name.to_owned(),
            "-kernel".to_owned(),
            "logger_level".to_owned(),
            "none".to_owned(),
            "-noshell".to_owned(),
            "-eval".to_owned(),
            expression,
        ]
    };
    assert_eq!(
        stdout(&erts_erl(&epmd, &run(&format!("fzert{id}"), 0, 1))),
        "delivered\n"
    );
    for fragment in [1, 2] {
        assert_eq!(
            stdout(&pbeam_erl(
                &epmd,
                &run(&format!("fzpb{fragment}{id}"), 0, fragment)
            )),
            "delivered\n"
        );
    }
    assert_eq!(
        stdout(&erts_erl(&epmd, &run(&format!("fsert{id}"), 3, 1))),
        "delivered\n"
    );
    assert_eq!(
        stdout(&pbeam_erl(&epmd, &run(&format!("fspbeam{id}"), 3, 1))),
        "delivered\n"
    );
}

// OTP 29.1.1 exits 139 for a complete first fragment with id 0;
// dist.c:2131-2163 decrements its unsigned fragment count below zero.
#[test]
fn closes_the_controller_for_zero_fragment_id() {
    let epmd = Epmd::start();
    let id = std::process::id();
    let expression = "{Pid,Ref}=spawn_monitor(fun() -> \
        Node=zero_id_peer@nohost, erts_internal:new_connection(Node), \
        Flags=element(3,erts_internal:get_dflags()) bor 16#800000, \
        {ok,H}=erts_internal:create_dist_channel(Node,self(),{Flags,1}), \
        <<131,C/binary>>=term_to_binary({2,empty,self()}), \
        <<131,M/binary>>=term_to_binary(ok), \
        erlang:dist_ctrl_put_data(H,<<131,69,3:64,0:64,0,C/binary,M/binary>>), \
        receive after 5000 -> ok end end), \
        receive {'DOWN',Ref,process,Pid,Reason} -> io:format(\"~p~n\",[{Reason,nodes(connected)}]) \
        after 5000 -> io:format(\"timeout~n\") end, halt().";
    let args = vec![
        "-sname".to_owned(),
        format!("fzid{id}"),
        "-kernel".to_owned(),
        "logger_level".to_owned(),
        "none".to_owned(),
        "-noshell".to_owned(),
        "-eval".to_owned(),
        expression.to_owned(),
    ];
    assert_eq!(stdout(&pbeam_erl(&epmd, &args)), "{killed,[]}\n");
}

#[test]
fn accepts_a_large_control_term_in_the_first_fragment_as_pinned_erts() {
    let epmd = Epmd::start();
    let id = std::process::id();
    let expression = "register(fragment_control_target, self()), \
        Node = fragment_control_peer@nohost, erts_internal:new_connection(Node), \
        Flags = element(3, erts_internal:get_dflags()) bor 16#800000, \
        {ok, Handle} = erts_internal:create_dist_channel(Node, self(), {Flags, 1}), \
        <<131, Control/binary>> = term_to_binary({6, self(), binary:copy(<<42>>, 70000), fragment_control_target}), \
        <<131, Message/binary>> = term_to_binary(ok), \
        erlang:dist_ctrl_put_data(Handle, <<131,69,3:64,2:64,0,Control/binary>>), \
        erlang:dist_ctrl_put_data(Handle, <<131,70,3:64,1:64,Message/binary>>), \
        receive ok -> io:format(\"ok~n\") after 5000 -> io:format(\"timeout~n\") end, halt().";
    let run = |name: &str| {
        vec![
            "-sname".to_owned(),
            name.to_owned(),
            "-kernel".to_owned(),
            "logger_level".to_owned(),
            "none".to_owned(),
            "-noshell".to_owned(),
            "-eval".to_owned(),
            expression.to_owned(),
        ]
    };
    for (label, output) in [
        ("ERTS", erts_erl(&epmd, &run(&format!("dfcerts{id}")))),
        ("PBeam", pbeam_erl(&epmd, &run(&format!("dfcpbeam{id}")))),
    ] {
        assert_eq!(stdout(&output), "ok\n", "{label}: {output:?}");
    }
}

#[test]
fn preserves_cache_order_for_concurrent_port_senders() {
    // The 64 processes interleave sends. Their scheduler assignment is not
    // deterministic, so the scheduler count is diagnostic only.
    let epmd = Epmd::start();
    let id = std::process::id();
    let name = format!("dcorder{id}");
    let server = ErtsNode::start(
        &epmd,
        &["+S", "4", "-sname", &name, "-setcookie", "ck"],
        "register(cache_order, self()), io:format(\"~s~n\", [node()]), \
         Expected = lists:sort(lists:append([[list_to_atom(\"cache_parallel_\" ++ integer_to_list(N)), \
         list_to_atom(\"cache_parallel_\" ++ integer_to_list(N))] || N <- lists:seq(1,6)])), \
         F = fun F(Atoms) -> receive {From, atom_value, Atom} -> From ! ack, F([Atom|Atoms]); \
         {From, finish} -> From ! {result, {length(Atoms),lists:sort(Atoms) =:= Expected}}, F([]) \
         after 120000 -> halt(2) end end, F([]).",
    );
    let node = server.line();
    let expression = format!(
        "true = net_kernel:connect_node('{node}'), Parent = self(), \
         Atoms = [list_to_atom(\"cache_parallel_\" ++ integer_to_list(N)) || N <- lists:seq(1,6)], \
         Worker = fun(A) -> receive go -> Sid = erlang:system_info(scheduler_id), \
         {{cache_order, '{node}'}} ! {{self(), atom_value, A}}, \
         receive ack -> Parent ! {{done, Sid}} after 120000 -> Parent ! timeout end end end, \
         Ps = lists:append([[spawn(fun() -> Worker(A) end),spawn(fun() -> Worker(A) end)] || A <- Atoms]), \
         [P ! go || P <- Ps], \
         Sids = [receive {{done, Sid}} -> Sid after 120000 -> timeout end || _ <- Ps], \
         {{cache_order, '{node}'}} ! {{self(), finish}}, \
         receive {{result, Good}} -> io:format(\"~p~n~p~n\", [Good,length(lists:usort([S || S <- Sids, is_integer(S)]))]) \
         after 120000 -> halt(2) end, halt()."
    );
    let run = |name: &str| {
        vec![
            "+S".to_owned(),
            "4".to_owned(),
            "-sname".to_owned(),
            name.to_owned(),
            "-setcookie".to_owned(),
            "ck".to_owned(),
            "-noshell".to_owned(),
            "-eval".to_owned(),
            expression.clone(),
        ]
    };
    for (label, node, output) in [
        ("ERTS", format!("dcoerts{id}"), None),
        ("PBeam", format!("dcopbeam{id}"), None),
    ] {
        let arguments = run(&node);
        let output = match output {
            Some(output) => output,
            None if label == "ERTS" => erts_erl(&epmd, &arguments),
            None => pbeam_erl(&epmd, &arguments),
        };
        // Name the arguments in the failure, because a node that another test
        // disturbs reports its own error, which does not name this test.
        assert!(
            output.status.success() && output.stderr.is_empty(),
            "{label} {node}: {output:?}"
        );
        let text = String::from_utf8(output.stdout).unwrap();
        let (result, schedulers) = text.split_once('\n').unwrap();
        assert_eq!(result, "{12,true}", "{label}: {text}");
        println!("{label} scheduler count: {}", schedulers.trim());
    }
}

/// Run a cached distribution encode whose control or payload contains an
/// alias whose owner has exited. ERTS keeps the owner in the alias, while
/// PBeam reports the documented external-term difference.
fn cache_encoder_rejects_dead_alias(operation: &str) {
    let epmd = Epmd::start();
    let id = std::process::id();
    let expression = format!(
        "Parent = self(), {{Owner,Monitor}} = spawn_monitor(fun() -> Parent ! {{self(),alias()}} end), \
         receive {{'DOWN',Monitor,process,Owner,_}} -> ok end, \
         Alias = receive {{Owner,A}} -> A end, \
         Node = cache_encode_peer@nohost, erts_internal:new_connection(Node), \
         Flags = element(3,erts_internal:get_dflags()) bor 16#2000, \
         {{ok,_}} = erts_internal:create_dist_channel(Node,self(),{{Flags,1}}), \
         Name = atom_to_binary(Node), \
         Pid = binary_to_term(<<131,88,118,(byte_size(Name)):16,Name/binary,1:32,0:32,1:32>>), \
         {operation}, io:format(\"ok~n\"), halt()."
    );
    let run = |name: &str| {
        vec![
            "-sname".to_owned(),
            name.to_owned(),
            "-kernel".to_owned(),
            "logger_level".to_owned(),
            "none".to_owned(),
            "-noshell".to_owned(),
            "-eval".to_owned(),
            expression.clone(),
        ]
    };
    assert_eq!(
        stdout(&erts_erl(&epmd, &run(&format!("dcaerts{id}")))),
        "ok\n"
    );
    let output = pbeam_erl(&epmd, &run(&format!("dcapbeam{id}")));
    assert_eq!(output.status.code(), Some(1), "{output:?}");
    assert_eq!(
        String::from_utf8_lossy(&output.stderr),
        "pbeam: unsupported external term of a pid reference whose owner exited: ERTS writes the pid of the owner in the reference, and PBeam does not keep it: see \"External term format\" in docs/erts-differences.md\n",
        "{output:?}"
    );
}

#[test]
fn reports_a_dead_alias_in_a_cached_control_term() {
    cache_encoder_rejects_dead_alias("erlang:exit(Pid, Alias)");
}

#[test]
fn reports_a_dead_alias_in_a_cached_message() {
    cache_encoder_rejects_dead_alias("Pid ! Alias");
}

#[test]
fn accepts_an_empty_distribution_atom_cache_header_as_pinned_erts() {
    let epmd = Epmd::start();
    let id = std::process::id();
    let expression = "erts_internal:new_connection(empty_cache_peer@nohost), \
        Flags = element(3, erts_internal:get_dflags()) bor 16#2000, \
        {ok, Handle} = erts_internal:create_dist_channel(empty_cache_peer@nohost, self(), {Flags,1}), \
        <<131,Control/binary>> = term_to_binary({2,'',self()}), \
        <<131,Message/binary>> = term_to_binary(123), \
        ok = erlang:dist_ctrl_put_data(Handle, <<131,68,0,Control/binary,Message/binary>>), \
        receive 123 -> io:format(\"ok~n\") after 5000 -> halt(2) end, halt().";
    let run = |name: &str| {
        vec![
            "-sname".to_owned(),
            name.to_owned(),
            "-kernel".to_owned(),
            "logger_level".to_owned(),
            "none".to_owned(),
            "-noshell".to_owned(),
            "-eval".to_owned(),
            expression.to_owned(),
        ]
    };
    assert_eq!(
        stdout(&erts_erl(&epmd, &run(&format!("dceerts{id}")))),
        "ok\n"
    );
    assert_eq!(
        stdout(&pbeam_erl(&epmd, &run(&format!("dcepbeam{id}")))),
        "ok\n"
    );
}

#[test]
fn drops_a_cache_header_on_a_stale_distribution_handle() {
    let epmd = Epmd::start();
    let id = std::process::id();
    let expression = "Old = erts_internal:new_connection(stale_cache_peer@nohost), \
        erts_internal:abort_pending_connection(stale_cache_peer@nohost, Old), \
        New = erts_internal:new_connection(stale_cache_peer@nohost), \
        true = Old =/= New, \
        Flags = element(3, erts_internal:get_dflags()) bor 16#2000, \
        {ok, _} = erts_internal:create_dist_channel(stale_cache_peer@nohost, self(), {Flags,1}), \
        <<131,Control/binary>> = term_to_binary({2,'',self()}), \
        <<131,Message/binary>> = term_to_binary(123), \
        ok = erlang:dist_ctrl_put_data(Old, <<131,68,1,8,0,1,97,Control/binary,Message/binary>>), \
        ok = erlang:dist_ctrl_put_data(Old, <<131,68,0,Control/binary,Message/binary>>), \
        ok = erlang:dist_ctrl_put_data(Old, <<131,68>>), \
        receive 123 -> io:format(\"ok~n\") after 2000 -> io:format(\"no_message~n\") end, halt().";
    let run = |name: &str| {
        vec![
            "-sname".to_owned(),
            name.to_owned(),
            "-kernel".to_owned(),
            "logger_level".to_owned(),
            "none".to_owned(),
            "-noshell".to_owned(),
            "-eval".to_owned(),
            expression.to_owned(),
        ]
    };
    let erts = erts_erl(&epmd, &run(&format!("dsterts{id}")));
    assert_eq!(stdout(&erts), "no_message\n");
    let pbeam = pbeam_erl(&epmd, &run(&format!("dstpbeam{id}")));
    assert_eq!(stdout(&pbeam), "no_message\n");
}

// A compiled fixture cannot make the ERTS encoder emit these malformed
// packets. These direct controller tests act as unit probes for the PBeam
// decoder (external.c:922-977, dist.c:3144-3165). The raw proxy probe below
// checks the same bytes against a real ERTS peer. PBeam reports a runtime
// error (row "Distribution").
fn assert_rejected_distribution_cache_header(packet: &str) {
    let epmd = Epmd::start();
    let name = format!("dcbad{}", std::process::id());
    let expression = format!(
        "erts_internal:new_connection(cache_reject_peer@nohost), \
         Flags = element(3, erts_internal:get_dflags()) bor 16#2000, \
         {{ok, Handle}} = erts_internal:create_dist_channel(cache_reject_peer@nohost, self(), {{Flags,1}}), \
         erlang:dist_ctrl_put_data(Handle, {packet}), halt()."
    );
    let output = pbeam_erl(
        &epmd,
        &[
            "-sname",
            &name,
            "-kernel",
            "logger_level",
            "none",
            "-noshell",
            "-eval",
            &expression,
        ],
    );
    assert_eq!(output.status.code(), Some(1), "{output:?}");
    assert_eq!(
        String::from_utf8_lossy(&output.stderr),
        "pbeam: unsupported distribution message that ERTS rejects: ERTS ends the connection: see \"Distribution\" in docs/erts-differences.md\n",
        "{output:?}"
    );
}

#[test]
#[expect(
    clippy::too_many_lines,
    reason = "the probe owns the peer, proxy, and each malformed packet"
)]
fn pinned_erts_closes_connections_for_malformed_cache_packets() {
    // A TCP proxy forwards the real handshake, then injects a framed packet
    // from the PBeam side. ERTS rejects the bytes and ends the connection
    // (external.c:922-977, dist.c:3144-3165).
    let epmd = Epmd::start();
    let id = std::process::id();
    let listen = TcpListener::bind("127.0.0.1:0")
        .unwrap()
        .local_addr()
        .unwrap()
        .port();
    let server_name = format!("dcreject{id}");
    let listen_text = listen.to_string();
    let server = ErtsNode::start(
        &epmd,
        &[
            "-sname",
            &server_name,
            "-setcookie",
            "ck",
            "-kernel",
            "logger_level",
            "none",
            "-kernel",
            "inet_dist_listen_min",
            &listen_text,
            "-kernel",
            "inet_dist_listen_max",
            &listen_text,
        ],
        "ok = net_kernel:monitor_nodes(true), io:format(\"~s~n\", [node()]), \
         F = fun F() -> receive {nodeup,_} -> io:format(\"up~n\"), F(); \
         {nodedown,_} -> io:format(\"down~n\"), F() after 60000 -> halt(2) end end, F().",
    );
    let node = server.line();
    let mut long_name = vec![131, 68, 1, 24, 0, 1, 0];
    long_name.extend([97; 256]);
    let mut bad_node = vec![131, 68, 0, 104, 3, 97, 2, 97, 0, 88, 82, 0];
    bad_node.extend([0; 12]);
    let packets = [
        vec![131, 68],
        vec![131, 68, 0, 82, 0],
        bad_node,
        vec![131, 88, 0],
        vec![131, 68, 1],
        vec![131, 68, 1, 0],
        vec![131, 68, 1, 0, 0],
        vec![131, 68, 1, 8, 0],
        vec![131, 68, 1, 24, 0, 0],
        vec![131, 68, 1, 8, 0, 2, 97],
        vec![131, 68, 1, 8, 0, 1, 255],
        long_name,
    ];
    for (index, packet) in packets.iter().enumerate() {
        let proxy = TcpListener::bind("0.0.0.0:0").unwrap();
        let proxy_port = proxy.local_addr().unwrap().port().to_string();
        let (sender, receiver) = mpsc::channel();
        let forward = thread::spawn(move || {
            let (client, _) = proxy.accept().unwrap();
            let peer = TcpStream::connect(("127.0.0.1", listen)).unwrap();
            let writer = Arc::new(Mutex::new(peer.try_clone().unwrap()));
            sender.send(Arc::clone(&writer)).unwrap();
            let mut from_client = client.try_clone().unwrap();
            let send_to_peer = thread::spawn(move || {
                let mut bytes = [0; 8192];
                while let Ok(count @ 1..) = from_client.read(&mut bytes) {
                    if writer.lock().unwrap().write_all(&bytes[..count]).is_err() {
                        break;
                    }
                }
            });
            let mut from_peer = peer;
            let mut to_client = client;
            let _ = std::io::copy(&mut from_peer, &mut to_client);
            let _ = send_to_peer.join();
        });
        let client_name = format!("dcbad{id}x{index}");
        let client = pbeam_node(
            &epmd,
            &[
                "-sname",
                &client_name,
                "-setcookie",
                "ck",
                "-pa",
                "fixtures/dist",
                "-epmd_module",
                "dist_epmd",
                "-dist_epmd_port",
                &proxy_port,
            ],
            &format!(
                "io:format(\"~p~n\", [net_kernel:connect_node('{node}')]), receive after 20000 -> halt(2) end."
            ),
        );
        let writer = receiver.recv_timeout(Duration::from_secs(20)).unwrap();
        assert_eq!(client.line(), "true", "packet {index}");
        assert_eq!(server.line(), "up", "packet {index}");
        let mut framed = u32::try_from(packet.len()).unwrap().to_be_bytes().to_vec();
        framed.extend(packet);
        writer.lock().unwrap().write_all(&framed).unwrap();
        assert_eq!(server.line(), "down", "packet {index}");
        drop(client);
        forward.join().unwrap();
    }
}

macro_rules! rejected_distribution_cache_header {
    ($name:ident, $packet:expr) => {
        #[test]
        fn $name() {
            assert_rejected_distribution_cache_header($packet);
        }
    };
}

rejected_distribution_cache_header!(rejects_distribution_cache_short_header, "<<131,68>>");
rejected_distribution_cache_header!(
    rejects_distribution_cache_unknown_term_reference,
    "<<131,68,0,82,0>>"
);
rejected_distribution_cache_header!(
    rejects_distribution_cache_unknown_node_reference,
    "<<131,68,0,104,3,97,2,97,0,88,82,0,0:96>>"
);
rejected_distribution_cache_header!(rejects_distribution_cache_bad_header, "<<131,88,0>>");
rejected_distribution_cache_header!(rejects_distribution_cache_missing_flags, "<<131,68,1>>");
rejected_distribution_cache_header!(rejects_distribution_cache_missing_index, "<<131,68,1,0>>");
rejected_distribution_cache_header!(rejects_distribution_cache_unknown_index, "<<131,68,1,0,0>>");
rejected_distribution_cache_header!(
    rejects_distribution_cache_missing_short_length,
    "<<131,68,1,8,0>>"
);
rejected_distribution_cache_header!(
    rejects_distribution_cache_missing_long_length,
    "<<131,68,1,24,0,0>>"
);
rejected_distribution_cache_header!(
    rejects_distribution_cache_truncated_name,
    "<<131,68,1,8,0,2,97>>"
);
rejected_distribution_cache_header!(
    rejects_distribution_cache_invalid_utf8,
    "<<131,68,1,8,0,1,255>>"
);
rejected_distribution_cache_header!(
    rejects_distribution_cache_oversized_name,
    "<<131,68,1,24,0,1,0,(binary:copy(<<97>>,256))/binary>>"
);

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
        erts.ends_with("{no_connection,none},{no_receiver,none},{monitor_priority,down},{monitor_alias,[{explicit_unalias,received,true},{demonitor,received,true},{reply_demonitor,received,false}]},{link_priority,exit_signal},{remote_leader,{true,true,badarg,true,true}}]\n"),
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
        (
            "spawn_pending_alias",
            "spawn_pending_alias: {dropped,ok,delivered,false}\n",
        ),
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
    assert!(
        erts.chars()
            .filter(|c| !c.is_whitespace())
            .collect::<String>()
            .contains("received_states:[{1,ordinary},{1,ordinary},{1,ordinary},{1,ordinary}]"),
        "{erts}"
    );
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
    // call and answers `pong` (row "Distribution").
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
            "[{unconnected,pong}]\n",
            "server pbeam {server_pbeam}, socket {socket}"
        );
    }
}

#[test]
fn fails_a_pending_name_monitor_as_pinned_erts() {
    // bif.c:838-858 and dist.c:5627-5674: abort detaches the monitor.
    let epmd = Epmd::start();
    let name = format!("dmonmissing{}", std::process::id());
    let expression = "N=missing@localhost, R=monitor(process,{missing_name,N}), \
        receive {'DOWN',R,process,{missing_name,N},Reason} -> \
        io:format(\"~w~n\",[{Reason,net_adm:ping(N)}]) \
        after 5000 -> halt(2) end, halt().";
    let args = [
        "-sname",
        &name,
        "-kernel",
        "logger_level",
        "none",
        "-noshell",
        "-eval",
        expression,
    ];
    assert_eq!(stdout(&erts_erl(&epmd, &args)), "{noconnection,pang}\n");
    assert_eq!(stdout(&pbeam_erl(&epmd, &args)), "{noconnection,pang}\n");
    let expression = format!(
        "{{ok,_}}=net_kernel:start(dmonshutdown{}, #{{name_domain=>shortnames}}), \
         N=monitor_pending@nohost, erts_internal:new_connection(N), \
         R=monitor(process,{{missing_name,N}}), ok=net_kernel:stop(), \
         receive {{'DOWN',R,process,{{missing_name,N}},Reason}} -> io:format(\"~w~n\",[Reason]) \
         after 5000 -> halt(2) end, halt().",
        std::process::id()
    );
    let args = [
        "-kernel",
        "logger_level",
        "none",
        "-noshell",
        "-eval",
        &expression,
    ];
    assert_eq!(stdout(&erts_erl(&epmd, &args)), "noconnection\n");
    assert_eq!(stdout(&pbeam_erl(&epmd, &args)), "noconnection\n");
    let expression = "N=monitor_flags@nohost, erts_internal:new_connection(N), \
        R=monitor(process,{missing_name,N}), \
        Flags=element(3,erts_internal:get_dflags()) band bnot 16#28, \
        {ok,H}=erts_internal:create_dist_channel(N,self(),{Flags,1}), \
        D=erlang:dist_ctrl_get_data(H), io:format(\"~w~n\",[{D=:=[],is_reference(R)}]), halt().";
    let args = [
        "-sname",
        &name,
        "-kernel",
        "logger_level",
        "none",
        "-noshell",
        "-eval",
        expression,
    ];
    assert_eq!(stdout(&erts_erl(&epmd, &args)), "{true,true}\n");
    assert_eq!(stdout(&pbeam_erl(&epmd, &args)), "{true,true}\n");
}

#[test]
fn preserves_a_demonitored_pending_monitor_as_pinned_erts() {
    let epmd = Epmd::start();
    let id = std::process::id();
    for pbeam in [false, true] {
        let server_name = format!("dmonorderserver{pbeam}{id}");
        let server = ErtsNode::start(
            &epmd,
            &["-sname", &server_name, "-setcookie", "ck"],
            "register(pending_receiver,self()), io:format(\"~s~n\",[node()]), \
             receive {inspect,P} -> \
             F=fun Loop(0) -> timeout; Loop(T) -> case process_info(self(),monitored_by) of \
                {monitored_by,[]} -> []; _ -> timer:sleep(10), Loop(T-1) end end, \
             P ! {monitors,F(50)} end, \
             receive stop -> ok after 5000 -> ok end, halt().",
        );
        let expression = format!(
            "N='{}', erts_internal:new_connection(N), \
             R=monitor(process,{{pending_receiver,N}}), true=demonitor(R), \
             {{pending_receiver,N}} ! {{inspect,self()}}, true=net_kernel:connect_node(N), \
             receive {{monitors,Ms}} -> io:format(\"~w~n\",[Ms]) after 5000 -> halt(2) end, halt().",
            server.line()
        );
        let client_name = format!("dmonorderclient{pbeam}{id}");
        let args = [
            "-sname",
            &client_name,
            "-setcookie",
            "ck",
            "-kernel",
            "logger_level",
            "none",
            "-noshell",
            "-eval",
            &expression,
        ];
        let output = if pbeam {
            pbeam_erl(&epmd, &args)
        } else {
            erts_erl(&epmd, &args)
        };
        assert_eq!(stdout(&output), "timeout\n", "pbeam={pbeam}");
    }
}

#[test]
fn demonitors_a_remote_pid_before_connection_setup_as_pinned_erts() {
    // The pending PID monitor and demonitor reach the peer in order
    // (bif.c:525-576, dist.c:1550-1570, erl_proc_sig_queue.c:6550-6585).
    let epmd = Epmd::start();
    let id = std::process::id();
    for pbeam in [false, true] {
        let server_name = format!("dpiddemonserver{pbeam}{id}");
        let server = ErtsNode::start(
            &epmd,
            &[
                "-sname",
                &server_name,
                "-setcookie",
                "ck",
                "-pa",
                "fixtures/dist",
            ],
            "dist_pending_demonitor_probe:server().",
        );
        let node = server.line();
        let pid_bytes = server.line();
        let expression =
            format!("dist_pending_demonitor_probe:client({node}, {pid_bytes}), halt().");
        let client_name = format!("dpiddemonclient{pbeam}{id}");
        let args = [
            "-sname",
            &client_name,
            "-setcookie",
            "ck",
            "-kernel",
            "logger_level",
            "none",
            "-noshell",
            "-pa",
            "fixtures/dist",
            "-eval",
            &expression,
        ];
        let output = if pbeam {
            pbeam_erl(&epmd, &args)
        } else {
            erts_erl(&epmd, &args)
        };
        assert_eq!(stdout(&output), "false\n", "pbeam={pbeam}");
    }
}

#[test]
fn monitors_a_pid_on_the_first_signal_as_pinned_erts() {
    let epmd = Epmd::start();
    let id = std::process::id();
    for pbeam in [false, true] {
        let server_name = format!("dmonpidserver{pbeam}{id}");
        let server = ErtsNode::start(
            &epmd,
            &["-sname", &server_name, "-setcookie", "ck"],
            "Parent=self(), Child=spawn(fun()->register(pending_receiver,self()), Parent ! ready, receive stop -> ok end end), \
             receive ready -> ok end, io:format(\"~w~n~w~n\",[node(),binary_to_list(term_to_binary(Child))]), \
             receive stop -> ok after 5000 -> ok end, halt().",
        );
        let node = server.line();
        let pid_bytes = server.line();
        let expression = format!(
            "N={node}, P=binary_to_term(list_to_binary({pid_bytes})), R=monitor(process,P), \
             {{pending_receiver,N}} ! stop, \
             receive {{'DOWN',R,process,P,Reason}} -> io:format(\"~w~n\",[Reason]) after 5000 -> halt(2) end, halt()."
        );
        let client_name = format!("dmonpidclient{pbeam}{id}");
        let args = [
            "-sname",
            &client_name,
            "-setcookie",
            "ck",
            "-kernel",
            "logger_level",
            "none",
            "-noshell",
            "-eval",
            &expression,
        ];
        let output = if pbeam {
            pbeam_erl(&epmd, &args)
        } else {
            erts_erl(&epmd, &args)
        };
        assert_eq!(stdout(&output), "normal\n", "pbeam={pbeam}");
    }
}

#[test]
fn links_a_pid_on_the_first_signal_as_pinned_erts() {
    let epmd = Epmd::start();
    let id = std::process::id();
    for pbeam in [false, true] {
        let server_name = format!("dlinkserver{pbeam}{id}");
        let server = ErtsNode::start(
            &epmd,
            &["-sname", &server_name, "-setcookie", "ck"],
            "Parent=self(), Child=spawn(fun()->register(pending_receiver,self()), Parent ! ready, receive crash -> exit(boom) end end), \
             receive ready -> ok end, io:format(\"~w~n~w~n\",[node(),binary_to_list(term_to_binary(Child))]), \
             receive stop -> ok after 5000 -> ok end, halt().",
        );
        let node = server.line();
        let pid_bytes = server.line();
        let expression = format!(
            "process_flag(trap_exit,true), N={node}, P=binary_to_term(list_to_binary({pid_bytes})), \
             true=link(P), R=monitor(process,P), {{pending_receiver,N}} ! crash, \
             X=receive {{'EXIT',P,Reason1}} -> Reason1 after 5000 -> timeout end, \
             Y=receive {{'DOWN',R,process,P,Reason2}} -> Reason2 after 5000 -> timeout end, \
             io:format(\"~w~n\",[{{X,Y}}]), halt()."
        );
        let client_name = format!("dlinkclient{pbeam}{id}");
        let args = [
            "-sname",
            &client_name,
            "-setcookie",
            "ck",
            "-kernel",
            "logger_level",
            "none",
            "-noshell",
            "-eval",
            &expression,
        ];
        let output = if pbeam {
            pbeam_erl(&epmd, &args)
        } else {
            erts_erl(&epmd, &args)
        };
        assert_eq!(stdout(&output), "{boom,boom}\n", "pbeam={pbeam}");
    }
}

#[test]
fn fails_and_unlinks_pending_links_as_pinned_erts() {
    let epmd = Epmd::start();
    let name = format!("dlinkmissing{}", std::process::id());
    let expression = "N=missing@localhost, B=atom_to_binary(N), \
        P=binary_to_term(<<131,88,118,(byte_size(B)):16,B/binary,1:32,0:32,1:32>>), \
        process_flag(trap_exit,true), true=link(P), \
        A=receive {'EXIT',P,X} -> X after 5000 -> timeout end, \
        {_,R}=spawn_monitor(fun()->link(P),receive after 5000 -> ok end end), \
        C=receive {'DOWN',R,process,_,Y} -> Y after 5000 -> timeout end, \
        io:format(\"~w~n\",[{A,C}]), halt().";
    let args = [
        "-sname",
        &name,
        "-kernel",
        "logger_level",
        "none",
        "-noshell",
        "-eval",
        expression,
    ];
    assert_eq!(
        stdout(&erts_erl(&epmd, &args)),
        "{noconnection,noconnection}\n"
    );
    assert_eq!(
        stdout(&pbeam_erl(&epmd, &args)),
        "{noconnection,noconnection}\n"
    );
    for operation in ["unlink", "exit"] {
        let action = if operation == "unlink" {
            "true=link(P),true=unlink(P)"
        } else {
            "{_,R}=spawn_monitor(fun()->link(P),exit(boom) end), receive {'DOWN',R,process,_,boom}->ok end"
        };
        let expression = format!(
            "N=link_pending@nohost, B=atom_to_binary(N), \
             P=binary_to_term(<<131,88,118,(byte_size(B)):16,B/binary,1:32,0:32,1:32>>), \
             erts_internal:new_connection(N), {action}, Flags=element(3,erts_internal:get_dflags()), \
             {{ok,H}}=erts_internal:create_dist_channel(N,self(),{{Flags,1}}), \
             A=erlang:dist_ctrl_get_data(H), C=erlang:dist_ctrl_get_data(H), \
             io:format(\"~w~n\",[{{A=/=none,C=/=none}}]), halt()."
        );
        let args = [
            "-sname",
            &name,
            "-kernel",
            "logger_level",
            "none",
            "-noshell",
            "-eval",
            &expression,
        ];
        assert_eq!(
            stdout(&erts_erl(&epmd, &args)),
            "{true,true}\n",
            "{operation}"
        );
        assert_eq!(
            stdout(&pbeam_erl(&epmd, &args)),
            "{true,true}\n",
            "{operation}"
        );
    }
}

#[test]
fn spawns_on_the_first_signal_as_pinned_erts() {
    let epmd = Epmd::start();
    let id = std::process::id();
    for socket in [false, true] {
        for pbeam in [false, true] {
            let server_name = format!("dspawnfirstserver{socket}{pbeam}{id}");
            let mut server_args = vec!["-sname", server_name.as_str(), "-setcookie", "ck"];
            if socket {
                server_args.extend(SOCKET_CARRIER);
            }
            let server = ErtsNode::start(
                &epmd,
                &server_args,
                "io:format(\"~w~n\",[node()]), receive stop -> ok after 5000 -> ok end, halt().",
            );
            let node = server.line();
            let expression = format!(
                "N={node}, R=spawn_request(N,erlang,send,[self(),spawned],[monitor]), \
                 P=receive {{spawn_reply,R,ok,Pid}} -> Pid after 5000 -> halt(2) end, \
                 receive spawned -> ok after 5000 -> halt(3) end, \
                 Reason=receive {{'DOWN',R,process,P,X}} -> X after 5000 -> timeout end, \
                 io:format(\"~w~n\",[Reason]), halt()."
            );
            let client_name = format!("dspawnfirstclient{socket}{pbeam}{id}");
            let mut args = vec![
                "-sname",
                client_name.as_str(),
                "-setcookie",
                "ck",
                "-kernel",
                "logger_level",
                "none",
            ];
            if socket {
                args.extend(SOCKET_CARRIER);
            }
            args.extend(["-noshell", "-eval", expression.as_str()]);
            let output = if pbeam {
                pbeam_erl(&epmd, &args)
            } else {
                erts_erl(&epmd, &args)
            };
            assert_eq!(stdout(&output), "normal\n", "socket={socket} pbeam={pbeam}");
        }
    }
}

#[test]
fn fails_a_pending_spawn_as_pinned_erts() {
    // dist.c:6081-6120 and erl_proc_sig_queue.c:4864-4900.
    let epmd = Epmd::start();
    let name = format!("dspawnmissing{}", std::process::id());
    let expression = "N=missing@localhost, R=spawn_request(N,erlang,node,[],[]), \
        A=receive {spawn_reply,R,error,E}->E after 5000->timeout end, \
        process_flag(trap_exit,true), P=spawn_link(N,erlang,node,[]), \
        B=receive {'EXIT',P,E2}->E2 after 5000->timeout end, \
        io:format(\"~w~n\",[{A,is_pid(P),B}]), halt().";
    let args = [
        "-sname",
        &name,
        "-kernel",
        "logger_level",
        "none",
        "-noshell",
        "-eval",
        expression,
    ];
    assert_eq!(
        stdout(&erts_erl(&epmd, &args)),
        "{noconnection,true,noconnection}\n"
    );
    assert_eq!(
        stdout(&pbeam_erl(&epmd, &args)),
        "{noconnection,true,noconnection}\n"
    );
    let expression = "N=spawn_peer@nohost, erts_internal:new_connection(N), \
        R=spawn_request(N,erlang,node,[],[]), \
        F=element(3,erts_internal:get_dflags()) band bnot (1 bsl 32), \
        {ok,H}=erts_internal:create_dist_channel(N,self(),{F,1}), \
        Before=receive {spawn_reply,R,_,_}->early after 0->none end, \
        D=erlang:dist_ctrl_get_data(H), \
        After=receive {spawn_reply,R,error,Why}->Why after 5000->timeout end, \
        io:format(\"~w~n\",[{Before,D,After}]), halt().";
    let args = [
        "-sname",
        &name,
        "-kernel",
        "logger_level",
        "none",
        "-noshell",
        "-eval",
        expression,
    ];
    assert_eq!(stdout(&erts_erl(&epmd, &args)), "{none,[],notsup}\n");
    assert_eq!(stdout(&pbeam_erl(&epmd, &args)), "{none,[],notsup}\n");
    let expression = "N=spawn_port@nohost, erts_internal:new_connection(N), \
        R=spawn_request(N,erlang,node,[],[]), \
        F=element(3,erts_internal:get_dflags()) band bnot (1 bsl 32), \
        {ok,L}=gen_tcp:listen(0,[]), {ok,_}=erts_internal:create_dist_channel(N,L,{F,1}), \
        Result=receive {spawn_reply,R,error,Why}->Why after 5000->timeout end, \
        io:format(\"~w~n\",[Result]), halt().";
    let args = [
        "-sname",
        &name,
        "-kernel",
        "logger_level",
        "none",
        "-noshell",
        "-eval",
        expression,
    ];
    assert_eq!(stdout(&erts_erl(&epmd, &args)), "notsup\n");
    assert_eq!(stdout(&pbeam_erl(&epmd, &args)), "notsup\n");
}

#[test]
fn sends_to_a_pid_and_alias_on_the_first_signal_as_pinned_erts() {
    let epmd = Epmd::start();
    let id = std::process::id();
    for pbeam in [false, true] {
        let server_name = format!("dsendidserver{pbeam}{id}");
        let server = ErtsNode::start(
            &epmd,
            &["-sname", &server_name, "-setcookie", "ck"],
            "register(pending_receiver,self()), A=alias(), \
             io:format(\"~w~n~w~n~w~n\",[node(),binary_to_list(term_to_binary(self())),binary_to_list(term_to_binary(A))]), \
             receive {one,P} -> receive two -> receive three -> P ! ack end end end, \
             receive stop -> ok after 5000 -> ok end, halt().",
        );
        let node = server.line();
        let pid = server.line();
        let alias = server.line();
        let expression = format!(
            "N={node}, P=binary_to_term(list_to_binary({pid})), A=binary_to_term(list_to_binary({alias})), \
             P ! {{one,self()}}, ok=erlang:send(A,two,[noconnect,priority]), \
             ok=erlang:send({{pending_receiver,N}},three,[noconnect]), \
             receive ack -> io:format(\"ok~n\") after 5000 -> halt(2) end, halt()."
        );
        let client_name = format!("dsendidclient{pbeam}{id}");
        let args = [
            "-sname",
            &client_name,
            "-setcookie",
            "ck",
            "-kernel",
            "logger_level",
            "none",
            "-noshell",
            "-eval",
            &expression,
        ];
        let output = if pbeam {
            pbeam_erl(&epmd, &args)
        } else {
            erts_erl(&epmd, &args)
        };
        assert_eq!(stdout(&output), "ok\n", "pbeam={pbeam}");
    }
    let name = format!("dsendaliasflags{id}");
    let expression = "N=alias_peer@nohost, B=atom_to_binary(N), \
        A=binary_to_term(<<131,90,0,1,118,(byte_size(B)):16,B/binary,1:32,1:32>>), \
        erts_internal:new_connection(N), ok=erlang:send(A,msg,[noconnect]), \
        F=element(3,erts_internal:get_dflags()) band bnot (8 bsl 32), \
        {ok,H}=erts_internal:create_dist_channel(N,self(),{F,1}), \
        D=erlang:dist_ctrl_get_data(H), io:format(\"~w~n\",[D=:=[]]), halt().";
    let args = [
        "-sname",
        &name,
        "-kernel",
        "logger_level",
        "none",
        "-noshell",
        "-eval",
        expression,
    ];
    assert_eq!(stdout(&erts_erl(&epmd, &args)), "true\n");
    assert_eq!(stdout(&pbeam_erl(&epmd, &args)), "true\n");
    let expression = expression.replace(
        "band bnot (8 bsl 32)",
        "band bnot (8 bsl 32) bor (32 bsl 32)",
    );
    let args = [
        "-sname",
        &name,
        "-kernel",
        "logger_level",
        "none",
        "-noshell",
        "-eval",
        &expression,
    ];
    assert_eq!(stdout(&erts_erl(&epmd, &args)), "false\n");
    assert_eq!(stdout(&pbeam_erl(&epmd, &args)), "false\n");
}

#[test]
fn orders_the_four_pending_signals_as_pinned_erts() {
    let epmd = Epmd::start();
    let id = std::process::id();
    for pbeam in [false, true] {
        let server_name = format!("dallserver{pbeam}{id}");
        let server = ErtsNode::start(
            &epmd,
            &["-sname", &server_name, "-setcookie", "ck"],
            "Parent=self(), Child=spawn(fun()->register(pending_receiver,self()), Parent ! ready, \
             receive {first,P} -> receive A -> receive B -> P ! {observed,lists:sort([A,B])} end end end, exit(boom) end), \
             receive ready -> ok end, io:format(\"~w~n~w~n\",[node(),binary_to_list(term_to_binary(Child))]), \
             receive stop -> ok after 5000 -> ok end, halt().",
        );
        let node = server.line();
        let pid = server.line();
        let expression = format!(
            "process_flag(trap_exit,true), N={node}, P=binary_to_term(list_to_binary({pid})), \
             {{pending_receiver,N}} ! {{first,self()}}, true=link(P), M=monitor(process,P), \
             R=spawn_request(N,erlang,send,[{{pending_receiver,N}},spawned],[]), \
             {{pending_receiver,N}} ! last, \
             receive {{spawn_reply,R,ok,_}} -> ok after 5000 -> halt(2) end, \
             Seen=receive {{observed,Items}} -> Items after 5000 -> timeout end, \
             X=receive {{'EXIT',P,R1}} -> R1 after 5000 -> timeout end, \
             Y=receive {{'DOWN',M,process,P,R2}} -> R2 after 5000 -> timeout end, \
             io:format(\"~w~n\",[{{Seen,X,Y}}]), halt()."
        );
        let client_name = format!("dallclient{pbeam}{id}");
        let args = [
            "-sname",
            &client_name,
            "-setcookie",
            "ck",
            "-kernel",
            "logger_level",
            "none",
            "-noshell",
            "-eval",
            &expression,
        ];
        let output = if pbeam {
            pbeam_erl(&epmd, &args)
        } else {
            erts_erl(&epmd, &args)
        };
        assert_eq!(
            stdout(&output),
            "{[last,spawned],boom,boom}\n",
            "pbeam={pbeam}"
        );
    }
}

#[test]
fn counts_dead_aliases_in_pending_signals_as_pinned_erts() {
    let epmd = Epmd::start();
    for (operation, expected) in [
        ("send", "{ok,266}\n"),
        ("exit_signal", "{ok,274}\n"),
        ("link_exit", "{ok,500}\n"),
    ] {
        let expression = format!("dist_pending_error_probe:main({operation}), halt().");
        let arguments = probe(
            &[
                "-name",
                "pending_q@localhost",
                "-kernel",
                "logger_level",
                "none",
            ],
            &expression,
        );
        assert_eq!(
            stdout(&erts_erl(&epmd, &arguments)),
            expected,
            "{operation}"
        );
        assert_eq!(
            stdout(&pbeam_erl(&epmd, &arguments)),
            expected,
            "{operation}"
        );
    }
}

#[test]
fn records_dead_alias_pending_delivery_difference() {
    // The External term format row and #1619 record the missing owner pid.
    let epmd = Epmd::start();
    let name = format!("dpenderr{}", std::process::id());
    let arguments = probe(
        &["-sname", &name, "-kernel", "logger_level", "none"],
        "dist_pending_error_probe:deliver(), halt().",
    );
    assert_eq!(stdout(&erts_erl(&epmd, &arguments)), "ok\n");
    let output = pbeam_erl(&epmd, &arguments);
    assert_eq!(output.status.code(), Some(1), "{output:?}");
    assert!(
        String::from_utf8_lossy(&output.stderr)
            .contains("unsupported external term of a pid reference whose owner exited"),
        "{output:?}"
    );
}

#[test]
fn measures_pending_queue_size_on_pinned_erts() {
    let epmd = Epmd::start();
    let arguments = probe(
        &[
            "-name",
            "pending_q@localhost",
            "-kernel",
            "logger_level",
            "none",
        ],
        "dist_pending_size_probe:main(), halt().",
    );
    let expected = "[0,230,460,214,254,309,220,1267]\n";
    assert_eq!(stdout(&erts_erl(&epmd, &arguments)), expected);
    assert_eq!(stdout(&pbeam_erl(&epmd, &arguments)), expected);
    let arguments = probe(
        &[
            "-name",
            "pending_q@localhost",
            "-kernel",
            "logger_level",
            "none",
        ],
        "dist_pending_size_probe:more(), halt().",
    );
    let expected = "[213,458,688,932,1179,1425]\n";
    assert_eq!(stdout(&erts_erl(&epmd, &arguments)), expected);
    assert_eq!(stdout(&pbeam_erl(&epmd, &arguments)), expected);
}

#[test]
fn orders_all_five_pending_signal_packets_as_pinned_erts() {
    let epmd = Epmd::start();
    let name = format!("dorder{}", std::process::id());
    let arguments = probe(
        &["-sname", &name, "-kernel", "logger_level", "none"],
        "dist_pending_order_probe:main(), halt().",
    );
    let expected = "[6,1,19,29,8]\n";
    assert_eq!(stdout(&erts_erl(&epmd, &arguments)), expected);
    assert_eq!(stdout(&pbeam_erl(&epmd, &arguments)), expected);
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
fn connects_and_queues_exit_signals_as_pinned_erts() {
    let epmd = Epmd::start();
    let id = std::process::id();
    for pbeam in [false, true] {
        let server_name = format!("dexitautoserver{pbeam}{id}");
        let server = ErtsNode::start(
            &epmd,
            &["-sname", &server_name, "-setcookie", "ck"],
            "process_flag(trap_exit,true), register(exit_receiver,self()), A=alias(), \
             io:format(\"~w~n~w~n~w~n\",[node(),binary_to_list(term_to_binary(self())),binary_to_list(term_to_binary(A))]), \
             receive {'EXIT',_,boom} -> ok after 5000 -> halt(2) end, \
             receive {probe,P} -> P ! ack after 5000 -> halt(3) end, \
             receive stop -> ok after 5000 -> ok end, halt().",
        );
        let node = server.line();
        let pid = server.line();
        let alias = server.line();
        let expression = format!(
            "N={node}, P=binary_to_term(list_to_binary({pid})), A=binary_to_term(list_to_binary({alias})), \
             true=exit(P,boom), true=exit(A,alias_boom), {{exit_receiver,N}} ! {{probe,self()}}, \
             receive ack -> io:format(\"ok~n\") after 5000 -> halt(2) end, halt()."
        );
        let client_name = format!("dexitautoclient{pbeam}{id}");
        let args = [
            "-sname",
            &client_name,
            "-setcookie",
            "ck",
            "-kernel",
            "logger_level",
            "none",
            "-noshell",
            "-eval",
            &expression,
        ];
        let output = if pbeam {
            pbeam_erl(&epmd, &args)
        } else {
            erts_erl(&epmd, &args)
        };
        assert_eq!(stdout(&output), "ok\n", "pbeam={pbeam}");
    }
    let name = format!("dexitautoflags{id}");
    let expression = "N=exit_peer@nohost, B=atom_to_binary(N), \
        A=binary_to_term(<<131,90,0,1,118,(byte_size(B)):16,B/binary,1:32,1:32>>), \
        erts_internal:new_connection(N), true=exit(A,boom), F=element(3,erts_internal:get_dflags()), \
        {ok,H}=erts_internal:create_dist_channel(N,self(),{F,1}), \
        io:format(\"~w~n\",[erlang:dist_ctrl_get_data(H)]), halt().";
    let args = [
        "-sname",
        &name,
        "-kernel",
        "logger_level",
        "none",
        "-noshell",
        "-eval",
        expression,
    ];
    assert_eq!(stdout(&erts_erl(&epmd, &args)), "[]\n");
    assert_eq!(stdout(&pbeam_erl(&epmd, &args)), "[]\n");
    let expression = "N=missing@localhost, B=atom_to_binary(N), \
        P=binary_to_term(<<131,88,118,(byte_size(B)):16,B/binary,1:32,0:32,1:32>>), \
        Result=exit(P,boom), io:format(\"~w~n\",[{Result,net_adm:ping(N)}]), halt().";
    let args = [
        "-sname",
        &name,
        "-kernel",
        "logger_level",
        "none",
        "-noshell",
        "-eval",
        expression,
    ];
    assert_eq!(stdout(&erts_erl(&epmd, &args)), "{true,pang}\n");
    assert_eq!(stdout(&pbeam_erl(&epmd, &args)), "{true,pang}\n");
}

#[test]
fn monitors_the_local_node_as_pinned_erts() {
    let epmd = Epmd::start();
    let name = format!("dml{}", std::process::id());
    for prefix in [vec![], vec!["-sname", name.as_str()]] {
        let arguments = probe(&prefix, "dist_lost_probe:monitor_local(), halt().");
        let expected = "[true,true,{error,badarg},{error,badarg},{error,badarg},{error,badarg}]\n";
        assert_eq!(stdout(&erts_erl(&epmd, &arguments)), expected);
        assert_eq!(stdout(&pbeam_erl(&epmd, &arguments)), expected);
    }
    let arguments = probe(&[], "dist_lost_probe:monitor_notalive(), halt().");
    assert_eq!(stdout(&erts_erl(&epmd, &arguments)), "{error,notalive}\n");
    assert_eq!(stdout(&pbeam_erl(&epmd, &arguments)), "{error,notalive}\n");
    for expression in [
        "dist_lost_probe:monitor_remote(), halt().",
        "dist_lost_probe:monitor_idle(), halt().",
    ] {
        let arguments = probe(&["-sname", &name], expression);
        assert_eq!(stdout(&erts_erl(&epmd, &arguments)), "");
        assert_eq!(stdout(&pbeam_erl(&epmd, &arguments)), "");
    }
}

#[test]
fn connects_and_fails_node_monitors_as_pinned_erts() {
    let epmd = Epmd::start();
    let id = std::process::id();
    for passive in [false, true] {
        for pbeam in [false, true] {
            let server_name = format!("dnodeautoserver{passive}{pbeam}{id}");
            let server = ErtsNode::start(
                &epmd,
                &["-sname", &server_name, "-setcookie", "ck"],
                "register(node_receiver,self()), io:format(\"~w~n\",[node()]), receive stop -> halt() end.",
            );
            let node = server.line();
            let options = if passive {
                "[allow_passive_connect]"
            } else {
                "[]"
            };
            let expression = format!(
                "N={node}, A=erlang:monitor_node(N,true,{options}), B=erlang:monitor_node(N,true), \
                 C=erlang:monitor_node(N,false), {{node_receiver,N}} ! stop, \
                 D=receive {{nodedown,N}} -> nodedown after 5000 -> timeout end, \
                 E=receive {{nodedown,N}} -> unexpected after 100 -> none end, \
                 io:format(\"~w~n\",[{{A,B,C,D,E}}]), halt()."
            );
            let client_name = format!("dnodeautoclient{passive}{pbeam}{id}");
            let args = [
                "-sname",
                &client_name,
                "-setcookie",
                "ck",
                "-kernel",
                "logger_level",
                "none",
                "-noshell",
                "-eval",
                &expression,
            ];
            let output = if pbeam {
                pbeam_erl(&epmd, &args)
            } else {
                erts_erl(&epmd, &args)
            };
            assert_eq!(
                stdout(&output),
                "{true,true,true,nodedown,none}\n",
                "passive={passive} pbeam={pbeam}"
            );
        }
    }
    let name = format!("dnodemissing{id}");
    for options in ["[]", "[allow_passive_connect]"] {
        let expression = format!(
            "N=missing@localhost, A=erlang:monitor_node(N,false,{options}), B=erlang:monitor_node(N,true,{options}), \
             C=receive {{nodedown,N}} -> nodedown after 5000 -> timeout end, \
             io:format(\"~w~n\",[{{A,B,C}}]), halt()."
        );
        let args = [
            "-sname",
            &name,
            "-kernel",
            "logger_level",
            "none",
            "-noshell",
            "-eval",
            &expression,
        ];
        assert_eq!(stdout(&erts_erl(&epmd, &args)), "{true,true,nodedown}\n");
        assert_eq!(stdout(&pbeam_erl(&epmd, &args)), "{true,true,nodedown}\n");
    }
}

#[test]
fn handles_an_unregistered_distribution_owner_as_pinned_erts() {
    // dist.c:5732-5736 aborts auto-connect when net_kernel has no name.
    let epmd = Epmd::start();
    let arguments = probe(&[], "dist_coverage_probe:missing_kernel(), halt().");
    let expected = "{true,nodedown,noconnection,none}\n";
    assert_eq!(stdout(&erts_erl(&epmd, &arguments)), expected);
    assert_eq!(stdout(&pbeam_erl(&epmd, &arguments)), expected);
}

#[test]
fn reports_the_monitor_node_badopt_cause_as_pinned_erts() {
    let epmd = Epmd::start();
    let arguments = probe(&[], "dist_lost_probe:monitor_badopt(), halt().");
    let reference = stdout(&erts_erl(&epmd, &arguments));
    let badopt = "#{cause => badopt,module => erl_erts_errors}";
    let plain = "#{module => erl_erts_errors}";
    assert_eq!(
        reference,
        format!("[{badopt},{badopt},{badopt},{badopt},{plain},{plain}]\n")
    );
    assert_eq!(stdout(&pbeam_erl(&epmd, &arguments)), reference);
}

#[test]
fn disconnect_node_returns_the_pinned_erts_values_and_ends_the_connection() {
    let epmd = Epmd::start();
    let id = std::process::id();
    let server_name = format!("ndisserver{id}");
    let server = ErtsNode::start(
        &epmd,
        &[
            "-sname",
            &server_name,
            "-setcookie",
            "ck",
            "-pa",
            "fixtures/dist",
        ],
        "dist_lost_probe:server().",
    );
    let expression = format!(
        "dist_lost_probe:disconnect_cases('{}'), halt().",
        server.line()
    );
    let client_name = format!("ndisclient{id}");
    let arguments = probe(&["-sname", &client_name, "-setcookie", "ck"], &expression);
    let expected = "[true,nodedown,false,false]\n";
    assert_eq!(stdout(&erts_erl(&epmd, &arguments)), expected);
    assert_eq!(stdout(&pbeam_erl(&epmd, &arguments)), expected);
    let arguments = probe(&[], "dist_lost_probe:disconnect_notalive(), halt().");
    assert_eq!(
        stdout(&erts_erl(&epmd, &arguments)),
        "[ignored,ignored,ignored]\n"
    );
    assert_eq!(
        stdout(&pbeam_erl(&epmd, &arguments)),
        "[ignored,ignored,ignored]\n"
    );
}

#[test]
fn expires_send_only_known_nodes_as_pinned_erts() {
    let epmd = Epmd::start();
    let id = std::process::id();
    let erts_name = format!("nexpireerts{id}");
    let pbeam_name = format!("nexpirepb{id}");
    let expression = "dist_lost_probe:nodes_expiry(), halt().";
    let erts_args = probe(&["-sname", &erts_name, "-setcookie", "ck"], expression);
    let pbeam_args = probe(&["-sname", &pbeam_name, "-setcookie", "ck"], expression);
    std::thread::scope(|scope| {
        let erts = scope.spawn(|| erts_erl(&epmd, &erts_args));
        let pbeam = scope.spawn(|| pbeam_erl(&epmd, &pbeam_args));
        assert_eq!(stdout(&erts.join().unwrap()), "[nmekeep@host]\n");
        assert_eq!(stdout(&pbeam.join().unwrap()), "[nmekeep@host]\n");
    });
}

#[test]
fn lists_node_types_and_mixed_known_order_as_pinned_erts() {
    let epmd = Epmd::start();
    let id = std::process::id();
    let visible_name = format!("nvisible{id}");
    let hidden_name = format!("nhidden{id}");
    let visible = ErtsNode::start(
        &epmd,
        &[
            "-sname",
            &visible_name,
            "-setcookie",
            "ck",
            "-pa",
            "fixtures/dist",
        ],
        "dist_lost_probe:server().",
    );
    let hidden = ErtsNode::start(
        &epmd,
        &[
            "-sname",
            &hidden_name,
            "-setcookie",
            "ck",
            "-hidden",
            "-pa",
            "fixtures/dist",
        ],
        "dist_lost_probe:server().",
    );
    let expression = format!(
        "dist_lost_probe:nodes_forms('{}','{}'), halt().",
        visible.line(),
        hidden.line()
    );
    let client = format!("nforms{id}");
    let arguments = probe(
        &["+S", "2", "-sname", &client, "-setcookie", "ck"],
        &expression,
    );
    let expected = stdout(&erts_erl(&epmd, &arguments));
    assert!(
        expected.contains("{connected,[hidden_peer,visible_peer]"),
        "{expected}"
    );
    assert!(
        expected.contains("{this,[local],[{local,this,undefined}]}"),
        "{expected}"
    );
    assert_eq!(stdout(&pbeam_erl(&epmd, &arguments)), expected);
    let arguments = probe(
        &["+S", "2", "-sname", &client, "-setcookie", "ck"],
        "dist_lost_probe:nodes_order(), halt().",
    );
    let expected = "[[nmea@host,nmeb@host,nmec@host],[nmeb@host,nmec@host,nmea@host]]\n";
    assert_eq!(stdout(&erts_erl(&epmd, &arguments)), expected);
    assert_eq!(stdout(&pbeam_erl(&epmd, &arguments)), expected);
}

#[test]
fn reports_node_events_as_pinned_erts() {
    let epmd = Epmd::start();
    let id = std::process::id();
    let server_name = format!("neserver{id}");
    let server = ErtsNode::start(
        &epmd,
        &[
            "-sname",
            &server_name,
            "-setcookie",
            "ck",
            "-pa",
            "fixtures/dist",
        ],
        "dist_lost_probe:server().",
    );
    let server_node = server.line();
    let expression = format!("dist_lost_probe:monitor_events('{server_node}'), halt().");
    let client_name = format!("neclient{id}");
    let arguments = probe(&["-sname", &client_name, "-setcookie", "ck"], &expression);
    let reference = stdout(&erts_erl(&epmd, &arguments));
    assert!(
        reference.contains("node_type => visible")
            && reference.contains("nodedown_reason => disconnect")
            && !reference.contains("timeout"),
        "{reference}"
    );
    assert_eq!(stdout(&pbeam_erl(&epmd, &arguments)), reference);
    let expression = format!("dist_lost_probe:monitor_forms('{server_node}'), halt().");
    let arguments = probe(&["-sname", &client_name, "-setcookie", "ck"], &expression);
    let reference = stdout(&erts_erl(&epmd, &arguments));
    assert_eq!(
        reference,
        "[up,up,none,true,true,true,true,visible,visible,disconnect,disconnect]\n"
    );
    assert_eq!(stdout(&pbeam_erl(&epmd, &arguments)), reference);
}

#[test]
fn disables_the_last_monitor_on_a_connected_node_as_pinned_erts() {
    let epmd = Epmd::start();
    let name = format!("nmedisableconnected{}", std::process::id());
    let arguments = probe(
        &["-sname", &name],
        "dist_lost_probe:monitor_disable_connected(), halt().",
    );
    assert_eq!(stdout(&erts_erl(&epmd, &arguments)), "ok\n");
    assert_eq!(stdout(&pbeam_erl(&epmd, &arguments)), "ok\n");
}

#[test]
fn resolves_remote_node_monitor_counts_at_receipt_as_pinned_erts() {
    let epmd = Epmd::start();
    let name = format!("nmedisable{}", std::process::id());
    for (disabled, expected) in [(0, "2\n"), (1, "2\n"), (2, "2\n")] {
        let expression = format!("dist_lost_probe:monitor_disable_signal({disabled}), halt().");
        let arguments = probe(&["-sname", &name], &expression);
        assert_eq!(stdout(&erts_erl(&epmd, &arguments)), expected, "{disabled}");
        assert_eq!(
            stdout(&pbeam_erl(&epmd, &arguments)),
            expected,
            "{disabled}"
        );
    }
}

#[test]
fn resolves_node_monitor_signals_at_receipt_as_pinned_erts() {
    let epmd = Epmd::start();
    for (mode, expected) in [("disable", "0\n"), ("increase", "2\n"), ("recreate", "1\n")] {
        let expression = format!("dist_lost_probe:self_signal({mode}), halt().");
        let arguments = probe(&[], &expression);
        assert_eq!(stdout(&erts_erl(&epmd, &arguments)), expected, "{mode}");
        assert_eq!(stdout(&pbeam_erl(&epmd, &arguments)), expected, "{mode}");
    }
}

#[test]
fn reports_controller_exit_reasons_as_pinned_erts() {
    let epmd = Epmd::start();
    let name = format!("nereason{}", std::process::id());
    let arguments = probe(
        &["-sname", &name],
        "dist_lost_probe:controller_reasons(), halt().",
    );
    let expected = "[connection_closed,custom,custom]\n";
    assert_eq!(stdout(&erts_erl(&epmd, &arguments)), expected);
    assert_eq!(stdout(&pbeam_erl(&epmd, &arguments)), expected);
    let arguments = probe(&[], "dist_lost_probe:no_network(), halt().");
    assert_eq!(stdout(&erts_erl(&epmd, &arguments)), "no_network\n");
    assert_eq!(stdout(&pbeam_erl(&epmd, &arguments)), "no_network\n");
}

#[test]
fn waits_for_both_controllers_before_local_nodedown_as_pinned_erts() {
    let epmd = Epmd::start();
    let arguments = probe(&["+S", "2"], "dist_lost_probe:local_multi(), halt().");
    let expected = "[true,true,false,false]\n";
    assert_eq!(stdout(&erts_erl(&epmd, &arguments)), expected);
    assert_eq!(stdout(&pbeam_erl(&epmd, &arguments)), expected);
}

#[test]
fn orders_local_node_events_when_owner_exits_during_setnode() {
    let epmd = Epmd::start();
    let arguments = probe(&["+S", "2"], "dist_lost_probe:local_race(), halt().");
    let expected = "true\n";
    assert_eq!(stdout(&erts_erl(&epmd, &arguments)), expected);
    assert_eq!(stdout(&pbeam_erl(&epmd, &arguments)), expected);
}

#[test]
fn orders_remote_and_local_nodedown_as_pinned_erts() {
    let epmd = Epmd::start();
    let id = std::process::id();
    let server_name = format!("nelocalserver{id}");
    let server = ErtsNode::start(
        &epmd,
        &[
            "-sname",
            &server_name,
            "-setcookie",
            "ck",
            "-pa",
            "fixtures/dist",
        ],
        "dist_lost_probe:server().",
    );
    let expression = format!("dist_lost_probe:local_order('{}'), halt().", server.line());
    let arguments = probe(&["+S", "2", "-setcookie", "ck"], &expression);
    let expected = "[{up,true,true},{up,true,true},{up,true,true}]\n";
    assert_eq!(stdout(&erts_erl(&epmd, &arguments)), expected);
    assert_eq!(stdout(&pbeam_erl(&epmd, &arguments)), expected);
}

#[test]
fn orders_node_events_when_peer_disconnects_at_connect() {
    let epmd = Epmd::start();
    let id = std::process::id();
    let server_name = format!("neorder{id}");
    let server = ErtsNode::start(
        &epmd,
        &[
            "-sname",
            &server_name,
            "-setcookie",
            "ck",
            "-pa",
            "fixtures/dist",
        ],
        "dist_lost_probe:ordering_server().",
    );
    let expression = format!(
        "dist_lost_probe:ordering_client('{}'), halt().",
        server.line()
    );
    let client_name = format!("neorderclient{id}");
    let arguments = probe(
        &[
            "+S",
            "2",
            "-sname",
            &client_name,
            "-setcookie",
            "ck",
            "-hidden",
        ],
        &expression,
    );
    let expected = "[ok,ok,ok,ok]\n";
    for pbeam in [false, true] {
        let mut command = if pbeam {
            let (root, bindir) = emulator_flags();
            let mut command = Command::new(env!("CARGO_BIN_EXE_pbeam"));
            command
                .args(["erl", "--root", &root, "--bindir"])
                .arg(bindir)
                .arg("--preloaded")
                .arg(pinned_beam_path("erlang").parent().unwrap())
                .arg("--");
            command
        } else {
            Command::new("erl")
        };
        let mut child = command
            .args(["-start_epmd", "false"])
            .args(&arguments)
            .env("ERL_EPMD_PORT", epmd.port())
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap();
        let (sender, receiver) = mpsc::channel();
        let pipes: [Box<dyn Read + Send>; 2] = [
            Box::new(child.stdout.take().unwrap()),
            Box::new(child.stderr.take().unwrap()),
        ];
        for (index, mut pipe) in pipes.into_iter().enumerate() {
            let sender = sender.clone();
            thread::spawn(move || {
                let mut bytes = Vec::new();
                let result = pipe.read_to_end(&mut bytes);
                let _ = sender.send((index, result, bytes));
            });
        }
        let deadline = Instant::now() + Duration::from_secs(110);
        let status = loop {
            if let Some(status) = child.try_wait().unwrap() {
                break status;
            }
            if Instant::now() >= deadline {
                let _ = child.kill();
                let _ = child.wait();
                panic!(
                    "ordering client deadline expired before fixture diagnostics: pbeam={pbeam}"
                );
            }
            thread::sleep(Duration::from_millis(50));
        };
        let first = receiver.recv_timeout(Duration::from_secs(5)).unwrap();
        let second = receiver.recv_timeout(Duration::from_secs(5)).unwrap();
        assert!(
            first.1.is_ok() && second.1.is_ok(),
            "ordering output read failed: {first:?} {second:?}"
        );
        let mut output = [Vec::new(), Vec::new()];
        output[first.0] = first.2;
        output[second.0] = second.2;
        assert!(
            status.success() && output[0] == expected.as_bytes() && output[1].is_empty(),
            "pbeam={pbeam} status={status} stdout={} stderr={}",
            String::from_utf8_lossy(&output[0]),
            String::from_utf8_lossy(&output[1])
        );
    }
}

#[test]
fn reports_local_node_events_as_pinned_erts() {
    let epmd = Epmd::start();
    let arguments = probe(&[], "dist_lost_probe:monitor_local_events(), halt().");
    let expected = stdout(&erts_erl(&epmd, &arguments));
    assert_eq!(stdout(&pbeam_erl(&epmd, &arguments)), expected);
}

#[test]
fn reports_hidden_node_events_as_pinned_erts() {
    let epmd = Epmd::start();
    let id = std::process::id();
    let server_name = format!("nehserver{id}");
    let server = ErtsNode::start(
        &epmd,
        &[
            "-sname",
            &server_name,
            "-setcookie",
            "ck",
            "-hidden",
            "-pa",
            "fixtures/dist",
        ],
        "dist_lost_probe:server().",
    );
    let expression = format!(
        "dist_lost_probe:monitor_hidden('{}'), halt().",
        server.line()
    );
    let client_name = format!("nehclient{id}");
    let arguments = probe(&["-sname", &client_name, "-setcookie", "ck"], &expression);
    let expected = "[#{node_type => hidden},none,#{node_type => hidden}]\n";
    assert_eq!(stdout(&erts_erl(&epmd, &arguments)), expected);
    assert_eq!(stdout(&pbeam_erl(&epmd, &arguments)), expected);
}

#[test]
fn monitors_a_connected_node_as_pinned_erts() {
    let epmd = Epmd::start();
    let id = std::process::id();
    let server_name = format!("dmserver{id}");
    let server = ErtsNode::start(
        &epmd,
        &[
            "-sname",
            &server_name,
            "-setcookie",
            "ck",
            "-pa",
            "fixtures/dist",
        ],
        "dist_lost_probe:server().",
    );
    let expression = format!(
        "dist_lost_probe:monitor_connected('{}'), halt().",
        server.line()
    );
    let client_name = format!("dmclient{id}");
    for socket in [false, true] {
        let mut prefix = vec![
            "-sname",
            client_name.as_str(),
            "-setcookie",
            "ck",
            "-hidden",
        ];
        if socket {
            prefix.extend(SOCKET_CARRIER);
        }
        let arguments = probe(&prefix, &expression);
        assert_eq!(
            stdout(&erts_erl(&epmd, &arguments)),
            "[nodedown,none,nodedown,nodedown,none]\n"
        );
        assert_eq!(
            stdout(&pbeam_erl(&epmd, &arguments)),
            "[nodedown,none,nodedown,nodedown,none]\n"
        );
    }
}

#[test]
fn disconnects_a_node_with_links_and_monitors_as_pinned_erts() {
    let epmd = Epmd::start();
    let id = std::process::id();
    let name = format!("ddsrv{id}");
    let arguments = ["-sname", &name, "-setcookie", "ck", "-pa", "fixtures/dist"];
    let server = ErtsNode::start(&epmd, &arguments, "dist_lost_probe:server().");
    let expression = format!(
        "dist_lost_probe:client_disconnect('{}'), halt().",
        server.line()
    );
    let client_name = format!("ddcli{id}");
    let client = [
        "-sname",
        &client_name,
        "-setcookie",
        "ck",
        "-hidden",
        "-noshell",
        "-pa",
        "fixtures/dist",
        "-eval",
        &expression,
    ];
    let expected = "[{linked,noconnection,noconnection,noconnection}]\n";
    assert_eq!(stdout(&erts_erl(&epmd, &client)), expected);
    assert_eq!(stdout(&pbeam_erl(&epmd, &client)), expected);
}

#[test]
fn ends_the_links_of_a_lost_connection_as_pinned_erts() {
    // The server node halts, and each process of the client node that has a
    // link to its process takes the exit signal `noconnection`: the process
    // that traps exits gets the message `{'EXIT', Pid, noconnection}`, and
    // the process that does not trap exits ends with the reason
    // `noconnection` (erts_do_net_exits, dist.c:1021-1117,
    // erl_proc_sig_queue.c:4632-4815). A process monitor on the same
    // connection also gets a DOWN message with reason `noconnection`
    // (dist.c:305-321). Each pair of ERTS and PBeam nodes
    // runs over the prim_socket carrier and over the tcp_inet port
    // controller. A server node serves one client node, because the client
    // stops it.
    let epmd = Epmd::start();
    let id = std::process::id();
    let expected = "[{linked,noconnection,noconnection,noconnection}]\n";
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

#[test]
fn synchronizes_global_names_and_pg_members_in_visible_clusters_as_pinned_erts() {
    // global.erl:338-452, 487-500; pg.erl:193-215, 225-259, 360-392.
    let epmd = Epmd::start();
    let id = std::process::id();
    for (servers_pbeam, client_pbeam) in [(false, false), (true, false), (false, true)] {
        let name_a = format!("dga{servers_pbeam}{client_pbeam}{id}");
        let name_b = format!("dgb{servers_pbeam}{client_pbeam}{id}");
        let start = |name: &str| {
            let args = ["-sname", name, "-setcookie", "ck", "-pa", "fixtures/dist"];
            if servers_pbeam {
                pbeam_node(&epmd, &args, "dist_global_pg_probe:server(), halt().")
            } else {
                ErtsNode::start(&epmd, &args, "dist_global_pg_probe:server(), halt().")
            }
        };
        let a = start(&name_a);
        let b = start(&name_b);
        let expression = format!(
            "dist_global_pg_probe:run('{}','{}'), halt().",
            a.line(),
            b.line()
        );
        let client_name = format!("dgc{servers_pbeam}{client_pbeam}{id}");
        let args = [
            "-sname",
            &client_name,
            "-setcookie",
            "ck",
            "-noshell",
            "-pa",
            "fixtures/dist",
            "-eval",
            &expression,
        ];
        let output = if client_pbeam {
            pbeam_erl(&epmd, &args)
        } else {
            erts_erl(&epmd, &args)
        };
        assert_eq!(
            stdout(&output),
            "cluster: ok\n",
            "servers_pbeam={servers_pbeam}, client_pbeam={client_pbeam}"
        );
    }
}

#[test]
fn decodes_fun_creator_pids_and_rejects_other_terms_as_pinned_erts() {
    // ERTS validates and discards the creator (external.c:5153-5160),
    // then writes the local init pid when it encodes the fun (external.c:3989).
    let epmd = Epmd::start();
    let arguments = [
        "-noshell",
        "-pa",
        "fixtures/dist",
        "-eval",
        "dist_fun_creator_probe:run(), halt().",
    ];
    let expected = "[{true,true,42},{true,true,42},{error,badarg}]\n";
    assert_eq!(stdout(&erts_erl(&epmd, &arguments)), expected);
    assert_eq!(stdout(&pbeam_erl(&epmd, &arguments)), expected);
}

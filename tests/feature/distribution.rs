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
    let exit = probe(&[], "dist_channel_probe:net_kernel_exit(), halt().");
    let erts = stdout(&erts_erl(&epmd, &exit));
    assert!(erts.contains("stopped: [killed,nonode@nohost,"), "{erts}");
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
    // A port controller (#1406), flags without a mandatory flag, where ERTS
    // sends an error report (dist.c:5176-5187), and a distribution message
    // (#1407) stop the run with a runtime error.
    let epmd = Epmd::start();
    let name = format!("dstop{}", std::process::id());
    for (function, error) in [
        (
            "port_controller",
            "unsupported erts_internal:create_dist_channel/3 with a port controller",
        ),
        (
            "no_mandatory",
            "unsupported erts_internal:create_dist_channel/3 with flags that miss a mandatory flag",
        ),
        (
            "message",
            "unsupported erlang:dist_ctrl_put_data/2 of a distribution message",
        ),
    ] {
        let expression = format!("dist_channel_probe:{function}(), halt().");
        let output = pbeam_erl(&epmd, &probe(&["-sname", &name], &expression));
        let stderr = String::from_utf8_lossy(&output.stderr);
        assert!(
            !output.status.success() && stderr.contains(error),
            "{function}: {output:?}\nstderr: {stderr}"
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
    assert_eq!(node.line(), format!("['{pbeam}@{host}']"));
    assert_eq!(node.line(), "[]");
    let erts_output = stdout(&erts_erl(&epmd, &client(&erts, "ck")));
    assert_eq!(node.line(), format!("['{erts}@{host}']"));
    assert_eq!(node.line(), "[]");
    let connected = format!("true\n['{server}']\n['{server}']\n");
    assert_eq!(erts_output, connected);
    assert_eq!(pbeam_output, connected);
    let refused = "false\n[]\n[]\n";
    assert_eq!(stdout(&erts_erl(&epmd, &client(&erts, "bad"))), refused);
    assert_eq!(stdout(&pbeam_erl(&epmd, &client(&pbeam, "bad"))), refused);
}

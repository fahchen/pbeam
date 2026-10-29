use std::{
    io::{Read, Write},
    net::{SocketAddr, TcpStream},
    path::{Path, PathBuf},
    process::Command,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    thread,
    time::Duration,
};

use pbeam_runtime::{AllowAll, AtomTable, BeamProgram, Term};

#[path = "../common/mod.rs"]
mod common;

use common::pinned_beam;

const TCP_PROBE: &[u8] = include_bytes!("../../fixtures/tcp/tcp_probe.beam");

const TCP_ACCEPT: &[u8] = include_bytes!("../../fixtures/tcp/tcp_accept.beam");

const UDP_PROBE: &[u8] = include_bytes!("../../fixtures/tcp/udp_probe.beam");

const TCP_TRANSFER: &[u8] = include_bytes!("../../fixtures/tcp/tcp_transfer.beam");

const TCP_ACTIVE: &[u8] = include_bytes!("../../fixtures/tcp/tcp_active.beam");

const TCP_CLOSE: &[u8] = include_bytes!("../../fixtures/tcp/tcp_close.beam");

const TCP_LOCAL: &[u8] = include_bytes!("../../fixtures/tcp/tcp_local.beam");
const UDP_LOCAL: &[u8] = include_bytes!("../../fixtures/tcp/udp_local.beam");

/// The pinned OTP modules that the probes call: `erlang` has
/// `open_port/2`, `port_control/3`, `port_close/1`, and `port_info/1`,
/// which call the `erts_internal` port BIFs, and `prim_inet` makes the
/// requests of `gen_tcp` (prim_inet.erl). `gen_tcp:controlling_process/2`
/// calls `inet_db:lookup_socket/1` and `inet:tcp_controlling_process/2`.
/// `timer` has `sleep/1`.
const MODULES: [&str; 8] = [
    "erlang",
    // `erlang:demonitor/2` with `flush` traps to
    // `erts_internal:flush_monitor_messages/3` after a port `DOWN`.
    "erts_internal",
    "lists",
    "prim_inet",
    "gen_tcp",
    "inet",
    "inet_db",
    "timer",
];

/// A policy module whose `initial/1` gives no capability.
const PROCESSES_POLICY: &[u8] = include_bytes!("../../fixtures/processes/processes_policy.beam");

/// Return a term as `io:format("~w")` prints it, for the atoms, integers,
/// binaries, lists, and tuples of the probe results. An atom that does not
/// start with a lowercase letter is quoted. Each atom name comes from
/// `atoms`, the table of the run that owns the term.
///
/// # Examples
///
/// ```rust,ignore
/// assert_eq!(text(&Term::binary(b"hi".to_vec()), &atoms), "<<104,105>>");
/// ```
fn text(term: &Term, atoms: &AtomTable) -> String {
    let join = |terms: &[Term]| {
        terms
            .iter()
            .map(|term| text(term, atoms))
            .collect::<Vec<_>>()
            .join(",")
    };
    match term {
        Term::Atom(atom)
            if atoms
                .name_of(atom)
                .starts_with(|c: char| c.is_ascii_lowercase()) =>
        {
            atoms.name_of(atom).to_owned()
        }
        Term::Atom(atom) => format!("'{}'", atoms.name_of(atom)),
        Term::Integer(value) => value.to_string(),
        Term::Binary(bytes) => format!(
            "<<{}>>",
            bytes
                .bytes()
                .expect("the term is a binary of whole bytes")
                .iter()
                .map(u8::to_string)
                .collect::<Vec<_>>()
                .join(",")
        ),
        _ if let Some(terms) = term.proper_list() => {
            format!("[{}]", join(&terms.cloned().collect::<Vec<_>>()))
        }
        Term::Tuple(terms) => format!("{{{}}}", join(terms)),
        other => panic!("unexpected term {other:?}"),
    }
}

/// Load the probes with the pinned modules of [`MODULES`].
fn load() -> BeamProgram {
    let beams: Vec<Vec<u8>> = MODULES.iter().map(|module| pinned_beam(module)).collect();
    let mut all: Vec<&[u8]> = vec![
        TCP_PROBE,
        TCP_ACCEPT,
        UDP_PROBE,
        TCP_TRANSFER,
        TCP_ACTIVE,
        TCP_CLOSE,
        TCP_LOCAL,
        UDP_LOCAL,
    ];
    all.extend(beams.iter().map(Vec::as_slice));
    BeamProgram::load(&all).unwrap()
}

/// Run `tcp_probe:Function()` on the pinned ERTS with one scheduler, and on
/// PBeam, and check that both give `expected` as `~w` text.
///
/// # Examples
///
/// ```rust,ignore
/// assert_runs("data", "[undefined,true,...]");
/// ```
fn assert_runs(function: &str, expected: &str) {
    assert_eq!(
        erts_text("tcp_probe", function),
        expected,
        "ERTS {function}"
    );
    assert_eq!(
        pbeam_text("tcp_probe", function),
        expected,
        "PBeam {function}"
    );
}

/// Return the `~w` text of `Module:Function()` on the pinned ERTS with one
/// scheduler.
///
/// # Examples
///
/// ```rust,ignore
/// assert_eq!(erts_text("tcp_probe", "enabled"), "[1,0,0,0,0,4,10,0,0,0,4]");
/// ```
fn erts_text(module: &str, function: &str) -> String {
    let reference = Command::new("erl")
        .args([
            "+S",
            "1",
            "-noshell",
            "-pa",
            "fixtures/tcp",
            "-eval",
            &format!("io:format(\"~w~n\", [{module}:{function}()]), halt()."),
        ])
        .output()
        .unwrap();
    assert!(reference.status.success(), "{reference:?}");
    let text = String::from_utf8(reference.stdout).unwrap();
    text.strip_suffix('\n').unwrap().to_owned()
}

/// Return the `~w` text of `Module:Function()` on PBeam.
///
/// # Examples
///
/// ```rust,ignore
/// assert_eq!(pbeam_text("tcp_probe", "enabled"), "[1,0,0,0,0,4,10,0,0,0,4]");
/// ```
fn pbeam_text(module: &str, function: &str) -> String {
    let result = load().execute(module, function, &[]).unwrap();
    text(&result.term, result.atoms())
}

#[test]
fn gives_the_reply_bytes_of_each_request_as_pinned_erts() {
    // The error of each argument and state check of `OPEN`, `BIND`,
    // `LISTEN`, and `NAME` (inet_drv.c:5119-5221, 11360-11437,
    // 11961-12062), `SETOPTS` and `GETOPTS` before and after `OPEN`
    // (inet_drv.c:6871-7060, 9071-9754), the 64 KiB limit of a reply
    // (inet_drv.c:1046, 11025-11044), and the `badarg` of a code that is
    // not `subs_empty_out_q` (inet_drv.c:10819-10848) and of each argument
    // that `erts_internal:port_control/3` rejects (erl_bif_port.c:322-376).
    // A bind of a port that is not 0 replies with that port. The limit
    // also counts the second 4 bytes of each `linger` (inet_drv.c:9749).
    assert_runs(
        "control",
        "[[[0,101,98,97,100,102],[0,101,120,98,97,100,112,111,114,116],[0,101,120,98,97,100,112,111,114,116],[0,101,105,110,118,97,108],[1],[1,24,0,0,0,1,22,0,0,0,0],[1]],[[0,101,105,110,118,97,108],[0,101,97,102,110,111,115,117,112,112,111,114,116],[0,101,105,110,118,97,108],[0,101,112,114,111,116,111,110,111,115,117,112,112,111,114,116],[1],[0,101,120,98,97,100,115,101,113]],[[1],[1,24,0,0,0,1,22,0,0,0,2,30,0,0,117,48,35,0,0,0,1,0,0,0,0,0,10,0,0,0,0,3,0,0,0,0,0,0,0,0],65536,[0,101,105,110,118,97,108],65530,[0,101,105,110,118,97,108]],[1,1,0,0,0,0,0,0],[[0,101,105,110,118,97,108],[0,101,97,102,110,111,115,117,112,112,111,114,116],[0,101,105,110,118,97,108]],true,true,[[0,101,105,110,118,97,108],[1],[0,101,120,98,97,100,112,111,114,116],[0,101,120,98,97,100,115,101,113]],[[1,1,0,0,0,0],[1,1,0,0,0,0,1,0,0,0,0],[1],{error,badarg},true,[]],[[1],[0,101,97,100,100,114,105,110,117,115,101]],[{error,badarg},{error,badarg},{error,badarg},{error,badarg},{error,badarg},{error,badarg}],true,[{'EXIT',port,normal},{'EXIT',port,normal},{'EXIT',port,normal},{'EXIT',port,normal}]]",
    );
}

#[test]
fn gives_the_getsockopt_value_of_an_option_that_is_on_as_pinned_erts() {
    // `GETOPTS` gives the value of `getsockopt` as is (inet_drv.c:9727-9750):
    // the flag bit of the option on macOS, 1 on Linux.
    let on = if cfg!(target_vendor = "apple") { 4 } else { 1 };
    assert_runs("enabled", &format!("[1,0,0,0,0,{on},10,0,0,0,{on}]"));
}

#[test]
#[cfg(not(target_vendor = "apple"))]
fn sets_a_linger_time_as_pinned_erts() {
    // `SETOPTS` of `linger` takes the time after the flag, and `GETOPTS`
    // gives both (inet_drv.c:7360-7385, 9749-9752).
    assert_runs("linger", "[[1],[1,3,0,0,0,1,0,0,0,5]]");
}

#[test]
#[cfg(target_vendor = "apple")]
fn stops_the_run_for_a_linger_time_on_macos() {
    // `socket2` sets `SO_LINGER_SEC` on macOS, and ERTS sets `SO_LINGER`,
    // which xnu reads in clock ticks.
    let program = load();
    let argument = Term::Atom(program.atom("linger").unwrap());
    let error = program
        .execute("tcp_probe", "unsupported", &[program.host_term(argument)])
        .unwrap_err()
        .to_string();
    assert!(
        error.starts_with("unsupported tcp_inet option linger with a time"),
        "{error}"
    );
}

#[test]
fn keeps_the_port_data_and_names_the_driver_as_pinned_erts() {
    // The data starts as `undefined` (erl_bif_port.c:646). A port of the
    // spawn driver has data, and its control rejects each command other than
    // `0x04c76a00` (sys_drivers.c:731-756).
    assert_runs(
        "data",
        "[undefined,true,{inet_tcp,[1,<<120>>]},[{name,[116,99,112,95,105,110,101,116]},{input,0},{output,0},{os_pid,undefined}],[{error,badarg},{error,badarg},{error,badarg},{error,badarg}],[[116,99,112,95,105,110,101,116],[116,99,112,95,105,110,101,116]],[undefined,true,cat,{error,badarg}],[{'EXIT',port,normal},{'EXIT',port,normal},{'EXIT',port,normal},{'EXIT',port,normal}]]",
    );
}

#[test]
fn stops_the_run_for_each_tcp_inet_part_that_is_not_implemented() {
    let program = load();
    for (case, message) in [
        ("sctp", "unsupported open_port/2 driver sctp_inet"),
        (
            "list",
            "unsupported open_port/2 settings of an inet driver port other than [binary]",
        ),
        ("mptcp", "unsupported tcp_inet protocol 4"),
        ("oobinline", "unsupported tcp_inet option 5"),
        ("recvttl", "unsupported tcp_inet option recvttl true"),
        // ERTS keeps the time of `{linger, {false, 5}}` (inet_drv.c:7362-7370).
        (
            "linger_off",
            "unsupported tcp_inet option linger with a time",
        ),
        (
            "active_n",
            "unsupported tcp_inet option active with a count",
        ),
        ("getopt", "unsupported tcp_inet option 5"),
        // ERTS replies `[]` and sets the exit state of the port
        // (sys_drivers.c:731-756).
        (
            "spawn_control",
            "unsupported erlang:port_control/3 command 16#04c76a00 on a spawn port",
        ),
    ] {
        let argument = Term::Atom(program.atom(case).unwrap());
        let error = program
            .execute("tcp_probe", "unsupported", &[program.host_term(argument)])
            .unwrap_err()
            .to_string();
        assert!(error.starts_with(message), "{case}: {error}");
    }
}

#[test]
fn stops_the_open_of_a_tcp_inet_port_in_a_run_with_a_policy() {
    // No capability kind permits a network effect yet, so a policy cannot
    // permit the listen (docs/tcp-socket-design.md, "Capability checks").
    let program = load();
    let argument = Term::Atom(program.atom("policy").unwrap());
    let error = program
        .execute_with_policy(
            "tcp_probe",
            "unsupported",
            &[program.host_term(argument)],
            &AllowAll,
        )
        .unwrap_err()
        .to_string();
    assert!(
        error.starts_with("unsupported inet driver port in a run with a policy"),
        "{error}"
    );
}

#[test]
fn gives_the_host_name_of_a_udp_inet_port_as_pinned_erts() {
    // The requests of `inet_config:set_hostname/0` (inet_config.erl:223-231)
    // in the order of a trace on pinned ERTS: `OPEN` (inet_drv.c:14577-14636),
    // `SETOPTS`, `BIND`, `GETHOSTNAME` (inet_drv.c:11219-11233), `GETOPTS`
    // of `linger`, and `SUBSCRIBE`, which `prim_inet:close/1` sends. The
    // reply of `GETHOSTNAME` is `[1 | Name]` with the name of the host.
    let host = String::from_utf8(Command::new("hostname").output().unwrap().stdout).unwrap();
    let name: Vec<String> = host
        .trim_end()
        .bytes()
        .map(|byte| byte.to_string())
        .collect();
    let expected = format!(
        "[[[1],[1],[1]],true,[1,{}],[[1,3,0,0,0,0,0,0,0,0],[1,1,0,0,0,0]],{{'EXIT',port,normal}}]",
        name.join(",")
    );
    assert_eq!(erts_text("udp_probe", "host_name"), expected, "ERTS");
    assert_eq!(pbeam_text("udp_probe", "host_name"), expected, "PBeam");
}

#[test]
fn gives_the_open_and_listen_errors_of_a_udp_inet_port_as_pinned_erts() {
    // `packet_inet_ctl` checks the size, the family, the type, and then the
    // protocol (inet_drv.c:14577-14636), and replies `exbadport` to
    // `LISTEN` of a socket that is not SCTP (inet_drv.c:14846-14858). A
    // stream socket with the UDP protocol gives the errno of the host.
    let expected = erts_text("udp_probe", "open");
    let prototype = if cfg!(target_vendor = "apple") {
        "eprototype"
    } else {
        "eprotonosupport"
    };
    let bytes: Vec<String> = prototype.bytes().map(|byte| byte.to_string()).collect();
    assert!(
        expected.contains(&format!("[0,{}]", bytes.join(","))),
        "{expected}"
    );
    assert_eq!(pbeam_text("udp_probe", "open"), expected, "PBeam");
}

#[test]
fn stops_the_run_for_each_udp_inet_part_that_is_not_implemented() {
    let program = load();
    for (case, message) in [
        ("stream", "unsupported udp_inet type 1"),
        ("seqpacket", "unsupported udp_inet type 3"),
        ("sctp", "unsupported udp_inet protocol 3"),
        ("accept", "unsupported udp_inet request 26"),
        (
            "disconnect",
            "unsupported udp_inet request CONNECT without an address",
        ),
        (
            "ancillary",
            "unsupported udp_inet output with ancillary data",
        ),
        ("recvtos", "unsupported udp_inet option recvtos with true"),
        (
            "short_count",
            "unsupported udp_inet option active with a count of fewer than 2 bytes",
        ),
        (
            "short_membership",
            "unsupported udp_inet option membership with fewer than 12 bytes",
        ),
        (
            "short_membership6",
            "unsupported udp_inet option membership with fewer than 20 bytes",
        ),
    ] {
        let argument = Term::Atom(program.atom(case).unwrap());
        let error = program
            .execute("udp_probe", "unsupported", &[program.host_term(argument)])
            .unwrap_err()
            .to_string();
        assert!(error.starts_with(message), "{case}: {error}");
    }
}

#[test]
fn runs_an_ipv6_udp_inet_port_as_pinned_erts() {
    // `OPEN` with `inet6` (inet_drv.c:14585-14595), a send and a receive on
    // `::1` (inet_drv.c:4726-4745, 4905-4913), a send to the tag
    // `loopback` and a bind to the tags `any` and `loopback`
    // (inet_drv.c:4817-4875). Both runtimes read `ipv6_v6only` from the
    // kernel (inet_drv.c:7806-7823). Linux sets it on a bind to a specific
    // IPv6 address such as `::1` (net/ipv6/af_inet6.c, `__inet6_bind`);
    // macOS leaves it off.
    let only = cfg!(target_os = "linux").to_string();
    let expected = "[true,ok,{ok,{{0,0,0,0,0,0,0,1},true,<<104,105>>}},ok,{ok,{{0,0,0,0,0,0,0,1},true,<<108>>}},{ok,[{ipv6_v6only,ONLY}]},[{{0,0,0,0,0,0,0,0},{ok,[{ipv6_v6only,false}]}},{{0,0,0,0,0,0,0,1},{ok,[{ipv6_v6only,ONLY}]}}]]"
        .replace("ONLY", &only);
    assert_eq!(erts_text("udp_probe", "inet6"), expected, "ERTS");
    assert_eq!(pbeam_text("udp_probe", "inet6"), expected, "PBeam");
}

#[test]
fn sends_and_receives_datagrams_on_a_passive_udp_inet_port_as_pinned_erts() {
    // The `prim_inet` calls of `gen_udp:send/4` and `gen_udp:recv/2,3` (inet_drv.c:14968-14998,
    // 15037-15316, 15361-15576): the waiter gets its datagram from the IO
    // poll, and the output and `PACKET_REQ_RECV` errors match.
    let expected = "[ok,{ok,{{127,0,0,1},true,<<104,105>>}},[{error,timeout},{error,timeout}],{ok,{{127,0,0,1},true,<<>>}},{ok,{{127,0,0,1},true,[97,98]}},{error,ealready},{ok,{{127,0,0,1},true,[119]}},{ok,[{recv_oct,5},{recv_cnt,4},{recv_max,2},{recv_avg,1},{recv_dvi,0},{send_oct,5},{send_cnt,4},{send_max,2},{send_avg,1},{send_pend,0}]},[{error,einval},{error,eafnosupport},{error,einval},{error,einval},{error,einval}],[[0,101,105,110,118,97,108],[1],[0,101,105,110,118,97,108]],[{error,einval},[0,101,120,98,97,100,112,111,114,116]],{'EXIT',port,einval}]";
    assert_eq!(erts_text("udp_probe", "passive"), expected, "ERTS");
    assert_eq!(pbeam_text("udp_probe", "passive"), expected, "PBeam");
}

#[test]
fn truncates_a_long_datagram_as_pinned_erts() {
    // The read takes at most the 9216 bytes of the buffer
    // (inet_drv.c:15469-15470); macOS sends at most 9216 bytes.
    let expected = if cfg!(target_vendor = "apple") {
        "{error,emsgsize}"
    } else {
        "{ok,9216,{ok,[{recv_oct,9216},{send_oct,9300}]}}"
    };
    assert_eq!(erts_text("udp_probe", "truncate"), expected, "ERTS");
    assert_eq!(pbeam_text("udp_probe", "truncate"), expected, "PBeam");
}

#[test]
fn gives_the_errno_of_a_failed_datagram_send_as_pinned_erts() {
    // A failed `sendto` gives its errno (inet_drv.c:15240-15253): xnu
    // rejects the port 0 with `EADDRNOTAVAIL`, and Linux with `EINVAL`.
    let expected = if cfg!(target_vendor = "apple") {
        "{error,eaddrnotavail}"
    } else {
        "{error,einval}"
    };
    assert_eq!(erts_text("udp_probe", "port_zero"), expected, "ERTS");
    assert_eq!(pbeam_text("udp_probe", "port_zero"), expected, "PBeam");
}

#[test]
fn gives_einval_for_the_count_of_active_at_the_getopts_limit_as_pinned_erts() {
    // `{active, N}` takes 7 reply bytes (inet_drv.c:9176-9184).
    let expected = "[[101,105,110,118,97,108],65533]";
    assert_eq!(erts_text("udp_probe", "active_options"), expected, "ERTS");
    assert_eq!(pbeam_text("udp_probe", "active_options"), expected, "PBeam");
}

#[test]
fn delivers_datagrams_to_the_owner_of_an_active_udp_inet_port_as_pinned_erts() {
    // `packet_inet_input` (inet_drv.c:15361-15588) sends `{udp, S, Address,
    // Port, Data}` for `true`, `once`, and `{active, N}`, which ends with
    // `{udp_passive, S}` (inet_drv.c:1551-1560, 4215-4239); `SETOPTS` adds
    // the count of `{active, N}` (inet_drv.c:6968-7000), read after the queued datagram arrives so the count does not depend on timing, and
    // `read_packets` below 1 gives `einval` (inet_drv.c:7138-7148).
    let expected = "[[{udp,socket,{127,0,0,1},true,<<97>>},{udp,socket,{127,0,0,1},true,<<98>>}],[{udp,socket,{127,0,0,1},true,[99]},none,{ok,[{active,false}]}],[{udp,socket,{127,0,0,1},true,[100]},{ok,[{active,1}]},ok,{udp,socket,{127,0,0,1},true,[101]},{udp_passive,socket},none,{ok,[{active,false}]}],[ok,{udp_passive,socket},{ok,[{active,false}]}],[ok,{error,einval},{ok,[{active,32767}]},ok,{ok,[{active,false}]},{udp_passive,socket}],[none,true],[{ok,[{read_packets,5}]},{error,einval},ok,{ok,[{read_packets,1}]}],[{udp,socket,{127,0,0,1},true,[103]},{udp,socket,{127,0,0,1},true,[104]},{udp,socket,{127,0,0,1},true,[105]},none]]";
    assert_eq!(erts_text("udp_probe", "active"), expected, "ERTS");
    assert_eq!(pbeam_text("udp_probe", "active"), expected, "PBeam");
}

#[test]
fn connects_a_udp_inet_port_as_pinned_erts() {
    // `CONNECT` sets the default peer (inet_drv.c:14710-14842), `PEER`
    // gives it (inet_drv.c:11272-11300), and a send of a connected socket
    // ignores its address (inet_drv.c:15142-15182); `any` and `loopback`
    // take only the port (inet_drv.c:4817-4844).
    let expected = "[{error,enotconn},ok,{ok,{{127,0,0,1},true}},ok,{ok,{{127,0,0,1},true,<<104,105>>}},{ok,{{127,0,0,1},true,<<112,101,101,114>>}},{error,timeout},[{ok,{{127,0,0,1},true,<<116,111>>}},{error,timeout}],{error,exbadport},[[0,101,105,110,118,97,108],[0,101,105,110,118,97,108],[0,101,97,102,110,111,115,117,112,112,111,114,116],[0,101,105,110,118,97,108],[0,101,120,98,97,100,112,111,114,116]],[ok,{ok,{{127,0,0,1},true,<<108>>}},{error,einval},{error,einval}]]";
    assert_eq!(erts_text("udp_probe", "connect"), expected, "ERTS");
    assert_eq!(pbeam_text("udp_probe", "connect"), expected, "PBeam");
}

#[test]
fn reconnects_a_udp_inet_port_as_pinned_erts_or_stops_on_linux() {
    // `CONNECT` of a connected socket dissolves the association first
    // (inet_drv.c:14819-14836); xnu does it in each connect, and Linux needs
    // an `AF_UNSPEC` connect, which safe Rust cannot make, also after a
    // failed connect (inet_drv.c:14832-14836).
    if cfg!(target_os = "linux") {
        let program = load();
        let error = program
            .execute("udp_probe", "reconnect", &[])
            .unwrap_err()
            .to_string();
        assert!(
            error.starts_with("unsupported udp_inet connect of a connected socket"),
            "{error}"
        );
        let argument = Term::Atom(program.atom("failed_connect").unwrap());
        let error = program
            .execute("udp_probe", "unsupported", &[program.host_term(argument)])
            .unwrap_err()
            .to_string();
        assert!(
            error.starts_with("unsupported failed udp_inet connect"),
            "{error}"
        );
        return;
    }
    let expected = "[ok,{error,eaddrnotavail},{error,enotconn},ok,{ok,{{127,0,0,1},true}},ok]";
    assert_eq!(erts_text("udp_probe", "reconnect"), expected, "ERTS");
    assert_eq!(pbeam_text("udp_probe", "reconnect"), expected, "PBeam");
}

#[test]
fn ends_a_waiting_receive_of_a_udp_inet_port_that_becomes_active_as_pinned_erts() {
    // An active socket sends each datagram to its owner, also with a
    // receive that waits (`packet_reply_binary_data`,
    // inet_drv.c:4335-4355). A socket that becomes passive after it, and a
    // change to passive, stop the timer and the read select
    // (inet_drv.c:15570-15573, 7960-7967); the timeout stops the read
    // select of an active socket (inet_drv.c:15021-15024).
    let expected = "[{[<<97>>],none,{error,closed}},{[<<97>>],none,{error,closed}},\
{[<<97>>,<<98>>],{error,timeout},none},{[],{error,timeout},none},\
{[],none,{error,closed}},{[<<97>>,<<98>>],{error,timeout},none}]";
    assert_eq!(erts_text("udp_probe", "active_waiter"), expected, "ERTS");
    assert_eq!(pbeam_text("udp_probe", "active_waiter"), expected, "PBeam");
}

#[test]
fn gives_econnrefused_on_a_connected_udp_inet_port_as_pinned_erts() {
    // A read error goes to the receive of a passive socket and to the
    // owner of an active socket as `{udp_error, S, Reason}`
    // (inet_drv.c:15473-15489).
    for (function, expected) in [
        ("refused_passive", "{error,econnrefused}"),
        ("refused_active", "{udp_error,socket,econnrefused}"),
    ] {
        assert_eq!(
            erts_text("udp_probe", function),
            expected,
            "ERTS {function}"
        );
        assert_eq!(
            pbeam_text("udp_probe", function),
            expected,
            "PBeam {function}"
        );
    }
}

/// A client of the test: it reads `"Port Count"` from the file `path` and
/// connects to `Port` on `127.0.0.1` until it has `Count` connections, and
/// then writes `Count` to the file of [`acknowledgement`]. It keeps each connection open until `stop`, and returns the local port of
/// each connection in order.
fn client(path: PathBuf, stop: Arc<AtomicBool>) -> thread::JoinHandle<Vec<u16>> {
    thread::spawn(move || {
        let ack = acknowledgement(&path);
        let temporary = PathBuf::from(format!("{}.ack.tmp", path.display()));
        let mut streams = Vec::new();
        while !stop.load(Ordering::SeqCst) {
            let text = std::fs::read_to_string(&path).unwrap_or_default();
            if let Some((port, count)) = text.trim().split_once(' ') {
                let address = SocketAddr::from(([127, 0, 0, 1], port.parse().unwrap()));
                while streams.len() < count.parse().unwrap() {
                    streams.push(TcpStream::connect(address).unwrap());
                }
                std::fs::write(&temporary, count).unwrap();
                std::fs::rename(&temporary, &ack).unwrap();
            }
            thread::sleep(Duration::from_millis(2));
        }
        streams
            .iter()
            .map(|stream| stream.local_addr().unwrap().port())
            .collect()
    })
}

/// Run `run` with a client of the test on the file `path`, and return the
/// result of `run` with each `{peer,P}` of a client port replaced by
/// `{peer,client_N}`, where N counts from 1 in connection order.
fn with_client(path: &Path, run: impl FnOnce() -> String) -> String {
    let _ = std::fs::remove_file(path);
    let _ = std::fs::remove_file(acknowledgement(path));
    let stop = Arc::new(AtomicBool::new(false));
    let handle = client(path.to_path_buf(), Arc::clone(&stop));
    let mut result = run();
    stop.store(true, Ordering::SeqCst);
    for (index, port) in handle.join().unwrap().into_iter().enumerate() {
        result = result.replace(
            &format!("{{peer,{port}}}"),
            &format!("{{peer,client_{}}}", index + 1),
        );
    }
    let _ = std::fs::remove_file(path);
    let _ = std::fs::remove_file(acknowledgement(path));
    result
}

/// Return the file where the client of the test writes `Count` after its
/// connections up to `Count`.
fn acknowledgement(path: &Path) -> PathBuf {
    PathBuf::from(format!("{}.ack", path.display()))
}

#[test]
fn accepts_in_many_processes_on_one_listen_socket_as_pinned_erts() {
    // Each connection goes to the first acceptor in the queue; an acceptor
    // that exits leaves it (inet_drv.c:12144-12258, 12732-12771,
    // 13461-13590). A timeout, the close of the listen socket, and the
    // options that `prim_inet:accept_opts/3` copies give the same values
    // (prim_inet.erl:488-535; inet_drv.c:11859-11890, 12547-12599).
    let directory = std::env::temp_dir();
    let erts_path = directory.join(format!("pbeam-tcp-accept-erts-{}", std::process::id()));
    let erts = with_client(&erts_path, || {
        let output = Command::new("erl")
            .args(["+S", "1", "-noshell", "-pa", "fixtures/tcp", "-eval"])
            .arg(format!(
                "io:format(\"~w~n\", [tcp_accept:main(\"{}\")]), halt().",
                erts_path.display()
            ))
            .output()
            .unwrap();
        assert!(output.status.success(), "{output:?}");
        String::from_utf8_lossy(&output.stdout)
            .trim_end()
            .to_owned()
    });
    let pbeam_path = directory.join(format!("pbeam-tcp-accept-pbeam-{}", std::process::id()));
    let pbeam = with_client(&pbeam_path, || {
        let program = load();
        let path = Term::list_from(
            pbeam_path
                .display()
                .to_string()
                .bytes()
                .map(|byte| Term::Integer(byte.into())),
        );
        let result = program
            .execute("tcp_accept", "main", &[program.host_term(path)])
            .unwrap();
        text(&result.term, result.atoms())
    });
    let options = "{ok,[{active,false},{nodelay,true},{keepalive,true},{delay_send,true},{priority,0},{linger,{false,0}},{reuseaddr,true},{tos,16},{ttl,33},{recvtos,true},{recvttl,false},{mode,binary},{send_timeout,30000},{send_timeout_close,true}]}";
    let accepted =
        |client: u8| format!("[ok,true,{{peer,client_{client}}},true,[true,true,true],{options}]");
    let expected = format!(
        "[[[[0,101,105,110,118,97,108],[0,101,110,111,116,99,111,110,110],[0,101,105,110,118,97,108]],[[0,101,105,110,118,97,108],[0,101,110,111,116,99,111,110,110],[1,1,0,0,0,0,8,0,0,0,0,9,0,0,0,0,31,0,0,0,0,43,0,0,0,0,46,0,0,0,64,47,0,0,0,0],[0,101,105,110,118,97,108]],[[0,101,105,110,118,97,108],[0,101,105,110,118,97,108]]],[{options},{{error,enotconn}},{{error,timeout}}],{{error,timeout}},{},{},{},[{{error,closed}},{{error,closed}}],[['EXIT',port,normal],['EXIT',port,normal],['DOWN',ref,port,port,normal],[inet_async,port,0,{{error,closed}}]]]",
        accepted(1),
        accepted(2),
        accepted(3),
    );
    assert_eq!(erts, expected, "ERTS");
    assert_eq!(pbeam, expected, "PBeam");
}

/// A client of the test that runs the commands of the probes: it reads
/// `"Step Command"` from the file `path`, runs each new step once, and then
/// writes `Step` to the file of [`acknowledgement`]. `connect Port` makes a
/// connection to `Port` on `127.0.0.1`, and each other command acts on the
/// last connection: `send Text` writes `Text`, `read Count` reads `Count`
/// bytes, `close` closes, `reset` closes with `linger` 0, so the peer
/// gets a reset, `shutdown` shuts down the write side, and `drain` reads
/// until the end of file and closes. It returns the bytes of each `read`
/// and `drain` in order.
fn command_client(path: PathBuf, stop: Arc<AtomicBool>) -> thread::JoinHandle<Vec<Vec<u8>>> {
    thread::spawn(move || {
        let ack = acknowledgement(&path);
        let temporary = PathBuf::from(format!("{}.ack.tmp", path.display()));
        let mut streams: Vec<TcpStream> = Vec::new();
        let mut reads = Vec::new();
        let mut done = String::new();
        while !stop.load(Ordering::SeqCst) {
            let text = std::fs::read_to_string(&path).unwrap_or_default();
            if let Some((step, command)) = text.trim_end().split_once(' ')
                && step != done
            {
                let (verb, argument) = command.split_once(' ').unwrap_or((command, ""));
                match verb {
                    "connect" => {
                        let address = SocketAddr::from(([127, 0, 0, 1], argument.parse().unwrap()));
                        streams.push(TcpStream::connect(address).unwrap());
                    }
                    "send" => streams
                        .last_mut()
                        .unwrap()
                        .write_all(argument.as_bytes())
                        .unwrap(),
                    "read" => {
                        let mut bytes = vec![0; argument.parse().unwrap()];
                        streams.last_mut().unwrap().read_exact(&mut bytes).unwrap();
                        reads.push(bytes);
                    }
                    "close" => drop(streams.pop()),
                    "shutdown" => streams
                        .last()
                        .unwrap()
                        .shutdown(std::net::Shutdown::Write)
                        .unwrap(),
                    "drain" => {
                        let mut bytes = Vec::new();
                        streams.pop().unwrap().read_to_end(&mut bytes).unwrap();
                        reads.push(bytes);
                    }
                    "reset" => {
                        let stream = streams.pop().unwrap();
                        socket2::SockRef::from(&stream)
                            .set_linger(Some(Duration::ZERO))
                            .unwrap();
                    }
                    other => panic!("unknown command {other}"),
                }
                std::fs::write(&temporary, step).unwrap();
                std::fs::rename(&temporary, &ack).unwrap();
                step.clone_into(&mut done);
            }
            thread::sleep(Duration::from_millis(2));
        }
        reads
    })
}

/// Run `tcp_transfer:Function(Path)` on the pinned ERTS and on PBeam, each
/// with a [`command_client`], and return the `~w` text and the reads of
/// the client of each run.
fn transfer_runs(function: &str) -> [(String, Vec<Vec<u8>>); 2] {
    client_runs("tcp_transfer", function)
}

/// Run `Module:Function(Path)` on the pinned ERTS and on PBeam, each with a
/// [`command_client`], and return the `~w` text and the reads of the client
/// of each run.
fn client_runs(module: &str, function: &str) -> [(String, Vec<Vec<u8>>); 2] {
    let directory = std::env::temp_dir();
    let id = std::process::id();
    let run = |name: &str, run: &dyn Fn(&Path) -> String| {
        let path = directory.join(format!("pbeam-{module}-{function}-{name}-{id}"));
        let _ = std::fs::remove_file(&path);
        let _ = std::fs::remove_file(acknowledgement(&path));
        let stop = Arc::new(AtomicBool::new(false));
        let handle = command_client(path.clone(), Arc::clone(&stop));
        let text = run(&path);
        stop.store(true, Ordering::SeqCst);
        let reads = handle.join().unwrap();
        let _ = std::fs::remove_file(&path);
        let _ = std::fs::remove_file(acknowledgement(&path));
        (text, reads)
    };
    let erts = run("erts", &|path| {
        let output = Command::new("erl")
            .args(["+S", "1", "-noshell", "-pa", "fixtures/tcp", "-eval"])
            .arg(format!(
                "io:format(\"~w~n\", [{module}:{function}(\"{}\")]), halt().",
                path.display()
            ))
            .output()
            .unwrap();
        assert!(output.status.success(), "{output:?}");
        String::from_utf8_lossy(&output.stdout)
            .trim_end()
            .to_owned()
    });
    let pbeam = run("pbeam", &|path| {
        let program = load();
        let result = program
            .execute(module, function, &[program.host_term(path_term(path))])
            .unwrap();
        text(&result.term, result.atoms())
    });
    [erts, pbeam]
}

/// Return the file name `path` as a list of bytes.
fn path_term(path: &Path) -> Term {
    Term::list_from(
        path.display()
            .to_string()
            .bytes()
            .map(|byte| Term::Integer(byte.into())),
    )
}

/// Return the 8 MiB that `tcp_transfer:double/2` makes from 16 bytes.
fn big() -> Vec<u8> {
    b"0123456789abcdef".repeat(1 << 19)
}

#[test]
fn receives_and_sends_in_passive_mode_as_pinned_erts() {
    // `TCP_REQ_RECV` of a length and of 0, its timeout, `ealready`, list
    // mode, the output queue and the wait of a send for the low watermark,
    // `GETSTAT`, and the end of file of the peer (prim_inet.erl:575-624,
    // 808-824; inet_drv.c:10732-10800, 12268-12325, 12657-12731,
    // 12783-12847, 13042-13240, 13736-13850, 14234-14370).
    let bytes = |text: &str| {
        text.bytes()
            .map(|byte| byte.to_string())
            .collect::<Vec<_>>()
            .join(",")
    };
    let stats = "{ok,[{recv_oct,14},{recv_cnt,6},{recv_max,5},{recv_avg,2},{recv_dvi,0},{send_oct,8388616},{send_cnt,4},{send_max,8388608},{send_avg,2097154},{send_pend,0}]}";
    let expected = format!(
        "[{{error,enotconn}},[{{ok,<<{abc}>>}},{{ok,<<{defgh}>>}},{{error,timeout}}],[{{error,timeout}},{{ok,<<120>>}},{{ok,<<121>>}}],{{error,ealready}},{{ok,<<{ok}>>}},{{ok,[{hi}]}},[[0,{einval}],[0,{enomem}],badarg,[]],[ok,ok],ok,true,ok,{stats},[{{error,closed}},{{error,enotconn}},{{error,closed}},{{error,enotconn}},{stats}],einval,[['EXIT',port,normal],['DOWN',ref,port,port,normal],[inet_async,port,0,{{error,closed}}]]]",
        abc = bytes("abc"),
        defgh = bytes("defgh"),
        ok = bytes("ok"),
        hi = bytes("hi"),
        einval = bytes("einval"),
        enomem = bytes("enomem"),
    );
    let mut tail = big();
    tail.extend(b"tail");
    for (name, (text, reads)) in ["ERTS", "PBeam"].into_iter().zip(transfer_runs("main")) {
        assert_eq!(text, expected, "{name}");
        assert!(reads == [b"pong".to_vec(), tail.clone()], "{name} reads");
    }
}

#[test]
fn ends_the_socket_at_a_reset_of_the_peer_as_pinned_erts() {
    // A failed write closes the socket: the next send and the next
    // receive give `closed` once (inet_drv.c:13613-13734, 12278-12289,
    // 12672-12686).
    let expected = "[{error,timeout},[{error,closed},{error,enotconn},{error,closed},{error,enotconn},{ok,[{send_pend,0}]}],[{error,closed},{error,closed},{error,enotconn},{error,enotconn}],[]]";
    for (name, (text, _)) in ["ERTS", "PBeam"].into_iter().zip(transfer_runs("reset")) {
        assert_eq!(text, expected, "{name}");
    }
}

#[test]
fn ends_a_waiting_send_and_receive_at_the_close_of_the_peer_as_pinned_erts() {
    // `tcp_recv_closed` replies `closed` to the send that waits and ends
    // the receive (inet_drv.c:12783-12847).
    for (name, (text, _)) in ["ERTS", "PBeam"]
        .into_iter()
        .zip(transfer_runs("busy_close"))
    {
        assert_eq!(text, "[{error,closed},{error,closed}]", "{name}");
    }
}

#[test]
fn stops_the_run_for_each_receive_and_send_part_that_is_not_implemented() {
    for (case, message) in [
        ("length", "unsupported tcp_inet receive length -1"),
        (
            "header",
            "unsupported tcp_inet output header other than term_to_binary(Ref, [local])",
        ),
        ("delay_send", "unsupported tcp_inet output with delay_send"),
    ] {
        let path = std::env::temp_dir().join(format!(
            "pbeam-tcp-unsupported-{case}-{}",
            std::process::id()
        ));
        let _ = std::fs::remove_file(&path);
        let _ = std::fs::remove_file(acknowledgement(&path));
        let stop = Arc::new(AtomicBool::new(false));
        let handle = command_client(path.clone(), Arc::clone(&stop));
        let program = load();
        let arguments = [
            program.host_term(path_term(&path)),
            program.host_term(Term::Atom(program.atom(case).unwrap())),
        ];
        let error = program
            .execute("tcp_transfer", "unsupported", &arguments)
            .unwrap_err()
            .to_string();
        stop.store(true, Ordering::SeqCst);
        handle.join().unwrap();
        assert!(error.starts_with(message), "{case}: {error}");
    }
}

#[test]
fn ends_the_send_that_waits_with_a_timeout_as_pinned_erts() {
    // `tcp_inet_send_timeout` replies `{error, timeout}` to the send that
    // waits for the queue, and `send_timeout_close` closes the passive
    // socket without a message, so the next send gives `enotconn`
    // (inet_drv.c:12458-12534, measured case c of the `send_timeout`
    // research).
    for (name, (text, _)) in ["ERTS", "PBeam"]
        .into_iter()
        .zip(transfer_runs("busy_timeout"))
    {
        assert_eq!(text, "[{error,timeout},{error,enotconn},[]]", "{name}");
    }
}

#[test]
fn closes_a_connection_from_each_side_as_pinned_erts() {
    // `prim_inet:close/1` waits for `{empty_out_q, S}` of `SUBSCRIBE`, and
    // a port closed with queued output writes its queue in the `CLOSING`
    // state after its exit signals (prim_inet.erl:175-275;
    // inet_drv.c:10795-10848, 12708-12730; io.c:3922-3948).
    let expected = "[[{ok,[{linger,{false,0}}]},[1,1,0,0,0,0],ok,undefined],[true,ok,undefined,[['EXIT',pid,normal],['EXIT',port,normal],[empty_out_q,port]]],[[true,normal,normal,true,{links,[]},true,{error,badarg},{error,badarg},{error,badarg},{error,badarg},true,noproc],noproc,undefined],shutdown,[{error,closed},ok,undefined],[normal,ok],[tcp_closed,normal],[]]";
    for (name, (text, reads)) in ["ERTS", "PBeam"]
        .into_iter()
        .zip(client_runs("tcp_close", "main"))
    {
        assert_eq!(text, expected, "{name}");
        // The end of the port at the shutdown of the client drops the queue
        // (`driver_failure_term`, io.c:7208-7216), so the client gets only
        // the bytes that the kernel took.
        let lengths: Vec<usize> = reads.iter().map(Vec::len).collect();
        assert_eq!(lengths[..4], [2, 1 << 23, 1 << 23, 1 << 23], "{name} reads");
        assert!(lengths[4] < 1 << 23, "{name} reads");
        assert!(
            reads[0] == b"hi" && reads[1..4].iter().all(|read| *read == big()),
            "{name} reads"
        );
        assert!(big().starts_with(&reads[4]), "{name} reads");
    }
}

#[test]
fn delivers_in_active_once_mode_and_moves_the_owner_as_pinned_erts() {
    // `{active, once}` reads in the setopts and in the IO poll, and
    // `gen_tcp:controlling_process/2` moves the socket and its queued
    // message (inet.erl:4595-4660; inet_drv.c:3987-4077, 7940-7990,
    // 11111-11148, 12783-12847, 13042-13128).
    let expected = "[[ok,{tcp,<<97,98,99>>},{ok,[{active,false}]}],[{ok,[{active,false}]},ok,{ok,[{active,once}]}],{error,not_owner},[true,true],ok,[true,true,ok,{ok,[{active,once}]}],{tcp,<<100,101,102>>},{tcp,<<103,104,105>>},[tcp_closed,none],[undefined,{error,closed}],[]]";
    for (name, (text, _)) in ["ERTS", "PBeam"]
        .into_iter()
        .zip(client_runs("tcp_active", "main"))
    {
        assert_eq!(text, expected, "{name}");
    }
}

// macOS gives `einval` for each `linger` value other than `{false, 0}`.
#[cfg(target_os = "linux")]
#[test]
fn closes_a_port_with_linger_zero_and_queued_output_as_pinned_erts() {
    // The close sets `CLOSING` before `tcp_inet_flush` drops the queue, so
    // the stop of the port sends no message to the receive that waits
    // (io.c:3659-3721, 3939-3948; inet_drv.c:12708-12730).
    let expected = "[{ok,[{linger,{true,0}}]},true,true,none,[['EXIT',port,normal]]]";
    for (name, (text, _)) in ["ERTS", "PBeam"]
        .into_iter()
        .zip(client_runs("tcp_close", "linger_zero"))
    {
        assert_eq!(text, expected, "{name}");
    }
}

#[test]
fn replies_to_a_close_message_when_the_flushed_port_terminates_as_pinned_erts() {
    // `{Pid, close}` sets `ERTS_PORT_SFLG_SEND_CLOSED`, and
    // `terminate_port` sends `{Port, closed}` after the queue is written
    // (io.c:3713-3719, 3753-3758).
    let expected = "[true,waiting,closed,[['EXIT',port,normal]]]";
    for (name, (text, _)) in ["ERTS", "PBeam"]
        .into_iter()
        .zip(client_runs("tcp_close", "message_close"))
    {
        assert_eq!(text, expected, "{name}");
    }
}

#[test]
fn delivers_each_read_in_active_mode_as_pinned_erts() {
    let expected =
        "[ok,{tcp,<<120>>},{tcp,<<121>>},{ok,[{active,true}]},[none,{ok,<<122>>}],{tcp,[119]},[]]";
    for (name, (text, _)) in ["ERTS", "PBeam"]
        .into_iter()
        .zip(client_runs("tcp_active", "active"))
    {
        assert_eq!(text, expected, "{name}");
    }
}

#[test]
fn ends_the_port_at_the_close_of_the_peer_in_active_mode_as_pinned_erts() {
    // `tcp_recv_closed` sends `{tcp_closed, S}`, and `driver_exit` unlinks
    // the owner and closes the port with the reason `normal`, also in the
    // reply of a setopts (inet_drv.c:12836-12844; io.c:7228-7249).
    let expected = "[ok,{tcp,<<98,121,101>>},ok,[[tcp_closed,port]],undefined]";
    for (name, (text, _)) in ["ERTS", "PBeam"]
        .into_iter()
        .zip(client_runs("tcp_active", "closed"))
    {
        assert_eq!(text, expected, "{name}");
    }
}

#[test]
fn ends_the_port_at_activation_after_a_passive_close_as_pinned_erts() {
    // A passive recv closes the socket at the end of file. A later
    // `{active, once}` or `{active, true}` sends `{tcp_closed, S}`, skips
    // the next options, and `driver_exit` closes the port with the reason
    // `normal` (inet_drv.c:7001-7011, 12783-12847; io.c:7228-7249).
    let expected = "[[{error,closed},[1],tcp_closed,{error,einval},undefined,[]],[{error,closed},ok,tcp_closed,{error,einval},undefined,[]]]";
    for (name, (text, _)) in ["ERTS", "PBeam"]
        .into_iter()
        .zip(client_runs("tcp_active", "passive_closed"))
    {
        assert_eq!(text, expected, "{name}");
    }
}

#[test]
fn links_and_connects_a_port_as_pinned_erts() {
    // link/1, unlink/1, erlang:port_connect/2, and erlang:port_info/2 with a
    // port (bif.c:186-232, 1252-1287; erl_bif_port.c:438-595; io.c:2160-2190,
    // 2355-2400, 3869-3872).
    let expected = "[[true,noproc],true,[true,true],true,true,true,{error,badarg},[true,true],[true],[true,{name,[116,99,112,95,105,110,101,116]},undefined],[{error,badarg},{error,badarg},{error,badarg},{error,badarg},{error,badarg},{error,badarg}],[killed,undefined],[true,noproc,{error,noproc}],[]]";
    assert_eq!(erts_text("tcp_active", "links"), expected, "ERTS");
    assert_eq!(pbeam_text("tcp_active", "links"), expected, "PBeam");
}

#[test]
fn gives_the_links_of_a_port_in_the_order_of_the_pinned_erts_tree() {
    // ERTS keeps the links in its red-black tree (`ml_cmp_keys`,
    // erl_monitor_link.c:91-233), walks it in post-order
    // (erl_rbtree.h:1343-1458), and conses each pid to the front
    // (erl_bif_info.c:3676-3696), so the order of the item depends on the
    // shape of the tree after each link and unlink. Each list of the result
    // gives the position of the linked process in the spawn order of the
    // probe, with the owner first. The last list is the order of the link
    // exit signals of a port that closes, which is the same walk
    // (`erts_deliver_port_exit`, io.c:3922-3925).
    let expected = "[[[1],[1,2],[2,3,1],[2,3,4,1],[2,4,5,3,1],[2,4,5,1],[2,5,6,4,1],[4,5,6,1],[4,5,6,1,3],[4,6,1,3],[4,6,2,3,1],[2,4,3,1]],[[1],[1,6],[5,6,1],[5,6,1,4],[4,5,1],[4,5,1,3],[4,5,2,3,1],[4,5,6,2,3,1],[5,6,2,3,1],[2,6,3,1],[2,4,6,3,1],[3,4,6,1]],[2,4,3,1],[3,4,2]]";
    assert_eq!(erts_text("tcp_active", "link_orders"), expected, "ERTS");
    assert_eq!(pbeam_text("tcp_active", "link_orders"), expected, "PBeam");
}

#[test]
fn stops_the_run_for_a_port_info_item_that_is_not_implemented() {
    let error = load()
        .execute("tcp_active", "unsupported_item", &[])
        .unwrap_err()
        .to_string();
    assert!(
        error.starts_with("unsupported erlang:port_info/2 item memory"),
        "{error}"
    );
}

#[test]
fn stops_the_run_for_an_active_count() {
    // `{active, N}` and its `tcp_passive` message (inet_drv.c:1551-1559,
    // 6975-6995) are not on the path of Thousand Island.
    let path = std::env::temp_dir().join(format!("pbeam-tcp-active-count-{}", std::process::id()));
    let _ = std::fs::remove_file(&path);
    let _ = std::fs::remove_file(acknowledgement(&path));
    let stop = Arc::new(AtomicBool::new(false));
    let handle = command_client(path.clone(), Arc::clone(&stop));
    let program = load();
    let arguments = [
        program.host_term(path_term(&path)),
        program.host_term(Term::Atom(program.atom("count").unwrap())),
    ];
    let error = program
        .execute("tcp_active", "unsupported", &arguments)
        .unwrap_err()
        .to_string();
    stop.store(true, Ordering::SeqCst);
    handle.join().unwrap();
    let _ = std::fs::remove_file(&path);
    let _ = std::fs::remove_file(acknowledgement(&path));
    assert!(
        error.starts_with("unsupported tcp_inet option active with a count"),
        "{error}"
    );
}

#[test]
fn stops_the_open_of_an_inet_port_in_a_runtime_process_of_a_boot_run() {
    // In boot mode, the root process is a boot process and can open an inet
    // port. After `pbeam_cap:enter_runtime/2`, it is a runtime process, and
    // no capability permits a network effect, so the open stops the run.
    let beams: Vec<Vec<u8>> = MODULES.iter().map(|module| pinned_beam(module)).collect();
    let mut all: Vec<&[u8]> = vec![TCP_PROBE, PROCESSES_POLICY];
    all.extend(beams.iter().map(Vec::as_slice));
    let program = BeamProgram::load(&all).unwrap();
    let argument = Term::Atom(program.atom("runtime").unwrap());
    let error = program
        .execute_with_policy_module(
            "tcp_probe",
            "unsupported",
            &[program.host_term(argument)],
            "processes_policy",
            true,
        )
        .unwrap_err()
        .to_string();
    assert!(
        error.starts_with("unsupported inet driver port in a run with a policy"),
        "{error}"
    );
}

#[test]
fn gives_the_reply_bytes_of_each_connect_check_as_pinned_erts() {
    // The connect of a listen socket gives the error of the host
    // (inet_drv.c:12129-12139): `eopnotsupp` on macOS, `eisconn` on Linux.
    let listen = if cfg!(target_os = "macos") {
        "[0,101,111,112,110,111,116,115,117,112,112]"
    } else {
        "[0,101,105,115,99,111,110,110]"
    };
    assert_runs(
        "connect",
        &format!(
            "[[0,101,120,98,97,100,112,111,114,116],[[0,101,105,110,118,97,108],[0,101,105,110,118,97,108],[0,101,97,102,110,111,115,117,112,112,111,114,116],[0,101,105,110,118,97,108]],{listen}]"
        ),
    );
}

#[test]
fn gives_the_bind_errors_of_a_local_socket_as_pinned_erts() {
    // `OPEN` with the TCP protocol makes the socket with the protocol 0
    // (inet_drv.c:5167). `BIND` has no `any` or `loopback` of `AF_UNIX`
    // (inet_drv.c:4822-4872), and fails for a short or long name
    // (inet_drv.c:4750-4768).
    let einval = "[0,101,105,110,118,97,108]";
    let expected = format!("[[1],{einval},{einval},{einval},{einval}]");
    assert_eq!(erts_text("tcp_local", "control"), expected);
    assert_eq!(pbeam_text("tcp_local", "control"), expected);
}

#[test]
fn runs_a_local_udp_inet_socket_as_pinned_erts() {
    // `OPEN` with the family 5 (inet_drv.c:14585-14595), `BIND`, `NAME`,
    // `CONNECT`, `PEER`, a send to a `local` address, and a passive and an
    // active receive with the `local` address of the sender
    // (inet_drv.c:1945-1948, 4915-4936). The probe binds fixed names in
    // `/tmp`, so the test removes them before and after each run.
    let names = ["a", "b", "c"].map(|name| format!("/tmp/pbeam_udp_local_{name}"));
    let clean = || {
        for name in &names {
            let _ = std::fs::remove_file(name);
        }
    };
    clean();
    let erts = erts_text("udp_local", "run");
    clean();
    let pbeam = pbeam_text("udp_local", "run");
    clean();
    // `~w` prints a binary as its bytes.
    let bin = |text: &str| {
        let bytes: Vec<String> = text.bytes().map(|byte| byte.to_string()).collect();
        format!("<<{}>>", bytes.join(","))
    };
    let [a, b, c] = names.each_ref().map(|name| bin(name));
    let (hi, yo, zz, q, r) = (bin("hi"), bin("yo"), bin("zz"), bin("q"), bin("r"));
    // Linux gives no address for the sender of an unbound socket, which
    // `inet_get_address` reads as `AF_UNSPEC` (inet_drv.c:4938-4941).
    let unbound = if cfg!(target_os = "linux") {
        "{unspec,<<>>}"
    } else {
        "{local,<<>>}"
    };
    let expected = format!(
        "[{{ok,{{local,{a}}}}},ok,{{udp,{{local,{a}}},0,{hi}}},{{ok,{{{{local,{b}}},0,{yo}}}}},{{error,enotconn}},ok,{{ok,{{local,{a}}}}},{{ok,{{{{local,{c}}},0,{zz}}}}},{{ok,{{{{local,{a}}},0,[108,105]}}}},{{ok,{{local,<<>>}}}},{{ok,{{{unbound},0,{q}}}}},{{udp,{unbound},0,{r}}}]"
    );
    assert_eq!(erts, expected, "ERTS");
    assert_eq!(pbeam, expected, "PBeam");
}

#[test]
fn sets_and_gets_the_udp_inet_socket_options_as_pinned_erts() {
    // `inet_set_opts` and `inet_fill_opts` (inet_drv.c:6958-6966,
    // 7293-7360, 7571-7587, 9156-9163, 9432-9446, 9487-9495). xnu and Linux
    // reject a `multicast_ttl` above 255 with `EINVAL`, which the driver
    // ignores.
    let expected = "{{ok,[{broadcast,false},{dontroute,false},{multicast_ttl,1},{multicast_loop,true},{buffer,9216}]},ok,{ok,[{broadcast,true},{dontroute,true},{multicast_ttl,255},{multicast_loop,false},{buffer,9216}]},ok,255,true,[{10000,true},{65536,true},{1,true},{1,true}]}";
    assert_eq!(erts_text("udp_probe", "options"), expected, "ERTS");
    assert_eq!(pbeam_text("udp_probe", "options"), expected, "PBeam");
}

#[test]
fn runs_the_udp_inet_multicast_options_as_pinned_erts() {
    // `multicast_if`, `add_membership`, and `drop_membership`
    // (inet_drv.c:7589-7802) and `GETOPTS` of `multicast_if`
    // (inet_drv.c:9497-9528), with a send to a group that a socket joined on
    // the loopback interface.
    let expected = "{[ok,ok],ok,{ok,{{127,0,0,1},true,<<109>>}},ok,{error,timeout},ok,100,[ok,{error,einval},ok,ok,{ok,[{buffer,9216}]},{error,einval},[0,101,105,110,118,97,108]],true,[{error,einval},{error,einval},{error,einval}],[[101,105,110,118,97,108],65535]}";
    assert_eq!(erts_text("udp_probe", "multicast"), expected, "ERTS");
    assert_eq!(pbeam_text("udp_probe", "multicast"), expected, "PBeam");
}

#[test]
fn gives_zero_for_the_bytes_of_multicast_if_that_erts_does_not_write() {
    // ERTS gives 4 bytes of reply memory that the driver does not write
    // (inet_drv.c:9502-9526), so only PBeam has one result.
    assert_eq!(
        pbeam_text("udp_probe", "multicast_if"),
        "{{ok,[{multicast_if,{0,0,0,0}}]},{ok,[{multicast_if,0}]}}"
    );
}

#[test]
fn sets_the_buffer_of_a_tcp_inet_port_from_recbuf_as_pinned_erts() {
    // A stream socket has no limit of 65536 (inet_drv.c:7333-7359).
    let expected = "{{100000,true},{100,true}}";
    assert_eq!(erts_text("udp_probe", "stream_buffer"), expected, "ERTS");
    assert_eq!(pbeam_text("udp_probe", "stream_buffer"), expected, "PBeam");
}

#[test]
fn runs_a_send_that_waits_when_the_busy_port_drains_as_pinned_erts() {
    // The busy send gets `ok` at the low watermark of a queue without the
    // bytes of the send that waits behind it, which join the queue only when
    // that send runs (inet_drv.c:13771-13792, 14350-14359).
    for (name, (text, reads)) in ["ERTS", "PBeam"]
        .into_iter()
        .zip(transfer_runs("busy_drain"))
    {
        assert_eq!(text, "[8388612,ok,ok,[['EXIT',port,killed]]]", "{name}");
        assert_eq!(
            reads.iter().map(Vec::len).collect::<Vec<_>>(),
            [(16 << 19) + 4],
            "{name} reads"
        );
    }
}

#[test]
fn ends_a_send_that_waits_behind_a_busy_port_as_pinned_erts() {
    // The end of the port gives the send that waits `{error, einval}`
    // (prim_inet.erl:625-628), and a passive close gives `{error, enotconn}`
    // when it runs (inet_drv.c:12681-12693).
    for (function, expected) in [
        (
            "busy_end",
            "[{error,timeout},{error,einval},[[tcp_closed,port]]]",
        ),
        (
            "busy_kill",
            "[{error,closed},{error,einval},[['EXIT',port,killed]]]",
        ),
        (
            "busy_reset",
            "[{error,closed},{error,enotconn},{error,enotconn},[]]",
        ),
    ] {
        for (name, (text, _)) in ["ERTS", "PBeam"].into_iter().zip(transfer_runs(function)) {
            assert_eq!(text, expected, "{function} {name}");
        }
    }
}

use std::process::Command;

use pbeam_runtime::{AllowAll, AtomTable, BeamProgram, Term};

#[path = "../common/mod.rs"]
mod common;

use common::pinned_beam;

const TCP_PROBE: &[u8] = include_bytes!("../../fixtures/tcp/tcp_probe.beam");

const UDP_PROBE: &[u8] = include_bytes!("../../fixtures/tcp/udp_probe.beam");

/// The pinned OTP modules that the probe calls: `erlang` has
/// `open_port/2`, `port_control/3`, `port_close/1`, and `port_info/1`,
/// which call the `erts_internal` port BIFs.
const MODULES: [&str; 2] = ["erlang", "lists"];

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
    let mut all: Vec<&[u8]> = vec![TCP_PROBE, UDP_PROBE];
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
        (
            "command",
            "unsupported erlang:port_command/2 on a tcp_inet port",
        ),
        ("inet6", "unsupported tcp_inet family 2"),
        ("mptcp", "unsupported tcp_inet protocol 4"),
        ("any", "unsupported tcp_inet bind address family 3"),
        ("keepalive", "unsupported tcp_inet option 1"),
        (
            "active_n",
            "unsupported tcp_inet option active with a count",
        ),
        ("getopt", "unsupported tcp_inet option 1"),
        ("accept", "unsupported tcp_inet request 26"),
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
    ] {
        let argument = Term::Atom(program.atom(case).unwrap());
        let error = program
            .execute("udp_probe", "unsupported", &[program.host_term(argument)])
            .unwrap_err()
            .to_string();
        assert!(error.starts_with(message), "{case}: {error}");
    }
}

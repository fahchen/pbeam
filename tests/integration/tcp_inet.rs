use std::{
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

/// The pinned OTP modules that the probes call: `erlang` has
/// `open_port/2`, `port_control/3`, `port_close/1`, and `port_info/1`,
/// which call the `erts_internal` port BIFs, and `prim_inet` makes the
/// requests of `gen_tcp` (prim_inet.erl).
const MODULES: [&str; 3] = ["erlang", "lists", "prim_inet"];

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
    let mut all: Vec<&[u8]> = vec![TCP_PROBE, TCP_ACCEPT, UDP_PROBE];
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
        (
            "command",
            "unsupported erlang:port_command/2 on a tcp_inet port",
        ),
        ("inet6", "unsupported tcp_inet family 2"),
        ("mptcp", "unsupported tcp_inet protocol 4"),
        ("any", "unsupported tcp_inet bind address family 3"),
        ("dontroute", "unsupported tcp_inet option 2"),
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
        ("getopt", "unsupported tcp_inet option 2"),
        ("connect", "unsupported tcp_inet request 3"),
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
        "[[[[0,101,105,110,118,97,108],[0,101,110,111,116,99,111,110,110],[0,101,105,110,118,97,108]],[[0,101,105,110,118,97,108],[0,101,110,111,116,99,111,110,110],[1,1,0,0,0,0,8,0,0,0,0,9,0,0,0,0,31,0,0,0,0,43,0,0,0,0,46,0,0,0,64,47,0,0,0,0],[0,101,105,110,118,97,108]],[[0,101,105,110,118,97,108],[0,101,105,110,118,97,108]]],[{options},{{error,enotconn}},{{error,timeout}}],{{error,timeout}},{},{},{},[{{error,closed}},{{error,closed}}],[['EXIT',port,normal]]]",
        accepted(1),
        accepted(2),
        accepted(3),
    );
    assert_eq!(erts, expected, "ERTS");
    assert_eq!(pbeam, expected, "PBeam");
}

#[test]
fn stops_the_run_for_an_active_mode_on_an_accepted_socket() {
    // Task #668 adds the data messages of an active socket
    // (inet_drv.c:7530-7560, 13461).
    let path = std::env::temp_dir().join(format!("pbeam-tcp-active-{}", std::process::id()));
    let error = with_client(&path, || {
        let program = load();
        let argument = Term::list_from(
            path.display()
                .to_string()
                .bytes()
                .map(|byte| Term::Integer(byte.into())),
        );
        program
            .execute("tcp_accept", "active", &[program.host_term(argument)])
            .unwrap_err()
            .to_string()
    });
    assert!(
        error.starts_with("unsupported tcp_inet option active on a connected socket"),
        "{error}"
    );
}

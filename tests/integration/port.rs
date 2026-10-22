use std::process::Command;

use pbeam_runtime::{AtomTable, BeamProgram, Term};

#[path = "../common/mod.rs"]
mod common;

use common::pinned_beam;

const PORT_PROBE: &[u8] = include_bytes!("../../fixtures/port/port_probe.beam");

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

/// Run `port_probe:Call` on the pinned ERTS with one scheduler, and on
/// PBeam, and check that both give `expected` as `~w` text. The ERTS run
/// logs nothing: a `port_command/2` with data that is not iodata sends the
/// report `Bad value on output port` to the logger (io.c:1267-1271), which
/// PBeam does not write, and the logger prints it before or after the
/// result.
///
/// # Examples
///
/// ```rust,ignore
/// assert_runs("close()", "[true,normal,undefined]");
/// ```
fn assert_runs(call: &str, expected: &str) {
    let reference = Command::new("erl")
        .args([
            "+S",
            "1",
            "-noshell",
            "-kernel",
            "logger_level",
            "none",
            "-pa",
            "fixtures/port",
            "-eval",
            &format!("io:format(\"~w~n\", [port_probe:{call}]), halt()."),
        ])
        .output()
        .unwrap();
    assert!(reference.status.success(), "{reference:?}");
    assert_eq!(
        String::from_utf8_lossy(&reference.stdout),
        format!("{expected}\n"),
        "ERTS {call}"
    );
    let function = call.strip_suffix("()").unwrap();
    let result = load().execute("port_probe", function, &[]).unwrap();
    assert_eq!(text(&result.term, result.atoms()), expected, "PBeam {call}");
}

/// Load the probe with the pinned `erlang` module, whose `open_port/2`,
/// `port_command/2`, `port_close/1`, and `port_info/1` call the
/// `erts_internal` port BIFs.
fn load() -> BeamProgram {
    let erlang = pinned_beam("erlang");
    BeamProgram::load(&[PORT_PROBE, &erlang]).unwrap()
}

#[test]
fn reports_the_exit_status_and_closes_the_port_as_pinned_erts() {
    // `exit 3` gives `{Port, {exit_status, 3}}`, and the port then closes
    // with the reason `normal` (sys_drivers.c:1242-1252): `port_info/1`
    // gives `undefined`, the owner gets `{'EXIT', Port, normal}`, and
    // `port_close/1` raises `badarg`. `kill -9 $$` gives 128 + 9
    // (sys_drivers.c:1246-1247). An atom argument is a file name
    // (erl_bif_port.c:1233). The `is_port` test passes only for a port.
    assert_runs(
        "exit_status()",
        "[port,other,3,undefined,normal,{error,badarg},137,4,[{'EXIT',port,normal},{'EXIT',port,normal}]]",
    );
}

#[test]
fn sends_the_output_of_the_program_as_pinned_erts() {
    // `stderr_to_stdout` makes fd 2 the output pipe, and `nouse_stdio` makes
    // fd 4 the output pipe (erl_child_setup.c:315-345). 100000 bytes come in
    // reads of at most 64 KiB (sys_drivers.c:1336-1347).
    assert_runs(
        "output()",
        "[{<<111,117,116,101,114,114>>,0},{<<102,111,117,114>>,0},100000,[{'EXIT',port,normal},{'EXIT',port,normal}]]",
    );
}

#[test]
fn writes_the_input_of_the_program_and_counts_the_bytes_as_pinned_erts() {
    assert_runs(
        "command()",
        "[[{name,[47,98,105,110,47,115,104]},{input,0},{output,0},integer,integer,[name,links,id,connected,input,output,os_pid]],[true,true],<<104,101,108,108,111>>,[{name,[47,98,105,110,47,115,104]},{input,5},{output,5},integer,integer,[name,links,id,connected,input,output,os_pid]],normal,undefined,[]]",
    );
}

#[test]
fn closes_the_port_with_port_close_as_pinned_erts() {
    assert_runs(
        "close()",
        "[true,normal,undefined,{error,badarg},{error,badarg},[]]",
    );
}

#[test]
fn closes_the_port_at_the_exit_of_its_owner_as_pinned_erts() {
    // The link exit signal of the owner closes the port, also with the
    // reason `normal`, because the owner is the connected process
    // (io.c:3876-3879).
    assert_runs(
        "owner_exit()",
        "[[open,normal,undefined],[open,boom,undefined]]",
    );
}

#[test]
fn closes_the_port_with_epipe_after_a_failed_write_as_pinned_erts() {
    // The program closed its input, so the write fails with `EPIPE`, and
    // the port closes with the reason `epipe` (sys_drivers.c:1180-1184).
    // The owner traps the exit, so the signal is the `'EXIT'` message of the
    // write result, and the child then exits with the reason `normal`.
    assert_runs("epipe()", "[[true,[{'EXIT',port,epipe}],undefined],normal]");
}

#[test]
fn gives_the_errors_of_the_port_bifs_as_pinned_erts() {
    assert_runs(
        "errors()",
        "[[{error,enoent},{error,eacces},{error,enoent},{error,enoent},{error,badarg},{error,badarg},{error,badarg},{error,badarg},{error,badarg},{error,badarg},{error,badarg},{error,badarg},{error,badarg},{error,badarg},{error,badarg},{error,badarg},{error,badarg},{error,badarg},{error,badarg},{error,badarg},{error,badarg}],[enoent,badarg,badopt,badopt,badopt,badopt],[undefined,{error,badarg},{error,badarg},{error,badarg},{error,badarg},{error,badarg}],{error,badarg}]",
    );
}

#[test]
fn frames_the_packets_of_a_packet_port_as_pinned_erts() {
    // `{packet, N}` gives a header of N bytes with the size of the data
    // (erl_bif_port.c:849-861). The output writes the header and then the
    // data, and data that does not fit in a header of 1 or 2 bytes closes
    // the port with `einval` (sys_drivers.c:1140-1203). The input gives
    // each complete packet as one message, also for a header or data in
    // more than one read, and the end of file drops a packet that is not
    // complete (sys_drivers.c:1336-1440). The counts are the bytes of the
    // data without the headers (io.c:1408, 6354).
    assert_runs(
        "packet()",
        "[[true,true,true],[<<104,101,108,108,111>>,<<>>,<<97,98,99>>],[{input,8},{output,8}],[{[<<104,105>>,<<>>,<<120>>],0},{[<<97,98,99>>],0}],[255,einval,65535,einval],[{'EXIT',port,normal},{'EXIT',port,normal},{'EXIT',port,normal},{'EXIT',port,normal},{'EXIT',port,normal}]]",
    );
}

#[test]
fn reads_the_packet_setting_as_pinned_erts() {
    // Only a small integer whose low 32 bits are 1, 2, or 4 is a header
    // size, as ERTS keeps it in a C `int` (erl_bif_port.c:849-861), and
    // `erlang:open_port/2` raises `badarg` for `badopt`.
    assert_runs(
        "packet_errors()",
        "[badopt,badopt,badopt,badopt,badopt,badopt,badopt,badopt,{error,badarg},<<104,105>>,<<104,105>>,<<104,105>>,<<104,105>>]",
    );
}

#[test]
fn sends_eof_and_keeps_the_port_open_as_pinned_erts() {
    // `eof` sets `ERTS_PORT_SFLG_SOFT_EOF` (erl_bif_port.c:995-996): the end
    // of file sends `{Port, eof}`, after the exit status with
    // `exit_status`, and the port stays open (io.c:7204-7209) until a write
    // to the program that exited fails with `epipe` or `port_close/1`
    // closes it. `overlapped_io` changes nothing on Unix
    // (erl_bif_port.c:1001-1002, erl_sys_driver.h:56), so the settings of
    // `inet_gethost_native:do_open_port/2` (inet_gethost_native.erl:415)
    // give one packet and then `eof`.
    assert_runs(
        "eof()",
        "[[{data,<<104,105>>},{exit_status,3},eof],[{input,2},true],true,epipe,[{data,<<104,105>>},eof],none,true,normal,[{data,<<104,105>>},eof],[{'EXIT',port,normal}]]",
    );
}

#[test]
fn gives_the_port_name_before_the_first_byte_0_as_pinned_erts() {
    // ERTS copies a binary name into a C string, and the port name is that
    // C string (erl_unicode.c:2107-2108, io.c:323, 366-367).
    assert_runs("nul_name()", "[47,98,105,110,47,115,104]");
}

#[test]
fn monitors_a_port_as_pinned_erts() {
    // A port that closes sends `{'DOWN', Ref, port, Port, Reason}` with its
    // exit reason to each monitor, after its link exit signal; a port that
    // is not open gives `noproc` (io.c:3922-3936; bif.c:758-981).
    assert_runs(
        "monitor()",
        "[normal,noproc,boom,normal,[{'EXIT',port,normal},{'EXIT',port,normal}]]",
    );
}

#[test]
fn stops_the_run_for_each_port_part_that_is_not_implemented() {
    let program = load();
    for (case, message) in [
        ("line", "unsupported open_port/2 option line"),
        ("stream", "unsupported open_port/2 option stream"),
        ("list", "unsupported open_port/2 without the binary option"),
        ("spawn", "unsupported open_port/2 port kind spawn"),
        (
            "exec",
            "unsupported open_port/2 of a program that posix_spawn does not start: eacces",
        ),
        ("force", "unsupported erlang:port_command/3 options"),
        ("exit", "unsupported erlang:exit/2 with a port"),
        ("register", "unsupported erlang:register/2 with a port"),
        ("send", "unsupported erlang:send/2 with a port"),
        ("send_options", "unsupported erlang:send/3 with a port"),
        ("bang", "unsupported erlang:send/2 with a port"),
    ] {
        let argument = Term::Atom(program.atom(case).unwrap());
        let error = program
            .execute("port_probe", "unsupported", &[program.host_term(argument)])
            .unwrap_err()
            .to_string();
        assert!(error.starts_with(message), "{case}: {error}");
    }
}

#[test]
fn charges_the_port_bifs_as_pinned_erts() {
    // Each cost includes the light BIF call (1) and, for `open`, the local
    // call of `sh/0` (2). An operation on an open port uses
    // `CONTEXT_REDS/100` (erl_port.h:369-370, 380; io.c:2071, 2282, 4906).
    // `close` of an own port then handles the `EXIT` signal of the port
    // before it returns (erl_bif_port.c:419-429). The ERTS run gives the
    // measured processes a large heap, so that no garbage collection
    // occurs.
    let expected = "[{open,3},{command,41},{info,41},{info_closed,1},{close,43},{close_other,41}]";
    let reference = Command::new("erl")
        .args([
            "+S",
            "1",
            "-noshell",
            "-pa",
            "fixtures/port",
            "-eval",
            "io:format(\"~w~n\", [port_probe:costs([{min_heap_size,4000000},{min_bin_vheap_size,40000000}])]), halt().",
        ])
        .output()
        .unwrap();
    assert!(reference.status.success(), "{reference:?}");
    assert_eq!(
        String::from_utf8_lossy(&reference.stdout),
        format!("{expected}\n")
    );
    let program = load();
    let result = program
        .execute("port_probe", "costs", &[program.host_term(Term::nil())])
        .unwrap();
    assert_eq!(text(&result.term, result.atoms()), expected);
}

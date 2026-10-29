use std::{io::Write, os::fd::AsRawFd, process::Command};

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
                .bytes()
                .unwrap()
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
fn runs_the_env_arg0_cd_hide_and_parallelism_options_as_pinned_erts() {
    // erl_bif_port.c:870-919 reads the options, a later `env` replaces an
    // earlier one, and merge_global_environment (erl_bif_port.c:1170-1211)
    // applies the changes to the copy of os:putenv/2. `{cd, []}` changes
    // no directory (erl_bif_port.c:1094-1098). `hide` is for Windows only
    // (erl_bif_port.c:997-998).
    assert_runs(
        "options()",
        "[{parallelism,true},[{data,<<122,122,32,49,32,117,110,115,101,116,32,117,110,115,101,116,32,107,32,117,110,115,101,116,32,47>>},{exit_status,0}],{parallelism,false},[<<47,10,99,10>>,0],{parallelism,false},true,[{error,badarg},{error,badarg},{error,badarg},{error,badarg},{error,badarg},{error,badarg},{error,badarg},{error,badarg}],[{'EXIT',port,normal},{'EXIT',port,normal},{'EXIT',port,normal},{'EXIT',port,normal},{'EXIT',port,normal}]]",
    );
}

#[test]
fn runs_the_in_and_out_options_as_pinned_erts() {
    // `in` sets DO_READ and `out` sets DO_WRITE (erl_bif_port.c:991-994),
    // and without both options the port keeps the two directions
    // (erl_bif_port.c:1015-1016). The driver closes the parent end of the
    // missing direction after the handshake (sys_drivers.c:1307-1321), so
    // a write to an `in` port fails with ebadf (sys_drivers.c:1145, 1177)
    // and an `out` port sends no data message and no exit status
    // (sys_drivers.c:746-753), and a write after the child exited fails
    // with epipe (sys_drivers.c:1180-1184).
    assert_runs(
        "directions()",
        "[<<104,105>>,0,true,ebadf,true,none,true,epipe,[{'EXIT',port,normal}]]",
    );
}

#[test]
fn gives_the_exit_status_of_a_start_that_ran_no_program_as_pinned_erts() {
    // The child setup program exits with the errno of a chdir it cannot do
    // (erl_child_setup.c:302-306) and of an execve that fails
    // (erl_child_setup.c:361-369), so `exit_status` gives the errno: enoent
    // for a missing directory, enotdir for a file, and eacces for the
    // execve of a directory.
    assert_runs(
        "cd_status()",
        "[2,normal,20,13,[{'EXIT',port,normal},{'EXIT',port,normal}]]",
    );
}

#[test]
fn runs_the_command_of_a_spawn_port_as_pinned_erts() {
    // `{spawn, Command}` runs `sh -c "exec Command"` (sys_drivers.c:474-475,
    // 528-544; erl_child_setup.c:364-366), so `exit 3` after the `exec` of
    // `echo` does not run, and `$0` is `sh`. A command that the shell does
    // not find exits with 127. A first word that names a driver opens the
    // driver (io.c:599-622). `exit_status` with a driver (io.c:625-628) and
    // `{args, Args}` without `spawn_executable` (erl_bif_port.c:1086-1092)
    // give `badarg`.
    assert_runs(
        "spawn()",
        "[{name,[101,99,104,111,32,104,105,59,32,101,120,105,116,32,51]},[{data,<<104,105,10>>},{exit_status,0}],[{data,<<115,104,32,48,10>>},{exit_status,0}],{data,<<120>>},127,{name,[116,99,112,95,105,110,101,116]},{name,[117,100,112,95,105,110,101,116,32,120]},[{error,badarg},{error,badarg},{error,badarg},{error,badarg},{error,badarg},{error,badarg},{error,badarg},{error,badarg},{error,badarg},{error,badarg}],[{'EXIT',port,normal},{'EXIT',port,normal},{'EXIT',port,normal},{'EXIT',port,normal},{'EXIT',port,normal},{'EXIT',port,normal}]]",
    );
}

#[test]
fn sends_the_line_and_list_data_modes_as_pinned_erts() {
    // `{line, L}` and `line` (512) take the low 32 bits of a positive small
    // integer (erl_bif_port.c:862-869, 985-986; io.c:3149-3155), and
    // `stream` sets no packet header (erl_bif_port.c:979-980). A line with
    // packet is badopt (erl_bif_port.c:1018-1022). `read_linebuf` sends
    // `{eol, Line}` at `\n` or `\r\n` and `{noeol, Part}` for each L bytes
    // (io.c:3246-3301). At the end of file, after the exit status,
    // `flush_linebuf` sends the rest as `noeol` (io.c:3197-3224, 7204-7205;
    // sys_drivers.c:1242-1252). Without `binary` the data is a list
    // (io.c:3399-3427).
    assert_runs(
        "lines()",
        "[[[{data,{noeol,<<97,98,99,100>>}},{data,{eol,<<101,102,103,104>>}},{data,{eol,<<120,121>>}},{data,{noeol,<<97,98,13,99>>}},{data,{eol,<<13>>}},{exit_status,0},{data,{noeol,<<108,97,115,116>>}},normal],[{data,{eol,[97,98,99,100]}},{data,{eol,[97,98,99,100]}},{data,{noeol,[97,98,99,100]}},{data,{eol,[101]}},{exit_status,0},{data,{noeol,[97,98]}},normal],[{data,{noeol,<<97,98,99,100>>}},{exit_status,0},{data,{noeol,<<13,120,121>>}},normal],[{data,[104,105]},{exit_status,0},normal],[{exit_status,0},{data,{noeol,[104,105]}},normal],[{data,{noeol,[97,98,99]}},{data,{eol,[100]}},{exit_status,0},normal]],[{data,{noeol,<<97,98,99,100>>}},{data,{noeol,<<13>>}},eof],[{error,badarg},{error,badarg},{error,badarg},{error,badarg},{error,badarg},{error,badarg}],[{'EXIT',port,normal}]]",
    );
}

#[test]
fn gives_the_port_name_before_the_first_byte_0_as_pinned_erts() {
    // ERTS copies a binary name into a C string, and the port name is that
    // C string (erl_unicode.c:2107-2108, io.c:323, 366-367).
    assert_runs("nul_name()", "[47,98,105,110,47,115,104]");
}

/// Return the `~w` text of `port_probe:fd/3` with the input fd `input`
/// and the output fd `output`, which give the port name `"input/output"`.
fn fd_result(input: i32, output: i32) -> String {
    let name: Vec<String> = format!("{input}/{output}")
        .bytes()
        .map(|byte| byte.to_string())
        .collect();
    format!(
        "{{[<<104,101,108,108,111>>,eof],[{{name,[{}]}},{{input,5}},{{output,4}},{{os_pid,undefined}}],true,undefined,<<98,121,101>>,normal,undefined,[badarg,badarg,badarg,badarg,einval,einval,einval,badarg,badarg]}}",
        name.join(",")
    )
}

#[test]
fn opens_an_fd_port_as_pinned_erts() {
    // `{fd, In, Out}` names the port "In/Out" (erl_bif_port.c:1067-1081),
    // and the port does not close its fds (sys_drivers.c:1002-1027), so the
    // second port writes to the same output fd. The ERTS run gets pipes as
    // its fds 3 and 4 and a file as its fd 5.
    let directory = std::env::temp_dir().join(format!("pbeam-fd-port-{}", std::process::id()));
    std::fs::create_dir_all(&directory).unwrap();
    let erts_output = directory.join("erts");
    let script = format!(
        "printf hello | {{ printf bye | erl +S 1 -noshell -kernel logger_level none -pa fixtures/port -eval 'io:format(\"~w~n\", [port_probe:fd(3, 4, 5)]), halt().' 4<&0 0</dev/null ; }} 3<&0 5>'{}'",
        erts_output.display()
    );
    let reference = Command::new("sh").args(["-c", &script]).output().unwrap();
    assert!(reference.status.success(), "{reference:?}");
    assert_eq!(
        String::from_utf8_lossy(&reference.stdout),
        format!("{}\n", fd_result(3, 5))
    );
    assert_eq!(std::fs::read(&erts_output).unwrap(), b"pingpong");

    let (first, mut first_writer) = std::io::pipe().unwrap();
    let (second, mut second_writer) = std::io::pipe().unwrap();
    first_writer.write_all(b"hello").unwrap();
    second_writer.write_all(b"bye").unwrap();
    drop((first_writer, second_writer));
    let pbeam_output = directory.join("pbeam");
    let output = std::fs::File::create(&pbeam_output).unwrap();
    let fds = [first.as_raw_fd(), second.as_raw_fd(), output.as_raw_fd()];
    let program = load();
    let args: Vec<_> = fds
        .iter()
        .map(|&fd| program.host_term(Term::Integer(fd.into())))
        .collect();
    let result = program.execute("port_probe", "fd", &args).unwrap();
    assert_eq!(
        text(&result.term, result.atoms()),
        fd_result(fds[0], fds[2])
    );
    assert_eq!(std::fs::read(&pbeam_output).unwrap(), b"pingpong");
    std::fs::remove_dir_all(&directory).unwrap();
}

#[test]
fn gives_the_fd_to_the_last_fd_port_as_pinned_erts() {
    // The last `driver_select` of a read fd takes it from the port that
    // selected it before (erl_check_io.c:1080-1085, 1711-1719).
    let expected = "{[<<104,105>>,eof],none,true,true}";
    let script = "printf hi | erl +S 1 -noshell -kernel logger_level none -pa fixtures/port -eval 'io:format(\"~w~n\", [port_probe:fd_steal(3, 1)]), halt().' 3<&0 0</dev/null";
    let reference = Command::new("sh").args(["-c", script]).output().unwrap();
    assert!(reference.status.success(), "{reference:?}");
    assert_eq!(
        String::from_utf8_lossy(&reference.stdout),
        format!("{expected}\n")
    );
    let (input, mut writer) = std::io::pipe().unwrap();
    writer.write_all(b"hi").unwrap();
    drop(writer);
    let program = load();
    let args = [input.as_raw_fd(), 1].map(|fd| program.host_term(Term::Integer(fd.into())));
    let result = program.execute("port_probe", "fd_steal", &args).unwrap();
    assert_eq!(text(&result.term, result.atoms()), expected);
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
fn opens_a_port_on_an_fd_that_is_not_open_as_pinned_erts() {
    // `fd_start` checks only `sys_max_files()` (sys_drivers.c:822-824): the
    // port opens, and its write fails with `ebadf` (sys_drivers.c:1180-1184).
    assert_runs(
        "fd_closed()",
        "[[],true,[{'EXIT',port,ebadf}],undefined,{error,einval}]",
    );
}

#[test]
fn stops_the_run_for_each_port_part_that_is_not_implemented() {
    let program = load();
    for (case, message) in [
        (
            "line",
            "unsupported open_port/2 option {line, L} whose low 32 bits are not positive",
        ),
        (
            "busy_limits",
            "unsupported open_port/2 option busy_limits_port",
        ),
        (
            "empty_line",
            "unsupported open_port/2 option {line, L} whose low 32 bits are not positive",
        ),
        (
            "fd_list",
            "unsupported open_port/2 without the binary option",
        ),
        ("force", "unsupported erlang:port_command/3 options"),
        ("register", "unsupported erlang:register/2 with a port"),
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
    // occurs. An exit signal or a close message to a port uses
    // `CONTEXT_REDS/100`, and a connect message `CONTEXT_REDS/200`
    // (erl_port.h:370-371).
    let expected = "[{open,3},{command,41},{info,41},{info_closed,1},{close,43},{close_other,41},{exit,41},{message_close,41},{message_connect,21}]";
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

#[test]
fn runs_port_signals_by_message_exit_and_port_call_as_pinned_erts() {
    // `Port ! {Owner, Op}` from each process (io.c:3974-4015): a command
    // writes, a close gives the link exit `normal` and then `{Port,
    // closed}` (io.c:2182-2198, 3876-3882), and a connect gives `{Port,
    // connected}` and no link (io.c:2341-2351). Each other message and a
    // pid that is not the owner send `badsig` (io.c:1246-1254). `exit/2`
    // closes the port with each reason, also `normal` from another process
    // (bif.c:1774-1797, io.c:3854, 3876-3879). `port_call/2,3` raise
    // `badarg`: the spawn driver has no `call` (sys_drivers.c:246,
    // io.c:4473-4474).
    assert_runs(
        "signals()",
        "[[{port,{data,<<104,105>>}}],[{port,{data,<<97,98>>}}],ok,[{port,{data,<<115>>}}],[[{'EXIT',port,normal},{port,closed}],undefined],[],[[[[{'EXIT',port,badsig}],true],[{'EXIT',port,normal}]],[[[{'EXIT',port,badsig}],true],[{'EXIT',port,normal}]],[[[{'EXIT',port,badsig}],true],[{'EXIT',port,normal}]],[[[{'EXIT',port,badsig}],true],[{'EXIT',port,normal}]],[[[{'EXIT',port,badsig}],true],[{'EXIT',port,normal}]],[[[{'EXIT',port,badsig}],true],[{'EXIT',port,normal}]],[[[{'EXIT',port,badsig}],true],[{'EXIT',port,normal}]],[[[{'EXIT',port,badsig}],true],[{'EXIT',port,normal}]],[[[{'EXIT',port,badsig}],true],[{'EXIT',port,normal}]]],[{port,connected}],[true,true,true,[{'EXIT',port,normal}]],[[true,[{'EXIT',port,normal}],undefined],[true,[{'EXIT',port,killed}],undefined],[true,[{'EXIT',port,boom}],undefined]],[[{'EXIT',port,normal}],undefined,true],[{error,badarg},{error,badarg}],[{'EXIT',port,normal}]]",
    );
}

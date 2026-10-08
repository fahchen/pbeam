use std::{
    path::{Path, PathBuf},
    process::{Command, Output, Stdio},
    sync::LazyLock,
    time::{Duration, Instant},
};

use pbeam_runtime::{BeamProgram, Term};

#[path = "common/boot.rs"]
mod boot;
#[expect(
    dead_code,
    reason = "this test crate gives BEAM paths, not bytes, to the binary"
)]
mod common;
#[expect(dead_code, reason = "this test crate does not mask log times")]
#[path = "common/log_event.rs"]
mod log_event;
#[path = "common/run.rs"]
mod run;

use boot::{BOOT_BEAM, BOOT_MODULES};
use common::pinned_beam_path;
use log_event::LOG_EVENT_MODULES;
use run::{broken_pipe, run_dumb_utf8, run_with_time_limit};

const RUN_TIME_LIMIT: Duration = Duration::from_secs(10);

/// The paths of the pinned `BOOT_MODULES`. Each `pinned_beam_path` call
/// starts `erl`, so the tests of this crate find the paths only once.
static BOOT_PATHS: LazyLock<[PathBuf; BOOT_MODULES.len()]> =
    LazyLock::new(|| BOOT_MODULES.map(pinned_beam_path));

#[test]
fn loads_compiled_elixir_greetings() {
    let hello = std::fs::read(beam_file("Elixir.Hello.beam")).unwrap();
    let variant = std::fs::read(beam_file("Elixir.HelloVariant.beam")).unwrap();

    BeamProgram::load(&[&hello, &variant]).unwrap();
}

#[test]
fn loads_the_pinned_elixir_io_module() {
    let bytes = elixir_io_beam();

    BeamProgram::load(&[&bytes]).unwrap();
}

#[test]
fn executes_compiled_elixir_puts_to_the_otp_io_boundary() {
    let hello = std::fs::read(beam_file("Elixir.Hello.beam")).unwrap();
    let io = elixir_io_beam();
    let program = BeamProgram::load(&[&hello, &io]).unwrap();

    let error = program.execute("Elixir.Hello", "main", &[]).unwrap_err();

    assert!(
        error
            .to_string()
            .contains("unresolved external MFA io:put_chars/2"),
        "expected execution to reach OTP IO, got {error}"
    );
}

#[test]
fn rejects_an_unsupported_io_device_with_a_function_clause_error() {
    let io = elixir_io_beam();
    let program = BeamProgram::load(&[&io]).unwrap();

    let error = program
        .execute(
            "Elixir.IO",
            "puts",
            &[Term::Integer(7), Term::Binary(b"message".to_vec())],
        )
        .unwrap_err();

    assert!(
        error
            .to_string()
            .contains("function clause Elixir.IO:puts/2")
    );
}

#[test]
fn reports_the_unavailable_string_conversion_for_non_binary_data() {
    let io = elixir_io_beam();
    let program = BeamProgram::load(&[&io]).unwrap();

    let error = program
        .execute(
            "Elixir.IO",
            "puts",
            &[Term::Atom(program.atom("stdio").unwrap()), Term::Integer(7)],
        )
        .unwrap_err();

    assert!(
        error
            .to_string()
            .contains("unresolved external MFA Elixir.String.Chars:to_string/1")
    );
}

#[test]
fn prints_each_greeting_through_the_booted_otp_io_servers() {
    // Each `main/0` moves its greeting literal to `x0` and calls
    // `Elixir.IO:puts/1`. `IO.puts/2` calls
    // `:io.put_chars(map_dev(:stdio), [to_chardata(item), ?\n])`, and
    // `map_dev(:stdio)` is `:standard_io` (Elixir 1.20.4 io.ex:331-332,
    // 853). `io:put_chars/2` sends the request to `group_leader()`
    // (io.erl:265-266, 1652-1655). `pbeam_boot` sets `user` as the group
    // leader of the root before it calls the entry, as `start_em/1` does
    // before `-eval` (init.erl:1539-1546). `user` and `user_drv` send the
    // output to the `prim_tty` writer, which calls `write_nif/2`
    // (prim_tty.erl:555-566). The PBeam child runs without `PATH`, so it
    // cannot start `erl` or `elixir` (`pbeam_boot`).
    let io = elixir_io_path();
    for (module, expected) in [
        ("Elixir.Hello", "Hello, world!\n"),
        ("Elixir.HelloVariant", "Another greeting.\n"),
    ] {
        let beams = hello_beams(module, &io);
        let erts = run_dumb_utf8(
            &mut erl_eval(module, "main", &io),
            Stdio::piped(),
            RUN_TIME_LIMIT,
        );
        let pbeam = run_dumb_utf8(
            &mut pbeam_boot(module, "main", &beams),
            Stdio::piped(),
            RUN_TIME_LIMIT,
        );
        assert_run(&format!("ERTS {module}"), &erts, (expected, "", Some(0)));
        assert_run(&format!("PBeam {module}"), &pbeam, (expected, "", Some(0)));
    }
}

#[test]
fn fails_visibly_for_a_missing_entry_through_the_boot() {
    // `start_it/1` writes the `Failed to eval` line and raises the
    // exception again (init.erl:1552-1567). The boot process exits, and
    // `init` halts with `Runtime terminating during boot` and the reason
    // (init.erl:678-679, 710-712). The `pbeam` binary accepts only an
    // argument atom that a loaded module holds: `start` is an atom of
    // `pbeam_boot`, and `Elixir.String.Chars` is an atom of `Elixir.IO`.
    // For a missing export, PBeam raises `undef` with the stack trace
    // `[{'Elixir.Hello',start,[],[]}]`: the first frame of ERTS and no
    // caller frames (rows "Exception stack trace" and "Undefined
    // functions"). PBeam writes its own report of the root exit (row "Run
    // end"). For a module that the run does not load, PBeam stops at the
    // call with a runtime error and no root exit (row "Undefined
    // functions"). Without `ERL_CRASH_DUMP_SECONDS=0`, ERTS writes
    // `erl_crash.dump` into the working directory.
    let io = elixir_io_path();
    let beams = hello_beams("Elixir.Hello", &io);
    for (module, function, erts_stderr, pbeam_stderr) in [
        (
            "Elixir.Hello",
            "start",
            "Error! Failed to eval: 'Elixir.Hello':start(), halt().\r\n\r\nRuntime terminating during boot ({undef,[{'Elixir.Hello',start,[],[]},{erl_eval,do_apply,7,[{file,\"erl_eval.erl\"},{line,1044}]},{erl_eval,exprs,6,[{file,\"erl_eval.erl\"},{line,276}]},{init,start_it,1,[]},{init,start_em,1,[]},{init,do_boot,3,[]}]})\r\n",
            "pbeam: root process exited with {undef,[{'Elixir.Hello',start,[],[]}]}\n",
        ),
        (
            "Elixir.String.Chars",
            "main",
            "Error! Failed to eval: 'Elixir.String.Chars':main(), halt().\r\n\r\nRuntime terminating during boot ({undef,[{'Elixir.String.Chars',main,[],[]},{erl_eval,do_apply,7,[{file,\"erl_eval.erl\"},{line,1044}]},{erl_eval,exprs,6,[{file,\"erl_eval.erl\"},{line,276}]},{init,start_it,1,[]},{init,start_em,1,[]},{init,do_boot,3,[]}]})\r\n",
            "pbeam: unresolved external MFA Elixir.String.Chars:main/0\n",
        ),
    ] {
        let erts = run_dumb_utf8(
            &mut erl_eval(module, function, &io),
            Stdio::piped(),
            RUN_TIME_LIMIT,
        );
        let pbeam = run_dumb_utf8(
            &mut pbeam_boot(module, function, &beams),
            Stdio::piped(),
            RUN_TIME_LIMIT,
        );
        assert_run(
            &format!("ERTS {module}:{function}"),
            &erts,
            ("", erts_stderr, Some(1)),
        );
        assert_run(
            &format!("PBeam {module}:{function}"),
            &pbeam,
            ("", pbeam_stderr, Some(1)),
        );
    }
}

#[test]
fn stops_at_the_binary_match_of_a_locale_that_is_not_utf8() {
    // With `LC_ALL=C`, the `prim_tty` writer converts the output with
    // `to_latin1/2`, which calls `is_usascii/1`. `is_usascii/1` matches
    // `<<Char/utf8, T/binary>>` (prim_tty.erl:1309-1321). PBeam has no
    // match context for a binary, so the run stops at `bs_start_match3`
    // (rows "Boot" and "Binary matching").
    let io = elixir_io_path();
    let beams = hello_beams("Elixir.Hello", &io);
    let erts = run_with_time_limit(
        erl_eval("Elixir.Hello", "main", &io)
            .env("TERM", "dumb")
            .env("LC_ALL", "C"),
        Stdio::piped(),
        RUN_TIME_LIMIT,
    );
    let pbeam = run_with_time_limit(
        pbeam_boot("Elixir.Hello", "main", &beams)
            .env("TERM", "dumb")
            .env("LC_ALL", "C"),
        Stdio::piped(),
        RUN_TIME_LIMIT,
    );

    assert_run("ERTS", &erts, ("Hello, world!\n", "", Some(0)));
    assert_run(
        "PBeam",
        &pbeam,
        (
            "",
            "pbeam: unsupported bs_start_match3 of a binary: needs a match context\n",
            Some(1),
        ),
    );
}

#[test]
fn raises_terminated_in_the_entry_after_a_failed_stdout_write() {
    // `write_nif/2` fails, and the `prim_tty` writer calls
    // `exit(self(), Reason)` (prim_tty.erl:564-565). `user_drv` logs
    // `Writer crashed (~p)` with `?LOG_ERROR` and stops
    // (user_drv.erl:603-615). The caller gets `DOWN`, and
    // `io:put_chars/2` raises `error:terminated` in the entry
    // (io.erl:197-202, 782, 1679-1684). `init` halts with the reason
    // (init.erl:1552-1567, 678-679, 710-712). Without
    // `-kernel logger_level none`, the ERTS report of `user_drv` can mix
    // with the halt text. PBeam ends the run at the root exit before the
    // `default` handler writes (row "Run end"), but `?LOG_ERROR` calls the
    // log event modules before the caller gets `DOWN`. Thus the run also
    // loads `LOG_EVENT_MODULES`. Without them, it stops with
    // `unresolved external MFA logger_backend:log_allowed/3` (measured).
    // PBeam gives the stack trace `[]` (row "Exception stack trace").
    let io = elixir_io_path();
    let mut beams = hello_beams("Elixir.Hello", &io);
    beams.extend(LOG_EVENT_MODULES.map(pinned_beam_path));
    let erts = run_dumb_utf8(
        erl_eval("Elixir.Hello", "main", &io).args(["-kernel", "logger_level", "none"]),
        broken_pipe(),
        RUN_TIME_LIMIT,
    );
    let pbeam = run_dumb_utf8(
        &mut pbeam_boot("Elixir.Hello", "main", &beams),
        broken_pipe(),
        RUN_TIME_LIMIT,
    );

    assert_run(
        "ERTS",
        &erts,
        (
            "",
            "Error! Failed to eval: 'Elixir.Hello':main(), halt().\r\n\r\nRuntime terminating during boot ({terminated,[{io,put_chars,[standard_io,[<<\"Hello, world!\">>,10]],[{file,\"io.erl\"},{line,200},{error_info,#{cause=>{io,terminated},module=>erl_stdlib_errors}}]},{erl_eval,do_apply,7,[{file,\"erl_eval.erl\"},{line,1044}]},{erl_eval,exprs,6,[{file,\"erl_eval.erl\"},{line,276}]},{init,start_it,1,[]},{init,start_em,1,[]},{init,do_boot,3,[]}]})\r\n",
            Some(1),
        ),
    );
    assert_run(
        "PBeam",
        &pbeam,
        (
            "",
            "pbeam: root process exited with {terminated,[]}\n",
            Some(1),
        ),
    );
}

#[test]
fn reports_missing_entry_and_input_failures() {
    let hello = beam_file("Elixir.Hello.beam");
    let output = run_pbeam(&["Elixir.Hello", "missing", "--", hello.to_str().unwrap()]);
    assert_eq!(output.status.code(), Some(1));
    assert!(output.stdout.is_empty());
    assert!(
        String::from_utf8_lossy(&output.stderr).contains("missing export Elixir.Hello:missing/0")
    );

    let output = run_pbeam(&[
        "Elixir.Hello",
        "main",
        "pbeam_unknown",
        "--",
        hello.to_str().unwrap(),
    ]);
    assert_eq!(
        (
            output.stdout.as_slice(),
            output.stderr.as_slice(),
            output.status.code()
        ),
        (
            &b""[..],
            &b"pbeam: argument pbeam_unknown is not an atom of the loaded modules\n"[..],
            Some(1)
        )
    );

    for arguments in [
        Vec::new(),
        vec!["Elixir.Hello"],
        vec!["Elixir.Hello", "main"],
        vec!["Elixir.Hello", "main", hello.to_str().unwrap()],
    ] {
        let output = run_pbeam(&arguments);
        assert_eq!(output.status.code(), Some(2));
        assert!(String::from_utf8_lossy(&output.stderr).contains("Usage: pbeam"));
    }

    let output = run_pbeam(&[
        "Elixir.Hello",
        "main",
        "--",
        "/pbeam-test-file-that-does-not-exist.beam",
    ]);
    assert_eq!(output.status.code(), Some(1));
    assert!(String::from_utf8_lossy(&output.stderr).contains("cannot read"));
}

#[test]
fn prints_help_and_exits_successfully() {
    let output = run_pbeam(&["--help"]);

    assert!(output.status.success());
    let stdout = String::from_utf8(output.stdout).unwrap();
    assert!(stdout.contains("Load BEAM files and run a function as the root process."));
    assert!(stdout.contains("Each argument is an atom that the loaded modules hold"));
    assert!(stdout.contains("Usage: pbeam <MODULE> <FUNCTION> [ATOM]... -- <BEAM_FILE>..."));
    assert!(stdout.contains("-h, --help"));
    assert!(output.stderr.is_empty());
}

#[test]
fn reports_invalid_beam_files() {
    let path = std::env::temp_dir().join(format!(
        "pbeam-invalid-{}-{}.beam",
        std::process::id(),
        Instant::now().elapsed().as_nanos()
    ));
    std::fs::write(&path, b"invalid BEAM").unwrap();
    let output = run_pbeam(&["Elixir.Hello", "main", "--", path.to_str().unwrap()]);
    std::fs::remove_file(path).unwrap();

    assert_eq!(output.status.code(), Some(1));
    assert!(String::from_utf8_lossy(&output.stderr).contains("cannot load BEAM modules"));
}

fn beam_file(file: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("fixtures/hello/_build/test/lib/hello_reference/ebin")
        .join(file)
}

/// Return the BEAM files of a booted Hello World run: the fixture module
/// `module`, the pinned `Elixir.IO` module at `io`, and the pinned boot
/// modules. `pbeam_boot` adds `BOOT_BEAM`.
///
/// # Examples
///
/// ```rust,ignore
/// let beams = hello_beams("Elixir.Hello", &elixir_io_path());
/// assert_eq!(beams.len(), 2 + BOOT_MODULES.len());
/// ```
fn hello_beams(module: &str, io: &Path) -> Vec<PathBuf> {
    let mut beams = vec![beam_file(&format!("{module}.beam")), io.to_path_buf()];
    beams.extend(BOOT_PATHS.iter().cloned());
    beams
}

fn elixir_io_beam() -> Vec<u8> {
    std::fs::read(elixir_io_path()).unwrap()
}

fn elixir_io_path() -> PathBuf {
    let path = Command::new("elixir")
        .args(["-e", "IO.write(:code.which(IO))"])
        .output()
        .unwrap();
    assert!(path.status.success());
    PathBuf::from(String::from_utf8(path.stdout).unwrap())
}

/// Return the `erl -noshell` command that evaluates `module:function()` and
/// `halt()` with the fixture and the pinned Elixir code paths. `io` is the
/// path of the pinned `Elixir.IO.beam`. The command has
/// `ERL_CRASH_DUMP_SECONDS=0`, so a crash does not write `erl_crash.dump`.
///
/// # Examples
///
/// ```rust,ignore
/// let io = elixir_io_path();
/// let output = run_dumb_utf8(&mut erl_eval("Elixir.Hello", "main", &io), Stdio::piped(), RUN_TIME_LIMIT);
/// assert_run("ERTS", &output, ("Hello, world!\n", "", Some(0)));
/// ```
fn erl_eval(module: &str, function: &str, io: &Path) -> Command {
    let mut command = Command::new("erl");
    command
        .args([
            "-noshell",
            "-pa",
            "fixtures/hello/_build/test/lib/hello_reference/ebin",
            "-pa",
        ])
        .arg(io.parent().unwrap())
        .args(["-eval", &format!("'{module}':{function}(), halt().")])
        .env("ERL_CRASH_DUMP_SECONDS", "0");
    command
}

/// Return the `pbeam` command that boots with `pbeam_boot` and runs
/// `module:function()` with the BEAM files `beams` and `BOOT_BEAM`. The
/// command runs without `PATH`, so it cannot start `erl` or `elixir`. It
/// keeps the other variables, because `cargo llvm-cov` gives
/// `LLVM_PROFILE_FILE` to the child.
///
/// # Examples
///
/// ```rust,ignore
/// let beams = hello_beams("Elixir.Hello", &elixir_io_path());
/// let output = run_dumb_utf8(&mut pbeam_boot("Elixir.Hello", "main", &beams), Stdio::piped(), RUN_TIME_LIMIT);
/// assert_run("PBeam", &output, ("Hello, world!\n", "", Some(0)));
/// ```
fn pbeam_boot(module: &str, function: &str, beams: &[PathBuf]) -> Command {
    let mut command = Command::new(env!("CARGO_BIN_EXE_pbeam"));
    command
        .args(["pbeam_boot", "start", module, function, "--"])
        .args(beams)
        .arg(BOOT_BEAM)
        .env_remove("PATH");
    command
}

/// Assert that a run gives exactly the stdout, the stderr, and the exit
/// status of `expected`. `name` identifies the run in the failure message.
///
/// # Examples
///
/// ```rust,ignore
/// assert_run("PBeam", &output, ("Hello, world!\n", "", Some(0)));
/// ```
#[track_caller]
fn assert_run(name: &str, output: &Output, expected: (&str, &str, Option<i32>)) {
    assert_eq!(
        (
            String::from_utf8_lossy(&output.stdout).as_ref(),
            String::from_utf8_lossy(&output.stderr).as_ref(),
            output.status.code()
        ),
        expected,
        "{name}"
    );
}

fn run_pbeam(arguments: &[&str]) -> Output {
    run_with_time_limit(
        Command::new(env!("CARGO_BIN_EXE_pbeam")).args(arguments),
        Stdio::piped(),
        Duration::from_secs(5),
    )
}

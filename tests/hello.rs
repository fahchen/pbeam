use std::{
    path::PathBuf,
    process::{Command, Output, Stdio},
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
#[path = "common/run.rs"]
mod run;

use boot::{BOOT_BEAM, BOOT_MODULES};
use common::pinned_beam_path;
use run::{run_dumb_utf8, run_with_time_limit};

const RUN_TIME_LIMIT: Duration = Duration::from_secs(10);

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
    // cannot start `erl` or `elixir`. It keeps the other variables, because
    // `cargo llvm-cov` gives `LLVM_PROFILE_FILE` to the child.
    let io = elixir_io_path();
    let beams = BOOT_MODULES.map(pinned_beam_path);
    for (module, expected) in [
        ("Elixir.Hello", "Hello, world!\n"),
        ("Elixir.HelloVariant", "Another greeting.\n"),
    ] {
        let erts = run_dumb_utf8(
            Command::new("erl")
                .args([
                    "-noshell",
                    "-pa",
                    "fixtures/hello/_build/test/lib/hello_reference/ebin",
                    "-pa",
                ])
                .arg(io.parent().unwrap())
                .args(["-eval", &format!("'{module}':main(), halt().")]),
            Stdio::piped(),
            RUN_TIME_LIMIT,
        );
        let pbeam = run_dumb_utf8(
            Command::new(env!("CARGO_BIN_EXE_pbeam"))
                .args(["pbeam_boot", "start", module, "main", "--"])
                .arg(beam_file(&format!("{module}.beam")))
                .arg(&io)
                .args(&beams)
                .arg(BOOT_BEAM)
                .env_remove("PATH"),
            Stdio::piped(),
            RUN_TIME_LIMIT,
        );
        for (name, output) in [("ERTS", erts), ("PBeam", pbeam)] {
            assert_eq!(
                (
                    String::from_utf8_lossy(&output.stdout).as_ref(),
                    String::from_utf8_lossy(&output.stderr).as_ref(),
                    output.status.code()
                ),
                (expected, "", Some(0)),
                "{name} {module}"
            );
        }
    }
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

fn run_pbeam(arguments: &[&str]) -> Output {
    run_with_time_limit(
        Command::new(env!("CARGO_BIN_EXE_pbeam")).args(arguments),
        Stdio::piped(),
        Duration::from_secs(5),
    )
}

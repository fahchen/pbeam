use std::{
    path::PathBuf,
    process::{Command, Output, Stdio},
    thread,
    time::{Duration, Instant},
};

use pbeam_runtime::{BeamProgram, Term};

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
            &[Term::Atom("stdio".into()), Term::Integer(7)],
        )
        .unwrap_err();

    assert!(
        error
            .to_string()
            .contains("unresolved external MFA Elixir.String.Chars:to_string/1")
    );
}

#[test]
fn reports_missing_entry_and_input_failures() {
    let hello = beam_file("Elixir.Hello.beam");
    let output = run_pbeam(&["Elixir.Hello", "missing", hello.to_str().unwrap()]);
    assert_eq!(output.status.code(), Some(1));
    assert!(output.stdout.is_empty());
    assert!(
        String::from_utf8_lossy(&output.stderr).contains("missing export Elixir.Hello:missing/0")
    );

    for arguments in [
        Vec::new(),
        vec!["Elixir.Hello"],
        vec!["Elixir.Hello", "main"],
    ] {
        let output = run_pbeam(&arguments);
        assert_eq!(output.status.code(), Some(2));
        assert!(String::from_utf8_lossy(&output.stderr).contains("Usage: pbeam"));
    }

    let output = run_pbeam(&[
        "Elixir.Hello",
        "main",
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
    assert!(stdout.contains("Run a zero-argument function from BEAM files."));
    assert!(stdout.contains("Usage: pbeam <ENTRY_MODULE> <ENTRY_FUNCTION> <BEAM_FILE>..."));
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
    let output = run_pbeam(&["Elixir.Hello", "main", path.to_str().unwrap()]);
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
    let path = Command::new("elixir")
        .args(["-e", "IO.write(:code.which(IO))"])
        .output()
        .unwrap();
    assert!(path.status.success());
    std::fs::read(PathBuf::from(String::from_utf8(path.stdout).unwrap())).unwrap()
}

fn run_pbeam(arguments: &[&str]) -> Output {
    let mut child = Command::new(env!("CARGO_BIN_EXE_pbeam"))
        .args(arguments)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    let started = Instant::now();
    loop {
        if child.try_wait().unwrap().is_some() {
            break;
        }
        if started.elapsed() >= Duration::from_secs(5) {
            child.kill().unwrap();
            panic!("PBeam test process exceeded its five-second limit");
        }
        thread::sleep(Duration::from_millis(10));
    }
    child.wait_with_output().unwrap()
}

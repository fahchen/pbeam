use std::process::Command;

use pbeam_runtime::{BeamProgram, Term};

const IO_ERROR_PROBE: &[u8] = include_bytes!("../fixtures/io_error/io_error_probe.beam");

#[test]
fn surfaces_a_writer_process_exit_as_an_io_protocol_error() {
    let program = BeamProgram::load(&[IO_ERROR_PROBE]).unwrap();
    let atom = |name: &str| Term::Atom(program.atom(name).unwrap());

    assert_eq!(
        program.execute("io_error_probe", "main", &[]).unwrap(),
        Term::Tuple(vec![
            Term::Tuple(vec![atom("error"), atom("write_failed"),]),
            atom("noise_ok"),
        ])
    );

    let reference = Command::new("erl")
        .args([
            "-noshell",
            "-pa",
            "fixtures/io_error",
            "-eval",
            "io:format(\"~p~n\", [io_error_probe:main()]), halt().",
        ])
        .output()
        .unwrap();
    assert!(reference.status.success());
    assert!(reference.stderr.is_empty());
    assert_eq!(reference.stdout, b"{{error,write_failed},noise_ok}\n");
}

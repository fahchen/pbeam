use std::process::Command;

use pbeam_runtime::{BeamProgram, Term};

const IO_ERROR_PROBE: &[u8] = include_bytes!("../fixtures/io_error/io_error_probe.beam");

#[test]
fn surfaces_a_writer_process_exit_as_an_io_protocol_error() {
    let program = BeamProgram::load(&[IO_ERROR_PROBE]).unwrap();

    assert_eq!(
        program.execute("io_error_probe", "main", &[]).unwrap(),
        Term::Tuple(vec![
            Term::Tuple(vec![
                Term::Atom("error".into()),
                Term::Atom("write_failed".into()),
            ]),
            Term::Atom("noise_ok".into()),
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

use std::process::Command;

use pbeam_runtime::{BeamProgram, Term};

const PROCESS_PROBE: &[u8] = include_bytes!("../fixtures/process/process_probe.beam");

#[test]
fn runs_a_compiled_process_probe_with_reference_matching_and_cleanup() {
    let program = BeamProgram::load(&[PROCESS_PROBE]).unwrap();

    assert_eq!(
        program.execute("process_probe", "main", &[]).unwrap(),
        Term::Tuple(
            ["io_ok", "down_ok", "exit_ok", "noise_ok"]
                .map(|name| Term::Atom(program.atom(name).unwrap()))
                .to_vec()
        )
    );

    let reference = Command::new("erl")
        .args([
            "-noshell",
            "-pa",
            "fixtures/process",
            "-eval",
            "io:format(\"~p~n\", [process_probe:main()]), halt().",
        ])
        .output()
        .unwrap();
    assert!(reference.status.success());
    assert!(reference.stderr.is_empty());
    assert_eq!(reference.stdout, b"{io_ok,down_ok,exit_ok,noise_ok}\n");
}

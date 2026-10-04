use std::process::Command;

use pbeam_runtime::{BeamProgram, Term};

const EXIT_SIGNAL_PROBE: &[u8] = include_bytes!("../fixtures/exit_signal/exit_signal_probe.beam");

#[test]
fn delivers_link_exit_before_monitor_down_and_normal_exit_to_trapping_process() {
    let program = BeamProgram::load(&[EXIT_SIGNAL_PROBE]).unwrap();
    let atom = |name: &str| Term::Atom(program.atom(name).unwrap());
    let pair = |tag: &str, reason: &str| Term::Tuple(vec![atom(tag), atom(reason)]);
    assert_eq!(
        program.execute("exit_signal_probe", "main", &[]).unwrap(),
        Term::Tuple(vec![
            Term::Tuple(vec![pair("EXIT", "boom"), pair("DOWN", "boom")]),
            pair("EXIT", "normal"),
        ])
    );

    let reference = Command::new("erl")
        .args([
            "-noshell",
            "-pa",
            "fixtures/exit_signal",
            "-eval",
            "io:format(\"~p~n\", [exit_signal_probe:main()]), halt().",
        ])
        .output()
        .unwrap();
    assert!(reference.status.success());
    assert!(reference.stderr.is_empty());
    assert_eq!(
        reference.stdout,
        b"{{{'EXIT',boom},{'DOWN',boom}},{'EXIT',normal}}\n"
    );
}

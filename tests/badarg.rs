use std::process::Command;

use pbeam_runtime::{BeamProgram, Term};

const BADARG_PROBE: &[u8] = include_bytes!("../fixtures/badarg/badarg_probe.beam");

#[test]
fn raises_badarg_that_catch_and_try_catch_as_pinned_erts() {
    // Each `catch` of the probe gets `{'EXIT', {badarg, Stack}}` with a list
    // as `Stack`. The probe runs `element/2` through a `bif` instruction
    // with fail label 0, and the other BIFs through `call_ext`. PBeam gives
    // the stack trace `[]`; this is the "Exception stack trace" row in
    // `docs/erts-differences.md`.
    let program = BeamProgram::load(&[BADARG_PROBE]).unwrap();
    let badarg = Term::Atom(program.atom("badarg").unwrap());
    let error = Term::Atom(program.atom("error").unwrap());
    let caught = Term::Tuple(vec![error, badarg.clone()]);
    let mut expected = vec![badarg; 9];
    expected.extend([caught.clone(), caught]);
    assert_eq!(
        program.execute("badarg_probe", "main", &[]).unwrap(),
        Term::Tuple(expected)
    );

    let reference = Command::new("erl")
        .args([
            "-noshell",
            "-pa",
            "fixtures/badarg",
            "-eval",
            "io:format(\"~w~n\", [badarg_probe:main()]), halt().",
        ])
        .output()
        .unwrap();
    assert!(reference.status.success());
    assert!(reference.stderr.is_empty());
    assert_eq!(
        reference.stdout,
        b"{badarg,badarg,badarg,badarg,badarg,badarg,badarg,badarg,badarg,{error,badarg},{error,badarg}}\n"
    );
}

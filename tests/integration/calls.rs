//! Checks local and external calls, unresolved imports and malformed modules on PBeam.

use std::process::Command;

use pbeam_runtime::{BeamProgram, Term};

const CALLS: &[u8] = include_bytes!("../../fixtures/calls/calls.beam");
const EXTERNAL: &[u8] = include_bytes!("../../fixtures/calls/external.beam");

#[test]
fn executes_local_and_startup_loaded_external_calls_like_erts() {
    let program = BeamProgram::load(&[CALLS, EXTERNAL]).unwrap();
    assert_eq!(
        program.execute("calls", "main", &[]).unwrap().term,
        Term::Integer(7)
    );

    let reference = Command::new("erl")
        .args([
            "-noshell",
            "-pa",
            "fixtures/calls",
            "-eval",
            "io:format(\"~p~n\", [calls:main()]), halt().",
        ])
        .output()
        .unwrap();
    assert!(reference.status.success());
    assert_eq!(reference.stderr, <[u8; 0]>::default());
    assert_eq!(reference.stdout, b"7\n");
}

#[test]
fn reports_the_unresolved_external_mfa() {
    let program = BeamProgram::load(&[CALLS]).unwrap();
    assert!(
        program
            .execute("calls", "main", &[])
            .unwrap_err()
            .to_string()
            .contains("external:effect/1")
    );
}

#[test]
fn rejects_duplicate_modules_and_malformed_imports() {
    assert!(
        BeamProgram::load(&[CALLS, CALLS])
            .unwrap_err()
            .to_string()
            .contains("duplicate BEAM module calls")
    );

    let mut bad_imports = CALLS.to_vec();
    let import_chunk = bad_imports
        .windows(4)
        .position(|bytes| bytes == b"ImpT")
        .unwrap();
    bad_imports[import_chunk + 8..import_chunk + 12].copy_from_slice(&100u32.to_be_bytes());
    assert!(
        BeamProgram::load(&[&bad_imports])
            .unwrap_err()
            .to_string()
            .contains("invalid import table size")
    );

    let mut wrong_arity = CALLS.to_vec();
    let code_chunk = wrong_arity
        .windows(4)
        .position(|bytes| bytes == b"Code")
        .unwrap();
    let call = wrong_arity[code_chunk + 8..]
        .windows(3)
        .position(|bytes| bytes == [7, 16, 16])
        .unwrap()
        + code_chunk
        + 8;
    wrong_arity[call + 1] = 0;
    assert!(
        BeamProgram::load(&[&wrong_arity])
            .unwrap_err()
            .to_string()
            .contains("does not match import arity")
    );
}

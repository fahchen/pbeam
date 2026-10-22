//! The declared SHA-256 native example. A Rust function registered for the
//! `sha256/1` declaration of an Elixir module computes the digest with the
//! `sha2` crate, and the declarations are checked against the registry
//! before a run.

use pbeam_runtime::{
    BeamError, BeamProgram, Job, NativeResult, Term, register_native, unregister_native,
};
use sha2::{Digest, Sha256};

const NATIVE_HASH: &[u8] = include_bytes!("../../fixtures/native_sha256/Elixir.NativeHash.beam");

/// The SHA-256 digest of `abc` (FIPS 180-4).
const SHA256_ABC: [u8; 32] = [
    0xba, 0x78, 0x16, 0xbf, 0x8f, 0x01, 0xcf, 0xea, 0x41, 0x41, 0x40, 0xde, 0x5d, 0xae, 0x22, 0x23,
    0xb0, 0x03, 0x61, 0xa3, 0x96, 0x17, 0x7a, 0x9c, 0xb4, 0x10, 0xff, 0x61, 0xf2, 0x00, 0x15, 0xad,
];

/// Return the SHA-256 digest of a binary argument, and `badarg` for any
/// other argument, as a NIF of the declaration does.
fn sha256(arguments: &[Term]) -> Result<NativeResult, BeamError> {
    match arguments.first() {
        Some(Term::Binary(data)) => Ok(NativeResult::Value(Term::binary(
            Sha256::digest(&data[..]).to_vec(),
        ))),
        _ => Ok(NativeResult::badarg()),
    }
}

/// Run an exported function of the fixture with `argument`.
fn run(program: &BeamProgram, function: &str, argument: Term) -> Term {
    program
        .execute(
            "Elixir.NativeHash",
            function,
            &[program.host_term(argument)],
        )
        .unwrap()
        .term
}

/// Return the tuple `{class, reason}` that the fixture gives for a caught
/// exception.
fn caught(program: &BeamProgram, class: &str, reason: &str) -> Term {
    Term::tuple(vec![
        Term::Atom(program.atom(class).unwrap()),
        Term::Atom(program.atom(reason).unwrap()),
    ])
}

#[test]
fn computes_the_declared_sha256_native_as_the_example() {
    // The registry is global. The test unregisters its own MFA first, so a
    // previous test of this binary leaves no entry, and it unregisters the
    // MFA again at the end.
    let program = BeamProgram::load(&[NATIVE_HASH]).unwrap();
    let _ = unregister_native("Elixir.NativeHash", "sha256", 1);

    // Without a registration, the declarations fail the check with the MFA,
    // and a call still raises the error of the stub before any native runs.
    let missing = program
        .validate_native_declarations()
        .unwrap_err()
        .to_string();
    assert!(missing.contains("Elixir.NativeHash:sha256/1"), "{missing}");
    assert_eq!(
        run(&program, "invalid", Term::binary(b"abc".to_vec())),
        caught(&program, "error", "pbeam_native_only")
    );

    register_native("Elixir.NativeHash", "sha256", 1, Job::Normal, sha256).unwrap();

    // The declaration passes the check, and the known vector returns.
    program.validate_native_declarations().unwrap();
    let data = Term::binary(b"abc".to_vec());
    let digest = Term::binary(SHA256_ABC.to_vec());
    assert_eq!(run(&program, "digest", data.clone()), digest);

    // The input binary stays valid across a second call.
    assert_eq!(run(&program, "digest", data.clone()), digest);
    assert_eq!(data, Term::binary(b"abc".to_vec()));

    // A non-binary argument raises `badarg` in the native.
    assert_eq!(
        run(&program, "invalid", Term::Integer(1)),
        caught(&program, "error", "badarg")
    );

    unregister_native("Elixir.NativeHash", "sha256", 1);
}

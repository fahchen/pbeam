//! The term and memory `enif_*` functions of exqlite 0.37.0: the library of
//! `fixtures/nif/nif_terms.c` calls each one, and
//! `nif_terms:mismatches/1` compares each result with the result of the
//! pinned ERTS, which `expected/0` of `fixtures/nif/nif_terms.erl` holds.
//!
//! The test compiles the library with the vendored
//! `vendor/erts/include/erl_nif.h` of OTP 29.1.1 at test time. The ERTS
//! run used the same library, built on macOS:
//!
//! ```text
//! cc -bundle -undefined dynamic_lookup -Ivendor/erts/include -o /tmp/pbeam1288/nif_terms.so fixtures/nif/nif_terms.c
//! mise run erts-eval --pa $PWD/fixtures/nif 'nif_terms:mismatches("/tmp/pbeam1288/nif_terms")'
//! []
//! ```

#[path = "../common/mod.rs"]
mod common;

use std::path::Path;
use std::process::Command;

use common::pinned_beam;
use pbeam_runtime::{BeamError, BeamProgram, HostTerm, Term};

const NIF_TERMS: &[u8] = include_bytes!("../../fixtures/nif/nif_terms.beam");

/// Compile `nif_terms.c` as `name.so` and return its path without `.so`,
/// the path that `erlang:load_nif/2` gets. Each test builds its own
/// library, so two tests do not write one file.
fn build(name: &str) -> String {
    let directory = Path::new(env!("CARGO_TARGET_TMPDIR")).join("nif_terms");
    std::fs::create_dir_all(&directory).unwrap();
    let path = directory.join(name);
    let output = format!("{}.so", path.display());
    let shared: &[&str] = if cfg!(target_os = "macos") {
        &["-bundle", "-undefined", "dynamic_lookup"]
    } else {
        &["-shared", "-fPIC"]
    };
    let source = concat!(env!("CARGO_MANIFEST_DIR"), "/fixtures/nif/nif_terms.c");
    let include = concat!("-I", env!("CARGO_MANIFEST_DIR"), "/vendor/erts/include");
    let result = Command::new("cc")
        .args(shared)
        .args([include, "-o", &output, source])
        .output()
        .unwrap();
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    path.display().to_string()
}

/// Return `text` as a list of its bytes.
fn string(text: &str) -> Term {
    Term::list_from(text.bytes().map(|byte| Term::Integer(byte.into())))
}

/// Run `nif_terms:Function(Arguments)` with the library `path` as the
/// first argument and return its result.
fn execute(function: &str, path: &str, rest: Vec<Term>) -> Result<HostTerm, BeamError> {
    let program = BeamProgram::load(&[
        &pinned_beam("erlang"),
        &pinned_beam("erts_internal"),
        NIF_TERMS,
    ])
    .unwrap();
    let arguments: Vec<_> = std::iter::once(string(path))
        .chain(rest)
        .map(|term| program.host_term(term))
        .collect();
    program.execute("nif_terms", function, &arguments)
}

#[test]
fn gives_the_erts_results_of_the_term_and_memory_enif_calls() {
    let path = build("probes");
    let mismatches = execute("mismatches", &path, Vec::new()).unwrap();
    assert_eq!(mismatches.term, Term::Nil, "{mismatches:?}");
}

#[test]
fn stops_the_run_for_a_term_that_no_enif_call_made() {
    // ERTS makes an undefined term; it asserts the term in a debug build
    // only (`ASSERT_IN_ENV`, `erl_nif.c:2188-2189`).
    let path = build("bad_term");
    let failure = execute("call", &path, vec![Term::Integer(16), Term::Integer(0)]).unwrap_err();
    assert_eq!(
        failure.to_string(),
        "the NIF used the term 0x3039, which no enif call of the call made"
    );
}

#[test]
fn stops_the_run_for_an_enif_function_that_pbeam_does_not_implement() {
    // ERTS gives 0 for the compare of a term with itself. The stub of
    // `src/runtime/nif/abi/unsupported.rs` records the runtime error.
    let path = build("unsupported");
    let failure = execute("call", &path, vec![Term::Integer(27), Term::Integer(1)]).unwrap_err();
    assert!(
        failure
            .to_string()
            .ends_with("the NIF called enif_compare, which PBeam does not implement yet"),
        "{failure}"
    );
}

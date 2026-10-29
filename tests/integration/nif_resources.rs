//! The resource `enif_*` functions of exqlite 0.37.0: the library of
//! `fixtures/nif/nif_resources.c` calls each one, and
//! `nif_resources:mismatches/1` compares each result, the resource terms in
//! the operations of the language, and the destructor calls with the
//! results of the pinned ERTS, which `expected/0` of
//! `fixtures/nif/nif_resources.erl` holds.
//!
//! The test compiles the library with the vendored
//! `vendor/erts/include/erl_nif.h` of OTP 29.1.1 at test time. The ERTS
//! run used the same library, built on macOS:
//!
//! ```text
//! cc -bundle -undefined dynamic_lookup -Ivendor/erts/include -o /tmp/pbeam1291/nif_resources.so fixtures/nif/nif_resources.c
//! mise run erts-eval --pa $PWD/fixtures/nif 'nif_resources:mismatches("/tmp/pbeam1291/nif_resources")'
//! []
//! ```

#[path = "../common/mod.rs"]
mod common;

use std::path::Path;
use std::process::Command;
use std::sync::{Mutex, PoisonError};

use common::pinned_beam;
use pbeam_runtime::{BeamError, BeamProgram, HostTerm, Term};

const NIF_RESOURCES: &[u8] = include_bytes!("../../fixtures/nif/nif_resources.beam");

/// Compile `nif_resources.c` as `name.so` and return its path without `.so`,
/// the path that `erlang:load_nif/2` gets. Each test builds its own
/// library, so two tests do not write one file.
fn build(name: &str) -> String {
    let directory = Path::new(env!("CARGO_TARGET_TMPDIR")).join("nif_resources");
    std::fs::create_dir_all(&directory).unwrap();
    let path = directory.join(name);
    let output = format!("{}.so", path.display());
    let shared: &[&str] = if cfg!(target_os = "macos") {
        &["-bundle", "-undefined", "dynamic_lookup"]
    } else {
        &["-shared", "-fPIC"]
    };
    let source = concat!(env!("CARGO_MANIFEST_DIR"), "/fixtures/nif/nif_resources.c");
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

/// Held for each run. A library thread and a down callback send through the
/// run of the last load of a NIF library in the OS process, so a run of
/// another test at the same time takes their messages.
static ONE_RUN: Mutex<()> = Mutex::new(());

/// Return `text` as a list of its bytes.
fn string(text: &str) -> Term {
    Term::list_from(text.bytes().map(|byte| Term::Integer(byte.into())))
}

/// Run `nif_resources:Function(Arguments)` with the library `path` as the
/// first argument and return its result.
fn execute(function: &str, path: &str, rest: Vec<Term>) -> Result<HostTerm, BeamError> {
    let _run = ONE_RUN.lock().unwrap_or_else(PoisonError::into_inner);
    let program = BeamProgram::load(&[
        &pinned_beam("erlang"),
        &pinned_beam("erts_internal"),
        &pinned_beam("lists"),
        &pinned_beam("maps"),
        NIF_RESOURCES,
    ])
    .unwrap();
    let arguments: Vec<_> = std::iter::once(string(path))
        .chain(rest)
        .map(|term| program.host_term(term))
        .collect();
    program.execute("nif_resources", function, &arguments)
}

#[test]
fn gives_the_erts_results_of_the_resource_enif_calls() {
    let path = build("probes");
    let mismatches = execute("mismatches", &path, Vec::new()).unwrap();
    assert_eq!(mismatches.term, Term::Nil, "{mismatches:?}");
}

#[test]
fn returns_a_resource_term_that_keeps_its_object() {
    let path = build("returned");
    let made = execute("call", &path, vec![Term::Integer(2), Term::Integer(7)]).unwrap();
    let text = format!("{:?}", made.term);
    assert!(text.starts_with("Resource(Reference("), "{text}");
}

#[test]
fn stops_the_run_for_a_resource_term_of_a_pointer_that_is_not_an_object() {
    // ERTS reads the header of the pointer as a resource, which is
    // undefined behavior (`DATA_TO_RESOURCE`, `erl_nif.c:3197-3204`).
    let path = build("bad_object");
    let failure = execute("call", &path, vec![Term::Integer(12), Term::Integer(0)]).unwrap_err();
    assert!(
        failure
            .to_string()
            .ends_with(", which is not a live resource object"),
        "{failure}"
    );
}

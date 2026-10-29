//! The process, message, and mutex `enif_*` functions of exqlite 0.37.0:
//! the library of `fixtures/nif/nif_messages.c` calls each one, also
//! `enif_send` from a dirty NIF with a process-independent env and from a
//! thread of the library while the receiver waits, and
//! `nif_messages:mismatches/1` compares each result and each received
//! message with the results of the pinned ERTS, which `expected/0` of
//! `fixtures/nif/nif_messages.erl` holds.
//!
//! The test compiles the library with the vendored
//! `vendor/erts/include/erl_nif.h` of OTP 29.1.1 at test time. The ERTS
//! run used the same library, built on macOS:
//!
//! ```text
//! cc -bundle -undefined dynamic_lookup -Ivendor/erts/include -o /tmp/pbeam1291/nif_messages.so fixtures/nif/nif_messages.c
//! mise run erts-eval --pa $PWD/fixtures/nif 'nif_messages:mismatches("/tmp/pbeam1291/nif_messages")'
//! []
//! ```

#[path = "../common/mod.rs"]
mod common;

use std::path::Path;
use std::process::Command;

use common::pinned_beam;
use pbeam_runtime::{BeamError, BeamProgram, HostTerm, Term};

const NIF_MESSAGES: &[u8] = include_bytes!("../../fixtures/nif/nif_messages.beam");

/// Held by each test while its run executes. A thread of a NIF library
/// sends to the node of the last NIF library load of the OS process, and
/// each test runs its own node, so two runs of this binary do not overlap.
static RUNS: std::sync::Mutex<()> = std::sync::Mutex::new(());

/// Compile `nif_messages.c` as `name.so` and return its path without `.so`,
/// the path that `erlang:load_nif/2` gets. Each test builds its own
/// library, so two tests do not write one file.
fn build(name: &str) -> String {
    let directory = Path::new(env!("CARGO_TARGET_TMPDIR")).join("nif_messages");
    std::fs::create_dir_all(&directory).unwrap();
    let path = directory.join(name);
    let output = format!("{}.so", path.display());
    let shared: &[&str] = if cfg!(target_os = "macos") {
        &["-bundle", "-undefined", "dynamic_lookup"]
    } else {
        &["-shared", "-fPIC"]
    };
    let source = concat!(env!("CARGO_MANIFEST_DIR"), "/fixtures/nif/nif_messages.c");
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

/// Run `nif_messages:Function(Arguments)` with the library `path` as the
/// first argument and return its result.
fn execute(function: &str, path: &str, rest: Vec<Term>) -> Result<HostTerm, BeamError> {
    let _run = RUNS
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let program =
        BeamProgram::load(&[&pinned_beam("erlang"), &pinned_beam("lists"), NIF_MESSAGES]).unwrap();
    let arguments: Vec<_> = std::iter::once(string(path))
        .chain(rest)
        .map(|term| program.host_term(term))
        .collect();
    program.execute("nif_messages", function, &arguments)
}

#[test]
fn gives_the_erts_results_of_the_process_message_and_mutex_enif_calls() {
    let path = build("probes");
    let mismatches = execute("mismatches", &path, Vec::new()).unwrap();
    assert_eq!(mismatches.term, Term::Nil, "{mismatches:?}");
}

#[test]
fn stops_the_run_for_a_message_that_no_env_has() {
    // ERTS reads the handle as a term, which is undefined behavior.
    let path = build("bad_message");
    let failure = execute("call", &path, vec![Term::Integer(9), Term::Integer(1)]).unwrap_err();
    assert!(
        failure
            .to_string()
            .ends_with("enif_send sent the term 0x7ff0, which its env does not have"),
        "{failure}"
    );
}

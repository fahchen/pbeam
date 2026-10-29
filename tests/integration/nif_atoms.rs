//! The atoms of `enif_make_atom` in each env: the library of
//! `fixtures/nif/nif_atoms.c` keeps the atoms of its load callback and of
//! a process-independent env in static variables, as exqlite 0.37.0 does,
//! and uses them in later calls, also in a dirty NIF and in a message. It
//! compares atom handles of arguments, of the load info, and of list
//! elements with C `==`, as an ERTS atom is an immediate term.
//! `nif_atoms:mismatches/1` compares each result with the results of the
//! pinned ERTS, which `expected/0` of `fixtures/nif/nif_atoms.erl` holds.
//!
//! The test compiles the library with the vendored
//! `vendor/erts/include/erl_nif.h` of OTP 29.1.1 at test time. The ERTS
//! run used the same library, built on macOS:
//!
//! ```text
//! cc -bundle -undefined dynamic_lookup -Ivendor/erts/include -o /tmp/pbeam1305/nif_atoms.so fixtures/nif/nif_atoms.c
//! mise run erts-eval --pa $PWD/fixtures/nif 'nif_atoms:mismatches("/tmp/pbeam1305/nif_atoms")'
//! []
//! ```
#![expect(
    clippy::unwrap_used,
    reason = "test helpers report a failure by a panic"
)]

#[path = "../common/mod.rs"]
mod common;

use std::path::Path;
use std::process::Command;

use common::pinned_beam;
use pbeam_runtime::{BeamProgram, Term};

const NIF_ATOMS: &[u8] = include_bytes!("../../fixtures/nif/nif_atoms.beam");

/// Compile `nif_atoms.c` and return its path without `.so`, the path that
/// `erlang:load_nif/2` gets.
fn build() -> String {
    let directory = Path::new(env!("CARGO_TARGET_TMPDIR")).join("nif_atoms");
    std::fs::create_dir_all(&directory).unwrap();
    let path = directory.join("nif_atoms");
    let output = format!("{}.so", path.display());
    let shared: &[&str] = if cfg!(target_os = "macos") {
        &["-bundle", "-undefined", "dynamic_lookup"]
    } else {
        &["-shared", "-fPIC"]
    };
    let source = concat!(env!("CARGO_MANIFEST_DIR"), "/fixtures/nif/nif_atoms.c");
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

#[test]
fn gives_the_erts_results_of_the_atoms_of_the_load_callback_in_later_calls() {
    let path = build();
    let program =
        BeamProgram::load(&[&pinned_beam("erlang"), &pinned_beam("lists"), NIF_ATOMS]).unwrap();
    let argument = Term::list_from(path.bytes().map(|byte| Term::Integer(byte.into())));
    let mismatches = program
        .execute("nif_atoms", "mismatches", &[program.host_term(argument)])
        .unwrap();
    assert_eq!(mismatches.term, Term::Nil, "{mismatches:?}");
}

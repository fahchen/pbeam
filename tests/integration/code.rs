use std::process::Command;

use pbeam_runtime::{BeamProgram, Term};

#[path = "../common/mod.rs"]
mod common;

use common::pinned_beam;

const CODE_PROBE: &[u8] = include_bytes!("../../fixtures/code/code_probe.beam");

/// Evaluate an expression on the pinned ERTS with `code_probe` in the code
/// path and return its `~w` output.
fn reference(expression: &str) -> String {
    let reference = Command::new("erl")
        .args([
            "-noshell",
            "-pa",
            "fixtures/code",
            "-eval",
            &format!("io:format(\"~w~n\", [{expression}]), halt()."),
        ])
        .output()
        .unwrap();
    assert!(
        reference.status.success() && reference.stderr.is_empty(),
        "{reference:?}"
    );
    String::from_utf8(reference.stdout).unwrap()
}

/// Load the probe with the pinned `code` and `erlang` modules. The run
/// does not load `code_server`, as the example runs do not.
fn load() -> BeamProgram {
    let (code, erlang) = (pinned_beam("code"), pinned_beam("erlang"));
    BeamProgram::load(&[CODE_PROBE, &code, &erlang]).unwrap()
}

#[test]
fn runs_code_ensure_loaded_for_a_loaded_module_as_pinned_erts() {
    // The pinned `code:ensure_loaded/1` returns `{module, Mod}` when
    // `erlang:module_loaded/1` returns `true` (`code.erl:581-583`).
    let program = load();
    let atom = |name: &str| Term::Atom(program.atom(name).unwrap());
    let module = |name: &str| Term::Tuple(vec![atom("module"), atom(name)]);
    assert_eq!(
        program.execute("code_probe", "loaded", &[]).unwrap(),
        Term::List(vec![module("code_probe"), module("code"), module("erlang")])
    );
    assert_eq!(
        reference("code_probe:loaded()"),
        "[{module,code_probe},{module,code},{module,erlang}]\n"
    );
}

#[test]
fn runs_erlang_module_loaded_as_pinned_erts() {
    // ERTS `module_loaded_1` returns `true` for a module with current code
    // and no pending `-on_load` function, `false` for another atom, and
    // raises `badarg` for a term that is not an atom
    // (`beam_bif_load.c:738-755`).
    let program = load();
    let atom = |name: &str| Term::Atom(program.atom(name).unwrap());
    let badarg = Term::Tuple(vec![atom("error"), atom("badarg")]);
    assert_eq!(
        program.execute("code_probe", "module_loaded", &[]).unwrap(),
        Term::List(vec![
            atom("true"),
            atom("true"),
            atom("false"),
            badarg.clone(),
            badarg.clone(),
            badarg.clone(),
            badarg,
        ])
    );
    assert_eq!(
        reference("code_probe:module_loaded()"),
        "[true,true,false,{error,badarg},{error,badarg},{error,badarg},{error,badarg}]\n"
    );
}

#[test]
fn stops_code_ensure_loaded_for_a_module_that_is_not_loaded() {
    // For a module that is not loaded, the pinned `code:ensure_loaded/1`
    // reads the code mode with `code_server:get_mode/0`
    // (`code.erl:585`, `1278-1279`; `code_server.erl:84-85`) and then asks
    // the code server to load the module. PBeam has no code server and no
    // code path, and it does not load `code_server`, so the run stops.
    // ERTS searches the code path and returns `{error,nofile}`.
    let program = load();
    assert_eq!(
        program
            .execute("code_probe", "not_loaded", &[])
            .unwrap_err()
            .to_string(),
        "unresolved external MFA code_server:get_mode/0"
    );
    assert_eq!(reference("code_probe:not_loaded()"), "{error,nofile}\n");
}

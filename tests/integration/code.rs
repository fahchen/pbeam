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
        Term::list_from(vec![module("code_probe"), module("code"), module("erlang")])
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
        Term::list_from(vec![
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

const LOAD_PROBE: &[u8] = include_bytes!("../../fixtures/code/load_probe.beam");
const LOAD_TARGET: &[u8] = include_bytes!("../../fixtures/code/load_target.beam");
const LOAD_ON_LOAD: &[u8] = include_bytes!("../../fixtures/code/load_on_load.beam");

/// Load `load_probe` with the pinned `erlang` and `zlib` modules. The run
/// does not load `load_target` or `load_on_load`: `load_probe` gets their
/// BEAM bytes as a binary argument. `erlang:prepare_loading/2` calls
/// `zlib:gunzip/1` for bytes that do not start with `FOR1`.
fn load_program() -> BeamProgram {
    let (erlang, zlib) = (pinned_beam("erlang"), pinned_beam("zlib"));
    BeamProgram::load(&[LOAD_PROBE, &erlang, &zlib]).unwrap()
}

/// Run `load_probe:Function(Code)` with the bytes of a fixture and return
/// the `~w` text of the result.
fn load_run(function: &str, code: &[u8]) -> String {
    let result = load_program()
        .execute("load_probe", function, &[Term::Binary(code.to_vec())])
        .unwrap();
    format!("{}\n", text(&result))
}

/// Run `load_probe:Function(Code)` on the pinned ERTS with one scheduler,
/// with the bytes of `fixtures/code/<file>.beam`, and return the `~w` text
/// of the result. `-kernel logger_level none` drops the error report that
/// the ERTS loader sends for bad bytes (row "Code loading" of
/// `docs/erts-differences.md`).
fn load_reference(function: &str, file: &str) -> String {
    let reference = Command::new("erl")
        .args([
            "+S",
            "1",
            "-noshell",
            "-kernel",
            "logger_level",
            "none",
            "-pa",
            "fixtures/code",
            "-eval",
            &format!(
                "{{ok, Code}} = file:read_file(\"fixtures/code/{file}.beam\"), \
                 io:format(\"~w~n\", [load_probe:{function}(Code)]), halt()."
            ),
        ])
        .output()
        .unwrap();
    assert!(
        reference.status.success() && reference.stderr.is_empty(),
        "{reference:?}"
    );
    String::from_utf8(reference.stdout).unwrap()
}

/// Return the `~w` text of a term of atoms, tuples, and lists.
fn text(term: &Term) -> String {
    let join = |terms: &[Term]| terms.iter().map(text).collect::<Vec<_>>().join(",");
    match term {
        Term::Atom(atom) => atom.name().to_owned(),
        _ if let Some(terms) = term.proper_list() => {
            format!("[{}]", join(&terms.cloned().collect::<Vec<_>>()))
        }
        Term::Tuple(terms) => format!("{{{}}}", join(terms)),
        other => panic!("unexpected term {other:?}"),
    }
}

#[test]
fn loads_a_module_from_its_bytes_as_pinned_erts() {
    // `erlang:load_module/2` prepares and finishes the code
    // (`erlang.erl:7513-7528`). `erlang:module_loaded/1` and
    // `erlang:function_exported/3` read the code table of the run, and the
    // atom `load_target_value` of the new module goes into the atom table
    // of the run, so `list_to_atom/1` gives the same atom.
    let expected = "{{false,false},{module,load_target},{true,true},load_target_value,true}\n";
    assert_eq!(load_run("load", LOAD_TARGET), expected);
    assert_eq!(load_reference("load", "load_target"), expected);
    assert_eq!(load_program().atom("load_target_value"), None);
}

#[test]
fn finishes_prepared_code_as_pinned_erts() {
    // A prepared value is a reference. Two prepared values of one module
    // give `{duplicated, Modules}` (`beam_bif_load.c:429-441`), and a
    // finished value raises `badarg` (`beam_bif_load.c:385-392`).
    let expected = "[true,{duplicated,[load_target]},{duplicated,[load_target,load_target]},\
                    false,ok,ok,true,load_target_value,{error,badarg}]\n";
    assert_eq!(load_run("prepared", LOAD_TARGET), expected);
    assert_eq!(load_reference("prepared", "load_target"), expected);
}

#[test]
fn rejects_bad_code_and_arguments_as_pinned_erts() {
    // Bytes that are not an IFF form and a module name that is not the
    // given name give `{error, badfile}` (`beam_file.c:2040-2058`,
    // `beam_load.c:185-187`). The `zlib` NIF stubs of PBeam raise, as
    // `zlib:gunzip/1` raises for bytes that are not gzip data on ERTS, so
    // `<<"bad">>` gives `{error, badfile}` too. Arguments of the wrong type raise `badarg`
    // (`beam_bif_load.c:241-248`, `365-392`).
    let expected = "[{error,badfile},{error,badfile},{error,badfile},{error,badfile},\
                    {error,badfile},{error,badarg},{error,badarg},{error,badarg},{error,badarg},\
                    {error,badarg},{error,badarg},{error,badarg},{error,badarg},false,false]\n";
    assert_eq!(load_run("errors", LOAD_TARGET), expected);
    assert_eq!(load_reference("errors", "load_target"), expected);
}

#[test]
fn switches_out_the_caller_of_erlang_finish_loading_as_pinned_erts() {
    // ERTS suspends the caller of `erlang:finish_loading/1` until the new
    // code is committed (`beam_bif_load.c:555-566`), so the new process
    // sends its message first.
    let expected = "[{module,load_target},spawned,loaded]\n";
    assert_eq!(load_run("order", LOAD_TARGET), expected);
    assert_eq!(load_reference("order", "load_target"), expected);
}

#[test]
fn stops_the_run_for_code_replacement() {
    // ERTS makes the current code old and loads the new code
    // (`beam_load.c:258-262`). PBeam has no old code, so the second load
    // stops the run.
    assert_eq!(
        load_program()
            .execute(
                "load_probe",
                "replace",
                &[Term::Binary(LOAD_TARGET.to_vec())]
            )
            .unwrap_err()
            .to_string(),
        "erlang:finish_loading/1 cannot replace the loaded module load_target"
    );
    assert_eq!(
        load_reference("replace", "load_target"),
        "[{module,load_target},{module,load_target}]\n"
    );
}

#[test]
fn stops_the_run_for_a_module_with_an_on_load_function() {
    // ERTS loads the module, `erlang:finish_loading/1` returns
    // `{on_load, [load_on_load]}`, and the module waits for its
    // `-on_load` function (`beam_load.c:239-241`, `beam_bif_load.c:738-755`).
    // The PBeam loader does not decode the `on_load` instruction.
    assert_eq!(
        load_program()
            .execute(
                "load_probe",
                "on_load",
                &[Term::Binary(LOAD_ON_LOAD.to_vec())]
            )
            .unwrap_err()
            .to_string(),
        "erts_internal:prepare_loading/2 cannot load module load_on_load: \
         unsupported BEAM opcode 149"
    );
    assert_eq!(
        load_reference("on_load", "load_on_load"),
        "[{error,on_load},false]\n"
    );
}

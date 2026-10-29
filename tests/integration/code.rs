//! Runs `code:ensure_loaded`, `erlang:module_loaded` and the module load functions on PBeam and
//! compares them with the pinned ERTS.

use std::process::Command;

use pbeam_runtime::{
    AtomTable, BeamProgram, Capability, CapabilityKind, Constraint, Delegation, Operation, Policy,
    Term,
};

#[path = "../common/mod.rs"]
mod common;

use common::pinned_beam;

/// A test policy: the root process gets these capabilities.
struct Grants(Vec<Capability>);

impl Policy for Grants {
    fn initial(&self) -> Vec<Capability> {
        self.0.clone()
    }
}

/// Return the policy of one `code` capability that permits `operations`
/// within `modules`.
fn code_capability(operations: &[Operation], modules: &[&str]) -> Grants {
    Grants(vec![
        Capability::new(
            CapabilityKind::Code,
            operations,
            Constraint::Only(modules.iter().map(|&module| module.to_owned()).collect()),
            Delegation::NotDelegable,
        )
        .unwrap(),
    ])
}

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
    let module = |name: &str| Term::tuple(vec![atom("module"), atom(name)]);
    assert_eq!(
        program.execute("code_probe", "loaded", &[]).unwrap().term,
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
    let badarg = Term::tuple(vec![atom("error"), atom("badarg")]);
    assert_eq!(
        program
            .execute("code_probe", "module_loaded", &[])
            .unwrap()
            .term,
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
    let program = load_program();
    let result = program
        .execute(
            "load_probe",
            function,
            &[program.host_term(Term::binary(code.to_vec()))],
        )
        .unwrap();
    format!("{}\n", text(&result.term, result.atoms()))
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
                "{{ok, Code}} = file:read_file(\"{file}.beam\"), \
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

/// Return the `~w` text of a term of atoms, integers, tuples, and lists.
/// The atom names come from `atoms`, the table that owns the term.
fn text(term: &Term, atoms: &AtomTable) -> String {
    let join = |terms: &[Term]| {
        terms
            .iter()
            .map(|term| text(term, atoms))
            .collect::<Vec<_>>()
            .join(",")
    };
    match term {
        Term::Atom(atom) => atoms.name_of(atom).to_owned(),
        Term::Integer(value) => value.to_string(),
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
    assert_eq!(
        load_reference("load", "fixtures/code/load_target"),
        expected
    );
    assert_eq!(load_program().atom("load_target_value"), None);
}

#[test]
fn calls_an_import_before_and_after_the_load_of_its_module_as_pinned_erts() {
    // Each `call_ext` names the export entry of its import, which the ERTS
    // loader patches into the code (`jit/asm_load.c:1198-1206`). Before the
    // load, the entry calls the error handler of the process
    // (`call_error_handler`, `beam_common.c:1294-1342`), here
    // `load_probe:undefined_function/3`. The load sets the address of each
    // exported function, so the same `call_ext_only`, `call_ext`, and
    // `call_ext_last` sites then run `load_target:value/0`. The undefined
    // `load_target:missing/0` calls the handler again after the load.
    // `erlang:apply/2,3` call through the export as well.
    let before = "{undefined,load_target,value,[]}";
    let missing = "{undefined,load_target,missing,[]}";
    let after = "load_target_value";
    let calls = |value: &str| {
        format!(
            "[{value},{value},[{value}],{value},{missing},{missing},{value},{missing},{value},[[{value}]]]"
        )
    };
    let expected = format!(
        "{{{},{{module,load_target}},{}}}\n",
        calls(before),
        calls(after)
    );
    assert_eq!(load_run("late", LOAD_TARGET), expected);
    assert_eq!(
        load_reference("late", "fixtures/code/load_target"),
        expected
    );
}

#[test]
fn finishes_prepared_code_as_pinned_erts() {
    // A prepared value is a reference. Two prepared values of one module
    // give `{duplicated, Modules}` (`beam_bif_load.c:429-441`), and a
    // finished value raises `badarg` (`beam_bif_load.c:385-392`).
    let expected = "[true,{duplicated,[load_target]},{duplicated,[load_target,load_target]},\
                    false,ok,ok,true,load_target_value,{error,badarg}]\n";
    assert_eq!(load_run("prepared", LOAD_TARGET), expected);
    assert_eq!(
        load_reference("prepared", "fixtures/code/load_target"),
        expected
    );
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
    assert_eq!(
        load_reference("errors", "fixtures/code/load_target"),
        expected
    );
}

#[test]
fn switches_out_the_caller_of_erlang_finish_loading_as_pinned_erts() {
    // ERTS suspends the caller of `erlang:finish_loading/1` until the new
    // code is committed (`beam_bif_load.c:555-566`), so the new process
    // sends its message first.
    let expected = "[{module,load_target},spawned,loaded]\n";
    assert_eq!(load_run("order", LOAD_TARGET), expected);
    assert_eq!(
        load_reference("order", "fixtures/code/load_target"),
        expected
    );
}

#[test]
fn stops_the_run_for_code_replacement() {
    // ERTS makes the current code old and loads the new code
    // (`beam_load.c:258-262`). PBeam has no old code, so the second load
    // stops the run.
    let program = load_program();
    assert_eq!(
        program
            .execute(
                "load_probe",
                "replace",
                &[program.host_term(Term::binary(LOAD_TARGET.to_vec()))]
            )
            .unwrap_err()
            .to_string(),
        "erlang:finish_loading/1 cannot replace the loaded module load_target"
    );
    assert_eq!(
        load_reference("replace", "fixtures/code/load_target"),
        "[{module,load_target},{module,load_target}]\n"
    );
}

#[test]
fn compares_the_module_name_of_code_load() {
    // The `modules` constraint of the `code` `load` capability compares the
    // module name of each element of the list of `erlang:finish_loading/1`
    // (`beam_bif_load.c:381-394`), after the argument checks and before any
    // module loads. An empty list has no module name.
    let program = load_program();
    let expected = |name| Term::Atom(program.atom(name).unwrap());
    let code = Term::binary(LOAD_TARGET.to_vec());
    let permitted = code_capability(&[Operation::Load], &["load_target"]);
    assert_eq!(
        program
            .execute_with_policy(
                "load_probe",
                "constraint",
                std::slice::from_ref(&program.host_term(code.clone())),
                &permitted
            )
            .unwrap()
            .term,
        expected("ok")
    );
    let denied = code_capability(&[Operation::Load], &["other_module"]);
    assert_eq!(
        program
            .execute_with_policy(
                "load_probe",
                "constraint",
                &[program.host_term(code)],
                &denied
            )
            .unwrap()
            .term,
        expected("denied")
    );
}

#[test]
fn loads_a_module_with_an_on_load_function_as_pinned_erts() {
    // `erlang:finish_loading/1` returns `{on_load, [load_on_load]}`
    // (`beam_bif_load.c:484-504`), so `erlang:load_module/2` returns
    // `{error, on_load}` (`erlang.erl:7513-7528`), and the module waits for
    // its `-on_load` function: `erlang:module_loaded/1` gives `false`
    // (`beam_bif_load.c:738-755`).
    let program = load_program();
    let result = program
        .execute(
            "load_probe",
            "on_load",
            &[program.host_term(Term::binary(LOAD_ON_LOAD.to_vec()))],
        )
        .unwrap();
    let expected = load_reference("on_load", "fixtures/code/load_on_load");
    assert_eq!(expected, "[{error,on_load},false]\n");
    assert_eq!(
        format!("{}\n", text(&result.term, result.atoms())),
        expected
    );
}

#[test]
fn runs_the_on_load_steps_of_the_code_server_as_pinned_erts() {
    // `load_probe:on_load_steps/2` runs the steps of `code_server` and
    // `erts_code_purger` (code_server.erl:1348-1352, 1386-1398;
    // erts_code_purger.erl:166-167): `erlang:finish_loading/1` gives
    // `{on_load, [Module]}` and hides the exports, a group with the module
    // raises `system_limit` (`beam_bif_load.c:395-408`),
    // `erlang:call_on_load_function/1` runs the function as the
    // `i_call_on_load_function` instruction and raises `badarg` through
    // `apply/3` (`jit/arm/ops.tab:630-641`, `beam_bif_load.c:793-817`), and
    // `erlang:finish_after_on_load/2` keeps or drops the code
    // (`beam_bif_load.c:819-941`). After `false`, the code stays until a
    // purge, so the function runs again and `false` gives `true` again.
    let program = load_program();
    for module in ["on_load_ok", "on_load_error", "on_load_crash"] {
        let path = format!("fixtures/code_server/{module}");
        let code = std::fs::read(format!("{path}.beam")).unwrap();
        let result = program
            .execute(
                "load_probe",
                module,
                &[program.host_term(Term::binary(code))],
            )
            .unwrap();
        assert_eq!(
            format!("{}\n", text(&result.term, result.atoms())),
            load_reference(module, &path),
            "{module}"
        );
    }
}

#[test]
fn stops_the_run_when_the_code_of_a_dropped_on_load_function_is_kept() {
    // ERTS makes the code current with exports that call the error handler
    // (`beam_bif_load.c:887-897`). PBeam has no such export state.
    let program = load_program();
    let code = std::fs::read("fixtures/code_server/on_load_ok.beam").unwrap();
    assert_eq!(
        program
            .execute(
                "load_probe",
                "keep_after_drop",
                &[program.host_term(Term::binary(code))]
            )
            .unwrap_err()
            .to_string(),
        "erlang:finish_after_on_load/2 cannot keep the code of on_load_ok after its \
         exports were dropped"
    );
}

#[test]
fn stops_the_run_for_a_module_that_the_loader_cannot_decode() {
    // ERTS loads the module (row "Code loading" of docs/erts-differences.md).
    let program = load_program();
    let code = include_bytes!("../../fixtures/code/load_unsupported.beam");
    assert_eq!(
        program
            .execute(
                "load_probe",
                "unsupported",
                &[program.host_term(Term::binary(code.to_vec()))]
            )
            .unwrap_err()
            .to_string(),
        "erts_internal:prepare_loading/2 cannot load module load_unsupported: \
         unsupported BEAM opcode 183"
    );
}

#[test]
fn stops_for_a_start_module_with_an_on_load_function() {
    // The ERTS boot runs the `-on_load` function of each module that the
    // boot script loads (init.erl:1381-1403, 1857-1880). The start set of
    // PBeam has no boot script.
    assert_eq!(
        BeamProgram::load(&[LOAD_ON_LOAD]).unwrap_err().to_string(),
        "the start module load_on_load has an -on_load function, which PBeam \
         runs only for a module that loads in the run"
    );
}

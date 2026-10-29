//! The static Rust extension API, as an extension crate uses it: the test
//! uses only the public API of `pbeam_runtime`. It registers a `Library` of
//! `Binding` entries for each fixture of `fixtures/extension` before the
//! run. The expected values are the values of the pinned ERTS with C NIF
//! libraries of the same functions, built with `cc -bundle`: `{"add", 2,
//! add, 0}` with a load callback that returns 0 only for the load info `ok`
//! for `extension_probe`, `{"unbound", 0, f, 0}` for `extension_strict`,
//! `{"twice", 0, f, 0}` two times for `extension_twice`, and `{"add", 2,
//! add, 0}` for `extension_bare`:
//!
//! ```text
//! mise run erts-eval --pa $PWD/fixtures/extension 'ok = file:set_cwd("/tmp/pbeam854"),
//!   {extension_probe:calls(error), extension_probe:calls(ok), extension_probe:add(1,2),
//!    extension_probe:invalid(), extension_twice:load(), extension_bare:calls()}'
//! {{{error,undef},{error,{load,"Library load-call unsuccessful (1)."}},
//!   {error,undef},{error,{load,"Library load-call unsuccessful (1)."}}},
//!  {{error,undef},ok,3,
//!   {error,{reload,"NIF library already loaded (reload disallowed since OTP 20)."}}},
//!  3, {error,badarg},
//!  {error,{bad_lib,"Duplicate NIF entry for extension_twice:twice/0"}},
//!  {fallback,ok,fallback}}
//! mise run erts-eval --pa $PWD/fixtures/extension 'ok = file:set_cwd("/tmp/pbeam854"),
//!   {ok, B} = file:read_file("fixtures/extension/extension_probe.beam"),
//!   extension_probe:reload(B)'
//! {{{error,undef},ok,3,{error,{reload,"NIF library already loaded (...)."}}},
//!  {{error,undef},{error,{upgrade,"Upgrade not supported by this NIF library."}},
//!   {error,undef},{error,{upgrade,"Upgrade not supported by this NIF library."}}}}
//! ```
//!
//! After the load of `extension_bare`, which has no `nif_start`, ERTS runs
//! the NIF for an external call and the BEAM body for a local call.

#[path = "../common/mod.rs"]
mod common;

use std::sync::Once;

use common::pinned_beam;
use pbeam_runtime::{
    BeamError, BeamModule, BeamProgram, Binding, HostTerm, Job, Library, NativeResult, NifContext,
    NifGap, RegistrationError, Term, nif_manifest_gaps, register_library,
};

const EXTENSION_PROBE: &[u8] = include_bytes!("../../fixtures/extension/extension_probe.beam");
const EXTENSION_STRICT: &[u8] = include_bytes!("../../fixtures/extension/extension_strict.beam");
const EXTENSION_TWICE: &[u8] = include_bytes!("../../fixtures/extension/extension_twice.beam");
const EXTENSION_BARE: &[u8] = include_bytes!("../../fixtures/extension/extension_bare.beam");

/// Return the sum of two integers, and `badarg` for any other argument.
fn add(context: &mut NifContext<'_>, arguments: &[Term]) -> Result<NativeResult, BeamError> {
    match arguments {
        [Term::Integer(left), Term::Integer(right)] => {
            Ok(NativeResult::Value(Term::Integer(left + right)))
        }
        _ => Ok(context.badarg()),
    }
}

/// Return the atom of a binary name, which the context makes.
fn make_atom(context: &mut NifContext<'_>, arguments: &[Term]) -> Result<NativeResult, BeamError> {
    let [Term::Binary(name)] = arguments else {
        return Ok(context.badarg());
    };
    let name = String::from_utf8(
        name.bytes()
            .expect("the name is a binary of whole bytes")
            .to_vec(),
    )
    .unwrap();
    Ok(NativeResult::Value(Term::Atom(context.atom(&name)?)))
}

/// Load the library only for the load info `ok`, as the load callback of
/// the C library of the probe does.
fn load(context: &mut NifContext<'_>, info: &Term) -> i32 {
    let ok = Term::Atom(context.atom("ok").unwrap());
    i32::from(*info != ok)
}

/// The binding `add/2`.
const ADD: Binding = Binding {
    name: "add",
    arity: 2,
    job: Job::Normal,
    run: add,
};

/// The library of `extension_probe`. `unbound/1` has no binding and runs
/// its stub body.
static PROBE_LIBRARY: Library = Library {
    module: "extension_probe",
    functions: &[
        ("add", 2),
        ("unbound", 1),
        ("make_atom", 1),
        ("dirty_atom", 1),
    ],
    bindings: &[
        ADD,
        Binding {
            name: "make_atom",
            arity: 1,
            job: Job::Normal,
            run: make_atom,
        },
        Binding {
            name: "dirty_atom",
            arity: 1,
            job: Job::CpuBound,
            run: make_atom,
        },
    ],
    load: Some(load),
    unbound_fails: false,
};

/// The library of `extension_strict`: a call of a function without a
/// binding fails.
static STRICT_LIBRARY: Library = Library {
    module: "extension_strict",
    functions: &[("unbound", 0)],
    bindings: &[],
    load: None,
    unbound_fails: true,
};

/// The library of `extension_twice`, which lists `twice/0` two times.
static TWICE_LIBRARY: Library = Library {
    module: "extension_twice",
    functions: &[("twice", 0), ("twice", 0)],
    bindings: &[],
    load: None,
    unbound_fails: false,
};

/// The library of `extension_bare`, whose module has no `nif_start`.
static BARE_LIBRARY: Library = Library {
    module: "extension_bare",
    functions: &[("add", 2)],
    bindings: &[ADD],
    load: None,
    unbound_fails: false,
};

/// Register the libraries of the fixtures once for the test binary, and
/// load the fixtures with the pinned modules that `erlang:load_module/2`
/// needs.
fn program() -> BeamProgram {
    static REGISTER: Once = Once::new();
    REGISTER.call_once(|| {
        for library in [
            &PROBE_LIBRARY,
            &STRICT_LIBRARY,
            &TWICE_LIBRARY,
            &BARE_LIBRARY,
        ] {
            register_library(library).unwrap();
        }
    });
    BeamProgram::load(&[
        &pinned_beam("erlang"),
        &pinned_beam("erts_internal"),
        EXTENSION_PROBE,
        EXTENSION_STRICT,
        EXTENSION_TWICE,
        EXTENSION_BARE,
    ])
    .unwrap()
}

/// Run `Module:Function(Arguments)` and return its result.
fn execute(
    program: &BeamProgram,
    module: &str,
    function: &str,
    arguments: Vec<Term>,
) -> Result<HostTerm, BeamError> {
    let arguments: Vec<_> = arguments
        .into_iter()
        .map(|term| program.host_term(term))
        .collect();
    program.execute(module, function, &arguments)
}

/// Return the atom `name` of the table of `result`.
fn atom(result: &HostTerm, name: &str) -> Term {
    Term::Atom(result.atom(name).unwrap())
}

/// Return `{error, {Kind, Text}}` with the atoms of `result` and `Text` as
/// a list of its bytes.
fn load_error(result: &HostTerm, kind: &str, text: &str) -> Term {
    let text = text.bytes().map(|byte| Term::Integer(byte.into()));
    Term::tuple(vec![
        atom(result, "error"),
        Term::tuple(vec![atom(result, kind), Term::list_from(text)]),
    ])
}

/// Return `{error, Reason}` with the atoms of `result`.
fn caught(result: &HostTerm, reason: &str) -> Term {
    Term::tuple(vec![atom(result, "error"), atom(result, reason)])
}

/// Return the atom `name` of `program` as an argument.
fn argument(program: &BeamProgram, name: &str) -> Term {
    Term::Atom(program.atom(name).unwrap())
}

#[test]
fn runs_a_binding_only_after_a_successful_load_as_erts() {
    let program = program();
    let vetoed = execute(
        &program,
        "extension_probe",
        "calls",
        vec![argument(&program, "error")],
    )
    .unwrap();
    let veto = load_error(&vetoed, "load", "Library load-call unsuccessful (1).");
    let undef = caught(&vetoed, "undef");
    assert_eq!(
        vetoed.term,
        Term::tuple(vec![undef.clone(), veto.clone(), undef, veto])
    );

    let loaded = execute(&program, "extension_probe", "loaded", vec![]).unwrap();
    let reload = "NIF library already loaded (reload disallowed since OTP 20).";
    assert_eq!(
        loaded.term,
        Term::tuple(vec![
            caught(&loaded, "undef"),
            atom(&loaded, "ok"),
            Term::Integer(3),
            load_error(&loaded, "reload", reload),
        ])
    );
}

#[test]
fn starts_the_new_code_of_the_module_without_its_library_as_erts() {
    let program = program();
    let binary = Term::binary(EXTENSION_PROBE.to_vec());
    let result = execute(&program, "extension_probe", "reload", vec![binary]).unwrap();
    let reload = "NIF library already loaded (reload disallowed since OTP 20).";
    let upgrade = load_error(
        &result,
        "upgrade",
        "Upgrade not supported by this NIF library.",
    );
    let undef = caught(&result, "undef");
    assert_eq!(
        result.term,
        Term::tuple(vec![
            Term::tuple(vec![
                undef.clone(),
                atom(&result, "ok"),
                Term::Integer(3),
                load_error(&result, "reload", reload),
            ]),
            Term::tuple(vec![undef.clone(), upgrade.clone(), undef, upgrade]),
        ])
    );
}

#[test]
fn refuses_a_load_from_old_code_and_leaves_the_new_code_unloaded_as_erts() {
    // `extension_probe:old_load(B)` on the pinned ERTS:
    // {{error,{old_code,"Calling load_nif from old module 'extension_probe' not allowed"}},
    //  {{error,undef},ok,3,{error,{reload,"NIF library already loaded (...)."}}}}
    let program = program();
    let binary = Term::binary(EXTENSION_PROBE.to_vec());
    let result = execute(&program, "extension_probe", "old_load", vec![binary]).unwrap();
    let old = "Calling load_nif from old module 'extension_probe' not allowed";
    let reload = "NIF library already loaded (reload disallowed since OTP 20).";
    assert_eq!(
        result.term,
        Term::tuple(vec![
            load_error(&result, "old_code", old),
            Term::tuple(vec![
                caught(&result, "undef"),
                atom(&result, "ok"),
                Term::Integer(3),
                load_error(&result, "reload", reload),
            ]),
        ])
    );
}

#[test]
fn gives_the_load_errors_of_erts_and_the_unbound_behavior_of_the_library() {
    let program = program();
    let twice = execute(&program, "extension_twice", "load", vec![]).unwrap();
    let text = "Duplicate NIF entry for extension_twice:twice/0";
    assert_eq!(twice.term, load_error(&twice, "bad_lib", text));

    let with_load = |name: &str, arguments: Vec<Term>| {
        let arguments = vec![argument(&program, name), Term::list_from(arguments)];
        execute(&program, "extension_probe", "with_load", arguments)
    };
    let invalid = with_load("invalid", vec![]).unwrap();
    assert_eq!(invalid.term, caught(&invalid, "badarg"));
    // A function without a binding runs its stub body, which raises `undef`.
    let stub = with_load("unbound", vec![Term::Integer(1)]).unwrap_err();
    assert!(stub.to_string().contains("undef"), "{stub}");
    // After the load, a function without a binding of a library that sets
    // `unbound_fails` gives the runtime error of the function.
    let strict = execute(&program, "extension_strict", "load", vec![]).unwrap_err();
    assert_eq!(
        strict.to_string(),
        "extension_strict:unbound/0 NIF is not supported"
    );
}

#[test]
fn refuses_the_load_of_a_module_without_nif_start() {
    let program = program();
    let bare = execute(&program, "extension_bare", "calls", vec![]).unwrap_err();
    assert_eq!(
        bare.to_string(),
        "unsupported: erlang:load_nif/2 of the extension library of extension_bare: the module has no nif_start instruction"
    );
}

#[test]
fn calls_a_library_that_another_scheduler_loaded() {
    // The node keeps the record of the load, so a call on the other
    // scheduler runs the binding (`beam/erl_nif.c:4990-4997`).
    let program = program();
    let ok = Term::Atom(program.atom("ok").unwrap());
    for _ in 0..10 {
        let results = program
            .execute_on_schedulers(&[
                ("extension_probe", "cross_load"),
                ("extension_probe", "cross_call"),
            ])
            .unwrap();
        assert_eq!(results[0].term, ok);
        assert_eq!(results[1].term, Term::Integer(3));
    }
}

#[test]
fn gives_the_atoms_of_the_run_on_the_scheduler_and_static_atoms_on_a_dirty_thread() {
    let program = program();
    let call = |function: &str, name: &str| {
        let name = Term::binary(name.as_bytes().to_vec());
        let arguments = vec![argument(&program, function), Term::list_from([name])];
        execute(&program, "extension_probe", "with_load", arguments).map(|result| result.term)
    };
    assert_eq!(
        call("atom_name", "extension_fresh").unwrap(),
        Term::binary(b"extension_fresh".to_vec())
    );
    // `erts_atom_put` takes at most 255 characters (`MAX_ATOM_CHARACTERS`).
    let longest = "é".repeat(255);
    assert_eq!(
        call("atom_name", &longest).unwrap(),
        Term::binary(longest.into_bytes())
    );
    assert_eq!(
        call("atom_name", &"a".repeat(256)).unwrap_err().to_string(),
        "the atom name has more than 255 characters"
    );
    assert_eq!(
        call("dirty_atom_name", "ok").unwrap(),
        Term::binary(b"ok".to_vec())
    );
    assert_eq!(
        call("dirty_atom_name", "extension_fresh")
            .unwrap_err()
            .to_string(),
        "unsupported: a dirty NIF cannot make the atom extension_fresh, which is not a static atom"
    );
}

#[test]
fn rejects_a_duplicate_library_and_a_binding_outside_the_functions() {
    static PRIM_TTY: Library = Library {
        module: "prim_tty",
        functions: &[],
        bindings: &[],
        load: None,
        unbound_fails: false,
    };

    static UNLISTED: Library = Library {
        module: "extension_unlisted",
        functions: &[("add", 1)],
        bindings: &[ADD],
        load: None,
        unbound_fails: false,
    };

    static DOUBLED: Library = Library {
        module: "extension_doubled",
        functions: &[("add", 2)],
        bindings: &[ADD, ADD],
        load: None,
        unbound_fails: false,
    };

    program();
    assert_eq!(
        register_library(&PROBE_LIBRARY),
        Err(RegistrationError::Duplicate)
    );
    assert_eq!(
        register_library(&PRIM_TTY),
        Err(RegistrationError::Duplicate)
    );
    assert_eq!(
        register_library(&UNLISTED),
        Err(RegistrationError::UnlistedBinding {
            name: "add",
            arity: 2
        })
    );
    assert_eq!(
        register_library(&DOUBLED),
        Err(RegistrationError::DuplicateBinding {
            name: "add",
            arity: 2
        })
    );
}

#[test]
fn shows_the_registered_library_to_the_nif_manifest_gate() {
    program();
    let probe = BeamModule::load(EXTENSION_PROBE).unwrap();
    assert_eq!(
        nif_manifest_gaps(&probe, &[]),
        [NifGap::Unbound {
            library: "extension_probe".to_owned(),
            function: "unbound".to_owned(),
            arity: 1,
            call_fails: false,
        }]
    );
    assert_eq!(
        nif_manifest_gaps(&probe, &[])[0].to_string(),
        "extension_probe:unbound/1: NIF function has no PBeam binding; a call runs the BEAM stub body"
    );
}

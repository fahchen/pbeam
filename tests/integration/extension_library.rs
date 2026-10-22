//! The static Rust extension API, as an extension crate uses it: the test
//! uses only the public API of `pbeam_runtime`. It registers a `Library` of
//! `Binding` entries for each fixture of `fixtures/extension` before the
//! run. The `erlang:load_nif/2` results are the results of the pinned ERTS
//! with C NIF libraries of the same functions, built with `cc -bundle` from
//! `ErlNifFunc` tables `{"add", 2, add, 0}` for `extension_probe`, with a
//! load callback that returns 0 only for the load info `ok`, and
//! `{"unbound", 0, f, 0}` two times for `extension_strict`:
//!
//! ```text
//! mise run erts-eval --pa $PWD/fixtures/extension 'ok = file:set_cwd("/tmp/pbeam854"),
//!   U = fun(F) -> try F() catch C:R -> {C, R} end end,
//!   {extension_strict:load(), extension_probe:load(nope), extension_probe:load(ok),
//!    extension_probe:load(ok), extension_probe:add(1,2), extension_probe:invalid(),
//!    U(fun() -> extension_probe:unbound(1) end)}'
//! {{error,{bad_lib,"Duplicate NIF entry for extension_strict:unbound/0"}},
//!  {error,{load,"Library load-call unsuccessful (1)."}},
//!  ok,
//!  {error,{reload,"NIF library already loaded (reload disallowed since OTP 20)."}},
//!  3,
//!  {error,badarg},
//!  {error,undef}}
//! ```

use std::sync::Once;

use pbeam_runtime::{
    BeamError, BeamModule, BeamProgram, Binding, Job, Library, NativeResult, NifContext,
    RegistrationError, Term, nif_manifest_gaps, register_library,
};

const EXTENSION_PROBE: &[u8] = include_bytes!("../../fixtures/extension/extension_probe.beam");
const EXTENSION_STRICT: &[u8] = include_bytes!("../../fixtures/extension/extension_strict.beam");

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
    let name = String::from_utf8(name.to_vec()).unwrap();
    Ok(NativeResult::Value(Term::Atom(context.atom(&name)?)))
}

/// Load the library only for the load info `ok`, as the load callback of
/// the C library of the probe does.
fn load(context: &mut NifContext<'_>, info: &Term) -> i32 {
    let ok = Term::Atom(context.atom("ok").unwrap());
    i32::from(*info != ok)
}

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
        Binding {
            name: "add",
            arity: 2,
            job: Job::Normal,
            run: add,
        },
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

/// The library of `extension_strict`: `unbound/0` is in it two times, and a
/// call of a function without a binding fails.
static STRICT_LIBRARY: Library = Library {
    module: "extension_strict",
    functions: &[("unbound", 0), ("unbound", 0)],
    bindings: &[],
    load: None,
    unbound_fails: true,
};

/// Register the libraries of the fixtures once for the test binary, and
/// load the fixtures.
fn program() -> BeamProgram {
    static REGISTER: Once = Once::new();
    REGISTER.call_once(|| {
        register_library(&PROBE_LIBRARY).unwrap();
        register_library(&STRICT_LIBRARY).unwrap();
    });
    BeamProgram::load(&[EXTENSION_PROBE, EXTENSION_STRICT]).unwrap()
}

/// Run `Module:Function(Arguments)` and return its value.
fn run(program: &BeamProgram, module: &str, function: &str, arguments: Vec<Term>) -> Term {
    let arguments: Vec<_> = arguments
        .into_iter()
        .map(|term| program.host_term(term))
        .collect();
    program.execute(module, function, &arguments).unwrap().term
}

/// Return the atom `name` of the program.
fn atom(program: &BeamProgram, name: &str) -> Term {
    Term::Atom(program.atom(name).unwrap())
}

/// Check that `Module:load(Arguments)` gives `{error, {Kind, Text}}`, with
/// `Text` as a list of its bytes. The run makes the atom `Kind`, so the
/// atom comes from the table of the result.
fn assert_load_error(
    program: &BeamProgram,
    module: &str,
    arguments: &[Term],
    kind: &str,
    text: &str,
) {
    let arguments: Vec<_> = arguments
        .iter()
        .map(|term| program.host_term(term.clone()))
        .collect();
    let result = program.execute(module, "load", &arguments).unwrap();
    let text = text.bytes().map(|byte| Term::Integer(byte.into()));
    let expected = Term::tuple(vec![
        Term::Atom(result.atom("error").unwrap()),
        Term::tuple(vec![
            Term::Atom(result.atom(kind).unwrap()),
            Term::list_from(text),
        ]),
    ]);
    assert_eq!(result.term, expected);
}

#[test]
fn loads_the_library_with_the_checks_and_the_load_callback_of_erts() {
    let program = program();
    let error = atom(&program, "error");
    let text = "Library load-call unsuccessful (1).";
    assert_load_error(&program, "extension_probe", &[error], "load", text);
    assert_eq!(
        run(
            &program,
            "extension_probe",
            "load",
            vec![atom(&program, "ok")]
        ),
        atom(&program, "ok")
    );
    let text = "Duplicate NIF entry for extension_strict:unbound/0";
    assert_load_error(&program, "extension_strict", &[], "bad_lib", text);
}

#[test]
fn runs_the_bindings_and_the_unbound_behavior_of_the_library() {
    let program = program();
    assert_eq!(
        run(
            &program,
            "extension_probe",
            "add",
            vec![Term::Integer(1), Term::Integer(2)]
        ),
        Term::Integer(3)
    );
    assert_eq!(
        run(&program, "extension_probe", "invalid", vec![]),
        Term::tuple(vec![atom(&program, "error"), atom(&program, "badarg")])
    );

    // A function without a binding runs its stub body, which raises `undef`.
    let stub = program
        .execute(
            "extension_probe",
            "unbound",
            &[program.host_term(Term::Integer(1))],
        )
        .unwrap_err();
    assert!(stub.to_string().contains("undef"), "{stub}");
    // A library whose functions without a binding fail gives the runtime
    // error of the function.
    let strict = program
        .execute("extension_strict", "unbound", &[])
        .unwrap_err();
    assert_eq!(
        strict.to_string(),
        "extension_strict:unbound/0 NIF is not supported"
    );
}

#[test]
fn gives_the_atoms_of_the_run_on_the_scheduler_and_static_atoms_on_a_dirty_thread() {
    let program = program();
    let name = |text: &str| vec![Term::binary(text.as_bytes().to_vec())];
    assert_eq!(
        run(
            &program,
            "extension_probe",
            "atom_name",
            name("extension_fresh")
        ),
        Term::binary(b"extension_fresh".to_vec())
    );
    assert_eq!(
        run(&program, "extension_probe", "dirty_atom_name", name("ok")),
        Term::binary(b"ok".to_vec())
    );
    let fresh = Term::binary(b"extension_fresh".to_vec());
    let dirty = program
        .execute(
            "extension_probe",
            "dirty_atom_name",
            &[program.host_term(fresh)],
        )
        .unwrap_err();
    assert_eq!(
        dirty.to_string(),
        "unsupported: a dirty NIF cannot make the atom extension_fresh, which is not a static atom"
    );
}

#[test]
fn rejects_a_duplicate_library_and_a_binding_outside_the_functions() {
    program();
    assert_eq!(
        register_library(&PROBE_LIBRARY),
        Err(RegistrationError::Duplicate)
    );
    static PRIM_TTY: Library = Library {
        module: "prim_tty",
        functions: &[],
        bindings: &[],
        load: None,
        unbound_fails: false,
    };
    assert_eq!(
        register_library(&PRIM_TTY),
        Err(RegistrationError::Duplicate)
    );
    static UNLISTED: Library = Library {
        module: "extension_unlisted",
        functions: &[("add", 2)],
        bindings: &[Binding {
            name: "add",
            arity: 1,
            job: Job::Normal,
            run: add,
        }],
        load: None,
        unbound_fails: false,
    };
    assert_eq!(
        register_library(&UNLISTED),
        Err(RegistrationError::UnlistedBinding {
            name: "add",
            arity: 1
        })
    );
    static TWICE: Library = Library {
        module: "extension_twice",
        functions: &[("add", 2)],
        bindings: &[
            Binding {
                name: "add",
                arity: 2,
                job: Job::Normal,
                run: add,
            },
            Binding {
                name: "add",
                arity: 2,
                job: Job::CpuBound,
                run: add,
            },
        ],
        load: None,
        unbound_fails: false,
    };
    assert_eq!(
        register_library(&TWICE),
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
        nif_manifest_gaps(&probe),
        ["extension_probe:unbound/1: NIF function has no PBeam binding"]
    );
}

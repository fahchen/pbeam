//! Reads the persisted native declarations of a compiled module and checks that each call path
//! reaches the registered native.

use pbeam_runtime::{
    BeamModule, BeamProgram, Binding, Job, Library, NativeResult, NifContext, Term,
    register_library,
};

const NATIVE_DECL_PROBE: &[u8] =
    include_bytes!("../../fixtures/native_decl/Elixir.NativeDeclProbe.beam");
const NATIVE_DECL_CALLER: &[u8] =
    include_bytes!("../../fixtures/native_decl/Elixir.NativeDeclProbeCaller.beam");

/// The value that the registered function returns. It differs from the
/// `:pbeam_native_only` error of the stub, so the test sees which one ran.
const MARKER: i64 = 90210;

/// Return the marker for a binary argument, and `badarg` for any other
/// argument, as a NIF of the fixture would.
fn sha256(
    context: &mut NifContext<'_>,
    arguments: &[Term],
) -> Result<NativeResult, pbeam_runtime::BeamError> {
    match arguments.first() {
        Some(Term::Binary(_)) => Ok(NativeResult::Value(Term::Integer(MARKER))),
        _ => Ok(context.badarg()),
    }
}

/// The extension library of the fixture: the declaration `sha256/1` and its
/// binding.
static NATIVE_DECL_LIBRARY: Library = Library {
    module: "Elixir.NativeDeclProbe",
    functions: &[("sha256", 1)],
    bindings: &[Binding {
        name: "sha256",
        arity: 1,
        job: Job::Normal,
        run: sha256,
    }],
    load: None,
    unbound_fails: false,
};

/// Run an exported function of `program` with `arguments`.
fn run(program: &BeamProgram, module: &str, function: &str, arguments: Vec<Term>) -> Term {
    let arguments = arguments
        .into_iter()
        .map(|term| program.host_term(term))
        .collect::<Vec<_>>();
    program.execute(module, function, &arguments).unwrap().term
}

/// The `Attr` chunk of a compiled module gives the declarations of the
/// `PBeam.Native` declaration, and a module that declares nothing gives an
/// empty list.
#[test]
fn reads_the_persisted_declarations_of_a_compiled_module() {
    let module = BeamModule::load(NATIVE_DECL_PROBE).unwrap();
    let declarations = module.native_declarations();
    assert_eq!(declarations.len(), 1);
    assert_eq!(declarations[0], ("sha256".to_owned(), 1));

    let caller = BeamModule::load(NATIVE_DECL_CALLER).unwrap();
    assert_eq!(caller.native_declarations(), []);
}

/// A registered native runs at the `nif_start` entry of the declared stub
/// through each call path that the declaration claims.
#[test]
fn reaches_the_registered_native_through_each_call_path() {
    let program = BeamProgram::load(&[NATIVE_DECL_PROBE, NATIVE_DECL_CALLER]).unwrap();
    let data = || Term::binary(vec![0x61, 0x62, 0x63]);

    // Without a registration, the stub body runs and raises
    // `:pbeam_native_only`. It does not give a value. The registry keeps a
    // library for the life of the process, so this check comes first.
    let error = program
        .execute(
            "Elixir.NativeDeclProbe",
            "local",
            &[program.host_term(data())],
        )
        .unwrap_err();
    assert!(
        error.to_string().contains("pbeam_native_only"),
        "expected the stub error, got {error}"
    );

    register_library(&NATIVE_DECL_LIBRARY).unwrap();

    // A local call enters the frame of the stub.
    assert_eq!(
        run(&program, "Elixir.NativeDeclProbe", "local", vec![data()]),
        Term::Integer(MARKER)
    );
    // An external call from another module uses `call_ext`.
    assert_eq!(
        run(
            &program,
            "Elixir.NativeDeclProbeCaller",
            "external",
            vec![data()]
        ),
        Term::Integer(MARKER)
    );
    // `erlang:apply/3` resolves the export of the declaration.
    assert_eq!(
        run(
            &program,
            "Elixir.NativeDeclProbe",
            "applied",
            vec![
                Term::Atom(program.atom("Elixir.NativeDeclProbe").unwrap()),
                Term::Atom(program.atom("sha256").unwrap()),
                Term::list_from([data()])
            ]
        ),
        Term::Integer(MARKER)
    );
    // A `fun M:F/A` reference calls the export through `call_fun`.
    assert_eq!(
        run(
            &program,
            "Elixir.NativeDeclProbeCaller",
            "referenced",
            vec![data()]
        ),
        Term::Integer(MARKER)
    );
}

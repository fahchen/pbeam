use pbeam_runtime::{
    Atom, BeamProgram, Binding, Job, Library, NativeResult, NifContext, Term, register_library,
};

const NATIVE_PROBE: &[u8] = include_bytes!("../../fixtures/native/native_probe.beam");

fn add(
    context: &mut NifContext<'_>,
    arguments: &[Term],
) -> Result<NativeResult, pbeam_runtime::BeamError> {
    match (arguments.first(), arguments.get(1)) {
        (Some(Term::Integer(left)), Some(Term::Integer(right))) => {
            Ok(NativeResult::Value(Term::Integer(left + right)))
        }
        _ => Ok(context.badarg()),
    }
}

fn hidden(
    context: &mut NifContext<'_>,
    arguments: &[Term],
) -> Result<NativeResult, pbeam_runtime::BeamError> {
    match arguments.first() {
        Some(Term::Integer(value)) => Ok(NativeResult::Value(Term::Integer(value + 100))),
        _ => Ok(context.badarg()),
    }
}

fn atom(name: &str) -> Term {
    Term::Atom(Atom::from_static(name).unwrap())
}

/// The library of the fixture: both functions of its `-nifs` list have a
/// binding.
static NATIVE_PROBE_LIBRARY: Library = Library {
    module: "native_probe",
    functions: &[("add", 2), ("hidden", 1)],
    bindings: &[
        Binding {
            name: "add",
            arity: 2,
            job: Job::Normal,
            run: add,
        },
        Binding {
            name: "hidden",
            arity: 1,
            job: Job::Normal,
            run: hidden,
        },
    ],
    load: None,
    unbound_fails: false,
};

#[test]
fn registered_mfa_reaches_rust_and_unregistered_resolves_to_beam() {
    register_library(&NATIVE_PROBE_LIBRARY).unwrap();

    let program = BeamProgram::load(&[NATIVE_PROBE]).unwrap();

    // An external call reaches the registered native, and an unregistered
    // ordinary function resolves to its BEAM body.
    let result = program.execute("native_probe", "run", &[]).unwrap();
    let elements = result.term.tuple_elements().expect("result is a tuple");
    assert_eq!(elements.len(), 3);
    let Term::Atom(tag) = &elements[0] else {
        panic!("first element is not an atom: {:?}", elements[0]);
    };
    assert_eq!(program.atoms().name_of(tag), "native_add");
    assert_eq!(elements[1], Term::Integer(3));
    assert_eq!(elements[2], Term::Integer(6));

    // A local call enters the frame, so it reaches the `nif_start`
    // instruction for the exported stub and for the private stub.
    let result = program.execute("native_probe", "local", &[]).unwrap();
    let elements = result.term.tuple_elements().expect("result is a tuple");
    assert_eq!(elements.len(), 2);
    assert_eq!(elements[0], Term::Integer(3));
    assert_eq!(elements[1], Term::Integer(107));

    // Invalid arguments raise a catchable `error:badarg`.
    let result = program.execute("native_probe", "invalid", &[]).unwrap();
    let elements = result.term.tuple_elements().expect("result is a tuple");
    assert_eq!(elements.len(), 2);
    assert_eq!(elements[0], atom("error"));
    assert_eq!(elements[1], atom("badarg"));
}

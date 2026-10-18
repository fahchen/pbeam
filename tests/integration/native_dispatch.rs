use pbeam_runtime::{Atom, BeamProgram, NativeResult, Term, register_native, unregister_native};

const NATIVE_PROBE: &[u8] = include_bytes!("../../fixtures/native/native_probe.beam");

fn add(arguments: &[Term]) -> Result<NativeResult, pbeam_runtime::BeamError> {
    match (arguments.first(), arguments.get(1)) {
        (Some(Term::Integer(left)), Some(Term::Integer(right))) => {
            Ok(NativeResult::Value(Term::Integer(left + right)))
        }
        _ => Ok(NativeResult::badarg()),
    }
}

fn hidden(arguments: &[Term]) -> Result<NativeResult, pbeam_runtime::BeamError> {
    match arguments.first() {
        Some(Term::Integer(value)) => Ok(NativeResult::Value(Term::Integer(value + 100))),
        _ => Ok(NativeResult::badarg()),
    }
}

fn atom(name: &str) -> Term {
    Term::Atom(Atom::from_static(name).unwrap())
}

#[test]
fn registered_mfa_reaches_rust_and_unregistered_resolves_to_beam() {
    let _ = unregister_native("native_probe", "add", 2);
    let _ = unregister_native("native_probe", "hidden", 1);
    register_native("native_probe", "add", 2, add).unwrap();
    register_native("native_probe", "hidden", 1, hidden).unwrap();

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

    let _ = unregister_native("native_probe", "add", 2);
    let _ = unregister_native("native_probe", "hidden", 1);
}

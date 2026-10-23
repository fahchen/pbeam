use std::process::Command;

use pbeam_runtime::{BeamProgram, Term};

const SIZE_PROBE: &[u8] = include_bytes!("../../fixtures/size/size_probe.beam");

#[test]
fn gives_the_size_of_a_tuple_or_bitstring_like_erts() {
    // `size_1` (`erl_bif_guard.c:315-335`) gives the arity of a tuple and the
    // whole bytes of a bitstring, rounded down, and raises `badarg` for each
    // other term. The probe runs the `gc_bif` of a body and of a guard, a
    // fun, `apply/3`, and a tail `apply/3`.
    let program = BeamProgram::load(&[SIZE_PROBE]).unwrap();
    let atom = |name: &str| Term::Atom(program.atom(name).unwrap());
    let int = Term::Integer;
    let badarg = Term::tuple(vec![atom("error"), atom("badarg")]);
    let mut expected = [0, 3, 0, 3, 0, 2, 1, 2, 2].map(int).to_vec();
    expected.extend(["big", "small", "other", "other"].map(atom));
    expected.extend(std::iter::repeat_n(badarg, 6));
    assert_eq!(
        program.execute("size_probe", "main", &[]).unwrap().term,
        Term::list_from(expected)
    );

    let reference = Command::new("erl")
        .args([
            "-noshell",
            "-pa",
            "fixtures/size",
            "-eval",
            "io:format(\"~w~n\", [size_probe:main()]), halt().",
        ])
        .output()
        .unwrap();
    assert!(reference.status.success(), "{reference:?}");
    assert_eq!(
        String::from_utf8(reference.stdout).unwrap(),
        "[0,3,0,3,0,2,1,2,2,big,small,other,other,{error,badarg},{error,badarg},\
         {error,badarg},{error,badarg},{error,badarg},{error,badarg}]\n"
    );
}

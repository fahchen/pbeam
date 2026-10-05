use std::process::Command;

use pbeam_runtime::{BeamProgram, Term};

const FUN_PROBE: &[u8] = include_bytes!("../fixtures/funs/fun_probe.beam");

/// Run `fun_probe:Function()` on the pinned ERTS and return its `~w` output.
fn reference(function: &str) -> Vec<u8> {
    let reference = Command::new("erl")
        .args([
            "-noshell",
            "-pa",
            "fixtures/funs",
            "-eval",
            &format!("io:format(\"~w~n\", [fun_probe:{function}()]), halt()."),
        ])
        .output()
        .unwrap();
    assert!(reference.status.success());
    assert!(reference.stderr.is_empty());
    reference.stdout
}

#[test]
fn applies_module_functions_as_pinned_erts() {
    // `fun_probe:mfas/0` calls `erlang:apply/3` as `call_ext`,
    // `call_ext_only`, and the `apply` instruction, as
    // `proc_lib:init_p_do_apply/3` (`proc_lib.erl:331-333`) does. An apply
    // of `erlang:apply/3` runs its inner call. A module or function that is
    // not an atom, and an argument term that is not a list, raise
    // `error:badarg`. A list of 1024 arguments raises `error:system_limit`,
    // as ERTS `apply` (`beam_common.c:1434-1526`) does.
    let program = BeamProgram::load(&[FUN_PROBE]).unwrap();
    let atom = |name| Term::Atom(program.atom(name).unwrap());
    let mut expected: Vec<Term> = ["v", "w", "z", "y", "s"].map(atom).into();
    expected.extend(std::iter::repeat_n(atom("badarg"), 8));
    expected.push(atom("system_limit"));
    assert_eq!(
        program.execute("fun_probe", "mfas", &[]).unwrap(),
        Term::List(expected)
    );
    assert_eq!(
        reference("mfas"),
        b"[v,w,z,y,s,badarg,badarg,badarg,badarg,badarg,badarg,badarg,badarg,system_limit]\n"
    );
}

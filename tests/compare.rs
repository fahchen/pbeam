use std::process::Command;

use pbeam_runtime::{BeamProgram, Term};

const COMPARE_PROBE: &[u8] = include_bytes!("../fixtures/compare/compare_probe.beam");

/// Run `compare_probe:Function()` on the pinned ERTS and return its `~w` output.
fn reference(function: &str) -> Vec<u8> {
    let reference = Command::new("erl")
        .args([
            "-noshell",
            "-pa",
            "fixtures/compare",
            "-eval",
            &format!("io:format(\"~w~n\", [compare_probe:{function}()]), halt()."),
        ])
        .output()
        .unwrap();
    assert!(reference.status.success());
    assert!(reference.stderr.is_empty());
    reference.stdout
}

#[test]
fn compares_terms_exactly_as_pinned_erts() {
    // `compare_probe:exact/0` runs the `bif` forms of `erlang:'=:='/2` and
    // `erlang:'=/='/2` with fail label 0, as `user_drv:init/1` does. Both
    // BIFs use `eq()` (`erl_bif_op.c:113-126`).
    let program = BeamProgram::load(&[COMPARE_PROBE]).unwrap();
    let exact = [
        true, false, false, true, false, true, false, true, false, true, false, true, false, true,
        true, false, true, false,
    ];
    let atoms = |negate: bool| {
        Term::List(
            exact
                .iter()
                .map(|&equal| {
                    Term::Atom(
                        program
                            .atom(if equal != negate { "true" } else { "false" })
                            .unwrap(),
                    )
                })
                .collect(),
        )
    };
    assert_eq!(
        program.execute("compare_probe", "exact", &[]).unwrap(),
        Term::List(vec![atoms(false), atoms(true)])
    );
    assert_eq!(
        reference("exact"),
        b"[[true,false,false,true,false,true,false,true,false,true,false,true,false,true,\
          true,false,true,false],[false,true,true,false,true,false,true,false,true,false,\
          true,false,true,false,false,true,false,true]]\n"
    );
}

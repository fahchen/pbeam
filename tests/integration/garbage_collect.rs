use pbeam_runtime::{BeamProgram, Term};

#[path = "../common/mod.rs"]
mod common;

use common::pinned_beam;

const GARBAGE_COLLECT_PROBE: &[u8] =
    include_bytes!("../../fixtures/garbage_collect/garbage_collect_probe.beam");

#[test]
fn runs_garbage_collect_as_pinned_erts() {
    // `erlang:garbage_collect/0` of the pinned `erlang.beam` calls
    // `erts_internal:garbage_collect(major)` (erlang.erl:3052-3053), which
    // `erl_prim_loader` calls after each reply (erl_prim_loader.erl:490).
    // `erts_internal_garbage_collect_1` gives `true` for `minor` and
    // `major`, and raises `badarg` for each other term (bif.c:4076-4090).
    // Measured on the pinned ERTS: `mise run erts-eval --pa
    // fixtures/garbage_collect 'garbage_collect_probe:main()'` gives
    // `[true,true,true,{error,badarg},{error,badarg}]`.
    let program = BeamProgram::load(&[&pinned_beam("erlang"), GARBAGE_COLLECT_PROBE]).unwrap();
    let atom = |name: &str| Term::Atom(program.atom(name).unwrap());
    let badarg = Term::tuple(vec![atom("error"), atom("badarg")]);
    assert_eq!(
        program
            .execute("garbage_collect_probe", "main", &[])
            .unwrap()
            .term,
        Term::list_from(vec![
            atom("true"),
            atom("true"),
            atom("true"),
            badarg.clone(),
            badarg,
        ])
    );
}

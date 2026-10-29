//! Compare `erts_internal:cmp_term/2` of the pinned ERTS with the same call on
//! PBeam. `CMP_TERM` of the BIF table calls `erts_cmp` with `exact = 1`
//! (`bif.c:5417-5427`, `erl_utils.h:122`), and `maps` reads the BIF to order
//! two keys.
#![expect(
    clippy::unwrap_used,
    reason = "test helpers report a failure by a panic"
)]

use std::process::Command;

use pbeam_runtime::{BeamProgram, Term};

const CMP_TERM_PROBE: &[u8] = include_bytes!("../../fixtures/cmp_term/cmp_term_probe.beam");

/// Run `cmp_term_probe:pairs/0` on the pinned ERTS and return its `~w` output.
fn reference() -> Vec<u8> {
    let reference = Command::new("erl")
        .args([
            "-noshell",
            "-pa",
            "fixtures/cmp_term",
            "-eval",
            "io:format(\"~w~n\", [cmp_term_probe:pairs()]), halt().",
        ])
        .output()
        .unwrap();
    assert!(reference.status.success());
    assert_eq!(reference.stderr, <[u8; 0]>::default());
    reference.stdout
}

#[test]
fn orders_terms_exactly_as_pinned_erts() {
    // Each integer is less than each float, `-0.0` is less than `0.0`, and a
    // term compares with itself as equal: exact order, not arithmetic order.
    let program = BeamProgram::load(&[CMP_TERM_PROBE]).unwrap();
    let expected = [
        0, -1, 1, -1, 1, -1, 1, 0, -1, -1, 1, -1, 1, -1, 0, 0, -1, 0, 1,
    ];
    assert_eq!(
        program
            .execute("cmp_term_probe", "pairs", &[])
            .unwrap()
            .term,
        Term::list_from(expected.map(Term::Integer))
    );
    assert_eq!(
        reference(),
        b"[0,-1,1,-1,1,-1,1,0,-1,-1,1,-1,1,-1,0,0,-1,0,1]\n"
    );
}

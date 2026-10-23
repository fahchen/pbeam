use std::process::Command;

use pbeam_runtime::BeamProgram;

#[path = "../common/mod.rs"]
mod common;

use common::pinned_beam;

const PROBE: &[u8] = include_bytes!("../../fixtures/flat_size/flat_size_probe.beam");

/// Evaluate `flat_size_probe:sizes()` on the pinned ERTS and return its
/// `~w` output.
fn reference() -> String {
    let reference = Command::new("erl")
        .args([
            "-noshell",
            "-pa",
            "fixtures/flat_size",
            "-eval",
            "io:format(\"~w~n\", [flat_size_probe:sizes()]), halt().",
        ])
        .output()
        .unwrap();
    assert!(
        reference.status.success() && reference.stderr.is_empty(),
        "{reference:?}"
    );
    String::from_utf8(reference.stdout).unwrap()
}

/// Run `flat_size_probe:show()` on PBeam and return the print of the exit
/// reason, which is the print of the result of `sizes/0`.
fn shown() -> String {
    let program = BeamProgram::load(&[
        PROBE,
        &pinned_beam("erlang"),
        &pinned_beam("lists"),
        &pinned_beam("maps"),
        &pinned_beam("binary"),
        &pinned_beam("ets"),
    ])
    .unwrap();
    let error = program
        .execute("flat_size_probe", "show", &[])
        .unwrap_err()
        .to_string();
    format!(
        "{}\n",
        error.strip_prefix("root process exited with ").unwrap()
    )
}

#[test]
fn gives_the_size_object_words_of_each_term_kind_as_pinned_erts() {
    // `erts_debug:flat_size/1` returns `size_object` of the term
    // (`beam_debug.c:70-76`, `copy.c:88-260`): integers, bignums, floats,
    // atoms, pids, references, tuples, lists, flatmaps, hashmaps, heap and
    // reference-counted binaries, sub-binaries, bitstrings, funs, and a hashmap with a collision node.
    let expected = reference();
    assert_eq!(
        expected,
        "[0,0,2,0,2,2,3,3,5,2,0,0,3,4,3,0,3,9,2,2,6,10,3,6,13,123,1134,191,2,3,3,3,10,8,8,8,4,3,2,3,7,2,128]\n"
    );
    assert_eq!(shown(), expected);
}

use std::process::Command;

use pbeam_runtime::BeamProgram;

#[path = "../common/mod.rs"]
mod common;

use common::pinned_beam;

const PROBE: &[u8] = include_bytes!("../../fixtures/erts_debug_size/size_probe.beam");

/// Evaluate `size_probe:sizes()` on the pinned ERTS and return its `~w`
/// output.
fn reference() -> String {
    let reference = Command::new("erl")
        .args([
            "-noshell",
            "-pa",
            "fixtures/erts_debug_size",
            "-eval",
            "io:format(\"~w~n\", [size_probe:sizes()]), halt().",
        ])
        .output()
        .unwrap();
    assert!(
        reference.status.success() && reference.stderr.is_empty(),
        "{reference:?}"
    );
    String::from_utf8(reference.stdout).unwrap()
}

/// Load the probe with the pinned `erts_debug` module and its callees.
fn program() -> BeamProgram {
    BeamProgram::load(&[
        PROBE,
        &pinned_beam("erlang"),
        &pinned_beam("erts_debug"),
        &pinned_beam("gb_trees"),
        &pinned_beam("lists"),
        &pinned_beam("maps"),
        &pinned_beam("binary"),
    ])
    .unwrap()
}

/// Run `size_probe:show()` on PBeam and return the print of the exit
/// reason, which is the print of the result of `sizes/0`.
fn shown() -> String {
    let error = program()
        .execute("size_probe", "show", &[])
        .unwrap_err()
        .to_string();
    format!(
        "{}\n",
        error.strip_prefix("root process exited with ").unwrap()
    )
}

#[test]
fn gives_the_size_of_shared_and_unshared_terms_as_pinned_erts() {
    // `erts_debug:size/1` runs the pinned Erlang code
    // (`lib/kernel/src/erts_debug.erl:257-362`) with `erts_debug:same/2`
    // (`beam_debug.c:65-68`), `erts_internal:term_type/1`
    // (`erl_map.c:3336-3417`), and `erts_internal:map_to_tuple_keys/1`
    // (`erl_map.c:3318-3328`).
    let expected = reference();
    assert_eq!(
        expected,
        "{[0,0,0,0,2,3,6,3,3,8,0,3,6,4,8,2,3,8,11,2,10],[true,true,true,false,true,true,true,true,true,true],[fixnum,bignum,bignum,hfloat,atom,nil,list,tuple,flatmap,hashmap,'fun',export,pid,reference],[{}],[{error,{badmap,a}},{error,badarg}]}\n"
    );
    // The host print of an exit reason does not quote the atom `fun`.
    assert_eq!(shown(), expected.replace("'fun'", "fun"));
}

#[test]
fn fails_on_a_hashmap_because_pbeam_has_no_hamt_node_term() {
    // ERTS walks the HAMT nodes of a map of more than 32 keys with
    // `erts_internal:map_hashmap_children/1` (`erts_debug.erl:317-322`).
    // PBeam has no term for a HAMT node, so the BIF is unsupported.
    let error = program()
        .execute("size_probe", "hashmap", &[])
        .unwrap_err()
        .to_string();
    assert!(
        error.contains("erts_internal:map_hashmap_children/1"),
        "{error}"
    );
}

#[test]
fn fails_on_a_flatmap_with_keys_because_pbeam_keeps_no_key_tuple() {
    // ERTS gives the key tuple that the flatmap holds (`erl_map.c:3318-3321`),
    // and flatmaps can share it, so `size/1` counts it once. PBeam keeps no
    // key tuple. On the pinned ERTS, `size_probe:flatmap()` gives 6.
    let error = program()
        .execute("size_probe", "flatmap", &[])
        .unwrap_err()
        .to_string();
    assert!(
        error.contains("erts_internal:map_to_tuple_keys/1 cannot give"),
        "{error}"
    );
}

#[test]
fn fails_on_two_equal_floats_because_pbeam_does_not_box_a_float() {
    // ERTS compares the term words (`beam_debug.c:65-68`), and a float is
    // boxed. On the pinned ERTS, `size_probe:float_pair()` gives 5.
    let error = program()
        .execute("size_probe", "float_pair", &[])
        .unwrap_err()
        .to_string();
    assert!(error.contains("erts_debug:same/2 cannot tell"), "{error}");
}

#[test]
fn fails_on_the_term_type_of_a_binary() {
    // PBeam does not keep the ERTS form of a binary, so it cannot name it
    // as `heap_binary`, `refc_binary`, or `sub_binary`.
    let error = program()
        .execute("size_probe", "binary_type", &[])
        .unwrap_err()
        .to_string();
    assert!(
        error.contains("erts_internal:term_type/1 cannot name"),
        "{error}"
    );
}

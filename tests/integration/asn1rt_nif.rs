//! Runs the `asn1rt_nif` functions on PBeam and compares the results with the pinned ERTS.

use std::process::Command;

use pbeam_runtime::{BeamProgram, Term};

#[path = "../common/mod.rs"]
mod common;

const PROBE: &[u8] = include_bytes!("../../fixtures/asn1_codec/asn1_probe.beam");
const CODEC: &[u8] = include_bytes!("../../fixtures/asn1_codec/AsnProbe.beam");
const STAND_IN: &[u8] = include_bytes!("../../fixtures/asn1_codec/asn1rt_nif.beam");

/// Load `asn1_probe`, the generated BER codec `AsnProbe`, and the stand-in
/// `asn1rt_nif`, which binds the static NIF library in `load/1`.
fn program() -> BeamProgram {
    let modules: Vec<Vec<u8>> = ["lists", "binary"]
        .into_iter()
        .map(common::pinned_beam)
        .collect();
    let mut beams: Vec<&[u8]> = modules.iter().map(Vec::as_slice).collect();
    beams.extend([STAND_IN, CODEC, PROBE]);
    BeamProgram::load(&beams).unwrap()
}

/// Run `asn1_probe:Function(Library, Arguments...)` on PBeam and return
/// the text of the binary that it gives.
fn run(program: &BeamProgram, function: &str, arguments: &[Term]) -> String {
    let library = Term::binary(b"asn1rt_nif".to_vec());
    let arguments: Vec<_> = std::iter::once(library)
        .chain(arguments.iter().cloned())
        .map(|term| program.host_term(term))
        .collect();
    let term = program
        .execute("asn1_probe", function, &arguments)
        .unwrap()
        .term;
    String::from_utf8(term.binary_bytes().unwrap().to_vec()).unwrap()
}

#[test]
fn gives_the_results_of_the_pinned_erts_for_each_asn1rt_nif_function() {
    // The expected results of `asn1_probe:check/0` come from the pinned
    // ERTS; the same check on ERTS keeps them current.
    let output = Command::new("erl")
        .args([
            "-noshell",
            "-pa",
            "fixtures/asn1_codec",
            "-eval",
            "io:format(\"~s~n\", [asn1_probe:check_text(filename:join([code:priv_dir(asn1), \"lib\", \"asn1rt_nif\"]))]), halt().",
        ])
        .output()
        .unwrap();
    assert_eq!(String::from_utf8(output.stdout).unwrap(), "ok\n");
    assert_eq!(run(&program(), "check_text", &[]), "ok");
}

#[test]
fn stops_the_run_for_a_ber_nesting_deeper_than_each_pinned_erts_build_decodes() {
    // ERTS stops `ber_decode_value` at a stack distance of 8192 pointer
    // words (asn1_erl_nif.c:954-958), so the depth of its `unknown` error
    // depends on the build. Each pinned ERTS host decodes 453 nested
    // values, and PBeam stops the run for a deeper input.
    let output = Command::new("erl")
        .args([
            "-noshell",
            "-pa",
            "fixtures/asn1_codec",
            "-eval",
            "L = filename:join([code:priv_dir(asn1), \"lib\", \"asn1rt_nif\"]), \
             io:format(\"~s~n\", [asn1_probe:nested_text(L, 453)]), halt().",
        ])
        .output()
        .unwrap();
    assert_eq!(String::from_utf8(output.stdout).unwrap(), "decoded\n");
    let program = program();
    assert_eq!(
        run(&program, "nested_text", &[Term::Integer(453)]),
        "decoded"
    );
    let arguments = [
        program.host_term(Term::binary(b"asn1rt_nif".to_vec())),
        program.host_term(Term::Integer(454)),
    ];
    let error = program
        .execute("asn1_probe", "nested_text", &arguments)
        .unwrap_err()
        .to_string();
    assert_eq!(
        error,
        "asn1rt_nif:decode_ber_tlv_raw/1 nests more than 453 values, where the ERTS result depends on the C stack frame size of the build, which PBeam does not support"
    );
}

#[test]
fn stops_the_run_where_the_per_complete_of_the_pinned_erts_reads_after_its_input() {
    // The opcode 10 reads two bytes before a check of the counter
    // (asn1_erl_nif.c:186-199), so ERTS reads memory outside the term.
    let program = program();
    for data in [vec![10], vec![1, 10, 3]] {
        let arguments = [
            program.host_term(Term::binary(b"asn1rt_nif".to_vec())),
            program.host_term(Term::binary(data)),
        ];
        let error = program
            .execute("asn1_probe", "per_complete", &arguments)
            .unwrap_err()
            .to_string();
        assert_eq!(
            error,
            "asn1rt_nif:encode_per_complete/1 reads after the end of its input at the opcode 10, which PBeam does not support"
        );
    }
}

#[test]
fn stops_get_module_info_nifs_for_the_linked_static_library() {
    // ERTS gives each function of the NIF library (erl_bif_info.c:
    // 5769-5805). PBeam links only some functions of a static library, so
    // the key stops the run (row "Module information" of
    // docs/erts-differences.md).
    let program = program();
    let library = program.host_term(Term::binary(b"asn1rt_nif".to_vec()));
    let failure = program
        .execute("asn1rt_nif", "nifs", &[library])
        .unwrap_err()
        .to_string();
    assert!(
        failure.contains(
            "erlang:get_module_info/2 key nifs is not implemented for the linked NIF library of asn1rt_nif"
        ),
        "{failure}"
    );
}

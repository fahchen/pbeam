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
fn stops_the_ber_decode_at_the_nesting_depth_of_the_pinned_erts() {
    // ERTS stops `ber_decode_value` at a stack distance of 8192 pointer
    // words (asn1_erl_nif.c:954-958): 453 nested values decode, and 454
    // give `unknown` at the index of the innermost length.
    let program = program();
    assert_eq!(
        run(&program, "nested_text", &[Term::Integer(453)]),
        "decoded"
    );
    assert_eq!(
        run(&program, "nested_text", &[Term::Integer(454)]),
        "unknown 909"
    );
}

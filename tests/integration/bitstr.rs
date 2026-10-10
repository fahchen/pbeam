use std::process::Command;

use pbeam_runtime::{BeamProgram, Term};

#[path = "../common/mod.rs"]
mod common;

use common::pinned_beam;

const BITSTR_PROBE: &[u8] = include_bytes!("../../fixtures/bitstr/bitstr_probe.beam");

/// Evaluate an expression on the pinned ERTS with `bitstr_probe` in the
/// code path and return its `~w` output.
fn reference(expression: &str) -> String {
    let reference = Command::new("erl")
        .args([
            "-noshell",
            "-pa",
            "fixtures/bitstr",
            "-eval",
            &format!("io:format(\"~w~n\", [{expression}]), halt()."),
        ])
        .output()
        .unwrap();
    assert!(
        reference.status.success() && reference.stderr.is_empty(),
        "{reference:?}"
    );
    String::from_utf8(reference.stdout).unwrap()
}

/// Load the probe with the pinned `erlang` module, whose `open_port/2` and
/// `port_close/1` call the `erts_internal` port BIFs.
fn load() -> BeamProgram {
    let erlang = pinned_beam("erlang");
    BeamProgram::load(&[BITSTR_PROBE, &erlang]).unwrap()
}

#[test]
fn runs_the_is_bitstr_test_as_pinned_erts() {
    // `bitstr_probe:kinds/0` runs `is_bitstr` on a literal binary, the
    // empty binary, a binary that `bs_create_bin` makes, and one term of
    // each other kind that PBeam has, also a port. ERTS loads `is_bitstr` as
    // `is_bitstring` (emu `ops.tab:796`; JIT `jit/arm/ops.tab:594`), which
    // passes for a boxed term with a bitstring header and jumps to the fail
    // label for each other term (emu `instrs.tab:945-949`; JIT
    // `emit_is_bitstring` in `jit/arm/instr_common.cpp:1158-1184`). 2^60
    // is an ERTS bignum, so the JIT compares the header of a boxed term
    // that is not a bitstring.
    let program = load();
    let other = Term::Atom(program.atom("other").unwrap());
    let binaries = [&b"b"[..], b"", b"ab"].map(|bytes| Term::Binary(bytes.to_vec()));
    let expected = Term::List(
        binaries
            .into_iter()
            .chain(std::iter::repeat_n(other, 12))
            .collect(),
    );
    assert_eq!(
        program.execute("bitstr_probe", "kinds", &[]).unwrap(),
        expected
    );
    assert_eq!(
        reference("bitstr_probe:kinds()"),
        "[<<98>>,<<>>,<<97,98>>,other,other,other,other,other,other,other,other,other,other,other,other]\n"
    );
}

#[test]
fn stops_at_a_bitstring_literal_that_is_not_a_binary() {
    // `bitstr_probe:bits/0` moves the literal `<<1:3>>` to `x0`. PBeam has
    // no bitstring whose size is not a multiple of 8, so the `move` stops
    // the run before `is_bitstr` runs. ERTS passes the test and returns the
    // bitstring.
    let program = load();
    assert_eq!(
        program
            .execute("bitstr_probe", "bits", &[])
            .unwrap_err()
            .to_string(),
        "unsupported BEAM literal index 8"
    );
    assert_eq!(reference("bitstr_probe:bits()"), "<<1:3>>\n");
}

//! Runs bitstring literals, the `is_bitstr` test and the order of bitstrings on PBeam and compares
//! them with the pinned ERTS.

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
    let binaries = [&b"b"[..], b"", b"ab"].map(|bytes| Term::binary(bytes.to_vec()));
    let expected = Term::list_from(binaries.into_iter().chain(std::iter::repeat_n(other, 12)));
    assert_eq!(
        program.execute("bitstr_probe", "kinds", &[]).unwrap().term,
        expected
    );
    assert_eq!(
        reference("bitstr_probe:kinds()"),
        "[<<98>>,<<>>,<<97,98>>,other,other,other,other,other,other,other,other,other,other,other,other]\n"
    );
}

/// Run `bitstr_probe:show(Name)` on PBeam and return the print of the exit
/// reason, which is the print of the result of `Name/0`.
fn shown(program: &BeamProgram, name: &str) -> String {
    let name = Term::Atom(program.atom(name).unwrap());
    let error = program
        .execute("bitstr_probe", "show", &[program.host_term(name)])
        .unwrap_err()
        .to_string();
    format!(
        "{}\n",
        error.strip_prefix("root process exited with ").unwrap()
    )
}

#[test]
fn loads_a_bitstring_literal_as_pinned_erts() {
    // The loader decodes `BIT_BINARY_EXT` (tag 77): a byte count, the
    // number of bits of the last byte, and the bytes
    // (`external.c:4895-4923`). `is_bitstr` passes for the bitstring
    // (emu `instrs.tab:945-949`).
    let program = load();
    assert_eq!(shown(&program, "bits"), reference("bitstr_probe:bits()"));
    assert_eq!(shown(&program, "bits"), "<<1:3>>\n");
    // A bitstring prints its whole bytes, then its tail bits as
    // `Value:Size` (`erl_printf_term.c:615-653`).
    let printed = shown(&program, "print");
    assert_eq!(printed, "{<<1:3>>,<<1,2:4>>,<<97,98,1:1>>}\n");
    assert_eq!(
        reference("catch bitstr_probe:print()"),
        format!("{{'EXIT',{}}}\n", printed.trim_end())
    );
}

#[test]
fn reads_the_sizes_of_a_bitstring_literal_as_pinned_erts() {
    // `bit_size/1` gives the bits, `byte_size/1` rounds up to the next
    // byte (`erl_bif_guard.c:376-393`), and `is_binary/1` is false for a
    // size that is not a multiple of 8 (`erl_term.h:384-391`).
    let program = load();
    assert_eq!(shown(&program, "sizes"), reference("bitstr_probe:sizes()"));
}

#[test]
fn orders_bitstrings_as_pinned_erts() {
    // ERTS compares two bitstrings by their bits, and a shorter prefix is
    // less (`erts_cmp_bits` in `utils.c:2320-2335`).
    let program =
        BeamProgram::load(&[BITSTR_PROBE, &pinned_beam("erlang"), &pinned_beam("lists")]).unwrap();
    assert_eq!(shown(&program, "order"), reference("bitstr_probe:order()"));
}

use std::process::Command;

use pbeam_runtime::{BeamProgram, Term};

#[path = "../common/mod.rs"]
mod common;

use common::pinned_beam;

const PROBE: &[u8] = include_bytes!("../../fixtures/bitstring_list/bitstring_list_probe.beam");

/// Evaluate `bitstring_list_probe:<name>()` on the pinned ERTS and return
/// its `~w` output.
fn reference(name: &str) -> String {
    let reference = Command::new("erl")
        .args([
            "-noshell",
            "-pa",
            "fixtures/bitstring_list",
            "-eval",
            &format!("io:format(\"~w~n\", [bitstring_list_probe:{name}()]), halt()."),
        ])
        .output()
        .unwrap();
    assert!(
        reference.status.success() && reference.stderr.is_empty(),
        "{reference:?}"
    );
    // PBeam prints a map association as `K=>V`, and OTP 29 prints it as
    // `K => V`.
    String::from_utf8(reference.stdout)
        .unwrap()
        .replace(" => ", "=>")
}

/// Run `bitstring_list_probe:show(Name)` on PBeam with the pinned `erlang`
/// and `lists` modules, and return the print of the exit reason, which is
/// the print of the result of `Name/0`.
fn shown(name: &str) -> String {
    let program =
        BeamProgram::load(&[PROBE, &pinned_beam("erlang"), &pinned_beam("lists")]).unwrap();
    let name = Term::Atom(program.atom(name).unwrap());
    let error = program
        .execute("bitstring_list_probe", "show", &[program.host_term(name)])
        .unwrap_err()
        .to_string();
    format!(
        "{}\n",
        error.strip_prefix("root process exited with ").unwrap()
    )
}

#[test]
fn converts_bitstrings_and_lists_as_pinned_erts() {
    // `bitstring_to_list/1` gives one integer for each whole byte and the
    // last bits as a bitstring at the end (`binary.c:413-474`).
    // `list_to_bitstring/1` walks the iolist with the unit 1, so a
    // bitstring of each size can be an element or a tail
    // (`erl_iolist.c:930-934`, `176-184`). A term that is not a list, a
    // list with an element that is not a byte or a bitstring, and a bad
    // tail raise `badarg` with the `error_info` of `erl_erts_errors`.
    assert_eq!(shown("small"), reference("small"));
}

#[test]
fn converts_long_bitstrings_and_lists_as_pinned_erts() {
    // The size walk and the copy walk of `list_to_bitstring/1` trap to
    // `erts_internal:list_to_bitstring_continue` (`erl_iolist.c:764-835`),
    // and the copy of `[<<1:1>> | <<0:16000000>>]` takes 62500 steps, so it
    // always stops inside the tail at a bit offset that is not a whole byte
    // of the result and resumes there. `bitstring_to_list/1` of more than
    // 256000 bytes builds its list in chunks (`binary.c:436-450`).
    let printed = shown("long");
    assert_eq!(printed, reference("long"));
    assert_eq!(
        printed,
        "{90000,true,2703600,true,300001,<<5:3>>,0,badarg,true}\n"
    );
}

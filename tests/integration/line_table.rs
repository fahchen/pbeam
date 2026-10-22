use std::process::Command;

use pbeam_runtime::BeamProgram;

#[path = "../common/mod.rs"]
mod common;

use common::pinned_beam;

const LINE_TABLE_PROBE: &[u8] = include_bytes!("../../fixtures/line_table/line_table_probe.beam");
const LINE_TABLE_BARE: &[u8] = include_bytes!("../../fixtures/line_table/line_table_bare.beam");

/// The `%T` text of the exit reason of `line_table_probe:uncaught()` on the
/// pinned ERTS. Each frame of `line_table_probe` has the location
/// `[{file, File}, {line, Line}]` of its `Line` chunk (`parse_line_chunk`,
/// `beam_file.c:443-600`; `erts_build_mfa_item`, `utils.c:3430-3446`): the
/// current frame at the instruction that raises, a caller frame at its call
/// (`beam_common.c:817-877`), and a `function_clause` frame at the line of
/// the function (`beam_common.c:1231-1233`). The `error_info` item of
/// `erlang:error/3` comes after the line, a BIF frame has no line, and each
/// frame of `line_table_bare`, which has no `Line` chunk, has the location
/// `[]`. `process_info(self(), current_stacktrace)` gives the frames of the
/// caller with their locations (`erl_bif_info.c:2397-2460`).
const EXPECTED: &str = include_str!("../../fixtures/line_table/line_table_probe.expected");

#[test]
fn gives_the_file_and_line_of_each_frame_as_pinned_erts() {
    let reference = Command::new("erl")
        .args([
            "-noshell",
            "-pa",
            "fixtures/line_table",
            "-eval",
            "logger:remove_handler(default), {'EXIT', R} = (catch line_table_probe:uncaught()), io:format(\"~s~n\", [erts_internal:term_to_string(R, 100000)]), halt().",
        ])
        .output()
        .unwrap();
    assert!(
        reference.status.success() && reference.stderr.is_empty(),
        "{reference:?}"
    );
    assert_eq!(String::from_utf8(reference.stdout).unwrap(), EXPECTED);

    // The root process exits with the result (row "Run end" of
    // `docs/erts-differences.md`).
    let erlang = pinned_beam("erlang");
    let program = BeamProgram::load(&[LINE_TABLE_PROBE, LINE_TABLE_BARE, &erlang]).unwrap();
    let error = program
        .execute("line_table_probe", "uncaught", &[])
        .unwrap_err();
    assert_eq!(
        format!("{error}\n"),
        format!("root process exited with {EXPECTED}")
    );
}

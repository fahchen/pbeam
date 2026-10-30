//! Native records of OTP 29: the `Recs` chunk, the record instructions, the
//! `is_record` BIFs, term order, equality, hashes, and flat size.
#![expect(
    clippy::unwrap_used,
    reason = "test helpers report a failure by a panic"
)]

#[path = "../common/mod.rs"]
mod common;

use std::process::Command;

use common::pinned_beam;
use pbeam_runtime::{BeamProgram, Term};

const PROBE: &[u8] = include_bytes!("../../fixtures/native_record/nr_probe.beam");
const DEF: &[u8] = include_bytes!("../../fixtures/native_record/nr_def.beam");

/// The `~w` print of `nr_probe:results()` on the pinned ERTS (OTP 29.1.1).
const EXPECTED: &str = "[{1,2},{0,5},{9,2},{error,{badrecord,{point,1,2}}},{error,{badrecord,pub}},2,{error,{badrecord,pub}},{error,{badrecord,7}},1,{error,{badfield,{{nr_probe,point},zz}}},point,record,other,record,one,other,other,[true,true,false,false],true,false,true,false,true,{error,badarg},true,false,false,true,{error,badarg},false,{error,badarg},{error,badarg},false,false,true,false,true,true,true,true,false,true,true,true,false,false,false,false,true,true,44816085,127471339,73805464,3845231052,1409962566,3073056421,15,8,33,[9,2,5,6,10,1,8,11,4,7,3],[{5,1},1,{error,{badrecord,{nr_def,priv}}},{error,{novalue,{{nr_def,pub},a}}},{error,{badfield,{{nr_def,pub},zz}}},{error,{badrecord,{nr_none,pub}}},{3,1},{5,4},{error,{badfield,{{nr_def,pub},zz}}},{error,{badrecord,priv}},{error,{badrecord,priv}},{error,{badrecord,priv}},{error,{badrecord,priv}},{error,{badfield,{{nr_def,pub},zz}}},1,{error,{badfield,{{nr_def,priv},a}}},{8,1},nomatch,nomatch,pub,true,false],{[],[a],[3,1]},{1,4,[3,5,4,2,1]},[{badrecord,{nr_none,pub}},{badrecord,7}]]\n";

/// Return the `~w` print of `nr_probe:results()` on the pinned ERTS.
fn reference() -> String {
    let reference = Command::new("erl")
        .args([
            "-noshell",
            "-pa",
            "fixtures/native_record",
            "-eval",
            "io:format(\"~w~n\", [nr_probe:results()]), halt().",
        ])
        .output()
        .unwrap();
    assert!(reference.status.success(), "{reference:?}");
    String::from_utf8(reference.stdout).unwrap()
}

#[test]
fn runs_native_records_as_pinned_erts() {
    // Local and remote `put_record`, `get_record_elements`,
    // `get_record_field`, the record tests, `is_record/1,2,3`, `badarg` of
    // `element/2` and `tuple_size/1`, term order, `==` and `=:=`,
    // `phash2/1`, `phash/2`, and `erts_debug:flat_size/1`
    // (`erl_record.c:214-568`, `erl_bif_op.c:279-350`, `utils.c:1181-1193`,
    // 1878-1953, `erl_term_hashing.c:369-405`, 1082-1110).
    assert_eq!(reference(), EXPECTED);
    let program = BeamProgram::load(&[
        PROBE,
        DEF,
        &pinned_beam("erlang"),
        &pinned_beam("erts_debug"),
        &pinned_beam("maps"),
    ])
    .unwrap();
    let error = program
        .execute("nr_probe", "show", &[])
        .unwrap_err()
        .to_string();
    let shown = format!(
        "{}\n",
        error
            .strip_prefix("root process exited with ")
            .unwrap_or_else(|| panic!("{error}"))
    );
    assert_eq!(shown, EXPECTED);
}

const EXT: &[u8] = include_bytes!("../../fixtures/native_record/nr_ext.beam");

/// The `~w` print of `nr_ext:results()` on the pinned ERTS (OTP 29.1.1).
const EXT_EXPECTED: &str = include_str!("../../fixtures/native_record/nr_ext.expected");

#[test]
fn encodes_and_decodes_records_as_pinned_erts() {
    // `RECORD_EXT` in `term_to_binary/1,2`, `external_size/1,2`, and
    // `binary_to_term/1,2`: the loaded definition when the export flag and
    // the fields match, and `badarg` for a bad flag, a field name that
    // occurs twice, and a module, a name, or a field name that is not an
    // atom (`external.c:4006-4036`, 5257-5380, 5917-5950;
    // `erl_record.c:106-153`).
    let reference = Command::new("erl")
        .args([
            "-noshell",
            "-pa",
            "fixtures/native_record",
            "-eval",
            "io:format(\"~w~n\", [nr_ext:results()]), halt().",
        ])
        .output()
        .unwrap();
    assert!(reference.status.success(), "{reference:?}");
    assert_eq!(String::from_utf8(reference.stdout).unwrap(), EXT_EXPECTED);
    let modules: Vec<Vec<u8>> = [
        "erlang",
        "lists",
        "maps",
        "unicode",
        "io_lib",
        "io_lib_format",
        "io_lib_pretty",
        "string",
        "epp",
        "erl_scan",
        "erl_features",
        "ets",
    ]
    .iter()
    .map(|module| pinned_beam(module))
    .collect();
    let mut beams: Vec<&[u8]> = modules.iter().map(Vec::as_slice).collect();
    beams.extend([EXT, DEF]);
    let program = BeamProgram::load(&beams).unwrap();
    let shown = program.execute("nr_ext", "show", &[]).unwrap().term;
    let Term::Binary(shown) = &shown else {
        panic!("not a binary: {shown:?}");
    };
    let shown = String::from_utf8(shown.bytes().unwrap().to_vec()).unwrap();
    assert_eq!(shown, EXT_EXPECTED);
}

const BIF: &[u8] = include_bytes!("../../fixtures/native_record/nr_bif.beam");

/// The `~w` print of `nr_bif:results()` on the pinned ERTS (OTP 29.1.1).
const BIF_EXPECTED: &str = include_str!("../../fixtures/native_record/nr_bif.expected");

#[test]
fn runs_the_records_bifs_and_prints_records_as_pinned_erts() {
    // `records:get_module/1`, `get_name/1`, `get/2`, `get_field_names/1`,
    // `is_exported/1`, `get_definition/2`, `create/4`, and `update/4` with
    // their errors (`erl_record.c:592-1092`), and the `~p`, `~w`, `~P`,
    // `~W`, and `~tp` print of records (`io_lib.erl:857-871`,
    // `io_lib_pretty.erl:983-1007`).
    let reference = Command::new("erl")
        .args([
            "-noshell",
            "-pa",
            "fixtures/native_record",
            "-eval",
            "io:format(\"~w~n\", [nr_bif:results()]), halt().",
        ])
        .output()
        .unwrap();
    assert!(reference.status.success(), "{reference:?}");
    assert_eq!(String::from_utf8(reference.stdout).unwrap(), BIF_EXPECTED);
    let modules: Vec<Vec<u8>> = [
        "erlang",
        "lists",
        "maps",
        "unicode",
        "io_lib",
        "io_lib_format",
        "io_lib_pretty",
        "string",
        "epp",
        "erl_scan",
        "erl_features",
        "ets",
    ]
    .iter()
    .map(|module| pinned_beam(module))
    .collect();
    let mut beams: Vec<&[u8]> = modules.iter().map(Vec::as_slice).collect();
    beams.extend([BIF, DEF]);
    let program = BeamProgram::load(&beams).unwrap();
    let shown = program.execute("nr_bif", "show", &[]).unwrap().term;
    let Term::Binary(shown) = &shown else {
        panic!("not a binary: {shown:?}");
    };
    assert_eq!(
        String::from_utf8(shown.bytes().unwrap().to_vec()).unwrap(),
        BIF_EXPECTED
    );
}

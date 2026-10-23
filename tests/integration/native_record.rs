//! Native records of OTP 29: the `Recs` chunk, the record instructions, the
//! `is_record` BIFs, term order, equality, hashes, and flat size.

#[path = "../common/mod.rs"]
mod common;

use std::process::Command;

use common::pinned_beam;
use pbeam_runtime::BeamProgram;

const PROBE: &[u8] = include_bytes!("../../fixtures/native_record/nr_probe.beam");
const DEF: &[u8] = include_bytes!("../../fixtures/native_record/nr_def.beam");

/// The `~w` print of `nr_probe:results()` on the pinned ERTS (OTP 29.1.1).
const EXPECTED: &str = "[{1,2},{0,5},{9,2},{error,{badrecord,{point,1,2}}},{error,{badrecord,pub}},2,{error,{badrecord,pub}},{error,{badrecord,7}},1,{error,{badfield,{{nr_probe,point},zz}}},point,record,other,record,one,other,other,[true,true,false,false],true,false,true,false,true,{error,badarg},true,false,false,true,{error,badarg},false,{error,badarg},{error,badarg},false,false,true,false,true,true,true,true,false,true,true,true,false,false,false,false,true,true,44816085,127471339,73805464,3845231052,1409962566,3073056421,15,8,33,[9,2,5,6,10,1,8,11,4,7,3],[{5,1},1,{error,{badrecord,{nr_def,priv}}},{error,{novalue,{{nr_def,pub},a}}},{error,{badfield,{{nr_def,pub},zz}}},{error,{badrecord,{nr_none,pub}}},{3,1},{5,4},{error,{badfield,{{nr_def,pub},zz}}},{error,{badrecord,priv}},{error,{badrecord,priv}},{error,{badrecord,priv}},{error,{badrecord,priv}},{error,{badfield,{{nr_def,pub},zz}}},1,{error,{badfield,{{nr_def,priv},a}}},{8,1},nomatch,nomatch,pub,true,false],{[],[a]}]\n";

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

#[test]
fn stops_the_run_at_term_to_binary_of_a_record() {
    // ERTS encodes a native record (`external.c`); PBeam stops the run
    // until #1039 (row "External term format").
    let program = BeamProgram::load(&[PROBE, DEF, &pinned_beam("erlang")]).unwrap();
    let error = program
        .execute("nr_probe", "t2b", &[])
        .unwrap_err()
        .to_string();
    assert!(
        error.contains("unsupported erlang:term_to_binary/1,2 of a native record"),
        "{error}"
    );
}

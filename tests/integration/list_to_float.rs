use std::process::Command;

use pbeam_runtime::BeamProgram;

const LIST_TO_FLOAT_PROBE: &[u8] =
    include_bytes!("../../fixtures/list_to_float/list_to_float_probe.beam");

/// The `%T` text of the exit reason of `list_to_float_probe:uncaught()` on
/// the pinned ERTS, as `erts_internal:term_to_string/2` (`bif.c:4156-4190`)
/// writes it. `list_to_float_1` (`bif.c:3699-3725`) reads a proper list of
/// bytes with `erts_list_length` (`utils.c:344-357`) and `intlist_to_buf`
/// (`utils.c:2828-2850`), and gives `badarg` for a term that is not a
/// proper list of bytes from 0 to 255. The scan of `sys_chars_to_double`
/// (`sys_float.c:85-105`) reads an optional `+` or `-`, digits, a point or
/// a comma, digits, an optional `e` or `E` with an optional sign and
/// digits, and then the end of the text; each other text gives `badarg`
/// (`bif.c:3711-3723`). A text that overflows an infinity gives `badarg`
/// (`sys_float.c:111-113`), and a text that underflows to zero gives `0.0`
/// (`sys_float.c:126-137`). The error of the BIF gives the stack trace
/// `[{erlang, list_to_float, [List], [{error_info, #{module =>
/// erl_erts_errors}}]} | CallerFrames]` (row "Exception stack trace" of
/// `docs/erts-differences.md`).
const EXPECTED: &str = "[{ok,\"1.0\"},{ok,\"1.5\"},{ok,\"-1.5\"},{ok,\"1.0\"},{ok,\"-0.0\"},{ok,\"0.\
    0\"},{ok,\"1.5e3\"},{ok,\"1.0e10\"},{ok,\"1.0e3\"},{ok,\"1.5\"},{ok,\"1.0e-320\"},{ok,\"0.0\"},\
    {ok,\"0.0\"},{ok,\"0.1\"},{ok,\"0.30000000000000004\"},{ok,\"1.7976931348623157e308\"},{error,b\
    adarg,[{erlang,list_to_float,[\"1.7976931348623159e308\"],[{error_info,#{module=>erl_erts_error\
    s}}]},{list_to_float_probe,result,1,[]},{list_to_float_probe,results,1,[]},{list_to_float_probe\
    ,results,1,[]},{list_to_float_probe,uncaught,0,[]}]},{ok,\"5.0e-324\"},{ok,\"5.0e-324\"},{ok,\"\
    3.141592653589793\"},{error,badarg,[{erlang,list_to_float,[\"1\"],[{error_info,#{module=>erl_er\
    ts_errors}}]},{list_to_float_probe,result,1,[]},{list_to_float_probe,results,1,[]},{list_to_flo\
    at_probe,results,1,[]},{list_to_float_probe,uncaught,0,[]}]},{error,badarg,[{erlang,list_to_flo\
    at,[[]],[{error_info,#{module=>erl_erts_errors}}]},{list_to_float_probe,result,1,[]},{list_to_f\
    loat_probe,results,1,[]},{list_to_float_probe,results,1,[]},{list_to_float_probe,uncaught,0,[]}\
    ]},{error,badarg,[{erlang,list_to_float,[\"abc\"],[{error_info,#{module=>erl_erts_errors}}]},{l\
    ist_to_float_probe,result,1,[]},{list_to_float_probe,results,1,[]},{list_to_float_probe,results\
    ,1,[]},{list_to_float_probe,uncaught,0,[]}]},{error,badarg,[{erlang,list_to_float,[\"1.0x\"],[{\
    error_info,#{module=>erl_erts_errors}}]},{list_to_float_probe,result,1,[]},{list_to_float_probe\
    ,results,1,[]},{list_to_float_probe,results,1,[]},{list_to_float_probe,uncaught,0,[]}]},{error,\
    badarg,[{erlang,list_to_float,[\"1.5e400\"],[{error_info,#{module=>erl_erts_errors}}]},{list_to\
    _float_probe,result,1,[]},{list_to_float_probe,results,1,[]},{list_to_float_probe,results,1,[]}\
    ,{list_to_float_probe,uncaught,0,[]}]},{error,badarg,[{erlang,list_to_float,[\"1.0e309\"],[{err\
    or_info,#{module=>erl_erts_errors}}]},{list_to_float_probe,result,1,[]},{list_to_float_probe,re\
    sults,1,[]},{list_to_float_probe,results,1,[]},{list_to_float_probe,uncaught,0,[]}]},{error,bad\
    arg,[{erlang,list_to_float,[\"1.\"],[{error_info,#{module=>erl_erts_errors}}]},{list_to_float_p\
    robe,result,1,[]},{list_to_float_probe,results,1,[]},{list_to_float_probe,results,1,[]},{list_t\
    o_float_probe,uncaught,0,[]}]},{error,badarg,[{erlang,list_to_float,[\".1\"],[{error_info,#{mod\
    ule=>erl_erts_errors}}]},{list_to_float_probe,result,1,[]},{list_to_float_probe,results,1,[]},{\
    list_to_float_probe,results,1,[]},{list_to_float_probe,uncaught,0,[]}]},{error,badarg,[{erlang,\
    list_to_float,[\"!1.0\"],[{error_info,#{module=>erl_erts_errors}}]},{list_to_float_probe,result\
    ,1,[]},{list_to_float_probe,results,1,[]},{list_to_float_probe,results,1,[]},{list_to_float_pro\
    be,uncaught,0,[]}]},{error,badarg,[{erlang,list_to_float,[\"1.0e\"],[{error_info,#{module=>erl_\
    erts_errors}}]},{list_to_float_probe,result,1,[]},{list_to_float_probe,results,1,[]},{list_to_f\
    loat_probe,results,1,[]},{list_to_float_probe,uncaught,0,[]}]},{error,badarg,[{erlang,list_to_f\
    loat,[\"1.0e+\"],[{error_info,#{module=>erl_erts_errors}}]},{list_to_float_probe,result,1,[]},{\
    list_to_float_probe,results,1,[]},{list_to_float_probe,results,1,[]},{list_to_float_probe,uncau\
    ght,0,[]}]},{error,badarg,[{erlang,list_to_float,[\"0x1.8p3\"],[{error_info,#{module=>erl_erts_\
    errors}}]},{list_to_float_probe,result,1,[]},{list_to_float_probe,results,1,[]},{list_to_float_\
    probe,results,1,[]},{list_to_float_probe,uncaught,0,[]}]},{error,badarg,[{erlang,list_to_float,\
    [\"1.0 \"],[{error_info,#{module=>erl_erts_errors}}]},{list_to_float_probe,result,1,[]},{list_t\
    o_float_probe,results,1,[]},{list_to_float_probe,results,1,[]},{list_to_float_probe,uncaught,0,\
    []}]},{error,badarg,[{erlang,list_to_float,[[1,2,3]],[{error_info,#{module=>erl_erts_errors}}]}\
    ,{list_to_float_probe,result,1,[]},{list_to_float_probe,results,1,[]},{list_to_float_probe,resu\
    lts,1,[]},{list_to_float_probe,uncaught,0,[]}]},{error,badarg,[{erlang,list_to_float,[[49,46,48\
    |foo]],[{error_info,#{module=>erl_erts_errors}}]},{list_to_float_probe,result,1,[]},{list_to_fl\
    oat_probe,results,1,[]},{list_to_float_probe,results,1,[]},{list_to_float_probe,uncaught,0,[]}]\
    },{error,badarg,[{erlang,list_to_float,[[1.000000e+00]],[{error_info,#{module=>erl_erts_errors}\
    }]},{list_to_float_probe,result,1,[]},{list_to_float_probe,results,1,[]},{list_to_float_probe,r\
    esults,1,[]},{list_to_float_probe,uncaught,0,[]}]},{error,badarg,[{erlang,list_to_float,[[4851,\
    700]],[{error_info,#{module=>erl_erts_errors}}]},{list_to_float_probe,result,1,[]},{list_to_flo\
    at_probe,results,1,[]},{list_to_float_probe,results,1,[]},{list_to_float_probe,uncaught,0,[]}]}\
    ,{error,badarg,[{erlang,list_to_float,[abc],[{error_info,#{module=>erl_erts_errors}}]},{list_to\
    _float_probe,result,1,[]},{list_to_float_probe,results,1,[]},{list_to_float_probe,results,1,[]}\
    ,{list_to_float_probe,uncaught,0,[]}]},{ok,\"1.0\"},{ok,\"1.0\"},{error,badarg,[{erlang,list_to\
    _float,[[49,0,46,48]],[{error_info,#{module=>erl_erts_errors}}]},{list_to_float_probe,result,1,\
    []},{list_to_float_probe,results,1,[]},{list_to_float_probe,results,1,[]},{list_to_float_probe,\
    uncaught,0,[]}]},{error,badarg,[{erlang,list_to_float,[[0]],[{error_info,#{module=>erl_erts_err\
    ors}}]},{list_to_float_probe,result,1,[]},{list_to_float_probe,results,1,[]},{list_to_float_pro\
    be,results,1,[]},{list_to_float_probe,uncaught,0,[]}]},{error,badarg,[{erlang,list_to_float,[[0\
    ,49,46,48]],[{error_info,#{module=>erl_erts_errors}}]},{list_to_float_probe,result,1,[]},{list_\
    to_float_probe,results,1,[]},{list_to_float_probe,results,1,[]},{list_to_float_probe,uncaught,0\
    ,[]}]},{ok,\"1.5\"},{error,badarg,[{erlang,list_to_float,[[49,46,48,0,4851]],[{error_info,#{mod\
    ule=>erl_erts_errors}}]},{list_to_float_probe,result,1,[]},{list_to_float_probe,results,1,[]},{\
    list_to_float_probe,results,1,[]},{list_to_float_probe,uncaught,0,[]}]},{error,badarg,[{erlang,\
    list_to_float,[[49,46,53,101,0,53]],[{error_info,#{module=>erl_erts_errors}}]},{list_to_float_p\
    robe,result,1,[]},{list_to_float_probe,results,1,[]},{list_to_float_probe,results,1,[]},{list_t\
    o_float_probe,uncaught,0,[]}]},{error,badarg,[{erlang,list_to_float,[[0,0]],[{error_info,#{modu\
    le=>erl_erts_errors}}]},{list_to_float_probe,result,1,[]},{list_to_float_probe,results,1,[]},{l\
    ist_to_float_probe,results,1,[]},{list_to_float_probe,uncaught,0,[]}]},{ok,\"1.0\"},{ok,\"1.0\"\
    },{error,badarg,[{erlang,list_to_float,[\"99999999999999999999999999999999999999999999999999999\
    99999999999999999999999999999999999999999999999999999999999999999999999999999999999999999999999\
    99999999999999999999999999999999999999999999999999999999999999999999999999999999999999999999999\
    999999999999999999999999999999999999999999999999999999999999999999,5e-309\"],[{error_info,#{mod\
    ule=>erl_erts_errors}}]},{list_to_float_probe,result,1,[]},{list_to_float_probe,results,1,[]},{\
    list_to_float_probe,results,1,[]},{list_to_float_probe,uncaught,0,[]}]}]";

#[test]
fn reads_floats_from_lists_as_pinned_erts() {
    // The run removes the default logger handler, so the error report of
    // the exit of the eval process does not go to the output.
    let reference = Command::new("erl")
        .args([
            "-noshell",
            "-pa",
            "fixtures/list_to_float",
            "-eval",
            "logger:remove_handler(default), {'EXIT', R} = (catch list_to_float_probe:uncaught()), io:format(\"~s~n\", [erts_internal:term_to_string(R, 100000)]), halt().",
        ])
        .output()
        .unwrap();
    assert!(
        reference.status.success() && reference.stderr.is_empty(),
        "{reference:?}"
    );
    assert_eq!(
        String::from_utf8(reference.stdout).unwrap(),
        format!("{EXPECTED}\n")
    );

    // The root process exits with the result (row "Run end" of
    // `docs/erts-differences.md`).
    let program = BeamProgram::load(&[LIST_TO_FLOAT_PROBE]).unwrap();
    let error = program
        .execute("list_to_float_probe", "uncaught", &[])
        .unwrap_err();
    assert_eq!(
        format!("{error}\n"),
        format!("root process exited with {EXPECTED}\n")
    );
}

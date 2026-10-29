//! Runs `maps:values`, `maps:update` and the `math` BIFs on PBeam and compares them with the pinned
//! ERTS.

use std::process::Command;

use pbeam_runtime::{BeamProgram, Term};

const MAPS_MATH_PROBE: &[u8] = include_bytes!("../../fixtures/maps_math/maps_math_probe.beam");

/// The spawn options of the measured processes in the ERTS reference run: a
/// large heap, so that no measured process collects garbage. PBeam has no
/// process heap and runs with `[]`.
const ERTS_OPTIONS: &str = "[{min_heap_size,4000000},{min_bin_vheap_size,40000000}]";

/// `maps_math_probe:show(values)` on the pinned ERTS with `erl +S 1` (JIT):
/// the values of `maps:values/1` in map order, and the `badmap` error of a
/// term that is not a map, with its stack trace.
const ERTS_VALUES: &str = "[[],[a,b,c],[2,4,6,8,10,12,14,16,18,20,22,24,26,28,30,32,34,36,38,40,42,44,46,48,50,52,54,56,58,\
    60,62,64],[36,8,24,38,58,26,4,14,62,16,20,46,18,30,64,2,50,56,40,12,22,34,48,28,66,6,32,60,42,\
    10,54,52,44],[36,8,68,24,38,58,26,4,14,76,62,16,20,46,18,30,70,72,64,2,50,56,40,12,22,34,48,74,\
    80,28,66,6,32,60,42,10,54,52,44,78],{error,{badmap,1},[{maps,values,[1]},{maps_math_probe,\
    -values/0-fun-2-,0},{maps_math_probe,run_caught,2}]},{error,{badmap,[]},[{maps,values,[[]]},\
    {maps_math_probe,-values/0-fun-1-,0},{maps_math_probe,run_caught,2}]},{error,{badmap,{a}},\
    [{maps,values,[{a}]},{maps_math_probe,-values/0-fun-0-,0},{maps_math_probe,run_caught,2}]}]";

/// `maps_math_probe:show(updates)` on the pinned ERTS: the map of
/// `maps:update/3` with the key set to the value, an error that names the
/// key that the map does not hold, and an error that names a term that is
/// not a map, each with its stack trace.
const ERTS_UPDATES: &str = "[#{1=>x,2=>a,3=>z},#{1=>x,2=>y},#{a=>1},#{1=>a,2=>4,3=>6,4=>8,5=>10,6=>12,7=>14,8=>16,9=>18,\
    10=>20,11=>22,12=>24,13=>26,14=>28,15=>30,16=>32,17=>34,18=>36,19=>38,20=>40,21=>42,22=>44,\
    23=>46,24=>48,25=>50,26=>52,27=>54,28=>56,29=>58,30=>60,31=>62,32=>64},#{1=>2,2=>4,3=>6,4=>8,\
    5=>10,6=>12,7=>14,8=>16,9=>18,10=>20,11=>22,12=>24,13=>26,14=>28,15=>30,16=>32,17=>34,18=>36,\
    19=>38,20=>40,21=>42,22=>44,23=>46,24=>48,25=>50,26=>52,27=>54,28=>56,29=>58,30=>60,31=>62,\
    32=>a},#{18=>36,4=>8,12=>24,19=>38,29=>58,13=>26,2=>4,7=>14,31=>62,8=>16,10=>20,23=>46,9=>18,\
    15=>30,32=>64,1=>a,25=>50,28=>56,20=>40,6=>12,11=>22,17=>34,24=>48,14=>28,33=>66,3=>6,16=>32,\
    30=>60,21=>42,5=>10,27=>54,26=>52,22=>44},#{18=>36,4=>8,12=>24,19=>38,29=>58,13=>26,2=>4,7=>14,\
    31=>62,8=>16,10=>20,23=>46,9=>18,15=>30,32=>64,1=>2,25=>50,28=>56,20=>40,6=>12,11=>22,17=>a,\
    24=>48,14=>28,33=>66,3=>6,16=>32,30=>60,21=>42,5=>10,27=>54,26=>52,22=>44},#{18=>36,4=>8,12=>24,\
    19=>38,29=>58,13=>26,2=>4,7=>14,31=>62,8=>16,10=>20,23=>46,9=>18,15=>30,32=>64,1=>2,25=>50,\
    28=>56,20=>40,6=>12,11=>22,17=>34,24=>48,14=>28,33=>a,3=>6,16=>32,30=>60,21=>42,5=>10,27=>54,\
    26=>52,22=>44},#{18=>36,4=>8,34=>68,12=>24,19=>38,29=>58,13=>26,2=>4,7=>14,38=>76,31=>62,8=>16,\
    10=>20,23=>46,9=>18,15=>30,35=>70,36=>72,32=>64,1=>2,25=>50,28=>56,20=>40,6=>12,11=>22,17=>34,\
    24=>48,37=>74,40=>80,14=>28,33=>66,3=>6,16=>32,30=>60,21=>42,5=>10,27=>54,26=>52,22=>44,39=>78},\
    {error,{badkey,9},[{maps,update,[9,x,#{1=>y}]},{maps_math_probe,-updates/0-fun-5-,0},\
    {maps_math_probe,run_caught,2}]},{error,{badkey,1},[{maps,update,[1,x,#{}]},{maps_math_probe,\
    -updates/0-fun-4-,0},{maps_math_probe,run_caught,2}]},{error,{badkey,a},[{maps,update,[a,x,\
    #{1=>y}]},{maps_math_probe,-updates/0-fun-3-,0},{maps_math_probe,run_caught,2}]},{error,{badkey,\
    34},[{maps,update,[34,x,#{18=>36,4=>8,12=>24,19=>38,29=>58,13=>26,2=>4,7=>14,31=>62,8=>16,\
    10=>20,23=>46,9=>18,15=>30,32=>64,1=>2,25=>50,28=>56,20=>40,6=>12,11=>22,17=>34,24=>48,14=>28,\
    33=>66,3=>6,16=>32,30=>60,21=>42,5=>10,27=>54,26=>52,22=>44}]},{maps_math_probe,\
    -updates/0-fun-2-,0},{maps_math_probe,run_caught,2}]},{error,{badmap,not_a_map},[{maps,update,\
    [a,x,not_a_map]},{maps_math_probe,-updates/0-fun-1-,0},{maps_math_probe,run_caught,2}]},{error,\
    {badmap,[]},[{maps,update,[a,x,[]]},{maps_math_probe,-updates/0-fun-0-,0},{maps_math_probe,\
    run_caught,2}]}]";

/// `maps_math_probe:show(math)` on the pinned ERTS: the double of each
/// `math` function of the C library, for a float, a small integer, and a
/// bignum operand, with the shortest text of each result.
const ERTS_MATH: &str = "[1.0,2.718281828459045,0.36787944117144233,0.0,1.7976931348622732e308,0.0,0.6931471805599453,\
    6.907755278982137,42.97512519471661,2.0,0.3010299956639812,2.0,0.0,1.4142135623730951,\
    2147483648.0,1024.0,1.4142135623730951,0.5,1.0,0.0,-8.0,4.0,9.332636185032189e-302]";

/// `maps_math_probe:show(math_errors)` on the pinned ERTS: `badarith` for a
/// result that is not finite and `badarg` for an operand that is not a
/// number, each with its stack trace.
const ERTS_MATH_ERRORS: &str = "[{error,badarith,[{math,sqrt,[-1]},{maps_math_probe,-math_errors/0-fun-19-,0},{maps_math_probe,\
    run_caught,2}]},{error,badarith,[{math,sqrt,[-1.0]},{maps_math_probe,-math_errors/0-fun-18-,0},\
    {maps_math_probe,run_caught,2}]},{error,badarith,[{math,log,[0]},{maps_math_probe,\
    -math_errors/0-fun-17-,0},{maps_math_probe,run_caught,2}]},{error,badarith,[{math,log,[0.0]},\
    {maps_math_probe,-math_errors/0-fun-16-,0},{maps_math_probe,run_caught,2}]},{error,badarith,\
    [{math,log,[-1]},{maps_math_probe,-math_errors/0-fun-15-,0},{maps_math_probe,run_caught,2}]},\
    {error,badarith,[{math,log10,[0]},{maps_math_probe,-math_errors/0-fun-14-,0},{maps_math_probe,\
    run_caught,2}]},{error,badarith,[{math,log10,[-1.0]},{maps_math_probe,-math_errors/0-fun-13-,0},\
    {maps_math_probe,run_caught,2}]},{error,badarith,[{math,exp,[1000]},{maps_math_probe,\
    -math_errors/0-fun-12-,0},{maps_math_probe,run_caught,2}]},{error,badarith,[{math,exp,\
    [4611686018427387904]},{maps_math_probe,-math_errors/0-fun-11-,0},{maps_math_probe,run_caught,\
    2}]},{error,badarith,[{math,pow,[0,-1]},{maps_math_probe,-math_errors/0-fun-10-,0},\
    {maps_math_probe,run_caught,2}]},{error,badarith,[{math,pow,[0.0,-0.5]},{maps_math_probe,\
    -math_errors/0-fun-9-,0},{maps_math_probe,run_caught,2}]},{error,badarith,[{math,pow,[-8,0.5]},\
    {maps_math_probe,-math_errors/0-fun-8-,0},{maps_math_probe,run_caught,2}]},{error,badarith,\
    [{math,pow,[1.0e300,2]},{maps_math_probe,-math_errors/0-fun-7-,0},{maps_math_probe,run_caught,\
    2}]},{error,badarith,[{math,pow,[2,4611686018427387904]},{maps_math_probe,-math_errors/0-fun-6-,\
    0},{maps_math_probe,run_caught,2}]},{error,badarg,[{math,pow,[a,1]},{maps_math_probe,\
    -math_errors/0-fun-5-,0},{maps_math_probe,run_caught,2}]},{error,badarg,[{math,pow,[1,a]},\
    {maps_math_probe,-math_errors/0-fun-4-,0},{maps_math_probe,run_caught,2}]},{error,badarg,[{math,\
    pow,[[],[]]},{maps_math_probe,-math_errors/0-fun-3-,0},{maps_math_probe,run_caught,2}]},{error,\
    badarg,[{math,sqrt,[[]]},{maps_math_probe,-math_errors/0-fun-2-,0},{maps_math_probe,run_caught,\
    2}]},{error,badarg,[{math,exp,[#{}]},{maps_math_probe,-math_errors/0-fun-1-,0},{maps_math_probe,\
    run_caught,2}]},{error,badarg,[{math,log10,[a]},{maps_math_probe,-math_errors/0-fun-0-,0},\
    {maps_math_probe,run_caught,2}]}]";

/// `maps_math_probe:show(math_bignums)` on the pinned ERTS: `badarith` for
/// a bignum operand that `big_to_double` cannot convert. The frames of
/// these errors are not compared: the text of a frame would hold all 309
/// digits of the operand.
const ERTS_MATH_BIGNUMS: &str = "[{error,badarith},{error,badarith},{error,badarith},{error,badarith},{error,badarith},{error,badarith}]";

/// `maps_math_probe:show_reductions/1` on the pinned ERTS with `erl +S 1`
/// (JIT): for each map shape, the count and the sum of the values that
/// `maps:values/1` returns, and the reductions of the call.
const ERTS_REDUCTIONS: &str = "[{empty,{0,0},2},{flat32,{32,1056},2},{hash33,{33,1122},4},{hash100,{100,10100},8},{hash100000,\
    {100000,10000100000},6668}]";

/// Return the text of one call on the pinned ERTS with `erl +S 1` (JIT).
///
/// # Examples
///
/// ```rust,ignore
/// assert_eq!(reference("maps_math_probe:show(values)"), b"[]\n");
/// ```
fn reference(call: &str) -> Vec<u8> {
    let output = Command::new("erl")
        .args([
            "+S",
            "1",
            "-noshell",
            "-pa",
            "fixtures/maps_math",
            "-eval",
            &format!("io:format(\"~s~n\", [{call}]), halt()."),
        ])
        .output()
        .unwrap();
    assert!(
        output.status.success() && output.stderr.is_empty(),
        "{output:?}"
    );
    output.stdout
}

/// Return the text of `maps_math_probe:show(Function)` on PBeam.
///
/// # Examples
///
/// ```rust,ignore
/// assert_eq!(show(&program, "values"), b"[]");
/// ```
fn show(program: &BeamProgram, function: &str) -> Vec<u8> {
    let name = Term::Atom(program.atom(function).unwrap());
    let result = program
        .execute("maps_math_probe", "show", &[program.host_term(name)])
        .unwrap()
        .term;
    let Term::Binary(text) = &result else {
        panic!("show/1 returns a binary, not {result:?}");
    };
    text.bytes().unwrap().to_vec()
}

/// Check that PBeam runs `maps_math_probe:Function/0` as the pinned ERTS
/// does, and that the text is `expected`.
fn assert_text(program: &BeamProgram, function: &str, expected: &str) {
    assert_eq!(String::from_utf8_lossy(&show(program, function)), expected);
    assert_eq!(
        String::from_utf8_lossy(&reference(&format!("maps_math_probe:show({function})"))),
        format!("{expected}\n")
    );
}

#[test]
fn runs_maps_values_as_pinned_erts() {
    // `maps_values_1` walks the map in ERTS map order, so a flatmap gives
    // its values in key order and a hashmap gives them in slot order
    // (`erl_map.c:2262-2293`, `2844-2879`). A term that is not a map raises
    // `badmap` (`erl_map.c:2290-2292`), with the frame of the BIF and the
    // frames of the caller (`beam_common.c:880-1055`).
    let program = BeamProgram::load(&[MAPS_MATH_PROBE]).unwrap();
    assert_text(&program, "values", ERTS_VALUES);
}

#[test]
fn runs_maps_update_as_pinned_erts() {
    // `erts_maps_update` keeps the form and the key order of a flatmap and
    // the layout of a hashmap (`erl_map.c:2036-2103`). A key that the map
    // does not hold raises `{badkey, Key}`, and a term that is not a map
    // raises `{badmap, Map}` (`erl_map.c:2241-2253`).
    let program = BeamProgram::load(&[MAPS_MATH_PROBE]).unwrap();
    assert_text(&program, "updates", ERTS_UPDATES);
}

#[test]
fn runs_the_math_bifs_as_pinned_erts() {
    // `math_call_1` and `math_call_2` call the C library function of the
    // platform for a float, a small integer, and a bignum operand
    // (`erl_math.c:43-56`, `107-133`), and make a float term of the result
    // (`erl_math.c:59-61`, `137-139`).
    let program = BeamProgram::load(&[MAPS_MATH_PROBE]).unwrap();
    assert_text(&program, "math", ERTS_MATH);
}

#[test]
fn raises_the_math_errors_of_pinned_erts() {
    // `ERTS_FP_ERROR_THOROUGH` raises `badarith` for a result that is not
    // finite (`sys/unix/erl_unix_sys.h:388`), and an operand that is not a
    // number raises `badarg` (`erl_math.c:53-55`, `130-132`).
    let program = BeamProgram::load(&[MAPS_MATH_PROBE]).unwrap();
    assert_text(&program, "math_errors", ERTS_MATH_ERRORS);
}

#[test]
fn raises_badarith_for_a_bignum_operand_of_a_math_bif() {
    // `big_to_double` fails for a bignum whose nearest double is not
    // finite, and `math_call_1` and `math_call_2` then raise `badarith`
    // (`erl_math.c:47-52`, `111-116`).
    let program = BeamProgram::load(&[MAPS_MATH_PROBE]).unwrap();
    assert_text(&program, "math_bignums", ERTS_MATH_BIGNUMS);
}

#[test]
fn charges_the_reductions_of_maps_values_as_pinned_erts() {
    // `maps:values/1` runs as YCF code with 15 units for each reduction
    // (`erl_map.c:2299`): a flatmap uses no unit, and a hashmap uses one
    // unit for each key (`erl_map.c:2280-2288`, `2872-2875`). A map of
    // 100000 keys uses more units than the budget of one turn, so the BIF
    // traps and continues in the next turn (`utils.c:3733-3819`).
    let program = BeamProgram::load(&[MAPS_MATH_PROBE]).unwrap();
    let result = program
        .execute(
            "maps_math_probe",
            "show_reductions",
            &[program.host_term(Term::nil())],
        )
        .unwrap()
        .term;
    let Term::Binary(text) = &result else {
        panic!("show_reductions/1 returns a binary, not {result:?}");
    };
    assert_eq!(
        String::from_utf8_lossy(text.bytes().unwrap()),
        ERTS_REDUCTIONS
    );
    assert_eq!(
        String::from_utf8_lossy(&reference(&format!(
            "maps_math_probe:show_reductions({ERTS_OPTIONS})"
        ))),
        format!("{ERTS_REDUCTIONS}\n")
    );
}

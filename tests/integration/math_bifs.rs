use std::process::Command;

use pbeam_runtime::{BeamProgram, Term};

const MATH_BIFS_PROBE: &[u8] = include_bytes!("../../fixtures/math_bifs/math_bifs_probe.beam");

/// The BIFs that PBeam computes with the `libm` crate, not with the C
/// library of the platform. Their results can differ from ERTS by up to
/// two units in the last place (`docs/erts-differences.md`).
const LIBM_CRATE: &[&str] = &["acosh", "asinh", "atanh", "erf", "erfc"];

/// Return the text of one call on the pinned ERTS of this platform. The
/// probe results depend on the C library, so the test compares with the
/// ERTS of the same host and holds no constant text.
///
/// # Examples
///
/// ```rust,ignore
/// assert!(reference("math_bifs_probe:show()").starts_with("[{acos,"));
/// ```
fn reference(call: &str) -> String {
    let output = Command::new("erl")
        .args([
            "+S",
            "1",
            "-noshell",
            "-pa",
            "fixtures/math_bifs",
            "-eval",
            &format!("io:format(\"~s\", [{call}]), halt()."),
        ])
        .output()
        .unwrap();
    assert!(
        output.status.success() && output.stderr.is_empty(),
        "{output:?}"
    );
    String::from_utf8(output.stdout).unwrap()
}

/// Return the text of `math_bifs_probe:Function()` on PBeam.
///
/// # Examples
///
/// ```rust,ignore
/// assert!(show("show").starts_with("[{acos,"));
/// ```
fn show(function: &str) -> String {
    let program = BeamProgram::load(&[MATH_BIFS_PROBE]).unwrap();
    let result = program
        .execute("math_bifs_probe", function, &[])
        .unwrap()
        .term;
    let Term::Binary(text) = &result else {
        panic!("{function}/0 returns a binary, not {result:?}");
    };
    String::from_utf8(text.bytes().unwrap().to_vec()).unwrap()
}

/// Split the probe text into one `(name, results)` entry for each BIF. The
/// text of an error, `{error,badarith}` or `{error,badarg}`, becomes one
/// result.
///
/// # Examples
///
/// ```rust,ignore
/// assert_eq!(entries("[{sin,[0,{error,badarg}]}]"), vec![("sin".into(), vec!["0".into(), "error:badarg".into()])]);
/// ```
fn entries(text: &str) -> Vec<(String, Vec<String>)> {
    let text = text.replace("{error,", "error:").replace("}", "");
    text.trim_start_matches("[{")
        .trim_end_matches("]]")
        .split("],{")
        .map(|entry| {
            let (name, results) = entry.split_once(",[").unwrap();
            (
                name.to_owned(),
                results.split(',').map(str::to_owned).collect(),
            )
        })
        .collect()
}

#[test]
fn runs_the_remaining_math_bifs_as_pinned_erts() {
    // `math_call_1` and `math_call_2` convert a number to a double, raise
    // `badarg` for a term that is not a number, call the C library function,
    // and raise `badarith` for a result that is not finite
    // (`erl_math.c:35-141`, `sys/unix/erl_unix_sys.h:388-398`). The BIFs of
    // `erl_math.c:143-297` call them. Each result is the 64 bits of the
    // float as a signed integer, so one unit in the last place is a
    // difference of one.
    let pbeam = show("show");
    let erts = reference("math_bifs_probe:show()");
    let (pbeam, erts) = (entries(&pbeam), entries(&erts));
    assert_eq!(pbeam.len(), 19);
    assert_eq!(pbeam.len(), erts.len());
    for ((name, results), (erts_name, erts_results)) in pbeam.iter().zip(&erts) {
        assert_eq!(name, erts_name);
        assert_eq!(results.len(), erts_results.len(), "{name}");
        for (index, (result, erts_result)) in results.iter().zip(erts_results).enumerate() {
            let close = LIBM_CRATE.contains(&name.as_str())
                && matches!(
                    (result.parse::<i64>(), erts_result.parse::<i64>()),
                    (Ok(bits), Ok(erts_bits)) if bits.abs_diff(erts_bits) <= 2
                );
            assert!(
                result == erts_result || close,
                "{name} argument {index}: {result} on PBeam, {erts_result} on ERTS"
            );
        }
    }
}

#[test]
fn charges_one_reduction_for_each_math_bif() {
    // A BIF call costs one reduction (`beam_common.c`, `erts_call_bif`
    // through `BIF_RET`), and no `math` BIF bumps more (`erl_math.c`).
    assert_eq!(
        show("show_reductions"),
        reference("math_bifs_probe:show_reductions()")
    );
}

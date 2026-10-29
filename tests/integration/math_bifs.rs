//! Runs the `math` BIFs of the C library on PBeam and checks that each one costs one reduction.

use std::process::Command;

use pbeam_runtime::{BeamProgram, Term};

const MATH_BIFS_PROBE: &[u8] = include_bytes!("../../fixtures/math_bifs/math_bifs_probe.beam");

/// Return the text of one call on the pinned ERTS of this host. The probe
/// results depend on the C library of the platform, so the test compares
/// with the ERTS of the same host and holds no constant text.
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

#[test]
fn runs_the_math_bifs_of_the_c_library_as_pinned_erts() {
    // `math_call_1` and `math_call_2` convert a number to a double, raise
    // `badarg` for a term that is not a number, call the C library function,
    // and raise `badarith` for a result that is not finite
    // (`erl_math.c:35-141`, `sys/unix/erl_unix_sys.h:388-398`). The BIFs of
    // `erl_math.c:143-297` call them. Each result is the 64 bits of the
    // float, so the texts compare bit for bit.
    let pbeam = show("show");
    assert!(pbeam.starts_with("[{acos,["), "{pbeam}");
    assert_eq!(pbeam, reference("math_bifs_probe:show()"));
}

#[test]
fn charges_one_reduction_for_each_math_bif() {
    // A BIF call costs one reduction, and no `math` BIF bumps more
    // (`erl_math.c:35-141`): each loop iteration costs 4 reductions.
    assert_eq!(
        show("show_reductions"),
        reference("math_bifs_probe:show_reductions()")
    );
}

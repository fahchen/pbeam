use std::process::Command;

use pbeam_runtime::{BeamProgram, Term};

const BIT_SIZE_PROBE: &[u8] = include_bytes!("../../fixtures/bit_size/bit_size_probe.beam");

/// Evaluate an expression on the pinned ERTS with `bit_size_probe` in the
/// code path and return its `~w` output.
fn erl_reference(expression: &str) -> String {
    let reference = Command::new("erl")
        .args([
            "-noshell",
            "-pa",
            "fixtures/bit_size",
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

#[test]
fn gives_the_bit_size_like_erts() {
    // The pinned compiler emits each `bit_size/1` call of `main/0` and
    // `body/1` as `gc_bif` with the fail label 0, and of the guard of
    // `guard/1` as `gc_bif` with a fail label. `rest/1` runs `gc_bif` on a
    // match context. The fun and `apply/3` run the BIF through its export.
    // PBeam gives the stack trace `[]`, so the probe catches only the class
    // and the reason.
    let program = BeamProgram::load(&[BIT_SIZE_PROBE]).unwrap();
    let atom = |name: &str| Term::Atom(program.atom(name).unwrap());
    let int = Term::Integer;
    let badarg = Term::tuple(vec![atom("error"), atom("badarg")]);
    let mut expected = vec![
        int(0),
        int(8),
        int(48),
        int(344),
        int(24),
        int(16),
        atom("small"),
        atom("big"),
        atom("other"),
        atom("other"),
    ];
    expected.extend(std::iter::repeat_n(badarg, 5));
    expected.extend([
        Term::tuple(vec![atom("long"), int(16)]),
        Term::tuple(vec![atom("short"), int(0)]),
        atom("none"),
    ]);
    assert_eq!(
        program.execute("bit_size_probe", "main", &[]).unwrap().term,
        Term::list_from(expected)
    );

    assert_eq!(
        erl_reference("bit_size_probe:main()"),
        "[0,8,48,344,24,16,small,big,other,other,{error,badarg},{error,badarg},\
         {error,badarg},{error,badarg},{error,badarg},{long,16},{short,0},none]\n"
    );
}

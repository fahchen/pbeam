use std::process::Command;

use pbeam_runtime::{BeamProgram, Term};

const UNIQUE_INTEGER_PROBE: &[u8] =
    include_bytes!("../../fixtures/unique_integer/unique_integer_probe.beam");

/// `MIN_SMALL` of a 64-bit emulator.
const MIN_SMALL: i64 = -(1 << 59);

#[test]
fn makes_unique_integers_from_the_counter_of_the_scheduler() {
    // `unique_integer_bif` makes the value from the thread id of the
    // scheduler and its counter, plus `MIN_SMALL` (erl_bif_unique.c:475-578).
    // PBeam has one scheduler with thread id 1 and no dirty scheduler that
    // runs Erlang code, so the counter goes after 1 bit: the values are
    // `MIN_SMALL + 1`, `+ 3`, `+ 5`, and so on, also for a new process.
    let program = BeamProgram::load(&[UNIQUE_INTEGER_PROBE]).unwrap();
    let values = program
        .execute("unique_integer_probe", "main", &[])
        .unwrap();
    assert_eq!(
        values.term,
        Term::list_from([1, 3, 5, 7].map(|step| Term::Integer(MIN_SMALL + step)))
    );

    // ERTS gives other values: its thread ids include the dirty schedulers
    // of the host, and its boot uses the counter first. Each value is a
    // negative small integer, and the values grow (row "Unique integers" of
    // docs/erts-differences.md).
    let reference = Command::new("erl")
        .args([
            "+S",
            "1",
            "-noshell",
            "-pa",
            "fixtures/unique_integer",
            "-eval",
            "io:format(\"~w~n\", [unique_integer_probe:main()]), halt().",
        ])
        .output()
        .unwrap();
    assert!(
        reference.status.success() && reference.stderr.is_empty(),
        "{reference:?}"
    );
    let text = String::from_utf8(reference.stdout).unwrap();
    let erts: Vec<i64> = text
        .trim()
        .trim_matches(['[', ']'])
        .split(',')
        .map(|value| value.parse().unwrap())
        .collect();
    assert_eq!(erts.len(), 4, "{text}");
    assert!(
        erts.iter().all(|value| (MIN_SMALL..0).contains(value)),
        "{text}"
    );
    assert!(erts.windows(2).all(|pair| pair[0] < pair[1]), "{text}");
}

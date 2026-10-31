//! Temporary measurement of the append tree candidate: the copy cost of a
//! repeated append and the match throughput of a doubling build.
//!
//! The test profile uses fewer runs. The release profile of `cargo test`
//! holds the measurement runs. CI runs the separate append correctness test.
#![expect(
    clippy::unwrap_used,
    reason = "test helpers report a failure by a panic"
)]

use std::{hint::black_box, time::Instant};

use pbeam_runtime::{BeamProgram, Term};

const BINARY_APPEND_PROBE: &[u8] =
    include_bytes!("../../fixtures/binary_append/binary_append_probe.beam");

const RUNS: usize = if cfg!(debug_assertions) { 2 } else { 9 };

fn execute(program: &BeamProgram, function: &str, arguments: &[Term]) -> Term {
    let arguments = arguments
        .iter()
        .cloned()
        .map(|term| program.host_term(term))
        .collect::<Vec<_>>();
    program
        .execute("binary_append_probe", function, &arguments)
        .unwrap()
        .term
}

/// The time in nanoseconds of one `execute` call of `function`.
#[expect(
    clippy::as_conversions,
    clippy::cast_precision_loss,
    reason = "timing probe: the nanosecond and round counts convert to f64 for a ratio"
)]
fn timed(program: &BeamProgram, function: &str, arguments: &[Term]) -> f64 {
    let start = Instant::now();
    let out = execute(program, function, arguments);
    let elapsed = start.elapsed().as_nanos() as f64;
    black_box(&out);
    elapsed
}

/// The least time of `RUNS` runs of `run`, which gives the time of one run.
fn best_of(mut run: impl FnMut() -> f64) -> f64 {
    let mut best = f64::MAX;
    for _ in 0..RUNS {
        best = best.min(run());
    }
    best
}

#[test]
#[expect(
    clippy::as_conversions,
    clippy::cast_precision_loss,
    reason = "timing probe: the nanosecond and round counts convert to f64 for a ratio"
)]
fn appends_and_matches() {
    let program = BeamProgram::load(&[BINARY_APPEND_PROBE]).unwrap();

    // The copy cost of one append to a prefix of `len` bytes, repeated.
    for (len, rounds) in [(1024_usize, 1000_usize), (100_000, 100), (1_000_000, 20)] {
        let prefix = Term::binary(vec![b'a'; len]);
        let suffix = Term::binary(b"b".to_vec());
        let arguments = [
            prefix,
            suffix,
            Term::Integer(i64::try_from(rounds).unwrap()),
        ];
        let best = best_of(|| timed(&program, "append_many", &arguments));
        println!(
            "APPEND-COPY len={len} rounds={rounds} per-append={:.3} ns",
            best / rounds as f64
        );
    }

    // The build cost of the doubling, which retains the prefix.
    for count in [1000_usize, 100_000, 1_000_000] {
        let arguments = [
            Term::Integer(i64::try_from(count).unwrap()),
            Term::binary(b"a".to_vec()),
        ];
        let best = best_of(|| timed(&program, "repeat", &arguments));
        println!("APPEND-BUILD count={count} total={best:.0} ns");
    }

    // The match throughput on a doubling subject of 1000000 bytes, warm.
    let subject = execute(
        &program,
        "repeat",
        &[Term::Integer(1_000_000), Term::binary(b"a".to_vec())],
    );
    let needle = Term::binary(b"b".to_vec());
    // Warm the contiguous bytes of the subject, so the rounds below measure
    // the scan and not the first join.
    let _ = execute(
        &program,
        "match_many",
        &[subject.clone(), needle.clone(), Term::Integer(1)],
    );
    for rounds in [1_usize, 100, 1000] {
        let arguments = [
            subject.clone(),
            needle.clone(),
            Term::Integer(i64::try_from(rounds).unwrap()),
        ];
        let best = best_of(|| timed(&program, "match_many", &arguments));
        println!("MATCH-WARM rounds={rounds} total={best:.0} ns");
    }

    // The first match of a fresh append-built subject. The build is outside
    // the timer, so the number holds the first read of the representation
    // and the scan.
    for count in [100_000_usize, 1_000_000] {
        let best = best_of(|| {
            let fresh = execute(
                &program,
                "repeat",
                &[
                    Term::Integer(i64::try_from(count).unwrap()),
                    Term::binary(b"a".to_vec()),
                ],
            );
            timed(
                &program,
                "match_many",
                &[fresh, needle.clone(), Term::Integer(1)],
            )
        });
        println!("MATCH-FIRST count={count} total={best:.0} ns");
    }

    // The build and one match together, which the doubling consumer does.
    for count in [100_000_usize, 1_000_000] {
        let arguments = [
            Term::Integer(i64::try_from(count).unwrap()),
            Term::binary(b"b".to_vec()),
            Term::Integer(1),
        ];
        let best = best_of(|| timed(&program, "match_built", &arguments));
        println!("MATCH-BUILD-MATCH count={count} total={best:.0} ns");
    }

    // The doubling build of 100000 bytes and of 1000000 bytes: the size.
    for count in [100_000_usize, 1_000_000] {
        let out = execute(
            &program,
            "repeat",
            &[
                Term::Integer(i64::try_from(count).unwrap()),
                Term::binary(b"a".to_vec()),
            ],
        );
        println!(
            "APPEND-SIZE count={count} bytes={}",
            out.clone().binary_len()
        );
    }
}

/// The append of a literal string and of an integer segment to a large
/// binary. The string segment shares its array and covers the `Bytes` arm of
/// the chunked path; the integer segment writes bits, so the append takes the
/// writer path and covers the `None` result of the classification.
#[test]
fn appends_a_string_and_an_integer_segment_to_a_large_binary() {
    let program = BeamProgram::load(&[BINARY_APPEND_PROBE]).unwrap();
    let prefix = Term::binary(vec![b'a'; 5000]);

    let with_string = execute(&program, "append_string", std::slice::from_ref(&prefix));
    let Term::Binary(bytes) = &with_string else {
        panic!("not a binary: {with_string:?}");
    };
    assert_eq!(bytes.bytes().unwrap().len(), 5004);
    assert_eq!(
        bytes.bytes().unwrap(),
        &[&[b'a'; 5000][..], b"tail"].concat()[..]
    );

    let with_integer = execute(&program, "append_integer", &[prefix, Term::Integer(7)]);
    let Term::Binary(bytes) = &with_integer else {
        panic!("not a binary: {with_integer:?}");
    };
    assert_eq!(bytes.bytes().unwrap().len(), 5001);
    assert_eq!(bytes.bytes().unwrap()[5000], 7);
    assert_eq!(&bytes.bytes().unwrap()[..5000], &[b'a'; 5000][..]);
}

trait BinaryLen {
    fn binary_len(self) -> usize;
}

impl BinaryLen for Term {
    fn binary_len(self) -> usize {
        match &self {
            Term::Binary(bytes) => bytes.bytes().unwrap().len(),
            other => panic!("not a binary: {other:?}"),
        }
    }
}

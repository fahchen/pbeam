//! The `select_val` dispatch measurement of issue #545. The interpreter has
//! two search arms: a linear walk over the encoded order for a list of at
//! most 10 choices, and a binary search over a list that the loader sorted
//! by the ERTS term order above that bound.
//!
//! The probe is `fixtures/dispatch/select_val_probe.erl`. `ordered_loop/2`
//! runs the 256-choice `select_val` of `bucket/1` one time for each
//! iteration, and `linear_loop/2` runs the 2-choice `select_val` of
//! `digit/1`. Each probe runs on PBeam and on the pinned ERTS, and the test
//! asserts that the two results are equal.
//!
//! Run the release measurement with
//! `cargo test --locked --release --test select_val records_the_select_val_cost -- --nocapture --test-threads=1`.
//! The check gate runs the same tests under the test profile, which measures
//! slower numbers but proves that the probes and the ERTS reference run.

use std::{
    collections::HashMap,
    hint::black_box,
    path::PathBuf,
    process::Command,
    time::{Duration, Instant},
};

use pbeam_runtime::{BeamProgram, HostTerm, Term};

const SELECT_VAL_PROBE: &[u8] = include_bytes!("../../fixtures/dispatch/select_val_probe.beam");

/// The number of timed runs of each probe. The report gives the minimum of
/// the runs, as the other dispatch measurements do.
const RUNS: usize = 9;

/// The loop length of the probe. The added cost of one instruction is stable
/// at this length.
const LENGTH: usize = 200_000;

/// The number of choices of the `select_val` of `bucket/1`, above
/// `SELECT_VALUE_LINEAR_LIMIT`, which selects the ordered form. The count is
/// in the more-than-64 bucket of the measured distribution of issue #545.
const ORDERED_CHOICES: usize = 256;

/// The number of choices of the `select_val` of `digit/1`, below the bound
/// of the ordered form, which keeps the linear form.
const LINEAR_CHOICES: usize = 2;

/// The path of the fixture, for the ERTS runs and the disassembly check.
///
/// # Examples
///
/// ```rust,ignore
/// assert!(probe_path().ends_with("fixtures/dispatch/select_val_probe.beam"));
/// ```
fn probe_path() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("fixtures/dispatch/select_val_probe.beam")
}

/// Measure one probe on PBeam and return the minimum of [`RUNS`] runs. The
/// result of each run goes to [`black_box`], so the compiler cannot remove
/// the work that the timer measures.
///
/// # Examples
///
/// ```rust,ignore
/// let best = measure_pbeam(&program, "ordered_loop", &[program.host_term(Term::Integer(200_000)), program.host_term(Term::Integer(0))]);
/// assert!(best > Duration::ZERO);
/// ```
fn measure_pbeam(program: &BeamProgram, function: &str, args: &[HostTerm]) -> Duration {
    let mut best = Duration::MAX;
    for _ in 0..RUNS {
        let start = Instant::now();
        let result = program.execute("select_val_probe", function, args).unwrap();
        let elapsed = start.elapsed();
        black_box(&result);
        best = best.min(elapsed);
    }
    best
}

/// Return the number of `select_val` choices of each function of the fixture
/// that holds one, from `beam_disasm`. The counts prove that each loop runs
/// the shape that the benchmark claims to measure.
///
/// # Examples
///
/// ```rust,ignore
/// let counts = select_val_choice_counts();
/// assert_eq!(counts["bucket"], 256);
/// ```
fn select_val_choice_counts() -> HashMap<String, usize> {
    let expression = format!(
        "{{beam_file, _, _, _, _, Functions}} = beam_disasm:file(\"{}\"), \
         Counts = [{{Name, length(List) div 2}} \
                   || {{function, Name, _, _, Instructions}} <- Functions, \
                      {{select_val, _, _, {{list, List}}}} <- Instructions], \
         [io:format(\"select_val ~p ~b~n\", [Name, Count]) || {{Name, Count}} <- Counts], \
         halt().",
        probe_path().display()
    );
    let output = Command::new("erl")
        .args(["+S", "1", "-noshell", "-eval", &expression])
        .output()
        .unwrap();
    assert!(
        output.status.success() && output.stderr.is_empty(),
        "{output:?}"
    );
    String::from_utf8(output.stdout)
        .unwrap()
        .lines()
        .filter_map(|line| {
            let rest = line.strip_prefix("select_val ")?;
            let (name, count) = rest.split_once(' ')?;
            Some((name.to_owned(), count.parse().unwrap()))
        })
        .collect()
}

/// The Erlang expression of the ERTS reference run. It prints one
/// `result <key> <value>` line for each probe and each `bucket/1` value. The
/// keys match the PBeam runs, so the two runs measure the same work.
///
/// # Examples
///
/// ```rust,ignore
/// assert!(erts_expression().contains("select_val_probe"));
/// ```
fn erts_expression() -> String {
    let mut expression = String::from(
        "{module, M} = code:ensure_loaded(select_val_probe), \
         jit = erlang:system_info(emu_flavor), \
         Run = fun(F) -> lists:min([begin \
             S = erlang:monotonic_time(nanosecond), _ = F(), \
             erlang:monotonic_time(nanosecond) - S end || _ <- lists:seq(1, 9)]) end, ",
    );
    expression.push_str(&format!(
        "io:format(\"result ordered_loop ~w~n\", [M:ordered_loop({LENGTH}, 0)]), \
         io:format(\"result linear_loop ~w~n\", [M:linear_loop({LENGTH}, 0)]), "
    ));
    for value in -1..=257 {
        expression.push_str(&format!(
            "io:format(\"result bucket_{value} ~w~n\", [M:bucket({value})]), "
        ));
    }
    expression.push_str(&format!(
        "io:format(\"time ordered_loop ~b~n\", [Run(fun() -> M:ordered_loop({LENGTH}, 0) end)]), \
         io:format(\"time linear_loop ~b~n\", [Run(fun() -> M:linear_loop({LENGTH}, 0) end)]), \
         halt()."
    ));
    expression
}

/// Run the ERTS reference of [`erts_expression`] with one scheduler and
/// return its output lines.
///
/// # Examples
///
/// ```rust,ignore
/// let lines = run_erts();
/// assert!(lines.iter().any(|line| line.starts_with("result ordered_loop ")));
/// ```
fn run_erts() -> Vec<String> {
    let directory = probe_path().parent().unwrap().to_path_buf();
    let output = Command::new("erl")
        .args([
            "+S",
            "1",
            "-noshell",
            "-pa",
            directory.to_str().unwrap(),
            "-eval",
            &erts_expression(),
        ])
        .output()
        .unwrap();
    assert!(
        output.status.success() && output.stderr.is_empty(),
        "{output:?}"
    );
    String::from_utf8(output.stdout)
        .unwrap()
        .lines()
        .map(str::to_owned)
        .collect()
}

/// Return the value of the `result <key> <value>` line of the ERTS run, as
/// the integer that every probe returns.
///
/// # Examples
///
/// ```rust,ignore
/// let lines = run_erts();
/// assert_eq!(erts_result(&lines, "bucket_0"), 0);
/// ```
fn erts_result(lines: &[String], key: &str) -> i64 {
    lines
        .iter()
        .find_map(|line| line.strip_prefix(&format!("result {key} ")))
        .unwrap_or_else(|| panic!("the ERTS run has no result for {key}"))
        .parse()
        .unwrap()
}

/// Return the nanoseconds of the `time <key> <ns>` line of the ERTS run.
///
/// # Examples
///
/// ```rust,ignore
/// let lines = run_erts();
/// assert!(erts_nanos(&lines, "ordered_loop") > 0);
/// ```
fn erts_nanos(lines: &[String], key: &str) -> u128 {
    lines
        .iter()
        .find_map(|line| line.strip_prefix(&format!("time {key} ")))
        .unwrap_or_else(|| panic!("the ERTS run has no time for {key}"))
        .parse()
        .unwrap()
}

/// Check that each `select_val` of the fixture keeps the choice count that
/// the measurement claims: 256 for `bucket/1`, which selects the ordered
/// form, and 2 for `digit/1`, which keeps the linear form. A change of the
/// compiler or of the fixture fails here, because the numbers would then
/// measure other work.
#[test]
fn keeps_the_measured_select_val_shapes() {
    let counts = select_val_choice_counts();
    assert_eq!(
        counts.get("bucket"),
        Some(&ORDERED_CHOICES),
        "the ordered probe must hold {ORDERED_CHOICES} choices"
    );
    assert_eq!(
        counts.get("digit"),
        Some(&LINEAR_CHOICES),
        "the linear probe must hold {LINEAR_CHOICES} choices"
    );
    assert_eq!(
        counts.len(),
        2,
        "the fixture holds one select_val in each of bucket/1 and digit/1"
    );
}

/// Check that each probe returns the same value on PBeam and on the pinned
/// ERTS, for every value that `bucket/1` meets, including a value that takes
/// its fail label.
#[test]
fn checks_the_select_val_results_against_erts() {
    let program = BeamProgram::load(&[SELECT_VAL_PROBE]).unwrap();
    let lines = run_erts();

    let counter = program.host_term(Term::Integer(LENGTH as i64));
    let zero = program.host_term(Term::Integer(0));
    for function in ["ordered_loop", "linear_loop"] {
        let args = [counter.clone(), zero.clone()];
        let result = program
            .execute("select_val_probe", function, &args)
            .unwrap();
        assert_eq!(
            result.term,
            Term::Integer(erts_result(&lines, function)),
            "PBeam and ERTS differ for {function}"
        );
    }

    for value in -1i64..=257 {
        let args = [program.host_term(Term::Integer(value))];
        let result = program
            .execute("select_val_probe", "bucket", &args)
            .unwrap();
        assert_eq!(
            result.term,
            Term::Integer(erts_result(&lines, &format!("bucket_{value}"))),
            "PBeam and ERTS differ for bucket({value})"
        );
    }
}

/// Record the cost of the ordered form and of the linear form: run each loop
/// on PBeam and on the pinned ERTS, and print the numbers as Markdown. The
/// minimum of [`RUNS`] runs is the reported number.
#[test]
fn records_the_select_val_cost() {
    let program = BeamProgram::load(&[SELECT_VAL_PROBE]).unwrap();
    let lines = run_erts();
    let counter = program.host_term(Term::Integer(LENGTH as i64));
    let zero = program.host_term(Term::Integer(0));
    let args = [counter, zero];

    println!("Select value dispatch: minimum of {RUNS} runs of each probe.");
    println!();
    println!("| shape | choices | PBeam ns for each iteration | ERTS ns for each iteration |");
    println!("| --- | --- | --- | --- |");
    for function in ["ordered_loop", "linear_loop"] {
        let result = program
            .execute("select_val_probe", function, &args)
            .unwrap();
        assert_eq!(
            result.term,
            Term::Integer(erts_result(&lines, function)),
            "PBeam and ERTS differ for {function}"
        );
        let pbeam = measure_pbeam(&program, function, &args);
        let erts = erts_nanos(&lines, function);
        println!(
            "| `select_val_probe:{function}/2` | {} | {:.3} | {:.2} |",
            if function == "ordered_loop" {
                ORDERED_CHOICES
            } else {
                LINEAR_CHOICES
            },
            pbeam.as_nanos() as f64 / LENGTH as f64,
            erts as f64 / LENGTH as f64,
        );
    }
    println!();
}

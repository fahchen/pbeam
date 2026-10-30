//! The comparison of the PBeam runtime with the pinned ERTS on the five
//! scenarios of `benchmark/`.
//!
//! Run `mise run bench-compare`, or `bash benchmark/run.sh`. The test
//! compiles `benchmark/*.erl` with the pinned `erlc` into
//! `target/benchmark/beams/`, runs each scenario on PBeam through the library
//! API and on one scheduler of the pinned ERTS with `erl +S 1 -noshell`, and
//! prints the table of the minimum of [`RUNS`] runs of each scenario.
//!
//! The two runs measure the same guest work: the ERTS run times the call in
//! the guest with `erlang:monotonic_time/1`, and this test times the call of
//! the runtime from the host. The start of the runtime and the start of ERTS
//! are part of neither number, so the ratio is the cost of the work and not
//! the cost of the boot.
//!
//! The check gate runs the test with [`RUNS`] and [`Scenario::count`] scaled
//! down, as `tests/integration/dispatch_baseline.rs` scales its probes, so the
//! gate stays short and still proves that the two runtimes return the same
//! result for each scenario.
#![expect(
    clippy::unwrap_used,
    reason = "test helpers report a failure by a panic"
)]

use std::{
    ffi::OsStr,
    fmt::Write as _,
    hint::black_box,
    path::{Path, PathBuf},
    process::Command,
    time::{Duration, Instant},
};

use pbeam_runtime::{AtomTable, BeamProgram, HostTerm, Term};

/// The number of timed runs of each scenario. The report gives the minimum,
/// which the other processes of the host disturb the least. The check gate
/// runs two rounds, because it needs the results and it must stay short.
const RUNS: usize = if cfg!(debug_assertions) { 2 } else { 9 };

/// One scenario of the benchmark. Its module exports `function/1`, which
/// takes the iteration count of the scenario and returns an integer or `ok`.
struct Scenario {
    /// The name of the fixture module in the beam directory.
    module: &'static str,
    /// The exported function of the module that takes the count.
    function: &'static str,
    /// The title of the scenario in the report.
    title: &'static str,
    /// The name of one unit of work in the report.
    unit: &'static str,
    /// The iteration count of one release run.
    count: usize,
}

impl Scenario {
    /// Return the count of one run. The recorded count applies to the release
    /// profile. The check gate runs one twentieth of it, so the debug
    /// interpreter runs each scenario for about the same time as the release
    /// interpreter.
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// assert!(SCENARIOS[0].count() > 0);
    /// ```
    fn count(&self) -> usize {
        if cfg!(debug_assertions) {
            (self.count / 20).max(1)
        } else {
            self.count
        }
    }
}

/// The five scenarios of `benchmark/`. The counts keep one release run below
/// a few hundred milliseconds, so the whole report stays short.
const SCENARIOS: [Scenario; 5] = [
    Scenario {
        module: "bench_arith",
        function: "loop",
        title: "Integer arithmetic loop",
        unit: "iteration",
        count: 500_000,
    },
    Scenario {
        module: "bench_spawn",
        function: "rounds",
        title: "Spawn and message round trip",
        unit: "round trip",
        count: 20_000,
    },
    Scenario {
        module: "bench_list",
        function: "build_sum",
        title: "List build and sum",
        unit: "list cell",
        count: 400_000,
    },
    Scenario {
        module: "bench_map",
        function: "build_lookup",
        title: "Map build and lookup",
        unit: "key",
        count: 100_000,
    },
    Scenario {
        module: "bench_ets",
        function: "insert_lookup",
        title: "Table insert and lookup",
        unit: "object",
        count: 100_000,
    },
];

/// Return the directory of the compiled fixtures that the test writes.
///
/// # Examples
///
/// ```rust,ignore
/// assert!(beam_directory().ends_with("target/benchmark/beams"));
/// ```
fn beam_directory() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("target/benchmark/beams")
}

/// Compile every `benchmark/*.erl` source with the pinned `erlc` into
/// `directory`. The runner does not compile the sources, so the beam files
/// always come from the sources of the current commit.
///
/// # Examples
///
/// ```rust,ignore
/// let modules = compile_scenarios(&beam_directory());
/// assert!(modules.contains(&"bench_arith".to_owned()));
/// ```
fn compile_scenarios(directory: &Path) -> Vec<String> {
    let source_directory = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("benchmark");
    let mut sources = std::fs::read_dir(&source_directory)
        .unwrap_or_else(|error| panic!("cannot read {}: {error}", source_directory.display()))
        .map(|entry| entry.unwrap().path())
        .filter(|path| path.extension() == Some(OsStr::new("erl")))
        .collect::<Vec<_>>();
    sources.sort();
    std::fs::create_dir_all(directory)
        .unwrap_or_else(|error| panic!("cannot create {}: {error}", directory.display()));
    let output = Command::new("erlc")
        .arg("+deterministic")
        .arg("-o")
        .arg(directory)
        .args(&sources)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "erlc failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    sources
        .iter()
        .map(|path| path.file_stem().unwrap().to_string_lossy().into_owned())
        .collect()
}

/// Read the BEAM file of one scenario module from `directory`.
///
/// # Examples
///
/// ```rust,ignore
/// let bytes = read_beam(&beam_directory(), "bench_arith");
/// assert!(!bytes.is_empty());
/// ```
fn read_beam(directory: &Path, module: &str) -> Vec<u8> {
    let path = directory.join(format!("{module}.beam"));
    std::fs::read(&path).unwrap_or_else(|error| panic!("cannot read {}: {error}", path.display()))
}

/// Return the arguments of one scenario for the PBeam run: the iteration
/// count as an integer term.
///
/// # Examples
///
/// ```rust,ignore
/// let program = BeamProgram::load(&[&read_beam(&beam_directory(), "bench_arith")]).unwrap();
/// assert_eq!(arguments(&program, &SCENARIOS[0]).len(), 1);
/// ```
fn arguments(program: &BeamProgram, scenario: &Scenario) -> Vec<HostTerm> {
    vec![program.host_term(Term::Integer(
        i64::try_from(scenario.count()).expect("the scenario count fits in i64"),
    ))]
}

/// Return a result term as `io:format("~w")` prints it, for the integers and
/// atoms of the scenario results. The names of atoms come from `atoms`, the
/// table that owns the term.
///
/// # Examples
///
/// ```rust,ignore
/// assert_eq!(text(&Term::Integer(3), program.atoms()), "3");
/// ```
fn text(term: &Term, atoms: &AtomTable) -> String {
    match term {
        Term::Atom(atom) => atoms.name_of(atom).to_owned(),
        Term::Integer(value) => value.to_string(),
        other => panic!("the scenario returned an unexpected term {other:?}"),
    }
}

/// Run one scenario once and return its result as [`text`] prints it.
///
/// # Examples
///
/// ```rust,ignore
/// assert_eq!(result_text(&program, &SCENARIOS[1]), "ok");
/// ```
fn result_text(program: &BeamProgram, scenario: &Scenario) -> String {
    let arguments = arguments(program, scenario);
    let result = program
        .execute(scenario.module, scenario.function, &arguments)
        .unwrap();
    text(&result.term, result.atoms())
}

/// Measure one scenario on PBeam and return the minimum of [`RUNS`] runs.
/// Each result goes to [`black_box`], so the compiler cannot remove the work
/// that the timer measures.
///
/// # Examples
///
/// ```rust,ignore
/// assert!(measure_pbeam(&program, &SCENARIOS[0]) > Duration::ZERO);
/// ```
fn measure_pbeam(program: &BeamProgram, scenario: &Scenario) -> Duration {
    let arguments = arguments(program, scenario);
    let mut best = Duration::MAX;
    for _ in 0..RUNS {
        let start = Instant::now();
        let result = program
            .execute(scenario.module, scenario.function, &arguments)
            .unwrap();
        let elapsed = start.elapsed();
        black_box(&result);
        best = best.min(elapsed);
    }
    best
}

/// Return the Erlang expression of the ERTS reference run. It loads the
/// scenario modules, prints the ERTS version and flavor, runs each scenario
/// once for its result, then times it [`RUNS`] times inside the guest, and
/// prints one `result <module> <value>` line and one `time <module> <ns>`
/// line for each scenario.
///
/// # Examples
///
/// ```rust,ignore
/// assert!(erts_expression().contains("bench_arith:loop("));
/// ```
fn erts_expression() -> String {
    let mut expression = String::from(
        "io:format(\"erts OTP ~s, ERTS ~s, ~s, schedulers ~b~n\", \
         [erlang:system_info(otp_release), erlang:system_info(version), \
          erlang:system_info(emu_flavor), erlang:system_info(schedulers)]), ",
    );
    for scenario in SCENARIOS {
        let _ = write!(
            expression,
            "{{module, _}} = code:ensure_loaded({}), ",
            scenario.module
        );
    }
    let _ = write!(
        expression,
        "Run = fun(F) -> lists:min([begin \
         S = erlang:monotonic_time(nanosecond), _ = F(), \
         erlang:monotonic_time(nanosecond) - S end || _ <- lists:seq(1, {RUNS})]) end, "
    );
    for scenario in SCENARIOS {
        let _ = write!(
            expression,
            "io:format(\"result {} ~w~n\", [{}:{}({})]), ",
            scenario.module,
            scenario.module,
            scenario.function,
            scenario.count()
        );
    }
    for scenario in SCENARIOS {
        let _ = write!(
            expression,
            "io:format(\"time {} ~b~n\", [Run(fun() -> {}:{}({}) end)]), ",
            scenario.module,
            scenario.module,
            scenario.function,
            scenario.count()
        );
    }
    expression.push_str("halt().");
    expression
}

/// Run the ERTS reference of [`erts_expression`] with one scheduler and the
/// scenario modules on the code path, and return its output lines. Panic when
/// the run fails or writes to stderr.
///
/// # Examples
///
/// ```rust,ignore
/// let lines = run_erts(&beam_directory());
/// assert!(lines.iter().any(|line| line.starts_with("time bench_arith ")));
/// ```
fn run_erts(directory: &Path) -> Vec<String> {
    let output = Command::new("erl")
        .args([OsStr::new("+S"), OsStr::new("1"), OsStr::new("-noshell")])
        .arg("-pa")
        .arg(directory)
        .args([OsStr::new("-eval"), OsStr::new(&erts_expression())])
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

/// Return the number of nanoseconds of the `time <module> <ns>` line of one
/// scenario in the ERTS output.
///
/// # Examples
///
/// ```rust,ignore
/// let lines = run_erts(&beam_directory());
/// assert!(erts_nanos(&lines, "bench_arith") > 0);
/// ```
fn erts_nanos(lines: &[String], module: &str) -> u128 {
    lines
        .iter()
        .find_map(|line| line.strip_prefix(&format!("time {module} ")))
        .unwrap_or_else(|| panic!("the ERTS run has no time for {module}"))
        .parse()
        .unwrap()
}

/// Return the result of the `result <module> <value>` line of one scenario in
/// the ERTS output.
///
/// # Examples
///
/// ```rust,ignore
/// let lines = run_erts(&beam_directory());
/// assert!(!erts_result(&lines, "bench_arith").is_empty());
/// ```
fn erts_result(lines: &[String], module: &str) -> String {
    lines
        .iter()
        .find_map(|line| line.strip_prefix(&format!("result {module} ")))
        .unwrap_or_else(|| panic!("the ERTS run has no result for {module}"))
        .to_owned()
}

/// Compile the scenarios, run each of them on PBeam and on the pinned ERTS,
/// check the results, and print the comparison as a Markdown table.
#[test]
#[expect(
    clippy::as_conversions,
    clippy::cast_precision_loss,
    reason = "the report converts nanosecond and unit counts to f64 for a ratio"
)]
fn compares_the_runtime_with_the_pinned_erts() {
    let directory = beam_directory();
    let modules = compile_scenarios(&directory);
    for scenario in SCENARIOS {
        assert!(
            modules.contains(&scenario.module.to_owned()),
            "benchmark/ has no source for {}",
            scenario.module
        );
    }
    let beams = SCENARIOS
        .iter()
        .map(|scenario| read_beam(&directory, scenario.module))
        .collect::<Vec<_>>();
    let slices = beams.iter().map(Vec::as_slice).collect::<Vec<_>>();
    let program = BeamProgram::load(&slices).unwrap();

    let lines = run_erts(&directory);
    for line in lines.iter().filter(|line| line.starts_with("erts ")) {
        println!("{line}");
    }
    println!("Minimum of {RUNS} runs of each scenario.");
    println!();
    println!(
        "| scenario | count | unit | ERTS ns for each unit | PBeam ns for each unit | PBeam / ERTS | ERTS ns for the run | PBeam ns for the run |"
    );
    println!("| --- | --- | --- | --- | --- | --- | --- | --- |");

    for scenario in SCENARIOS {
        let pbeam_result = result_text(&program, &scenario);
        assert_eq!(
            pbeam_result,
            erts_result(&lines, scenario.module),
            "PBeam and ERTS differ for {}",
            scenario.title
        );
        let pbeam = measure_pbeam(&program, &scenario).as_nanos();
        let erts = erts_nanos(&lines, scenario.module);
        let units = scenario.count() as f64;
        println!(
            "| {} | {} | {} | {:.3} | {:.3} | {:.2} | {} | {} |",
            scenario.title,
            scenario.count(),
            scenario.unit,
            erts as f64 / units,
            pbeam as f64 / units,
            pbeam as f64 / erts as f64,
            erts,
            pbeam
        );
    }
}

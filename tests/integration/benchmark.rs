//! The comparison of the PBeam runtime with the pinned ERTS on the five
//! scenarios of `benchmark/`.
//!
//! Run `mise run bench-compare` or `bash benchmark/run.sh`. The test
//! compiles `benchmark/*.erl` with the pinned `erlc` into
//! `target/benchmark/beams/`, runs each scenario on PBeam through the library
//! API and on one scheduler of the pinned ERTS with `erl +S 1 -noshell`, and
//! prints the table of the minimum of [`RUNS`] runs of each scenario.
//!
//! The two runs measure the same guest work in the same way: on each runtime,
//! `bench_run:measure/4` runs the scenario [`RUNS`] times in one process and
//! times each run in the guest with `erlang:monotonic_time(nanosecond)`. The
//! start of the runtime, of ERTS, and of the process is part of neither
//! number, so the ratio is the cost of the work and not the cost of the boot.
//!
//! A separate correctness test compares the scenario results on both runtimes.
#![expect(
    clippy::unwrap_used,
    reason = "test helpers report a failure by a panic"
)]

use std::{
    ffi::OsStr,
    fmt::Write as _,
    path::{Path, PathBuf},
    process::Command,
};

use pbeam_runtime::{AtomTable, BeamProgram, HostTerm, Term};

/// The number of timed runs of each scenario. The report gives the minimum,
/// which the other processes of the host disturb the least. The test profile
/// uses two runs.
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
    /// profile. The test profile uses one twentieth of it, so the debug
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
/// `directory`. The test compiles the sources on each run, so the beam files
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

/// The module and function of the timing driver of `benchmark/bench_run.erl`.
const DRIVER: (&str, &str) = ("bench_run", "measure");

/// Return the arguments of `bench_run:measure/4` for one scenario on PBeam:
/// the module and function atoms, the iteration count, and [`RUNS`].
///
/// # Examples
///
/// ```rust,ignore
/// assert_eq!(arguments(&program, &SCENARIOS[0]).len(), 4);
/// ```
fn arguments(program: &BeamProgram, scenario: &Scenario) -> Vec<HostTerm> {
    let atom = |name: &str| {
        program.host_term(Term::Atom(
            program.atom(name).expect("a loaded module holds the atom"),
        ))
    };
    let integer = |value: usize| {
        program.host_term(Term::Integer(
            i64::try_from(value).expect("the count fits in i64"),
        ))
    };
    vec![
        atom(scenario.module),
        atom(scenario.function),
        integer(scenario.count()),
        integer(RUNS),
    ]
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

/// Run `bench_run:measure/4` for one scenario on PBeam and return the result
/// of the scenario as [`text`] prints it and the minimum nanoseconds of one
/// run.
///
/// # Examples
///
/// ```rust,ignore
/// let (result, nanos) = measure_pbeam(&program, &SCENARIOS[1]);
/// assert_eq!(result, "ok");
/// ```
fn measure_pbeam(program: &BeamProgram, scenario: &Scenario) -> (String, u128) {
    let (module, function) = DRIVER;
    let measured = program
        .execute(module, function, &arguments(program, scenario))
        .unwrap();
    let Some([result, Term::Integer(nanos)]) = measured.term.tuple_elements() else {
        panic!("bench_run:measure/4 returned {:?}", measured.term);
    };
    (
        text(result, measured.atoms()),
        u128::try_from(*nanos).expect("a run time is not negative"),
    )
}

/// Return the Erlang expression of the ERTS reference run. It loads the
/// scenario modules, prints the ERTS version and flavor, and prints one
/// `measure <module> <result> <ns>` line for each scenario from
/// `bench_run:measure/4`.
///
/// # Examples
///
/// ```rust,ignore
/// assert!(erts_expression().contains("P(bench_arith, loop, "));
/// ```
fn erts_expression() -> String {
    let (module, function) = DRIVER;
    let mut expression = String::from(
        "io:format(\"erts OTP ~s, ERTS ~s, ~s, schedulers ~b~n\", \
         [erlang:system_info(otp_release), erlang:system_info(version), \
          erlang:system_info(emu_flavor), erlang:system_info(schedulers)]), ",
    );
    let _ = write!(expression, "{{module, _}} = code:ensure_loaded({module}), ");
    for scenario in SCENARIOS {
        let _ = write!(
            expression,
            "{{module, _}} = code:ensure_loaded({}), ",
            scenario.module
        );
    }
    let _ = write!(
        expression,
        "P = fun(M, F, C) -> {{R, T}} = {module}:{function}(M, F, C, {RUNS}), \
         io:format(\"measure ~s ~w ~b~n\", [M, R, T]) end, "
    );
    for scenario in SCENARIOS {
        let _ = write!(
            expression,
            "P({}, {}, {}), ",
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
/// assert!(lines.iter().any(|line| line.starts_with("measure bench_arith ")));
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

/// Return the result and the minimum nanoseconds of the
/// `measure <module> <result> <ns>` line of one scenario in the ERTS output.
///
/// # Examples
///
/// ```rust,ignore
/// let lines = run_erts(&beam_directory());
/// assert!(erts_measure(&lines, "bench_arith").1 > 0);
/// ```
fn erts_measure(lines: &[String], module: &str) -> (String, u128) {
    let line = lines
        .iter()
        .find_map(|line| line.strip_prefix(&format!("measure {module} ")))
        .unwrap_or_else(|| panic!("the ERTS run has no measure line for {module}"));
    let (result, nanos) = line
        .rsplit_once(' ')
        .unwrap_or_else(|| panic!("the ERTS measure line of {module} has no time"));
    (result.to_owned(), nanos.parse().unwrap())
}

#[test]
fn scenarios_return_the_pinned_erts_results() {
    let directory = beam_directory().with_file_name(format!("correctness-{}", std::process::id()));
    compile_scenarios(&directory);
    let beams: Vec<_> = SCENARIOS
        .iter()
        .map(|scenario| read_beam(&directory, scenario.module))
        .collect();
    let slices: Vec<_> = beams.iter().map(Vec::as_slice).collect();
    let program = BeamProgram::load(&slices).unwrap();
    let mut expression = String::new();
    for scenario in SCENARIOS {
        write!(
            expression,
            "io:format(\"~w~n\", [{}:{}(40)]), ",
            scenario.module, scenario.function
        )
        .unwrap();
    }
    expression.push_str("halt().");
    let reference = Command::new("erl")
        .args(["+S", "1", "-noshell", "-pa"])
        .arg(&directory)
        .args(["-eval", &expression])
        .output()
        .unwrap();
    assert!(
        reference.status.success() && reference.stderr.is_empty(),
        "ERTS failed: {reference:?}"
    );
    let output = String::from_utf8(reference.stdout).unwrap();
    let lines: Vec<_> = output.lines().collect();
    assert_eq!(lines.len(), SCENARIOS.len());
    for (scenario, expected) in SCENARIOS.iter().zip(lines) {
        let argument = program.host_term(Term::Integer(40));
        let result = program
            .execute(scenario.module, scenario.function, &[argument])
            .unwrap();
        assert_eq!(
            text(&result.term, result.atoms()),
            expected,
            "{}",
            scenario.title
        );
    }
    std::fs::remove_dir_all(directory).unwrap();
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
    let names = std::iter::once(DRIVER.0)
        .chain(SCENARIOS.iter().map(|scenario| scenario.module))
        .collect::<Vec<_>>();
    for name in &names {
        assert!(
            modules.iter().any(|module| module == name),
            "benchmark/ has no source for {name}"
        );
    }
    let beams = names
        .iter()
        .map(|name| read_beam(&directory, name))
        .collect::<Vec<_>>();
    let slices = beams.iter().map(Vec::as_slice).collect::<Vec<_>>();
    let program = BeamProgram::load(&slices).unwrap();

    let lines = run_erts(&directory);
    for line in lines.iter().filter(|line| line.starts_with("erts ")) {
        println!("{line}");
    }
    println!("Minimum of {RUNS} runs of each scenario, timed in the guest.");
    println!();
    println!(
        "| scenario | count | unit | ERTS ns for each unit | PBeam ns for each unit | PBeam / ERTS | ERTS ns for the run | PBeam ns for the run |"
    );
    println!("| --- | --- | --- | --- | --- | --- | --- | --- |");

    for scenario in SCENARIOS {
        let (pbeam_result, pbeam) = measure_pbeam(&program, &scenario);
        let (erts_result, erts) = erts_measure(&lines, scenario.module);
        assert_eq!(
            pbeam_result, erts_result,
            "PBeam and ERTS differ for {}",
            scenario.title
        );
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

//! The interpreter dispatch baseline of `docs/interpreter-dispatch-plan.md`.
//! The benchmark measures the floor loop (`test`, `gc_bif '-'`, and the
//! call) at 20000 and 200000 iterations, and the cost of one added
//! instruction on that loop. Each probe runs on PBeam and on the pinned
//! ERTS, and the report gives the minimum of [`RUNS`] runs of each probe.
//!
//! The probes are in `fixtures/dispatch/dispatch_probe.erl`. The test
//! consumes each result, and it checks the disassembly of each loop, so the
//! interpreter cannot remove the work that the benchmark measures.
//!
//! Run the release measurement with `mise run bench-dispatch`. The check
//! gate runs the same test under the test profile, which measures slower
//! numbers but proves that the probes and the ERTS reference run.

use std::{
    collections::HashMap,
    hint::black_box,
    path::PathBuf,
    process::Command,
    time::{Duration, Instant},
};

use pbeam_runtime::{BeamProgram, Term};

const DISPATCH_PROBE: &[u8] = include_bytes!("../../fixtures/dispatch/dispatch_probe.beam");

/// The number of timed runs of each probe. The report gives the minimum of
/// the runs, as `docs/interpreter-dispatch-plan.md` does.
const RUNS: usize = 9;

/// The floor-loop lengths of `docs/interpreter-dispatch-plan.md`.
const FLOOR_LENGTHS: [usize; 2] = [20_000, 200_000];

/// The length of the added-instruction runs. The added cost of one
/// instruction is stable at this length.
const DELTA_LENGTH: usize = 200_000;

/// The fixture functions of the disassembly check, in module order. The
/// module holds no other function that runs an instruction of a loop.
const LOOP_FUNCTIONS: [&str; 7] = [
    "floor_ext_loop",
    "floor_loop",
    "plus_loop",
    "tuple_size_loop",
    "element_loop",
    "hd_loop",
    "map_get_loop",
];

/// One probe of the benchmark: the fixture function, the instruction that
/// the function adds to the floor loop, and the loop length.
struct Probe {
    function: &'static str,
    instruction: &'static str,
    length: usize,
}

impl Probe {
    /// The unique key of the probe in the ERTS reference lines.
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// assert_eq!(probes()[0].key(), "floor_ext_loop-20000");
    /// ```
    fn key(&self) -> String {
        format!("{}-{}", self.function, self.length)
    }
}

/// The probe set: the two floor loops at both lengths, then the five
/// added-instruction probes at [`DELTA_LENGTH`]. The plan names `call_only`
/// for the floor loop and says that the loop "includes one external call",
/// so the benchmark measures both call shapes.
///
/// # Examples
///
/// ```rust,ignore
/// assert_eq!(probes()[0].function, "floor_ext_loop");
/// ```
fn probes() -> Vec<Probe> {
    let mut probes = Vec::new();
    for (function, instruction) in [
        (
            "floor_ext_loop",
            "the floor loop, with a module-qualified tail call (`call_ext_only`)",
        ),
        (
            "floor_loop",
            "the floor loop, with a local tail call (`call_only`)",
        ),
    ] {
        for &length in &FLOOR_LENGTHS {
            probes.push(Probe {
                function,
                instruction,
                length,
            });
        }
    }
    for (function, instruction) in [
        ("plus_loop", "`gc_bif '+'`"),
        ("tuple_size_loop", "the `bif` of `tuple_size/1`"),
        (
            "element_loop",
            "the `bif` of `element/2` (`get_tuple_element`)",
        ),
        ("hd_loop", "the `bif` of `hd/1` (`get_hd`)"),
        ("map_get_loop", "the `bif` of `map_get/2` on a 1000-key map"),
    ] {
        probes.push(Probe {
            function,
            instruction,
            length: DELTA_LENGTH,
        });
    }
    probes
}

/// The path of the fixture, for the ERTS runs and the disassembly check.
///
/// # Examples
///
/// ```rust,ignore
/// assert!(probe_path().ends_with("fixtures/dispatch/dispatch_probe.beam"));
/// ```
fn probe_path() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("fixtures/dispatch/dispatch_probe.beam")
}

/// The arguments of one probe for the PBeam run.
///
/// # Examples
///
/// ```rust,ignore
/// let args = arguments("floor_loop", 20_000, &tuple, &list, &map, &key);
/// assert_eq!(args.len(), 1);
/// ```
fn arguments(
    function: &str,
    length: usize,
    tuple: &Term,
    list: &Term,
    map: &Term,
    key: &Term,
) -> Vec<Term> {
    let counter = Term::Integer(length as i64);
    let zero = Term::Integer(0);
    match function {
        "floor_ext_loop" | "floor_loop" => vec![counter],
        "plus_loop" => vec![counter, zero],
        "tuple_size_loop" | "element_loop" => vec![counter, zero, tuple.clone()],
        "hd_loop" => vec![counter, zero, list.clone()],
        "map_get_loop" => vec![counter, zero, map.clone(), key.clone()],
        other => panic!("unknown probe function {other}"),
    }
}

/// The arguments of one probe as Erlang source for the ERTS run. The names
/// `Tuple`, `List`, `Map`, and `Key` are bound by [`erts_expression`].
///
/// # Examples
///
/// ```rust,ignore
/// assert_eq!(arguments_text("plus_loop", 20_000), "20000, 0");
/// ```
fn arguments_text(function: &str, length: usize) -> String {
    match function {
        "floor_ext_loop" | "floor_loop" => format!("{length}"),
        "plus_loop" => format!("{length}, 0"),
        "tuple_size_loop" | "element_loop" => format!("{length}, 0, Tuple"),
        "hd_loop" => format!("{length}, 0, List"),
        "map_get_loop" => format!("{length}, 0, Map, Key"),
        other => panic!("unknown probe function {other}"),
    }
}

/// The result of one probe on PBeam.
///
/// # Examples
///
/// ```rust,ignore
/// assert_eq!(expected(&program, "tuple_size_loop", 20_000), Term::Integer(3));
/// ```
fn expected(program: &BeamProgram, function: &str, length: usize) -> Term {
    match function {
        "floor_ext_loop" | "floor_loop" => Term::Integer(0),
        "plus_loop" => Term::Integer(length as i64),
        "tuple_size_loop" => Term::Integer(3),
        "element_loop" => Term::Atom(program.atom("b").unwrap()),
        "hd_loop" => Term::Atom(program.atom("a").unwrap()),
        "map_get_loop" => Term::Integer(500),
        other => panic!("unknown probe function {other}"),
    }
}

/// Return a term as `io:format("~w")` prints it, for the integers and atoms
/// of the probe results.
///
/// # Examples
///
/// ```rust,ignore
/// assert_eq!(text(&Term::Integer(3)), "3");
/// ```
fn text(term: &Term) -> String {
    match term {
        Term::Atom(atom) => atom.name().to_owned(),
        Term::Integer(value) => value.to_string(),
        other => panic!("unexpected term {other:?}"),
    }
}

/// The Erlang expression of the ERTS reference run. It runs each probe once
/// for its result, then [`RUNS`] times for its time in nanoseconds, and
/// prints one `result <key> <value>` line and one `time <key> <ns>` line for
/// each probe. The keys come from [`probes`], so the two runs measure the
/// same work.
///
/// # Examples
///
/// ```rust,ignore
/// assert!(erts_expression().contains("dispatch_probe"));
/// ```
fn erts_expression() -> String {
    let mut expression = String::from(
        "{module, M} = code:ensure_loaded(dispatch_probe), \
         jit = erlang:system_info(emu_flavor), \
         Map = M:map_1000(), Tuple = {a, b, c}, List = [a, b, c], Key = 500, \
         Run = fun(F) -> lists:min([begin \
             S = erlang:monotonic_time(nanosecond), _ = F(), \
             erlang:monotonic_time(nanosecond) - S end || _ <- lists:seq(1, 9)]) end, ",
    );
    for probe in probes() {
        expression.push_str(&format!(
            "io:format(\"result {} ~w~n\", [M:{}({})]), ",
            probe.key(),
            probe.function,
            arguments_text(probe.function, probe.length)
        ));
    }
    for probe in probes() {
        expression.push_str(&format!(
            "io:format(\"time {} ~b~n\", [Run(fun() -> M:{}({}) end)]), ",
            probe.key(),
            probe.function,
            arguments_text(probe.function, probe.length)
        ));
    }
    expression.push_str("halt().");
    expression
}

/// Measure one probe on PBeam and return the minimum of [`RUNS`] runs. The
/// result of each run goes to [`black_box`], so the compiler cannot remove
/// the work that the timer measures.
///
/// # Examples
///
/// ```rust,ignore
/// let best = measure_pbeam(&program, "floor_loop", &[Term::Integer(20_000)]);
/// assert!(best > Duration::ZERO);
/// ```
fn measure_pbeam(program: &BeamProgram, function: &str, args: &[Term]) -> Duration {
    let mut best = Duration::MAX;
    for _ in 0..RUNS {
        let start = Instant::now();
        let result = program.execute("dispatch_probe", function, args).unwrap();
        let elapsed = start.elapsed();
        black_box(&result);
        best = best.min(elapsed);
    }
    best
}

/// Run the ERTS reference of [`erts_expression`] with one scheduler and
/// return its output lines.
///
/// # Examples
///
/// ```rust,ignore
/// let lines = run_erts();
/// assert!(lines.iter().any(|line| line.starts_with("time floor_loop ")));
/// ```
fn run_erts() -> Vec<String> {
    let output = Command::new("erl")
        .args([
            "+S",
            "1",
            "-noshell",
            "-pa",
            probe_path().parent().unwrap().to_str().unwrap(),
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

/// Return the `beam_disasm` instructions of each function of
/// [`LOOP_FUNCTIONS`], without the `{line, _}` entries, as `mise run disasm`
/// prints them. The result proves that each loop holds the work that the
/// benchmark claims to measure.
///
/// # Examples
///
/// ```rust,ignore
/// let loops = disassemble();
/// assert!(loops["floor_loop"].contains(&"{call_only,1,{dispatch_probe,floor_loop,1}}".to_owned()));
/// ```
fn disassemble() -> HashMap<String, Vec<String>> {
    let expression = format!(
        "{{beam_file, _, _, _, _, Functions}} = beam_disasm:file(\"{}\"), \
         [begin \
             {{function, Name, _, _, Instructions}} = Function, \
             io:format(\"function ~p~n\", [Name]), \
             [io:format(\"~p~n\", [Instruction]) || Instruction <- Instructions, \
              not is_tuple(Instruction) orelse element(1, Instruction) =/= line] \
          end || Function <- Functions], halt().",
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
    let mut loops: HashMap<String, Vec<String>> = HashMap::new();
    let mut current: Option<String> = None;
    for line in String::from_utf8(output.stdout).unwrap().lines() {
        // The `~p` of a name atom is the name, for example `floor_loop`.
        if let Some(name) = line.strip_prefix("function ") {
            loops.insert(name.to_owned(), Vec::new());
            current = Some(name.to_owned());
        } else if let Some(name) = &current {
            loops.get_mut(name).unwrap().push(line.to_owned());
        }
    }
    for function in LOOP_FUNCTIONS {
        assert!(
            loops.contains_key(function),
            "{function} is not in the fixture"
        );
    }
    loops
}

/// Check that each loop holds the instructions that the benchmark measures:
/// the floor loop has `test`, `gc_bif '-'`, and the call, and each other
/// loop has these three instructions and one added instruction. A change of
/// the compiled shape fails here, because the numbers would then measure
/// other work.
#[test]
fn keeps_the_measured_instructions_in_each_loop() {
    let loops = disassemble();
    assert_eq!(
        loops["floor_ext_loop"],
        [
            "{label,1}",
            "{func_info,{atom,dispatch_probe},{atom,floor_ext_loop},1}",
            "{label,2}",
            "{test,is_eq_exact,{f,3},[{x,0},{integer,0}]}",
            "return",
            "{label,3}",
            "{gc_bif,'-',{f,0},1,[{x,0},{integer,1}],{x,0}}",
            "{call_ext_only,1,{extfunc,dispatch_probe,floor_ext_loop,1}}",
        ]
    );
    assert_eq!(
        loops["floor_loop"],
        [
            "{label,4}",
            "{func_info,{atom,dispatch_probe},{atom,floor_loop},1}",
            "{label,5}",
            "{test,is_eq_exact,{f,6},[{x,0},{integer,0}]}",
            "return",
            "{label,6}",
            "{gc_bif,'-',{f,0},1,[{x,0},{integer,1}],{x,0}}",
            "{call_only,1,{dispatch_probe,floor_loop,1}}",
        ]
    );
    assert_eq!(
        loops["plus_loop"],
        [
            "{label,7}",
            "{func_info,{atom,dispatch_probe},{atom,plus_loop},2}",
            "{label,8}",
            "{test,is_eq_exact,{f,9},[{x,0},{integer,0}]}",
            "{move,{x,1},{x,0}}",
            "return",
            "{label,9}",
            "{gc_bif,'-',{f,0},2,[{x,0},{integer,1}],{x,0}}",
            "{gc_bif,'+',{f,0},2,[{x,1},{integer,1}],{x,1}}",
            "{call_only,2,{dispatch_probe,plus_loop,2}}",
        ]
    );
    assert_eq!(
        loops["tuple_size_loop"],
        [
            "{label,10}",
            "{func_info,{atom,dispatch_probe},{atom,tuple_size_loop},3}",
            "{label,11}",
            "{test,is_eq_exact,{f,12},[{x,0},{integer,0}]}",
            "{move,{x,1},{x,0}}",
            "return",
            "{label,12}",
            "{gc_bif,'-',{f,0},3,[{x,0},{integer,1}],{x,0}}",
            "{bif,tuple_size,{f,0},[{x,2}],{x,1}}",
            "{call_only,3,{dispatch_probe,tuple_size_loop,3}}",
        ]
    );
    assert_eq!(
        loops["element_loop"],
        [
            "{label,13}",
            "{func_info,{atom,dispatch_probe},{atom,element_loop},3}",
            "{label,14}",
            "{test,is_eq_exact,{f,15},[{x,0},{integer,0}]}",
            "{move,{x,1},{x,0}}",
            "return",
            "{label,15}",
            "{gc_bif,'-',{f,0},3,[{x,0},{integer,1}],{x,0}}",
            "{bif,element,{f,0},[{integer,2},{x,2}],{x,1}}",
            "{call_only,3,{dispatch_probe,element_loop,3}}",
        ]
    );
    assert_eq!(
        loops["hd_loop"],
        [
            "{label,16}",
            "{func_info,{atom,dispatch_probe},{atom,hd_loop},3}",
            "{label,17}",
            "{test,is_eq_exact,{f,18},[{x,0},{integer,0}]}",
            "{move,{x,1},{x,0}}",
            "return",
            "{label,18}",
            "{gc_bif,'-',{f,0},3,[{x,0},{integer,1}],{x,0}}",
            "{bif,hd,{f,0},[{x,2}],{x,1}}",
            "{call_only,3,{dispatch_probe,hd_loop,3}}",
        ]
    );
    assert_eq!(
        loops["map_get_loop"],
        [
            "{label,19}",
            "{func_info,{atom,dispatch_probe},{atom,map_get_loop},4}",
            "{label,20}",
            "{test,is_eq_exact,{f,21},[{x,0},{integer,0}]}",
            "{move,{x,1},{x,0}}",
            "return",
            "{label,21}",
            "{gc_bif,'-',{f,0},4,[{x,0},{integer,1}],{x,0}}",
            "{bif,map_get,{f,0},[{x,3},{x,2}],{x,1}}",
            "{call_only,4,{dispatch_probe,map_get_loop,4}}",
        ]
    );
}

/// Record the interpreter dispatch baseline: run each probe on PBeam and on
/// the pinned ERTS, check each result, and print the numbers as Markdown
/// tables. The minimum of [`RUNS`] runs is the reported number.
#[test]
fn records_the_interpreter_dispatch_baseline() {
    let program = BeamProgram::load(&[DISPATCH_PROBE]).unwrap();
    let tuple = program.execute("dispatch_probe", "tuple3", &[]).unwrap();
    let list = program.execute("dispatch_probe", "list3", &[]).unwrap();
    let map = program.execute("dispatch_probe", "map_1000", &[]).unwrap();
    let key = program.execute("dispatch_probe", "key", &[]).unwrap();
    assert_eq!(text(&key), "500");

    let lines = run_erts();
    let erts_result = |key: &str| {
        lines
            .iter()
            .find_map(|line| line.strip_prefix(&format!("result {key} ")))
            .unwrap_or_else(|| panic!("the ERTS run has no result for {key}"))
            .to_owned()
    };
    let erts_nanos = |key: &str| -> u128 {
        lines
            .iter()
            .find_map(|line| line.strip_prefix(&format!("time {key} ")))
            .unwrap_or_else(|| panic!("the ERTS run has no time for {key}"))
            .parse()
            .unwrap()
    };

    let mut rows = Vec::new();
    for probe in probes() {
        let args = arguments(probe.function, probe.length, &tuple, &list, &map, &key);
        let result = program
            .execute("dispatch_probe", probe.function, &args)
            .unwrap();
        assert_eq!(
            text(&result),
            text(&expected(&program, probe.function, probe.length)),
            "{} at {} iterations",
            probe.function,
            probe.length
        );
        assert_eq!(
            text(&result),
            erts_result(&probe.key()),
            "PBeam and ERTS differ for {} at {} iterations",
            probe.function,
            probe.length
        );
        let pbeam = measure_pbeam(&program, probe.function, &args);
        let erts = erts_nanos(&probe.key());
        rows.push((probe, pbeam.as_nanos(), erts));
    }

    // The added cost of one instruction uses the local floor loop, because
    // the plan and the task name `call_only` for the call of that loop.
    let floor = rows
        .iter()
        .find(|(probe, _, _)| probe.function == "floor_loop" && probe.length == DELTA_LENGTH)
        .unwrap();

    println!("Interpreter dispatch baseline: minimum of {RUNS} runs of each probe.");
    println!();
    println!(
        "| probe | iterations | PBeam ns for each iteration | ERTS ns for each iteration | PBeam ns for the run | ERTS ns for the run |"
    );
    println!("| --- | --- | --- | --- | --- | --- |");
    for (probe, pbeam, erts) in &rows {
        println!(
            "| {} | {} | {:.1} | {:.2} | {} | {} |",
            probe.function,
            probe.length,
            *pbeam as f64 / probe.length as f64,
            *erts as f64 / probe.length as f64,
            pbeam,
            erts
        );
    }
    println!();
    println!(
        "| added instruction | added PBeam ns for each iteration | added ERTS ns for each iteration |"
    );
    println!("| --- | --- | --- |");
    for (probe, pbeam, erts) in &rows {
        if probe.function.starts_with("floor") {
            continue;
        }
        println!(
            "| {} | {:.1} | {:.2} |",
            probe.instruction,
            (*pbeam as f64 - floor.1 as f64) / probe.length as f64,
            (*erts as f64 - floor.2 as f64) / probe.length as f64,
        );
    }
    println!();
}

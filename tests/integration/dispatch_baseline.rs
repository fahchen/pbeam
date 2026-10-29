//! The interpreter dispatch baseline of `docs/interpreter-dispatch-plan.md`.
//! The benchmark measures the floor loop (`test`, `gc_bif '-'`, and the
//! call) at 20000 and 200000 iterations, and the cost of one added
//! instruction on that loop. Each probe runs on PBeam and on the pinned
//! ERTS, and the report gives the minimum of [`RUNS`] runs of each probe.
//!
//! The probes are in `fixtures/dispatch/dispatch_probe.erl` and
//! `fixtures/dispatch/dispatch_position_probe.erl`. The test consumes each
//! result, and it checks the disassembly of each loop, so the interpreter
//! cannot remove the work that the benchmark measures.
//!
//! Run the release measurement with `mise run bench-dispatch`. The check
//! gate runs the same test under the test profile, which measures slower
//! numbers but proves that the probes and the ERTS reference run. The test
//! profile holds debug assertions, so the constants scale the loop lengths
//! and the runs down and the gate stays short.
#![expect(
    clippy::unwrap_used,
    reason = "test helpers report a failure by a panic"
)]

use std::fmt::Write as _;
use std::{
    collections::HashMap,
    hint::black_box,
    path::PathBuf,
    process::Command,
    time::{Duration, Instant},
};

use pbeam_runtime::{AtomTable, BeamProgram, HostTerm, Term};

const DISPATCH_PROBE: &[u8] = include_bytes!("../../fixtures/dispatch/dispatch_probe.beam");
const POSITION_PROBE: &[u8] =
    include_bytes!("../../fixtures/dispatch/dispatch_position_probe.beam");

/// The number of timed runs of each probe. The report gives the minimum of
/// the runs, as `docs/interpreter-dispatch-plan.md` does.
const RUNS: usize = if cfg!(debug_assertions) { 2 } else { 9 };

/// The floor-loop lengths of `docs/interpreter-dispatch-plan.md`.
const FLOOR_LENGTHS: [usize; 2] = if cfg!(debug_assertions) {
    [2_000, 20_000]
} else {
    [20_000, 200_000]
};

/// The length of the added-instruction runs. The added cost of one
/// instruction is stable at this length.
const DELTA_LENGTH: usize = if cfg!(debug_assertions) {
    20_000
} else {
    200_000
};

/// The number of guest functions before `floor_loop/1` in
/// `fixtures/dispatch/dispatch_position_probe.erl`. That module holds the
/// floor loop after this many functions, so that the loop runs the
/// `function_at_label` scan that the plan's quoted baseline runs.
const POSITION_FUNCTIONS: usize = 20;

/// The fixture functions of the disassembly check of
/// `dispatch_probe.beam`, in module order.
const LOOP_FUNCTIONS: [&str; 7] = [
    "floor_ext_loop",
    "floor_loop",
    "plus_loop",
    "tuple_size_loop",
    "element_loop",
    "hd_loop",
    "map_get_loop",
];

/// One probe of the benchmark: the module, the fixture function, the label
/// of the shape in the report, and the loop length.
struct Probe {
    module: &'static str,
    function: &'static str,
    shape: String,
    length: usize,
}

impl Probe {
    /// The unique key of the probe in the ERTS reference lines.
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// assert_eq!(probes()[0].key(), "dispatch_probe-floor_ext_loop-20000");
    /// ```
    fn key(&self) -> String {
        format!("{}-{}-{}", self.module, self.function, self.length)
    }
}

/// The probe set: the three floor-loop shapes at both lengths, then the
/// five added-instruction probes at [`DELTA_LENGTH`]. The plan names
/// `call_only` for the floor loop, says that the loop "includes one external
/// call", and quotes a baseline from a module with about 20 functions, so
/// the benchmark measures all three shapes.
///
/// # Examples
///
/// ```rust,ignore
/// assert_eq!(probes()[0].function, "floor_ext_loop");
/// ```
fn probes() -> Vec<Probe> {
    let mut probes = Vec::new();
    for &length in &FLOOR_LENGTHS {
        for (module, function, shape) in [
            (
                "dispatch_probe",
                "floor_ext_loop",
                "`call_ext_only` floor loop in `dispatch_probe`".to_owned(),
            ),
            (
                "dispatch_probe",
                "floor_loop",
                "`call_only` floor loop in `dispatch_probe`".to_owned(),
            ),
            (
                "dispatch_position_probe",
                "floor_loop",
                format!(
                    "`call_only` floor loop after {POSITION_FUNCTIONS} guest functions in `dispatch_position_probe`"
                ),
            ),
        ] {
            probes.push(Probe {
                module,
                function,
                shape,
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
            module: "dispatch_probe",
            function,
            shape: format!("`call_only` floor loop plus {instruction}"),
            length: DELTA_LENGTH,
        });
    }
    probes
}

/// The path of a fixture, for the ERTS runs and the disassembly check.
///
/// # Examples
///
/// ```rust,ignore
/// assert!(probe_path("dispatch_probe").ends_with("fixtures/dispatch/dispatch_probe.beam"));
/// ```
fn probe_path(module: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join(format!("fixtures/dispatch/{module}.beam"))
}

/// The Erlang variable of the loaded module `module` in
/// [`erts_expression`].
///
/// # Examples
///
/// ```rust,ignore
/// assert_eq!(erts_module("dispatch_probe"), "M");
/// ```
fn erts_module(module: &str) -> &'static str {
    match module {
        "dispatch_probe" => "M",
        "dispatch_position_probe" => "P",
        other => panic!("unknown probe module {other}"),
    }
}

/// The arguments of one probe for the PBeam run.
///
/// # Examples
///
/// ```rust,ignore
/// let args = arguments(&program, "floor_loop", 20_000, &tuple, &list, &map, &key);
/// assert_eq!(args.len(), 1);
/// ```
fn arguments(
    program: &BeamProgram,
    function: &str,
    length: usize,
    tuple: &Term,
    list: &Term,
    map: &Term,
    key: &Term,
) -> Vec<HostTerm> {
    let counter = Term::Integer(i64::try_from(length).unwrap());
    let zero = Term::Integer(0);
    let terms = match function {
        "floor_ext_loop" | "floor_loop" => vec![counter],
        "plus_loop" => vec![counter, zero],
        "tuple_size_loop" | "element_loop" => vec![counter, zero, tuple.clone()],
        "hd_loop" => vec![counter, zero, list.clone()],
        "map_get_loop" => vec![counter, zero, map.clone(), key.clone()],
        other => panic!("unknown probe function {other}"),
    };
    terms
        .into_iter()
        .map(|term| program.host_term(term))
        .collect()
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
        "plus_loop" => Term::Integer(i64::try_from(length).unwrap()),
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
/// The atom names come from `atoms`, the table that owns the term.
///
/// # Examples
///
/// ```rust,ignore
/// assert_eq!(text(&Term::Integer(3), &atoms), "3");
/// ```
fn text(term: &Term, atoms: &AtomTable) -> String {
    match term {
        Term::Atom(atom) => atoms.name_of(atom).to_owned(),
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
         {module, P} = code:ensure_loaded(dispatch_position_probe), \
         jit = erlang:system_info(emu_flavor), \
         Map = M:map_1000(), Tuple = {a, b, c}, List = [a, b, c], Key = 500, \
         Run = fun(F) -> lists:min([begin \
             S = erlang:monotonic_time(nanosecond), _ = F(), \
             erlang:monotonic_time(nanosecond) - S end || _ <- lists:seq(1, 9)]) end, ",
    );
    for probe in probes() {
        let _ = write!(
            expression,
            "io:format(\"result {} ~w~n\", [{}]), ",
            probe.key(),
            call_text(&probe)
        );
    }
    for probe in probes() {
        let _ = write!(
            expression,
            "io:format(\"time {} ~b~n\", [Run(fun() -> {} end)]), ",
            probe.key(),
            call_text(&probe)
        );
    }
    expression.push_str("halt().");
    expression
}

/// The Erlang call of one probe, for example `M:floor_loop(20000)`.
///
/// # Examples
///
/// ```rust,ignore
/// assert_eq!(call_text(&probes()[0]), "M:floor_ext_loop(20000)");
/// ```
fn call_text(probe: &Probe) -> String {
    format!(
        "{}:{}({})",
        erts_module(probe.module),
        probe.function,
        arguments_text(probe.function, probe.length)
    )
}

/// Measure one probe on PBeam and return the minimum of [`RUNS`] runs. The
/// result of each run goes to [`black_box`], so the compiler cannot remove
/// the work that the timer measures.
///
/// # Examples
///
/// ```rust,ignore
/// let best = measure_pbeam(&program, "dispatch_probe", "floor_loop", &[program.host_term(Term::Integer(20_000))]);
/// assert!(best > Duration::ZERO);
/// ```
fn measure_pbeam(
    program: &BeamProgram,
    module: &str,
    function: &str,
    args: &[HostTerm],
) -> Duration {
    let mut best = Duration::MAX;
    for _ in 0..RUNS {
        let start = Instant::now();
        let result = program.execute(module, function, args).unwrap();
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
/// assert!(lines.iter().any(|line| line.starts_with("time dispatch_probe-floor_loop-20000 ")));
/// ```
fn run_erts() -> Vec<String> {
    let directory = probe_path("dispatch_probe").parent().unwrap().to_path_buf();
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

/// Return the `beam_disasm` instructions of each function of the fixture
/// `module`, without the `{line, _}` entries, as `mise run disasm` prints
/// them. The result proves that each loop holds the work that the benchmark
/// claims to measure.
///
/// # Examples
///
/// ```rust,ignore
/// let loops = disassemble("dispatch_probe");
/// assert!(loops["floor_loop"].contains(&"{call_only,1,{dispatch_probe,floor_loop,1}}".to_owned()));
/// ```
fn disassemble(module: &str) -> HashMap<String, Vec<String>> {
    let expression = format!(
        "{{beam_file, _, _, _, _, Functions}} = beam_disasm:file(\"{}\"), \
         [begin \
             {{function, Name, _, _, Instructions}} = Function, \
             io:format(\"function ~p~n\", [Name]), \
             [io:format(\"~p~n\", [Instruction]) || Instruction <- Instructions, \
              not is_tuple(Instruction) orelse element(1, Instruction) =/= line] \
          end || Function <- Functions], halt().",
        probe_path(module).display()
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
    loops
}

/// Check that each loop holds the instructions that the benchmark measures:
/// the floor loop has `test`, `gc_bif '-'`, and the call, and each other
/// loop has these three instructions and one added instruction. A change of
/// the compiled shape fails here, because the numbers would then measure
/// other work.
#[test]
#[expect(
    clippy::too_many_lines,
    reason = "case table: one expected disassembly for each loop function"
)]
fn keeps_the_measured_instructions_in_each_loop() {
    let loops = disassemble("dispatch_probe");
    for function in LOOP_FUNCTIONS {
        assert!(
            loops.contains_key(function),
            "{function} is not in dispatch_probe"
        );
    }
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

    // The position probe holds the same floor loop after its guest
    // functions. The guest entries before the loop are the scan work, so the
    // first label of the loop asserts the position and the tail asserts the
    // loop clause. The map holds one key for each distinct function name, so
    // the two `module_info` clauses share one key.
    let position = disassemble("dispatch_position_probe");
    for index in 1..=POSITION_FUNCTIONS {
        assert!(
            position.contains_key(&format!("guest_{index}")),
            "guest_{index} is not in dispatch_position_probe"
        );
    }
    assert_eq!(
        position.len(),
        POSITION_FUNCTIONS + 2,
        "the position probe must hold {POSITION_FUNCTIONS} guest functions, floor_loop/1, and module_info/0,1"
    );
    let instructions = &position["floor_loop"];
    assert_eq!(instructions.len(), 8);
    assert_eq!(
        instructions[0],
        format!("{{label,{}}}", 2 * POSITION_FUNCTIONS + 1),
        "the floor loop must come after the {POSITION_FUNCTIONS} guest functions"
    );
    assert_eq!(
        instructions[instructions.len() - 2..].to_vec(),
        [
            "{gc_bif,'-',{f,0},1,[{x,0},{integer,1}],{x,0}}",
            "{call_only,1,{dispatch_position_probe,floor_loop,1}}",
        ]
    );
    assert!(
        instructions[3].starts_with("{test,is_eq_exact,")
            && instructions[3].ends_with("[{x,0},{integer,0}]}"),
        "the floor loop must start with the test of the counter: {}",
        instructions[3]
    );
}

/// Record the interpreter dispatch baseline: run each probe on PBeam and on
/// the pinned ERTS, check each result, and print the numbers as Markdown
/// tables. The minimum of [`RUNS`] runs is the reported number.
#[test]
fn records_the_interpreter_dispatch_baseline() {
    let program = BeamProgram::load(&[DISPATCH_PROBE, POSITION_PROBE]).unwrap();
    let tuple = program.execute("dispatch_probe", "tuple3", &[]).unwrap();
    let list = program.execute("dispatch_probe", "list3", &[]).unwrap();
    let map = program.execute("dispatch_probe", "map_1000", &[]).unwrap();
    let key = program.execute("dispatch_probe", "key", &[]).unwrap();
    assert_eq!(text(&key.term, key.atoms()), "500");

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
        let args = arguments(
            &program,
            probe.function,
            probe.length,
            &tuple.term,
            &list.term,
            &map.term,
            &key.term,
        );
        let result = program
            .execute(probe.module, probe.function, &args)
            .unwrap();
        assert_eq!(
            text(&result.term, result.atoms()),
            text(
                &expected(&program, probe.function, probe.length),
                program.atoms()
            ),
            "{}:{} at {} iterations",
            probe.module,
            probe.function,
            probe.length
        );
        assert_eq!(
            text(&result.term, result.atoms()),
            erts_result(&probe.key()),
            "PBeam and ERTS differ for {}:{} at {} iterations",
            probe.module,
            probe.function,
            probe.length
        );
        let pbeam = measure_pbeam(&program, probe.module, probe.function, &args);
        let erts = erts_nanos(&probe.key());
        rows.push((probe, pbeam.as_nanos(), erts));
    }

    // The added cost of one instruction uses the local `call_only` floor
    // loop, because the plan and the task name `call_only` for the call of
    // that loop.
    let floor = rows
        .iter()
        .find(|(probe, _, _)| {
            probe.module == "dispatch_probe"
                && probe.function == "floor_loop"
                && probe.length == DELTA_LENGTH
        })
        .unwrap();

    print_baseline(&rows, floor);
}

/// Print the tables of the baseline: the rows of each probe, and the added
/// cost of one instruction over the `floor` row of the local floor loop.
#[expect(
    clippy::as_conversions,
    clippy::cast_precision_loss,
    reason = "timing probe: the nanosecond and record counts convert to f64 for a ratio"
)]
fn print_baseline(rows: &[(Probe, u128, u128)], floor: &(Probe, u128, u128)) {
    println!("Interpreter dispatch baseline: minimum of {RUNS} runs of each probe.");
    println!();
    println!(
        "| shape | iterations | PBeam ns for each iteration | ERTS ns for each iteration | PBeam ns for the run | ERTS ns for the run |"
    );
    println!("| --- | --- | --- | --- | --- | --- |");
    for (probe, pbeam, erts) in rows {
        println!(
            "| {} | {} | {:.1} | {:.2} | {} | {} |",
            probe.shape,
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
    for (probe, pbeam, erts) in rows {
        if probe.function.starts_with("floor") {
            continue;
        }
        println!(
            "| {} | {:.1} | {:.2} |",
            probe.shape,
            (*pbeam as f64 - floor.1 as f64) / probe.length as f64,
            (*erts as f64 - floor.2 as f64) / probe.length as f64,
        );
    }
    println!();
}

/// The working sets of the cache proxy, in records: the six 136-byte slots
/// that the floor loop spans, then sets that cross the measured L1 data cache
/// of the host (65536 bytes for the efficiency cores, or 482 records of 136
/// bytes, and 131072 bytes for the performance cores) and the measured L2
/// cache, up to the estimated 280000 instructions of one Terra Counter redraw
/// frame.
const STREAM_RECORDS: [usize; 5] = [6, 1_000, 10_000, 100_000, 300_000];

/// The bytes of one record of the decoded form, measured by
/// `size_of::<Instruction>()` in `tests/runtime/interpreter.rs`, and the
/// bytes of one record of the prepared form that the plan names.
const DECODED_STRIDE: usize = 136;
const PREPARED_STRIDE: usize = 32;

/// The records that one iteration of the floor loop reads. The loop retires
/// 4 instructions (see `records_the_decoded_loop_body` in
/// `tests/runtime/interpreter.rs`), and each instruction occupies one record.
const FLOOR_LOOP_RECORDS: usize = 4;

/// The target record reads of one measurement and the runs of each
/// measurement. Each working set is read about the same number of times, so a
/// difference between two working sets is not a difference of the work. The
/// actual count is the largest multiple of the record count that is at or
/// below the target, and the table prints it: 1999998 reads for the 6-record
/// set and 1800000 reads for the 300000-record set.
const STREAM_VISITS: usize = 2_000_000;
const STREAM_RUNS: usize = 5;

/// Return a buffer of `records` records of `stride` bytes with a
/// deterministic content, so the walk cannot fold the bytes to a constant.
///
/// # Examples
///
/// ```rust,ignore
/// assert_eq!(stream_buffer(2, 4).len(), 8);
/// ```
fn stream_buffer(records: usize, stride: usize) -> Vec<u8> {
    (0..records * stride)
        .map(|index| u8::try_from(index % 251).unwrap())
        .collect()
}

/// Read the first byte of each record of `buffer` at `stride` for `rounds`
/// passes and return the sum. The loaded byte goes through [`black_box`], so
/// the compiler keeps the loads.
///
/// # Examples
///
/// ```rust,ignore
/// assert_eq!(walk(&[1, 2, 3, 4], 2, 1), 1 + 3);
/// ```
fn walk(buffer: &[u8], stride: usize, rounds: usize) -> u64 {
    let mut sum = 0u64;
    for _ in 0..rounds {
        let mut index = 0;
        while index < buffer.len() {
            sum = sum.wrapping_add(u64::from(black_box(buffer[index])));
            index += stride;
        }
    }
    black_box(sum)
}

/// Measure the cache proxy of the decoded form: the time to read one record
/// of a stream that a loop visits again and again, at the 136-byte stride of
/// `Instruction` and at the 32-byte stride of the prepared form of step 8.
/// The record count is the same for both strides, and the minimum of
/// [`STREAM_RUNS`] runs is the reported number.
///
/// The proxy covers one byte of each record of a contiguous byte stream, in
/// order, with the hardware prefetch that a sequential read gets. It omits
/// the other field loads of an instruction arm, the heap operand vectors and
/// the name strings of an instruction, the dependent work between two
/// instructions, and the interpreter's own access pattern. Thus it compares
/// the memory cost of the two record strides, and it is not a bound on the
/// gain of a compact form in the interpreter.
///
/// One iteration of the floor loop retires 4 instructions and reads 4
/// records, so the last column multiplies the per-record difference by 4.
#[test]
#[expect(
    clippy::as_conversions,
    clippy::cast_precision_loss,
    reason = "timing probe: the nanosecond and record counts convert to f64 for a ratio"
)]
fn records_the_instruction_stream_cache_proxy() {
    println!(
        "Instruction stream cache proxy: target of {STREAM_VISITS} record reads for each measurement, minimum of {STREAM_RUNS} runs. One floor-loop iteration reads {FLOOR_LOOP_RECORDS} records."
    );
    println!();
    println!(
        "| records | reads | decoded bytes | decoded ns for each record | prepared bytes | prepared ns for each record | difference ns for each record | difference ns for each floor-loop iteration |"
    );
    println!("| --- | --- | --- | --- | --- | --- | --- | --- |");
    for records in STREAM_RECORDS {
        let mut measured = Vec::new();
        for stride in [DECODED_STRIDE, PREPARED_STRIDE] {
            let buffer = stream_buffer(records, stride);
            let rounds = (STREAM_VISITS / records).max(1);
            let reads = rounds * records;
            let mut best = f64::MAX;
            for _ in 0..STREAM_RUNS {
                let start = Instant::now();
                let sum = walk(&buffer, stride, rounds);
                let elapsed = start.elapsed().as_nanos() as f64;
                black_box(sum);
                best = best.min(elapsed / reads as f64);
            }
            measured.push(best);
        }
        let difference = measured[0] - measured[1];
        let reads = (STREAM_VISITS / records).max(1) * records;
        println!(
            "| {records} | {reads} | {} | {:.2} | {} | {:.2} | {difference:.2} | {:.2} |",
            records * DECODED_STRIDE,
            measured[0],
            records * PREPARED_STRIDE,
            measured[1],
            difference * FLOOR_LOOP_RECORDS as f64,
        );
    }
    println!();
}

/// Record the two reference points of the instruction attribution of task
/// [#546](https://github.com/fahchen/pbeam/issues/546) in a machine-readable
/// form. `scripts/dispatch_attribution.sh` runs this test in the base build
/// and in each scratch ablation variant, so the same command measures the
/// same loop in each build.
///
/// The two numbers are the local `call_only` floor loop of `dispatch_probe`
/// for each iteration, and the added cost of one `gc_bif '+'` on the same
/// loop for each iteration. The added `+` is the unit of one instruction that
/// `docs/interpreter-dispatch-plan.md` measures, and the floor loop holds the
/// `test`, the `gc_bif '-'`, and the `call_only` that the attribution names.
#[test]
#[expect(
    clippy::as_conversions,
    clippy::cast_precision_loss,
    reason = "timing probe: the nanosecond and record counts convert to f64 for a ratio"
)]
fn records_the_attribution_reference_points() {
    let program = BeamProgram::load(&[DISPATCH_PROBE]).unwrap();
    let length = DELTA_LENGTH;
    let floor_arguments = [program.host_term(Term::Integer(i64::try_from(length).unwrap()))];
    let plus_arguments = [
        program.host_term(Term::Integer(i64::try_from(length).unwrap())),
        program.host_term(Term::Integer(0)),
    ];
    assert_eq!(
        program
            .execute("dispatch_probe", "floor_loop", &floor_arguments)
            .unwrap()
            .term,
        Term::Integer(0)
    );
    assert_eq!(
        program
            .execute("dispatch_probe", "plus_loop", &plus_arguments)
            .unwrap()
            .term,
        Term::Integer(i64::try_from(length).unwrap())
    );
    let floor = measure_pbeam(&program, "dispatch_probe", "floor_loop", &floor_arguments);
    let plus = measure_pbeam(&program, "dispatch_probe", "plus_loop", &plus_arguments);
    let floor_ns = floor.as_nanos() as f64 / length as f64;
    let added_ns = plus.as_nanos() as f64 / length as f64 - floor_ns;
    println!(
        "attribution floor_loop iterations {length} runs {RUNS} ns_per_iteration {floor_ns:.3}"
    );
    println!(
        "attribution added_gc_bif_plus iterations {length} runs {RUNS} ns_per_iteration {added_ns:.3}"
    );
}

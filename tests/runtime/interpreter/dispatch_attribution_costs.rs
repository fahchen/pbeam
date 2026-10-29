//! The micro-measurement of the parts of one interpreter instruction, for the
//! attribution of task [#546](https://github.com/fahchen/pbeam/issues/546) and
//! the cursor measurement of task [#550](https://github.com/fahchen/pbeam/issues/550).
//!
//! The ablation of the record neutralises one sub-step of the interpreter in a
//! scratch build and measures the difference on the floor loop of
//! `fixtures/dispatch/dispatch_probe.erl`. An ablation cannot separate a part
//! that the floor loop does not execute, and it cannot separate a part whose
//! removal changes the measured loop. This module measures those parts one
//! operation at a time, at the same revision, through the private helpers of
//! the interpreter.
//!
//! The module lives beside the interpreter, because it reads the private
//! operand, label, frame, reduction, and BIF argument helpers. A copy of those
//! helpers would measure the copy and not the runtime.
//!
//! Every number is the minimum of [`RUNS`] timed runs of one operation, in
//! nanosecond for each operation, and every result passes through
//! [`black_box`]. The test asserts no time, because the load of the host
//! changes the number. The check gate runs the same test under the test
//! profile, which proves that every measured operation runs; those numbers are
//! not the report numbers.
//!
//! Run the release measurement with
//! `cargo test --locked --release --lib -- dispatch_attribution_costs -- --nocapture --test-threads=1`
//! and read the `COST` lines.

use std::{collections::HashMap, hint::black_box, sync::Arc, time::Instant};

use super::{
    BeamProgram, CodeTable, Cursor, bif::BifId, code_table, current_frame, function_at_label,
    light_bif_switch, read_bif_arguments, read_operand, write_operand,
};
use crate::{
    Term,
    beam::{
        BeamModule,
        code::{Instruction, Operand},
    },
    runtime::{
        process::{ProcessState, StackSlot, Start, tests::frame},
        scheduler::Scheduler,
    },
    term::{AtomTable, ProcessId},
};

/// The number of timed runs of one operation. The report gives the minimum
/// of the runs, as the term and drop measurements do.
const RUNS: usize = 9;

/// The number of operations of one timed run. The count is large enough that
/// the read of the clock and the loop overhead are below the resolution of
/// one operation.
const OPERATIONS: usize = 100_000;

/// Print one measured line: the name of the operation and the nanosecond for
/// each operation.
fn report(name: &str, ns: f64) {
    println!("COST {name} {ns:.3} ns");
}

/// Return the nanoseconds for one operation of `operation`, as the minimum of
/// [`RUNS`] timed runs of [`OPERATIONS`] operations. The checksum keeps the
/// result alive, so the compiler cannot remove the work of the loop.
///
/// # Examples
///
/// ```rust,ignore
/// let ns = cost(|| black_box(1u64));
/// assert!(ns >= 0.0);
/// ```
fn cost(mut operation: impl FnMut() -> u64) -> f64 {
    let mut best = f64::INFINITY;
    for _ in 0..RUNS {
        let start = Instant::now();
        let mut checksum = 0u64;
        for _ in 0..OPERATIONS {
            checksum = checksum.wrapping_add(operation());
        }
        let elapsed = start.elapsed().as_nanos() as f64;
        black_box(checksum);
        best = best.min(elapsed / OPERATIONS as f64);
    }
    best
}

/// Return the nanoseconds for one operation of `operation`, as the minimum of
/// [`RUNS`] timed runs of [`OPERATIONS`] operations. `setup` builds the state
/// of each run outside the timer, so an operation that changes its input
/// starts each run from the same state.
///
/// # Examples
///
/// ```rust,ignore
/// let ns = cost_with(|| 1u64, |state| { *state += 1; *state });
/// assert!(ns >= 0.0);
/// ```
fn cost_with<State>(
    mut setup: impl FnMut() -> State,
    mut operation: impl FnMut(&mut State) -> u64,
) -> f64 {
    let mut best = f64::INFINITY;
    for _ in 0..RUNS {
        let mut state = setup();
        let start = Instant::now();
        let mut checksum = 0u64;
        for _ in 0..OPERATIONS {
            checksum = checksum.wrapping_add(operation(&mut state));
        }
        let elapsed = start.elapsed().as_nanos() as f64;
        black_box(checksum);
        black_box(&state);
        best = best.min(elapsed / OPERATIONS as f64);
    }
    best
}

/// Return the module of the floor loop of `dispatch_probe:floor_loop/1`:
/// `test is_eq_exact`, `gc_bif '-'`, `call_only`, and the `return` of the
/// clause that the counter reaches.
///
/// # Examples
///
/// ```rust,ignore
/// assert_eq!(floor_module().instructions.len(), 7);
/// ```
fn floor_module() -> Arc<BeamModule> {
    Arc::new(
        BeamModule::new(
            "fixture".into(),
            HashMap::from([(("floor_loop".into(), 1), 1)]),
            Vec::new(),
            vec![
                Instruction::FunctionInfo {
                    module: "fixture".into(),
                    name: "floor_loop".into(),
                    arity: 1,
                },
                Instruction::Label(1),
                Instruction::TestExactEqual {
                    fail_label: 3,
                    left: Operand::X(0),
                    right: Operand::Value(Term::Integer(0)),
                },
                Instruction::Return,
                Instruction::Label(3),
                Instruction::Bif {
                    fail_label: 0,
                    bif: BifId::ErlangMinus2,
                    arguments: vec![Operand::X(0), Operand::Value(Term::Integer(1))],
                    destination: Operand::X(0),
                },
                Instruction::CallOnly {
                    arity: 1,
                    function: 1,
                    target: 1,
                },
            ],
        )
        .expect("the fixture module loads"),
    )
}

/// Return the number of the opcode arm that the run loop of the floor loop
/// selects for `instruction`. The match is the dispatch of the floor loop and
/// not the whole match of the run loop.
///
/// # Examples
///
/// ```rust,ignore
/// assert_eq!(dispatch_arm(&Instruction::Return), 4);
/// ```
fn dispatch_arm(instruction: &Instruction) -> u64 {
    match instruction {
        Instruction::TestExactEqual { .. } => 1,
        Instruction::Bif { .. } => 2,
        Instruction::CallOnly { .. } => 3,
        Instruction::Return => 4,
        _ => 0,
    }
}

/// Return a scheduler that runs [`floor_module`] at the entry of its
/// `floor_loop/1` function, for the cursor rows of task
/// [#550](https://github.com/fahchen/pbeam/issues/550).
///
/// # Examples
///
/// ```rust,ignore
/// let (scheduler, module) = cursor_scheduler();
/// assert_eq!(scheduler.current_module().unwrap().id, module.id);
/// ```
fn cursor_scheduler() -> (Scheduler, Arc<BeamModule>) {
    let module = floor_module();
    let mut scheduler = Scheduler::new(AtomTable::new());
    scheduler.code = CodeTable::new(HashMap::from([("fixture".to_owned(), Arc::clone(&module))]));
    let frame = code_table::frame(&module, 1, 1).expect("floor_loop/1 exists");
    scheduler.start(frame, vec![Term::Integer(7)], Vec::new());
    scheduler
        .schedule_next()
        .expect("the root process is runnable");
    (scheduler, module)
}

/// Print the cost of every part that an ablation cannot separate.
#[test]
fn prints_the_cost_of_every_instruction_part() {
    let module = floor_module();
    let instructions = &module.instructions;
    let registers = vec![Some(Term::Integer(7)), Some(Term::Integer(1))];
    let stack: Vec<StackSlot> = Vec::new();
    let x0 = Operand::X(0);
    let literal = Operand::Value(Term::Integer(1));

    // The dispatch of one instruction: the fetch of the record and the match
    // of the opcode. The branch of the loop is the `call_only` back edge and
    // it is outside this measurement.
    report(
        "dispatch_fetch_and_match",
        cost_with(
            || 0usize,
            |position| {
                let instruction = instructions.get(*position).unwrap();
                let arm = dispatch_arm(black_box(instruction));
                *position += 1;
                if *position == instructions.len() {
                    *position = 0;
                }
                arm
            },
        ),
    );

    // The operand and register access.
    report(
        "read_operand_x",
        cost(|| {
            let value = read_operand(black_box(&x0), black_box(&registers), black_box(&stack))
                .expect("x0 is set");
            black_box(value);
            1
        }),
    );
    report(
        "read_operand_literal",
        cost(|| {
            let value = read_operand(
                black_box(&literal),
                black_box(&registers),
                black_box(&stack),
            )
            .expect("a literal reads");
            black_box(value);
            1
        }),
    );
    report(
        "write_operand_x",
        cost_with(
            || vec![Some(Term::Integer(0))],
            |registers| {
                write_operand(black_box(&x0), Term::Integer(7), registers, &mut [])
                    .expect("x0 writes");
                1
            },
        ),
    );

    // The lookups of the loop.
    report(
        "label_index",
        cost(|| {
            let index = *module
                .tables
                .jump_targets
                .get(black_box(&3))
                .expect("label 3 exists");
            index as u64
        }),
    );
    report(
        "function_at_label",
        cost(|| {
            let function = function_at_label(black_box(&module), black_box(1))
                .expect("label 1 starts a function");
            function.start as u64
        }),
    );
    report(
        "frame_creation",
        cost(|| {
            let frame = code_table::frame(black_box(&module), black_box(1), black_box(1))
                .expect("floor_loop/1 exists");
            frame.next_instruction as u64
        }),
    );

    // The state that the loop changes. The floor loop charges no reduction for
    // its `gc_bif` and its `call_only`, so the reduction accounting of the
    // loop is the `return` at the end of the loop. The switch-out check of a
    // light BIF belongs to the `call_ext` path, and this row measures it for
    // that path.
    report(
        "light_bif_switch",
        cost_with(
            || {
                let mut process = ProcessState::new(ProcessId(0), Start::Entry(frame()), vec![]);
                process.fcalls = OPERATIONS as i32 + 1;
                process
            },
            |process| u64::from(light_bif_switch(process)),
        ),
    );

    // The three parts of the cursor of step 2, which task
    // [#550](https://github.com/fahchen/pbeam/issues/550) measures apart: the
    // module compare of each iteration, the refresh of the cursor when the
    // active module changes, and the commit of the position at each exit.
    // Each operation reads its state through `black_box`, because the state
    // does not change between two iterations of a plain loop and the
    // compiler would hoist the read out of the timed region.
    let (_, module) = cursor_scheduler();
    let module_id = module.id;
    report(
        "cursor_module_compare",
        cost_with(
            || ProcessState::new(ProcessId(0), Start::Entry(frame()), vec![]),
            |process| u64::from(current_frame(black_box(process)).module_id != module_id),
        ),
    );
    report(
        "cursor_commit",
        cost_with(
            || {
                let (mut scheduler, _module) = cursor_scheduler();
                let cursor = Cursor::new(&mut scheduler).expect("the active frame has code");
                (
                    ProcessState::new(ProcessId(0), Start::Entry(frame()), vec![]),
                    cursor,
                    scheduler,
                )
            },
            |(process, cursor, _scheduler)| {
                cursor.commit(black_box(process));
                cursor.position as u64
            },
        ),
    );
    report(
        "cursor_refresh_same_module",
        cost_with(
            || {
                let (mut scheduler, module) = cursor_scheduler();
                let cursor = Cursor::new(&mut scheduler).expect("the active frame has code");
                (scheduler, cursor, module)
            },
            |(scheduler, cursor, _module)| {
                cursor
                    .refresh(black_box(scheduler))
                    .expect("the module is loaded");
                cursor.position as u64
            },
        ),
    );
    // The changed path reads the code table and clones the module of the
    // active frame, and the reset of `cursor.module` between the operations
    // adds one more clone-and-drop pair. The `cursor_module_reset` row gives
    // the cost of that reset alone, so a reader can subtract it.
    let other = Arc::new(
        BeamModule::new("other".into(), HashMap::new(), Vec::new(), vec![])
            .expect("the fixture module loads"),
    );
    assert_ne!(other.id, module_id);
    report(
        "cursor_module_reset",
        cost_with(
            || {
                let (mut scheduler, module) = cursor_scheduler();
                let cursor = Cursor::new(&mut scheduler).expect("the active frame has code");
                (cursor, module)
            },
            |(cursor, _module)| {
                cursor.module = Arc::clone(black_box(&other));
                cursor.position as u64
            },
        ),
    );
    report(
        "cursor_refresh_changed_module",
        cost_with(
            || {
                let (mut scheduler, module) = cursor_scheduler();
                let cursor = Cursor::new(&mut scheduler).expect("the active frame has code");
                (scheduler, cursor, module)
            },
            |(scheduler, cursor, _module)| {
                cursor.module = Arc::clone(black_box(&other));
                cursor
                    .refresh(black_box(scheduler))
                    .expect("the module is loaded");
                cursor.position as u64
            },
        ),
    );

    // The BIF call overhead: the borrowed argument path and the body dispatch.
    let process = ProcessState::new(ProcessId(0), Start::Entry(frame()), vec![Term::Integer(7)]);
    let arguments = [Operand::X(0), Operand::Value(Term::Integer(1))];
    report(
        "read_bif_arguments",
        cost(|| {
            let values =
                read_bif_arguments(black_box(&process), black_box(&arguments)).expect("x0 is set");
            values.with(<[&crate::term::Term]>::len) as u64
        }),
    );
    let mut scheduler = Scheduler::new(AtomTable::new());
    let terms = [Term::Integer(7), Term::Integer(1)];
    report(
        "bif_slice_minus",
        cost(|| {
            let result = BeamProgram::bif_slice(
                black_box(&mut scheduler),
                BifId::ErlangMinus2,
                black_box(&terms),
            )
            .expect("an integer subtraction does not fail");
            match result {
                super::bif::BifResult::Value(value) => {
                    black_box(&value);
                    1
                }
                other => panic!("unexpected result {other:?}"),
            }
        }),
    );

    // The term work. The floor loop holds immediate terms, so its term work is
    // the pair stack of `Term::eq` and not an `Arc` count. The two `Term`
    // clone rows give the `Arc` pair of a shared term and the copy of an
    // immediate, so a later loop that holds a compound term has a bound.
    let left = Term::Integer(0);
    let right = Term::Integer(0);
    report(
        "term_eq_integer",
        cost(|| u64::from(black_box(black_box(&left) == black_box(&right)))),
    );
    let shared = Term::tuple(vec![Term::Integer(1)]);
    report(
        "term_clone_drop_shared_tuple",
        cost(|| {
            let copy = black_box(&shared).clone();
            black_box(copy);
            1
        }),
    );
    let immediate = Term::Integer(7);
    report(
        "term_clone_drop_immediate",
        cost(|| {
            let copy = black_box(&immediate).clone();
            black_box(copy);
            1
        }),
    );
}

use std::{collections::HashMap, rc::Rc};

use super::{
    Frame, Origin, OriginMonitor, PendingBif, ProcessState, ReceiveTimer, StackSlot, Start,
};
use crate::{
    BeamModule,
    beam::code::Instruction,
    runtime::{interpreter::CodeTable, scheduler::Scheduler},
    term::{Atom, AtomTable, ProcessId, Reference, Term},
};

thread_local! {
    /// The module of the frames of the tests. A test scheduler loads it, so
    /// a frame of [`frame`] names a module of its code table.
    static MODULE: Rc<BeamModule> = Rc::new(BeamModule::new(
        "fixture".into(),
        HashMap::new(),
        vec![
            Instruction::FunctionInfo {
                module: "fixture".into(),
                name: "main".into(),
                arity: 0,
            },
            Instruction::Label(1),
        ],
    ));
}

/// Return the module of the frames of the tests.
pub(in crate::runtime) fn module() -> Rc<BeamModule> {
    MODULE.with(Rc::clone)
}

/// Return the entry frame of the tests: `fixture:main/0`.
pub(in crate::runtime) fn frame() -> Frame {
    Frame {
        module_id: MODULE.with(|module| module.id),
        function: MODULE.with(|module| module.tables.name_id("main").unwrap()),
        next_instruction: 0,
        stack: Vec::new(),
    }
}

/// Return a scheduler with the module of the frames of the tests loaded.
pub(in crate::runtime) fn test_scheduler() -> Scheduler {
    let mut scheduler = Scheduler::new(AtomTable::new());
    scheduler.code = CodeTable::new(HashMap::from([("fixture".into(), module())]));
    scheduler
}

/// Return a scheduler that skips the capability checks, with the module of
/// the frames of the tests loaded.
pub(in crate::runtime) fn test_scheduler_without_checks() -> Scheduler {
    let mut scheduler = Scheduler::without_checks(AtomTable::new());
    scheduler.code = CodeTable::new(HashMap::from([("fixture".into(), module())]));
    scheduler
}

#[test]
fn expires_the_receive_timer_and_wakes_only_a_waiting_process() {
    let mut process = ProcessState::new(ProcessId(0), Start::Entry(frame()), vec![]);
    process.receive_timer = ReceiveTimer::Set {
        key: (1100, 0),
        resume: 7,
    };
    process.waiting = true;
    assert!(process.time_out());
    assert_eq!(process.receive_timer, ReceiveTimer::Expired { resume: 7 });
    assert!(!process.time_out());
    assert_eq!(process.receive_timer, ReceiveTimer::Expired { resume: 7 });
}

#[test]
fn uses_reductions_down_to_zero_as_erts_bump_reds() {
    let mut process = ProcessState::new(ProcessId(0), Start::Entry(frame()), vec![]);
    process.fcalls = 6;
    process.bump_reductions(4);
    assert_eq!(process.fcalls, 2);
    process.bump_reductions(4);
    assert_eq!(process.fcalls, 0);
}

#[test]
fn returns_a_runtime_error_for_the_negative_budget_of_a_bif() {
    let mut process = ProcessState::new(ProcessId(0), Start::Entry(frame()), vec![]);
    process.fcalls = 0;
    assert_eq!(process.bif_reductions_left(), Ok(0));
    process.fcalls = -1;
    let failure = process.bif_reductions_left().unwrap_err().to_string();
    assert!(failure.starts_with("unsupported negative reduction budget in a BIF"));
    assert!(failure.contains("\"Map iteration\""));
}

#[test]
fn keeps_the_mfa_of_an_undefined_start_without_a_frame() {
    let module = Atom::from_static("erlang").unwrap();
    let function = Atom::from_static("ok").unwrap();
    let start = Start::Undefined {
        module: module.clone(),
        function: function.clone(),
    };
    let process = ProcessState::new(ProcessId(1), start, vec![Term::Integer(1)]);
    assert!(process.frames.is_empty());
    assert_eq!(process.undefined_start, Some((module, function)));
    assert_eq!(process.registers, vec![Some(Term::Integer(1))]);
}

/// Return a term of one integer, for a test that tells the terms of a
/// process apart.
///
/// # Examples
///
/// ```rust,ignore
/// assert_eq!(marker(1), Term::tuple(vec![Term::Integer(1)]));
/// ```
fn marker(number: i64) -> Term {
    Term::tuple(vec![Term::Integer(number)])
}

#[test]
fn takes_each_term_of_a_process_and_leaves_no_term() {
    // Row 11 of docs/term-performance-design-codex-v2.md: the scheduler moves
    // each term of an exiting process to its release work list, so that the
    // walk of a large graph runs in bounded scheduler work. A term that this
    // reader forgets drops in the exit step and not in a release batch.
    let mut process = ProcessState::new(ProcessId(0), Start::Entry(frame()), vec![]);
    let register = marker(1);
    let stack = marker(2);
    let message = marker(3);
    let dictionary = marker(4);
    let pending = marker(5);
    let length_trap = marker(6);
    let exiting = marker(7);
    let item = marker(8);
    let tag = marker(9);
    process.registers = vec![Some(register.clone())];
    // The frame holds each kind of slot, so that the reader covers a slot of
    // no term and a catch tag as well as a term.
    process.frames[0].stack = vec![
        StackSlot::Empty,
        StackSlot::Catch(1),
        StackSlot::Term(stack.clone()),
    ];
    process.mailbox.push(message.clone());
    process
        .dictionary
        .push((Term::Integer(10), dictionary.clone()));
    process.pending_bif = Some(PendingBif {
        module: "erlang",
        function: "length",
        arguments: vec![pending.clone()],
        tail: false,
    });
    process.length_trap = Some(vec![length_trap.clone()]);
    process.exiting = Some(exiting.clone());
    process.monitors.insert(
        Reference(1),
        Origin {
            monitor: Some(OriginMonitor {
                kind: "process",
                target: None,
                item: item.clone(),
                tag: tag.clone(),
            }),
            alias: None,
        },
    );
    // An alias without a monitor holds no term, so the reader skips it.
    process.monitors.insert(
        Reference(2),
        Origin {
            monitor: None,
            alias: None,
        },
    );
    let mut pending_terms = Vec::new();
    process.take_terms(&mut pending_terms);
    for term in [
        &register,
        &stack,
        &message,
        &dictionary,
        &pending,
        &length_trap,
        &exiting,
        &item,
        &tag,
    ] {
        assert!(
            pending_terms.contains(term),
            "{term:?} must move to the release work list"
        );
    }
    assert_eq!(pending_terms.len(), 10);
    // The process owns no term afterwards, so its drop releases nothing.
    assert!(process.registers.is_empty());
    assert!(process.frames.is_empty());
    assert!(process.dictionary.is_empty());
    assert!(process.pending_bif.is_none());
    assert!(process.length_trap.is_none());
    assert!(process.exiting.is_none());
    assert_eq!(
        process.monitors.get(&Reference(1)).unwrap().monitor,
        Some(OriginMonitor {
            kind: "process",
            target: None,
            item: Term::Nil,
            tag: Term::Nil,
        })
    );
}

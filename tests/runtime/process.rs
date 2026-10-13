use std::{collections::HashMap, rc::Rc};

use super::{Frame, ProcessState, ReceiveTimer, Start};
use crate::{
    BeamModule,
    beam::code::Instruction,
    runtime::{interpreter::CodeTable, scheduler::Scheduler},
    term::{Atom, AtomTable, ProcessId, Term},
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
    ).expect("the fixture module loads"));
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

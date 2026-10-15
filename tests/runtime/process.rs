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
        Vec::new(),
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

#[test]
fn reports_the_bytes_of_the_frame_slots() {
    let mut frame = frame();
    assert_eq!(frame.allocated_bytes(), 0);
    frame.stack.push(StackSlot::Term(Term::Integer(1)));
    assert!(frame.allocated_bytes() > 0);
}

#[test]
fn reports_the_bytes_of_the_process_containers_and_its_terms() {
    let mut process =
        ProcessState::new(ProcessId(0), Start::Entry(frame()), vec![Term::Integer(1)]);
    assert!(process.allocated_bytes() > 0);
    let mut live = crate::term::LiveBytes::new();
    process.account(&mut live);
    assert!(live.bytes() >= process.allocated_bytes());
    // A term in the dictionary, the mailbox, and a frame slot is part of the
    // walk.
    process
        .dictionary
        .push((Term::Integer(2), Term::tuple(vec![Term::Integer(3)])));
    assert!(process.dictionary_bytes() > 0);
    process.mailbox.push(Term::tuple(vec![Term::Integer(4)]));
    let mut frame = frame();
    frame
        .stack
        .push(StackSlot::Term(Term::tuple(vec![Term::Integer(5)])));
    process.frames.push(frame);
    let mut live = crate::term::LiveBytes::new();
    process.account(&mut live);
    assert!(live.bytes() > 0);
}

#[test]
fn walks_the_terms_of_the_monitors_the_pending_bif_and_the_length_trap() {
    let mut process = ProcessState::new(ProcessId(0), Start::Entry(frame()), vec![]);
    process.monitors.insert(
        Reference(1),
        Origin {
            monitor: Some(OriginMonitor {
                kind: "process",
                target: Some(ProcessId(1)),
                item: Term::tuple(vec![Term::Integer(1)]),
                tag: Term::static_atom("DOWN"),
            }),
            alias: None,
        },
    );
    process.pending_bif = Some(PendingBif {
        module: "lists",
        function: "member",
        arguments: vec![Term::tuple(vec![Term::Integer(2)])],
        tail: false,
    });
    process.length_trap = Some(vec![Term::tuple(vec![Term::Integer(3)])]);
    let mut live = crate::term::LiveBytes::new();
    process.account(&mut live);
    assert!(live.bytes() > 0);
}

#[test]
fn reports_the_bytes_of_the_pending_bif_arguments_and_the_length_trap() {
    let mut process = ProcessState::new(ProcessId(0), Start::Entry(frame()), vec![]);
    assert_eq!(process.pending_bif_bytes(), 0);
    assert_eq!(process.length_trap_bytes(), 0);
    let before = process.allocated_bytes();
    process.pending_bif = Some(PendingBif {
        module: "lists",
        function: "member",
        arguments: vec![Term::Integer(1), Term::Integer(2)],
        tail: false,
    });
    let arguments = process.pending_bif.as_ref().unwrap().arguments.capacity();
    assert_eq!(
        process.pending_bif_bytes(),
        arguments * std::mem::size_of::<Term>()
    );
    assert_eq!(
        process.allocated_bytes(),
        before + arguments * std::mem::size_of::<Term>()
    );
    process.length_trap = Some(vec![Term::Integer(1), Term::nil()]);
    let trap = process.length_trap.as_ref().unwrap().capacity();
    assert_eq!(
        process.length_trap_bytes(),
        trap * std::mem::size_of::<Term>()
    );
}

#[test]
fn walks_the_exit_reason_of_an_exiting_process() {
    let mut process = ProcessState::new(ProcessId(0), Start::Entry(frame()), vec![]);
    let before = {
        let mut live = crate::term::LiveBytes::new();
        process.account(&mut live);
        live.bytes()
    };
    process.exiting = Some(Term::tuple(vec![Term::Integer(1)]));
    let after = {
        let mut live = crate::term::LiveBytes::new();
        process.account(&mut live);
        live.bytes()
    };
    assert!(after > before);
}

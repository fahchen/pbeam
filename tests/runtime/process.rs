use super::{Frame, ProcessState, ReceiveTimer, Start};
use crate::term::{Atom, ProcessId, Term};

pub(in crate::runtime) fn frame() -> Frame {
    Frame {
        module: "fixture".into(),
        function: "main".into(),
        next_instruction: 0,
        stack: Vec::new(),
    }
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

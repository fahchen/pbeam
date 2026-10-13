//! Unit tests of the term walk of `src/runtime/signal.rs`.

use super::Signal;
use crate::runtime::process::LinkId;
use crate::term::{ProcessId, Term};

/// Return the terms of `signal` in the order of [`Signal::visit_terms`].
fn terms(signal: &Signal) -> Vec<Term> {
    let mut terms = Vec::new();
    signal.visit_terms(&mut |term| terms.push(term.clone()));
    terms
}

#[test]
fn visits_the_terms_of_a_signal() {
    let exit = Signal::Exit {
        from: LinkId::Process(ProcessId(1)),
        reason: Term::Integer(1),
        linked: false,
        normal_kills: false,
        alias: None,
    };
    assert_eq!(terms(&exit), vec![Term::Integer(1)]);
    let down = Signal::MonitorDown {
        reference: crate::term::Reference(1),
        reason: Term::Integer(2),
    };
    assert_eq!(terms(&down), vec![Term::Integer(2)]);
    let info = Signal::ProcessInfo {
        requester: ProcessId(1),
        reference: crate::term::Reference(1),
        item: Term::Integer(3),
    };
    assert_eq!(terms(&info), vec![Term::Integer(3)]);
    let message = Signal::AliasMessage {
        alias: crate::term::Reference(1),
        message: Term::Integer(4),
    };
    assert_eq!(terms(&message), vec![Term::Integer(4)]);
    // A signal with no term gives no term.
    assert!(terms(&Signal::Flush).is_empty());
}

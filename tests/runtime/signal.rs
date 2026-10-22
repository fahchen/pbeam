//! The unit tests of the control signals of `src/runtime/signal.rs`.

use super::Signal;
use crate::runtime::process::LinkId;
use crate::term::{Atom, PortId, ProcessId, Reference, Term};

/// Return a term of one integer, for a test that tells terms apart.
///
/// # Examples
///
/// ```rust,ignore
/// assert_eq!(marker(1), Term::tuple(vec![Term::Integer(1)]));
/// ```
fn marker(number: i64) -> Term {
    Term::tuple(vec![Term::Integer(number)])
}

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
        reference: Reference(1),
        reason: Term::Integer(2),
    };
    assert_eq!(terms(&down), vec![Term::Integer(2)]);
    let info = Signal::ProcessInfo {
        requester: ProcessId(1),
        reference: Reference(1),
        item: Term::Integer(3),
    };
    assert_eq!(terms(&info), vec![Term::Integer(3)]);
    let message = Signal::AliasMessage {
        alias: Reference(1),
        message: Term::Integer(4),
    };
    assert_eq!(terms(&message), vec![Term::Integer(4)]);
    let check = Signal::CheckProcessCode {
        requester: ProcessId(1),
        tag: Term::Integer(5),
        module: Atom::from_static("erlang").unwrap(),
    };
    assert_eq!(terms(&check), vec![Term::Integer(5)]);
    // A signal with no term gives no term.
    assert!(terms(&Signal::Flush).is_empty());
}

#[test]
fn takes_the_term_of_each_signal_that_holds_one() {
    // The scheduler moves the term of each signal of the mailbox of an
    // exiting process to its release work list (`Signal::take_terms`, row 11
    // of docs/term-performance-design-codex-v2.md).
    for (mut signal, expected) in [
        (
            Signal::Exit {
                from: LinkId::Process(ProcessId(1)),
                reason: marker(1),
                linked: false,
                normal_kills: false,
                alias: None,
            },
            marker(1),
        ),
        (
            Signal::MonitorDown {
                reference: Reference(1),
                reason: marker(2),
            },
            marker(2),
        ),
        (
            Signal::ProcessInfo {
                requester: ProcessId(1),
                reference: Reference(1),
                item: marker(3),
            },
            marker(3),
        ),
        (
            Signal::AliasMessage {
                alias: Reference(1),
                message: marker(4),
            },
            marker(4),
        ),
    ] {
        let mut pending = Vec::new();
        signal.take_terms(&mut pending);
        assert_eq!(pending, vec![expected]);
    }
}

#[test]
fn takes_no_term_of_a_signal_that_holds_none() {
    for mut signal in [
        Signal::Link { from: ProcessId(1) },
        Signal::Unlink {
            from: ProcessId(1),
            id: 1,
        },
        Signal::UnlinkAck {
            from: ProcessId(1),
            id: 1,
        },
        Signal::Monitor {
            watcher: ProcessId(1),
            reference: Reference(1),
        },
        Signal::TtyMonitor { tty: Reference(1) },
        Signal::FileMonitor { file: Reference(1) },
        Signal::PortMonitor {
            port: PortId(1),
            reference: Reference(1),
        },
        Signal::PortDemonitor {
            port: PortId(1),
            reference: Reference(1),
        },
        Signal::Demonitor {
            watcher: ProcessId(1),
            reference: Reference(1),
        },
        Signal::GroupLeader {
            leader: ProcessId(1),
            requester: ProcessId(2),
            reference: Reference(1),
        },
        Signal::IsAlive {
            requester: ProcessId(1),
            reference: Reference(1),
        },
        Signal::Flush,
        Signal::Grant { capability: None },
        Signal::EnterRuntime {
            capabilities: Vec::new(),
        },
    ] {
        let mut pending = Vec::new();
        signal.take_terms(&mut pending);
        assert!(pending.is_empty(), "{signal:?} holds no term");
    }
}

//! The unit tests of the control signals of `src/runtime/signal.rs`.

use super::Signal;
use crate::runtime::process::{LinkId, MonitorKey, RemotePid, RemoteReference};
use crate::term::{Atom, PortId, ProcessId, Reference, Term};

/// Return a reference of another node with the number `number`.
///
/// # Examples
///
/// ```rust,ignore
/// assert_eq!(remote_reference(1).numbers(), &[1]);
/// ```
fn remote_reference(number: u32) -> RemoteReference {
    RemoteReference {
        node: 1,
        creation: 1,
        numbers: [number, 0, 0, 0, 0],
        count: 1,
    }
}

/// Return a pid of another node with the number `number`.
///
/// # Examples
///
/// ```rust,ignore
/// assert_eq!(remote_pid(2).number(), 2);
/// ```
fn remote_pid(number: u32) -> RemotePid {
    RemotePid {
        node: 1,
        creation: 1,
        number,
        serial: 0,
    }
}

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
    let dist_exit = Signal::DistExit {
        from: Term::Pid(ProcessId(1)),
        reason: Term::Integer(6),
    };
    assert_eq!(terms(&dist_exit), vec![Term::Integer(6)]);
    let down = Signal::MonitorDown {
        reference: MonitorKey::Local(Reference(1)),
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
    let node_monitor = Signal::NodeMonitorDown {
        node: Atom::from_static("nonode@nohost").unwrap(),
        connection: 1,
    };
    assert_eq!(
        terms(&node_monitor),
        vec![Term::static_atom("nonode@nohost")]
    );
    let persistent_monitor = Signal::NodesMonitor {
        options: 0,
        message: Term::Integer(7),
    };
    assert_eq!(terms(&persistent_monitor), vec![Term::Integer(7)]);
    let check = Signal::CheckProcessCode {
        requester: ProcessId(1),
        tag: Term::Integer(5),
        module: Atom::from_static("erlang").unwrap(),
    };
    assert_eq!(terms(&check), vec![Term::Integer(5)]);
    // A signal with no term gives no term.
    assert_eq!(terms(&Signal::Flush), <[crate::term::Term; 0]>::default());
}

#[test]
fn takes_the_term_of_each_signal_that_holds_one() {
    // The scheduler moves the term of each signal of the mailbox of an
    // exiting process to its release work list (`Signal::take_terms`, row 11
    // of docs/term-performance-design-codex-v2.md).
    let node = Atom::from_static("nonode@nohost").unwrap();
    for (mut signal, expected) in [
        (
            Signal::Exit {
                from: LinkId::Process(ProcessId(1)),
                reason: marker(1),
                linked: false,
                normal_kills: false,
                alias: None,
            },
            vec![marker(1)],
        ),
        (
            Signal::DistExit {
                from: Term::Pid(ProcessId(1)),
                reason: marker(5),
            },
            vec![marker(5)],
        ),
        (
            Signal::MonitorDown {
                reference: MonitorKey::Local(Reference(1)),
                reason: marker(2),
            },
            vec![marker(2)],
        ),
        (
            Signal::ProcessInfo {
                requester: ProcessId(1),
                reference: Reference(1),
                item: marker(3),
            },
            vec![marker(3)],
        ),
        (
            Signal::AliasMessage {
                alias: Reference(1),
                message: marker(4),
            },
            vec![marker(4)],
        ),
        (
            Signal::NodeMonitorDown {
                node: node.clone(),
                connection: 1,
            },
            vec![Term::Atom(node.clone())],
        ),
        (
            Signal::NodesMonitor {
                options: 0,
                message: marker(7),
            },
            vec![marker(7)],
        ),
        // A spawn reply holds the node atom of the request and the pid or
        // the error atom of the result, in that order
        // (`dist.c:3095-3129`).
        (
            Signal::SpawnReply {
                node: node.clone(),
                reference: MonitorKey::Local(Reference(1)),
                flags: 0,
                result: marker(6),
                connection: 1,
            },
            vec![Term::Atom(node), marker(6)],
        ),
    ] {
        let mut pending = Vec::new();
        signal.take_terms(&mut pending);
        assert_eq!(pending, expected);
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
            reference: MonitorKey::Local(Reference(1)),
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
            reference: MonitorKey::Remote(remote_reference(1)),
        },
        Signal::DistMonitor {
            watcher: remote_pid(2),
            reference: MonitorKey::Remote(remote_reference(1)),
            name: None,
            connection: 1,
        },
        Signal::DistDemonitor {
            watcher: remote_pid(2),
            reference: MonitorKey::Remote(remote_reference(1)),
        },
        Signal::GroupLeader {
            leader: Term::Pid(ProcessId(1)),
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

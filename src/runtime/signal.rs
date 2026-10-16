//! Control signals: the non-message signals that a process sends to another
//! process, or to itself, through the signal queue of the receiver
//! (`erl_proc_sig_queue.c`).
//!
//! A signal goes to the end of the signal queue of its receiver when it is
//! sent, after the messages and signals that came before it. The receiver
//! handles it later, in queue order, when it handles its signals: at
//! schedule-in, in `loop_rec`, in a BIF that flushes the signals of the
//! caller, or at its exit (see `Scheduler::handle_signals` and
//! `Scheduler::exit_links`).
//!
//! Each variant names its ERTS operation, the extra cost of its handling in
//! the signal count of `erts_proc_sig_handle_incoming`, and its result when
//! the receiver exits before it handles the signal
//! (`erts_proc_sig_handle_exit`, `erl_proc_sig_queue.c:7259-7473`). Each
//! handled signal also costs 1 (`erl_proc_sig_queue.c:6968`).

use super::{
    capability::{Capability, CapabilityId},
    process::LinkId,
};
use crate::term::{PortId, ProcessId, Reference, Term};

/// A non-message signal in the signal queue of a process.
#[derive(Debug, PartialEq)]
pub(super) enum Signal {
    /// `ERTS_SIG_Q_OP_EXIT` from `erlang:exit/2`, or `ERTS_SIG_Q_OP_EXIT_LINKED`
    /// from the exit of a linked process (`send_gen_exit_signal`,
    /// `erl_proc_sig_queue.c:1627-1840`). `handle_exit_signal` costs 1, 1
    /// more when the signal ends the receiver, and 1 more when the receiver
    /// ignores it (`erl_proc_sig_queue.c:4632-4815`). At exit it is dropped.
    Exit {
        /// The sender, the second element of the `EXIT` message: a process,
        /// or a port for the link exit signal of a port.
        from: LinkId,
        /// The exit reason.
        reason: Term,
        /// True for `ERTS_SIG_Q_OP_EXIT_LINKED`: the receiver acts only when
        /// it has an active link to `from`, and removes the link.
        linked: bool,
        /// True for the exit signal of `exit(self(), Reason)` that ends the
        /// caller (`exit2_suicide`): `normal` then also ends the receiver
        /// (`ERTS_SIG_LNK_X_FLAG_NORMAL_KILLS`).
        normal_kills: bool,
        /// The alias that the signal went through, for `exit(Alias, Reason)`.
        /// The receiver ignores the signal when the alias is not active
        /// (`erl_proc_sig_queue.c:4659-4672`).
        alias: Option<Reference>,
    },
    /// `ERTS_SIG_Q_OP_LINK`: the sender linked to the receiver. The receiver
    /// adds the link unless it has a link or an unlinking link to the
    /// sender; the cost is 0 (`erl_proc_sig_queue.c:6635-6663`). At exit it
    /// gives a link exit signal `noproc` to the sender
    /// (`erl_proc_sig_queue.c:7363-7368`).
    Link {
        /// The process that linked.
        from: ProcessId,
    },
    /// `ERTS_SIG_Q_OP_UNLINK`: the sender removed its link. The receiver
    /// removes an active link for 4 and answers with `UnlinkAck`
    /// (`erl_proc_sig_queue.c:6665-6731`). At exit it also answers with
    /// `UnlinkAck` (`erl_proc_sig_queue.c:7369-7386`).
    Unlink {
        /// The process that unlinked.
        from: ProcessId,
        /// The unlink operation identifier (`erts_proc_sig_new_unlink_id`).
        id: u64,
    },
    /// `ERTS_SIG_Q_OP_UNLINK_ACK`: the answer to `Unlink`. The receiver
    /// removes its unlinking link with the same identifier for 4
    /// (`erl_proc_sig_queue.c:6733-6776`). At exit it is dropped.
    UnlinkAck {
        /// The process that handled the `Unlink`.
        from: ProcessId,
        /// The identifier of the `Unlink`.
        id: u64,
    },
    /// `ERTS_SIG_Q_OP_MONITOR`: the watcher monitors the receiver. The
    /// receiver adds the target monitor for 2 (`erl_proc_sig_queue.c:6544-6562`).
    /// At exit it gives a monitor down signal `noproc` to the watcher
    /// (`erl_proc_sig_queue.c:7347-7355`).
    Monitor {
        /// The process that gets the `DOWN` message.
        watcher: ProcessId,
        /// The reference of the monitor.
        reference: Reference,
    },
    /// `ERTS_SIG_Q_OP_MONITOR` of a resource monitor: `enif_monitor_process`
    /// (`erl_nif.c:3957-3989`) in `prim_tty:tty_init/2` or
    /// `prim_tty:tty_select/2` monitors the caller for the TTY resource. The
    /// receiver adds the target monitor for 2 (`erl_proc_sig_queue.c:6544-6562`).
    /// At exit it runs the TTY monitor (`erl_proc_sig_queue.c:7347-7355`;
    /// `erl_process.c:13827-13833`).
    TtyMonitor {
        /// The reference of the TTY resource.
        tty: Reference,
    },
    /// `ERTS_SIG_Q_OP_LINK` from a port: `erlang:port_connect/2` linked
    /// the port `port` to the receiver (io.c:2355-2380). The receiver adds
    /// the link unless it has one; the cost is 0. At exit it gives a link
    /// exit signal `noproc` to the port (`erl_proc_sig_queue.c:7363-7368`).
    PortLink {
        /// The port that linked.
        port: PortId,
    },
    /// `ERTS_SIG_Q_OP_MONITOR` of a port monitor: `driver_monitor_process`
    /// (`io.c`) in `INET_REQ_ACCEPT` monitors the caller for the listen
    /// port `port` (inet_drv.c:12172, 12198, 12214). The receiver adds the
    /// target monitor for 2 (`erl_proc_sig_queue.c:6544-6562`). At exit it
    /// runs the monitor of the port (`tcp_inet_process_exit`,
    /// inet_drv.c:12732-12771).
    PortMonitor {
        /// The port that holds the monitor.
        port: PortId,
        /// The reference of the monitor.
        reference: Reference,
    },
    /// `ERTS_SIG_Q_OP_DEMONITOR` of a port monitor: the port removed its
    /// monitor at the end of an accept (`driver_demonitor_process`). The
    /// receiver removes the target monitor; the cost is 1, and 1 more when
    /// the monitor was there (`erl_proc_sig_queue.c:6564-6633`). At exit it
    /// is dropped.
    PortDemonitor {
        /// The port that removed the monitor.
        port: PortId,
        /// The reference of the monitor.
        reference: Reference,
    },
    /// `ERTS_SIG_Q_OP_DEMONITOR`: the watcher removed its monitor of the
    /// receiver. The receiver removes the target monitor; the cost is 1, and
    /// 1 more when the monitor was there (`erl_proc_sig_queue.c:6564-6633`).
    /// At exit it is dropped.
    Demonitor {
        /// The process that removed the monitor.
        watcher: ProcessId,
        /// The reference of the monitor.
        reference: Reference,
    },
    /// `ERTS_SIG_Q_OP_MONITOR_DOWN`: the monitored target exited, or it did
    /// not exist. The receiver, which is the watcher, makes the `DOWN`
    /// message from its origin monitor and puts it in the place of the
    /// signal; it costs 8 for an immediate reason (`convert_to_down_message`,
    /// `erl_proc_sig_queue.c:4837-5060`) and 1 for another reason
    /// (`convert_prepared_down_message`, `erl_proc_sig_queue.c:4817-4835`).
    /// The receiver drops the signal for 0 when it has no such origin
    /// monitor (`erl_proc_sig_queue.c:6370-6381`, `6449-6457`). At exit it is
    /// dropped.
    MonitorDown {
        /// The reference of the monitor.
        reference: Reference,
        /// The exit reason of the target, or `noproc`.
        reason: Term,
    },
    /// `ERTS_SIG_Q_OP_GROUP_LEADER` from `erts_internal:group_leader/3`: the
    /// receiver sets its group leader and replies `{Ref, true}`; the cost is
    /// 0 (`handle_group_leader`, `erl_proc_sig_queue.c:5295-5310`). At exit
    /// it replies `{Ref, badarg}` (`erl_proc_sig_queue.c:7394-7399`).
    GroupLeader {
        /// The new group leader.
        leader: ProcessId,
        /// The process that gets the reply.
        requester: ProcessId,
        /// The reference of the reply.
        reference: Reference,
    },
    /// `ERTS_SIG_Q_OP_IS_ALIVE` from `erts_internal:is_process_alive/2`: the
    /// receiver replies `{Ref, true}`; the cost is 0 (`is_alive_response`,
    /// `erl_proc_sig_queue.c:3625-3660`). At exit it replies `{Ref, false}`.
    IsAlive {
        /// The process that gets the reply.
        requester: ProcessId,
        /// The reference of the reply.
        reference: Reference,
    },
    /// `ERTS_SIG_Q_OP_PROCESS_INFO` from `erlang:process_info/2`: the
    /// receiver replies `{Ref, Result}` with the item of itself; the cost is
    /// 0 (`handle_process_info`, `erl_proc_sig_queue.c:5312-5420`). At exit
    /// it replies `{Ref, undefined}`.
    ProcessInfo {
        /// The process that gets the reply.
        requester: ProcessId,
        /// The reference of the reply.
        reference: Reference,
        /// The item, which `erlang:process_info/2` checked.
        item: Term,
    },
    /// `ERTS_SIG_Q_OP_ALTACT_MSG`: a message through an alias. The receiver
    /// puts the message in the place of the signal for 1 when the alias is
    /// active, else it drops the signal for 2 (`handle_altact_msg`,
    /// `erl_proc_sig_queue.c:6009-6221`). At exit it is dropped.
    AliasMessage {
        /// The alias that the message went through.
        alias: Reference,
        /// The message.
        message: Term,
    },
    /// `ERTS_SIG_Q_OP_FLUSH`: a BIF of the receiver handles its signals up to
    /// this signal (`erts_proc_sig_init_flush_signals`,
    /// `erl_proc_sig_queue.c:3520-3557`). It costs 0 and stops the handling
    /// (`erl_proc_sig_queue.c:6853-6865`).
    Flush,
    /// PBeam only: revoke the capability copy `capability` of the receiver.
    /// ERTS has no capabilities and no such signal. The receiver deletes its
    /// copy and sends a revocation signal to each live downstream copy, also
    /// through a holder that exited (`Delegations::revoke`); the cost is 0,
    /// as for the other signals that change only the receiver. At exit it
    /// sends the revocation on in the same way, because the receiver
    /// released its copy when it left the process table.
    Revoke {
        /// The copy that the receiver holds.
        capability: CapabilityId,
    },
    /// PBeam only: the answer of the policy process to the
    /// `pbeam_cap:request/2` call of the receiver. ERTS has no capabilities
    /// and no such signal. The receiver adds a copy of the granted
    /// capability, with the source `policy`, to its security context, and
    /// its call continues; the cost is 0, as for `Revoke`. At exit it is
    /// dropped, so the copy has no delegation record.
    Grant {
        /// The capability that the policy granted, or `None` for a denial.
        capability: Option<Capability>,
    },
    /// PBeam only: the answer of `initial/1` of the policy process to the
    /// `pbeam_cap:enter_runtime/2` call of the root process in boot mode.
    /// ERTS has no capabilities and no such signal. The receiver replaces
    /// its boot authority with a copy of each capability, with the source
    /// `launcher`, one way, and its call continues; the cost is 0, as for
    /// `Grant`. At exit it is dropped. A denial of `initial/1` stops the run
    /// and sends no signal.
    EnterRuntime {
        /// The capabilities that `initial/1` gave.
        capabilities: Vec<Capability>,
    },
}

impl Signal {
    /// Call `visit` for each guest term that the signal holds: the exit
    /// reason of an exit or a monitor down signal, the item of a
    /// `process_info/2` signal, and the message of an alias signal.
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// let mut terms = Vec::new();
    /// Signal::Exit { from: LinkId::Process(ProcessId(1)), reason: Term::nil(), linked: false, normal_kills: false, alias: None }
    ///     .visit_terms(&mut |term| terms.push(term.clone()));
    /// assert_eq!(terms, vec![Term::nil()]);
    /// ```
    pub(super) fn visit_terms(&self, visit: &mut impl FnMut(&Term)) {
        match self {
            Signal::Exit { reason, .. } => visit(reason),
            Signal::MonitorDown { reason, .. } => visit(reason),
            Signal::ProcessInfo { item, .. } => visit(item),
            Signal::AliasMessage { message, .. } => visit(message),
            _ => {}
        }
    }
}

impl Signal {
    /// Move each term of this signal to `pending`, and leave the signal with
    /// no term. The `Exit`, `MonitorDown`, `ProcessInfo`, and `AliasMessage`
    /// signals hold a term; the other signals hold identifiers, references,
    /// and capabilities. Only a signal that no guest code can read again
    /// gives up its terms (see
    /// [`Scheduler::release_process`](super::Scheduler::release_process)).
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// let mut signal = Signal::Exit { from: LinkId::Process(ProcessId(1)), reason: Term::Integer(1), linked: false, normal_kills: false, alias: None };
    /// let mut pending = Vec::new();
    /// signal.take_terms(&mut pending);
    /// assert_eq!(pending, vec![Term::Integer(1)]);
    /// ```
    pub(super) fn take_terms(&mut self, pending: &mut Vec<Term>) {
        match self {
            Signal::Exit {
                from: _,
                reason,
                linked: _,
                normal_kills: _,
                alias: _,
            } => pending.push(std::mem::replace(reason, Term::Nil)),
            Signal::MonitorDown {
                reference: _,
                reason,
            } => pending.push(std::mem::replace(reason, Term::Nil)),
            Signal::ProcessInfo {
                requester: _,
                reference: _,
                item,
            } => pending.push(std::mem::replace(item, Term::Nil)),
            Signal::AliasMessage { alias: _, message } => {
                pending.push(std::mem::replace(message, Term::Nil))
            }
            Signal::Link { from: _ }
            | Signal::Unlink { from: _, id: _ }
            | Signal::UnlinkAck { from: _, id: _ }
            | Signal::Monitor {
                watcher: _,
                reference: _,
            }
            | Signal::TtyMonitor { tty: _ }
            | Signal::PortLink { port: _ }
            | Signal::PortMonitor {
                port: _,
                reference: _,
            }
            | Signal::PortDemonitor {
                port: _,
                reference: _,
            }
            | Signal::Demonitor {
                watcher: _,
                reference: _,
            }
            | Signal::GroupLeader {
                leader: _,
                requester: _,
                reference: _,
            }
            | Signal::IsAlive {
                requester: _,
                reference: _,
            }
            | Signal::Flush
            | Signal::Revoke { capability: _ }
            | Signal::Grant { capability: _ }
            | Signal::EnterRuntime { capabilities: _ } => {}
        }
    }
}

#[cfg(test)]
#[path = "../../tests/runtime/signal.rs"]
mod tests;

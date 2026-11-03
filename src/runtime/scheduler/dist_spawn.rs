//! Spawn requests to another node (`dist.c:5850-6260`, `erl_process.c`).

use super::Scheduler;
use crate::{
    error::BeamError,
    runtime::{
        process::{MonitorKey, Origin, OriginMonitor},
        signal::Signal,
    },
    term::{Atom, ProcessId, Reference, Term},
};

/// `DOP_SPAWN_REQUEST` (`dist.h:171`).
const SPAWN_REQUEST: i64 = 29;
/// `DOP_SPAWN_REPLY` (`dist.h:173`).
const SPAWN_REPLY: i64 = 31;
/// `DFLAG_SPAWN` (`dist.h:64`).
const DIST_SPAWN: u64 = 1 << 32;
/// `ERTS_DIST_SPAWN_FLAG_LINK` (`dist.h:186`).
pub(in crate::runtime) const SPAWN_LINK: i64 = 1;
/// `ERTS_DIST_SPAWN_FLAG_MONITOR` (`dist.h:187`).
pub(in crate::runtime) const SPAWN_MONITOR: i64 = 2;

impl Scheduler {
    /// Start a spawn request of `module:function` with `arity` arguments of
    /// node `node`, as `erts_internal_dist_spawn_request_4` does
    /// (`dist.c:5850-6120`): make the reference, keep a pending monitor when
    /// `monitor` is set, and send `DOP_SPAWN_REQUEST` with the argument list
    /// as the message. Return the reference and whether a monitor is set.
    ///
    /// # Errors
    ///
    /// The errors of the encode of the control message.
    #[expect(clippy::too_many_arguments, reason = "the fields of the request")]
    pub(in crate::runtime) fn dist_spawn_request(
        &mut self,
        node: &Atom,
        module: &Atom,
        function: &Atom,
        arity: i64,
        arguments: &Term,
        options: Term,
        monitor: Option<&Term>,
        reply_tag: &Term,
    ) -> Result<(Reference, bool), BeamError> {
        let watcher = self.current_process_mut().id;
        let reference = self.make_ref()?;
        let key = MonitorKey::Local(reference);
        let pending = Origin {
            monitor: Some(OriginMonitor {
                kind: "process",
                target: None,
                item: reply_tag.clone(),
                tag: monitor
                    .cloned()
                    .unwrap_or_else(|| Term::static_atom("DOWN")),
                connection: 0,
                remote: None,
            }),
            alias: None,
            watched_by: None,
        };
        self.current_process_mut().monitors.insert(key, pending);
        let from = Term::Pid(watcher);
        let group_leader = Term::Pid(self.current_process_mut().group_leader);
        let mfa = Term::tuple(vec![
            Term::Atom(module.clone()),
            Term::Atom(function.clone()),
            Term::Integer(arity),
        ]);
        let key_term = Term::Reference(reference);
        let control = Term::tuple(vec![
            Term::Integer(SPAWN_REQUEST),
            key_term,
            from,
            group_leader,
            mfa,
            options,
        ]);
        let sent = self.dist_spawn_output(watcher, node, &control, arguments)?;
        if !sent {
            self.spawn_reply_error(watcher, key, reply_tag, Term::static_atom("noconnection"));
        }
        Ok((reference, monitor.is_some()))
    }

    /// Give the caller of a spawn request that names a bad option the reply
    /// `{Tag, Ref, error, badopt}` and return the reference
    /// (`dist.c:5955-5975`).
    ///
    /// # Errors
    ///
    /// The errors of the reference allocation.
    pub(in crate::runtime) fn dist_spawn_badopt(
        &mut self,
        reply_tag: &Term,
    ) -> Result<Reference, BeamError> {
        let caller = self.current_process().id;
        let reference = self.make_ref()?;
        let message = Term::tuple(vec![
            reply_tag.clone(),
            Term::Reference(reference),
            Term::static_atom("error"),
            Term::static_atom("badopt"),
        ]);
        self.send_message(caller, caller, message);
        Ok(reference)
    }

    /// Send the spawn request `control` with the message `arguments` on the
    /// connection of `node`, and return whether it went out.
    fn dist_spawn_output(
        &mut self,
        sender: ProcessId,
        node: &Atom,
        control: &Term,
        arguments: &Term,
    ) -> Result<bool, BeamError> {
        let mut sent = false;
        self.dist_output(sender, node, |flags, _| {
            sent = flags & DIST_SPAWN != 0;
            sent.then(|| (control.clone(), Some(arguments.clone())))
        })
    }

    /// Give the caller of a spawn request the message `{Tag, Ref, error,
    /// Reason}` and remove the pending monitor, as a failed request does
    /// (`dist.c:6111-6121`).
    pub(super) fn spawn_reply_error(
        &mut self,
        process: ProcessId,
        key: MonitorKey,
        tag: &Term,
        reason: Term,
    ) {
        let reference = key.term();
        self.processes
            .get_mut(process)
            .map(|process| process.monitors.remove(&key));
        let message = Term::tuple(vec![
            tag.clone(),
            reference,
            Term::static_atom("error"),
            reason,
        ]);
        self.send_message(process, process, message);
    }
}

impl Scheduler {
    /// Start the process of a spawn request on this node, as
    /// `erl_create_process` with `dist_spawn_init` does
    /// (`dist.c:2980-3030`): the start is `module:function` with `arguments`,
    /// the group leader is `group_leader`, and no local parent exists.
    ///
    /// # Errors
    ///
    /// The errors of the export lookup of the start.
    pub(super) fn spawn_dist(
        &mut self,
        module: &Atom,
        function: &Atom,
        arguments: Vec<Term>,
        group_leader: Option<ProcessId>,
    ) -> Result<ProcessId, BeamError> {
        let start =
            crate::runtime::interpreter::initial_start(self, module, function, arguments.len())?;
        let mut child =
            crate::runtime::process::ProcessState::new(self.next_pid()?, start, arguments);
        child.parent = None;
        child.group_leader = group_leader.unwrap_or_else(|| {
            self.processes
                .get(super::ROOT)
                .map_or(child.id, |root| root.group_leader)
        });
        child.security = crate::runtime::capability::SecurityContext::boot();
        let id = child.id;
        self.insert(child);
        Ok(id)
    }

    /// Receive a `DOP_SPAWN_REQUEST` packet (`dist.c:2902-3032`): check the
    /// request, start the process, and answer `DOP_SPAWN_REPLY` with the pid
    /// or with the error.
    ///
    /// # Errors
    ///
    /// A request that ERTS rejects, and the errors of the encode of the
    /// answer.
    #[expect(clippy::too_many_arguments, reason = "the fields of the packet")]
    pub(super) fn net_spawn_request(
        &mut self,
        node: &Atom,
        connection: u32,
        reference: &Term,
        from: &Term,
        group_leader: &Term,
        mfa: &Term,
        options: &Term,
        arguments: &Term,
    ) -> Result<(), BeamError> {
        let Some(key) = super::dist_monitor::monitor_key(reference) else {
            return Err(crate::error::error(super::dist_input::REJECTED));
        };
        let Ok(_) = super::dist_monitor::remote_of(node, from) else {
            return Err(crate::error::error(super::dist_input::REJECTED));
        };
        let group_leader = match group_leader {
            Term::Pid(id) => Some(*id),
            // A group leader of another node is not a local pid; PBeam gives
            // the process the group leader of the root process and records
            // the gap in row "Distribution".
            Term::External(_) if crate::term::external_id::pid_parts(group_leader).is_some() => {
                None
            }
            _ => return Err(crate::error::error(super::dist_input::REJECTED)),
        };
        let Term::Tuple(triple) = mfa else {
            return Err(crate::error::error(super::dist_input::REJECTED));
        };
        let [Term::Atom(module), Term::Atom(function), Term::Integer(_)] = triple.as_ref() else {
            return Err(crate::error::error(super::dist_input::REJECTED));
        };
        let Some(arguments) = arguments.proper_list() else {
            return Err(crate::error::error(super::dist_input::REJECTED));
        };
        let arguments: Vec<Term> = arguments.cloned().collect();
        let mut link = false;
        let mut monitor = false;
        if let Some(options) = options.proper_list() {
            for option in options {
                match option {
                    Term::Atom(name) if name.is_static("link") => link = true,
                    Term::Atom(name) if name.is_static("monitor") => monitor = true,
                    _ => {}
                }
            }
        }
        let spawned = self.spawn_dist(module, function, arguments, group_leader);
        if let Ok(pid) = spawned {
            let remote = super::dist_monitor::remote_of(node, from)
                .expect("the request names a pid of the node");
            if link {
                self.link_on(
                    pid,
                    crate::runtime::process::LinkId::Remote(remote),
                    remote,
                    connection,
                );
            }
            if monitor {
                let key = crate::runtime::process::MonitorKey::Remote(
                    crate::runtime::process::RemoteReference::of(reference)
                        .expect("the request names a reference of the node"),
                );
                let watcher = crate::runtime::process::RemoteWatcher {
                    watcher: remote,
                    name: None,
                    connection,
                };
                self.processes
                    .get_mut(pid)
                    .expect("the new process is alive")
                    .monitors
                    .insert(
                        key,
                        crate::runtime::process::Origin {
                            monitor: None,
                            alias: None,
                            watched_by: Some(watcher),
                        },
                    );
                self.node.dist().insert_monitor(
                    pid,
                    node,
                    Some(remote),
                    key,
                    connection,
                    super::dist_table::MonitorSide::Watched,
                );
            }
        }
        let result = spawned.map_or_else(|_| Term::static_atom("system_limit"), Term::Pid);
        // The reply names the link and the monitor, so the caller joins them
        // (`dist.c:3033-3130`).
        let mut reply_flags = 0;
        if link {
            reply_flags |= SPAWN_LINK;
        }
        if monitor {
            reply_flags |= SPAWN_MONITOR;
        }
        let control = Term::tuple(vec![
            Term::Integer(SPAWN_REPLY),
            key.term(),
            from.clone(),
            Term::Integer(reply_flags),
            result,
        ]);
        self.dist_output(ProcessId(0), node, |_, current| {
            (current == connection).then(|| (control.clone(), None))
        })?;
        Ok(())
    }

    /// Receive a `DOP_SPAWN_REPLY` packet (`dist.c:3033-3130`): give the
    /// signal to the owner of the caller, which answers its
    /// `{Tag, Ref, ok | error, Result}` message.
    ///
    /// # Errors
    ///
    /// A reply that ERTS rejects.
    pub(super) fn net_spawn_reply(
        &mut self,
        node: &Atom,
        connection: u32,
        reference: &Term,
        to: &Term,
        flags: &Term,
        result: &Term,
    ) -> Result<(), BeamError> {
        let Term::Pid(process) = to else {
            return Err(crate::error::error(super::dist_input::REJECTED));
        };
        let Some(key) = super::dist_monitor::monitor_key(reference) else {
            return Err(crate::error::error(super::dist_input::REJECTED));
        };
        let Some(flags) = flags.small_integer() else {
            return Err(crate::error::error(super::dist_input::REJECTED));
        };
        if !matches!(result, Term::Atom(_))
            && crate::runtime::process::RemotePid::of(result).is_none()
        {
            return Err(crate::error::error(super::dist_input::REJECTED));
        }
        let signal = Signal::SpawnReply {
            node: node.clone(),
            reference: key,
            flags,
            result: result.clone(),
            connection,
        };
        let _ = self.send_signal(None, *process, signal);
        Ok(())
    }

    /// Receive a `DOP_GROUP_LEADER` packet (`dist.c:2889-2901`): make
    /// `leader` the group leader of the local process `to`.
    ///
    /// # Errors
    ///
    /// A term that is not a pid.
    pub(super) fn net_group_leader(&mut self, leader: &Term, to: &Term) -> Result<(), BeamError> {
        let (Term::Pid(leader), Term::Pid(to)) = (leader, to) else {
            return Err(crate::error::error(super::dist_input::REJECTED));
        };
        let signal = Signal::DistGroupLeader { leader: *leader };
        let _ = self.send_signal(None, *to, signal);
        Ok(())
    }
}

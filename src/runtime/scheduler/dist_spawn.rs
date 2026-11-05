//! Spawn requests to another node (`dist.c:5850-6260`, `erl_process.c`).

use super::Scheduler;
use crate::{
    error::BeamError,
    runtime::{
        process::{MonitorKey, Origin, OriginMonitor, Priority, SpawnReply, StartFlags},
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
    /// (`dist.c:5850-6120`): make the reference, keep a pending monitor with
    /// the reply policy, the `link` flag, and the `monitor` tag, and send
    /// `DOP_SPAWN_REQUEST` with the argument list as the message. Return the
    /// reference and whether a monitor is set.
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
        reply: SpawnReply,
        link: bool,
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
                spawn: Some(reply),
                abandoned: false,
                spawn_link: link,
            }),
            alias: None,
            watched_by: None,
        };
        self.current_process_mut().monitors.insert(key, pending);
        let from = Term::Pid(watcher);
        let group_leader = self.current_process_mut().group_leader.clone();
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
        match self.dist_spawn_output(watcher, node, key, &control, arguments)? {
            // The pending request joined the monitor list of its connection
            // before the send, so the end of the connection gives the caller
            // `noconnection` (`dist.c:6112-6120`,
            // `erl_proc_sig_queue.c:4864-4900`).
            SpawnSend::Sent => {}
            SpawnSend::NoConnection => {
                self.spawn_reply_error(
                    watcher,
                    key,
                    reply_tag,
                    Term::static_atom("noconnection"),
                    reply,
                );
            }
            SpawnSend::NoSupport => {
                self.spawn_reply_error(watcher, key, reply_tag, Term::static_atom("notsup"), reply);
            }
        }
        Ok((reference, monitor.is_some()))
    }

    /// Give the caller of a spawn request that names a bad option the reply
    /// `{Tag, Ref, error, badopt}` and return the reference
    /// (`dist.c:5955-5975`, `6204-6211`).
    ///
    /// # Errors
    ///
    /// The errors of the reference allocation.
    pub(in crate::runtime) fn dist_spawn_badopt(
        &mut self,
        reply_tag: &Term,
        reply: SpawnReply,
    ) -> Result<Reference, BeamError> {
        let caller = self.current_process().id;
        let reference = self.make_ref()?;
        if !reply.no_emsg {
            let message = Term::tuple(vec![
                reply_tag.clone(),
                Term::Reference(reference),
                Term::static_atom("error"),
                Term::static_atom("badopt"),
            ]);
            self.send_message(caller, caller, message);
        }
        Ok(reference)
    }

    /// Send the spawn request `control` with the message `arguments` on the
    /// connection of `node`, and return how the send ended.
    fn dist_spawn_output(
        &mut self,
        sender: ProcessId,
        node: &Atom,
        key: MonitorKey,
        control: &Term,
        arguments: &Term,
    ) -> Result<SpawnSend, BeamError> {
        let monitor = super::dist_table::DistMonitor {
            process: sender,
            remote: None,
            key,
            side: super::dist_table::MonitorSide::Watcher,
        };
        let mut connection = None;
        let sent = self.dist_output_monitor(sender, node, Some(monitor), |flags, current| {
            if flags & DIST_SPAWN == 0 {
                return None;
            }
            connection = Some(current);
            Some((control.clone(), Some(arguments.clone())))
        })?;
        Ok(match (sent, connection) {
            (false, _) => SpawnSend::NoConnection,
            (true, Some(_)) => SpawnSend::Sent,
            // A connection without `DFLAG_SPAWN` drops the request
            // (`dist.c:6089-6093`).
            (true, None) => SpawnSend::NoSupport,
        })
    }

    /// Give the caller of a spawn request the message `{Tag, Ref, error,
    /// Reason}` and remove the pending monitor, as a failed request does
    /// (`dist.c:6111-6121`, `6204-6211`). A `{reply, ...}` option that
    /// suppresses the error message removes the monitor with no message.
    pub(super) fn spawn_reply_error(
        &mut self,
        process: ProcessId,
        key: MonitorKey,
        tag: &Term,
        reason: Term,
        reply: SpawnReply,
    ) {
        let reference = key.term();
        self.processes
            .get_mut(process)
            .map(|process| process.monitors.remove(&key));
        if reply.no_emsg {
            return;
        }
        let message = Term::tuple(vec![
            tag.clone(),
            reference,
            Term::static_atom("error"),
            reason,
        ]);
        self.send_message(process, process, message);
    }
}

/// How the send of a spawn request ended (`dist.c:6081-6093`,
/// `6194-6202`).
enum SpawnSend {
    /// The request went out on the connection.
    Sent,
    /// The node has no connection.
    NoConnection,
    /// The connection has no `DFLAG_SPAWN`.
    NoSupport,
}

impl Scheduler {
    /// Start the process of a spawn request on this node, as
    /// `erl_create_process` with `dist_spawn_init` does
    /// (`dist.c:2980-3030`): the start is `module:function` with `arguments`,
    /// the group leader is `group_leader`, and no local parent exists. The
    /// process is a runtime process with no capability copy, because PBeam
    /// has no channel for the policy of another node, and row "Distribution"
    /// of `docs/erts-differences.md` records the gap.
    ///
    /// # Errors
    ///
    /// The errors of the export lookup of the start.
    pub(super) fn spawn_dist(
        &mut self,
        module: &Atom,
        function: &Atom,
        arguments: Vec<Term>,
        group_leader: &Term,
        flags: StartFlags,
    ) -> Result<ProcessId, BeamError> {
        let start =
            crate::runtime::interpreter::initial_start(self, module, function, arguments.len())?;
        let mut child =
            crate::runtime::process::ProcessState::new(self.next_pid()?, start, arguments);
        child.parent = None;
        // The request names the group leader of the caller: a pid of this
        // node or of another node (`bif.c:4914-4944`).
        child.group_leader = group_leader.clone();
        // The options set the start values of the same process flags that
        // `erlang:spawn_opt/4` sets (`erl_process.c:12552-12558`).
        child.off_heap_message_queue = flags.off_heap_message_queue;
        child.priority = flags.priority;
        child.async_dist = flags.async_dist;
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
    /// A request that ERTS rejects, the errors of the encode of the answer,
    /// and the error of a process that this node cannot start. ERTS answers
    /// `system_limit` for the last one (`dist.c:3020-3032`), and PBeam stops
    /// the run with the error of its process table (row "Distribution" of
    /// `docs/erts-differences.md`).
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
        // ERTS requires an external reference (`dist.c:2913-2914`), so a
        // request that names a reference of this node ends the connection.
        let Some(remote_reference) = crate::runtime::process::RemoteReference::of(reference) else {
            return Err(crate::error::error(super::dist_input::REJECTED));
        };
        let key = super::dist_monitor::monitor_key(reference)
            .expect("an external reference gives a monitor key");
        let Ok(_) = super::dist_monitor::remote_of(node, from) else {
            return Err(crate::error::error(super::dist_input::REJECTED));
        };
        if crate::term::external_id::pid_parts(group_leader).is_none() {
            return Err(crate::error::error(super::dist_input::REJECTED));
        }
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
        let (link, monitor, flags) = match spawn_options(options, self.scheduler_count()) {
            SpawnOptions::Ok {
                link,
                monitor,
                flags,
            } => (link, monitor, flags),
            // ERTS answers each rejected option list, also an improper
            // list, with `badopt` and no process (`dist.c:2949-2984`).
            SpawnOptions::Badopt => {
                return self.spawn_request_error(node, connection, &key.term(), from);
            }
            // PBeam does not bind a process to a scheduler, as its local
            // spawn does (`erl_process.c:12500-12507`).
            SpawnOptions::Scheduler(index) => {
                let option =
                    Term::tuple(vec![Term::static_atom("scheduler"), Term::Integer(index)]);
                return Err(crate::error::error(format!(
                    "unsupported spawn option {}",
                    crate::term::debug_term(&option, &self.atoms)
                )));
            }
        };
        let pid = self.spawn_dist(module, function, arguments, group_leader, flags)?;
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
            let key = crate::runtime::process::MonitorKey::Remote(remote_reference);
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
        let result = Term::Pid(pid);
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
        self.dist_reply(connection, node, &control)?;
        Ok(())
    }

    /// Answer a spawn request that ERTS rejects with `{31, Ref, From, 0,
    /// badopt}` and no process (`dist.c:2931-2980`).
    ///
    /// # Errors
    ///
    /// The errors of the encode of the answer.
    fn spawn_request_error(
        &mut self,
        node: &Atom,
        connection: u32,
        reference: &Term,
        from: &Term,
    ) -> Result<(), BeamError> {
        let control = Term::tuple(vec![
            Term::Integer(SPAWN_REPLY),
            reference.clone(),
            from.clone(),
            Term::Integer(0),
            Term::static_atom("badopt"),
        ]);
        self.dist_reply(connection, node, &control)
    }

    /// Send the `DOP_SPAWN_REPLY` control message `control` on the
    /// connection `connection` of `node` (`dist.c:2968-2979`,
    /// `3090-3120`).
    ///
    /// # Errors
    ///
    /// The errors of the encode of the control message.
    fn dist_reply(
        &mut self,
        connection: u32,
        node: &Atom,
        control: &Term,
    ) -> Result<(), BeamError> {
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
        // A reply names an internal reference of the caller, or an external
        // reference of an old incarnation of this node (`dist.c:3055-3068`).
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
        // ERTS requires a pid for the leader and the target
        // (`dist.c:2895-2897`).
        if crate::term::external_id::pid_parts(to).is_none() {
            return Err(crate::error::error(super::dist_input::REJECTED));
        }
        if crate::term::external_id::pid_parts(leader).is_none() {
            return Err(crate::error::error(super::dist_input::REJECTED));
        }
        let Term::Pid(to) = to else {
            // A target pid of another node has no local process, so the
            // signal drops, as `proc_queue_signal` drops a signal to a pid
            // that no local process has (`erl_proc_sig_queue.c:1170-1174`).
            return Ok(());
        };
        let signal = Signal::DistGroupLeader {
            leader: leader.clone(),
        };
        let _ = self.send_signal(None, *to, signal);
        Ok(())
    }
}

/// The options of a spawn request that this node receives, as
/// `erts_parse_spawn_opts` reads them (`erl_process.c:12222-12428`).
enum SpawnOptions {
    /// The request names a valid option list.
    Ok {
        /// The request asks for a link.
        link: bool,
        /// The request asks for a monitor.
        monitor: bool,
        /// The start values of the `message_queue_data` flag, the priority,
        /// and the `async_dist` flag (`erl_process.c:12275-12320`).
        flags: crate::runtime::process::StartFlags,
    },
    /// An option list that ERTS answers with `badopt` and no process
    /// (`dist.c:2949-2984`).
    Badopt,
    /// The request names a scheduler above 0, which PBeam does not bind
    /// (`erl_process.c:12500-12507`).
    Scheduler(i64),
}

/// Read the options `options` of a spawn request that this node receives,
/// as `erts_parse_spawn_opts` reads them (`erl_process.c:12222-12428`).
/// PBeam has no heap accounting, so a heap size option keeps its value for
/// the checks only (row "Heap limit" in `docs/erts-differences.md`).
#[expect(
    clippy::too_many_lines,
    reason = "the option table follows `erts_parse_spawn_opts`"
)]
fn spawn_options(options: &Term, schedulers: i64) -> SpawnOptions {
    let Some(options) = options.proper_list() else {
        return SpawnOptions::Badopt;
    };
    let mut link = false;
    let mut monitor = false;
    let mut flags = crate::runtime::process::StartFlags::default();
    // `H_DEFAULT_SIZE` and `H_MAX_SIZE` (`erl_vm.h:60-61`,
    // `erlang.c:12245-12250`).
    let mut min_heap_size = 233;
    let mut max_heap_size = 0;
    for option in options {
        let valid = match option {
            Term::Atom(name) if name.is_static("link") => {
                link = true;
                true
            }
            Term::Atom(name) if name.is_static("monitor") => {
                monitor = true;
                true
            }
            Term::Tuple(fields) if fields.len() == 2 => {
                let (Term::Atom(name), value) = (&fields[0], &fields[1]) else {
                    return SpawnOptions::Badopt;
                };
                if name.is_static("priority") {
                    match value {
                        Term::Atom(priority) if priority.is_static("max") => {
                            flags.priority = Priority::Max;
                            true
                        }
                        Term::Atom(priority) if priority.is_static("high") => {
                            flags.priority = Priority::High;
                            true
                        }
                        Term::Atom(priority) if priority.is_static("normal") => {
                            flags.priority = Priority::Normal;
                            true
                        }
                        Term::Atom(priority) if priority.is_static("low") => {
                            flags.priority = Priority::Low;
                            true
                        }
                        _ => false,
                    }
                } else if name.is_static("async_dist") {
                    match value {
                        Term::Atom(value) if value.is_static("true") => {
                            flags.async_dist = true;
                            true
                        }
                        Term::Atom(value) if value.is_static("false") => {
                            flags.async_dist = false;
                            true
                        }
                        _ => false,
                    }
                } else if name.is_static("message_queue_data") {
                    match value {
                        Term::Atom(value) if value.is_static("on_heap") => {
                            flags.off_heap_message_queue = false;
                            true
                        }
                        Term::Atom(value) if value.is_static("off_heap") => {
                            flags.off_heap_message_queue = true;
                            true
                        }
                        _ => false,
                    }
                } else if name.is_static("min_heap_size") {
                    match non_negative(value) {
                        Some(size) => {
                            min_heap_size = size.max(min_heap_size);
                            true
                        }
                        None => false,
                    }
                } else if name.is_static("min_bin_vheap_size") || name.is_static("fullsweep_after")
                {
                    non_negative(value).is_some()
                } else if name.is_static("max_heap_size") {
                    match max_heap_size_option(value) {
                        Some(size) => {
                            max_heap_size = size;
                            true
                        }
                        None => false,
                    }
                } else if name.is_static("scheduler") {
                    match non_negative(value) {
                        // ERTS rejects an index above the scheduler count
                        // only (`erl_process.c:12355-12368`).
                        Some(index) if index <= schedulers => {
                            if index == 0 {
                                true
                            } else {
                                return SpawnOptions::Scheduler(index);
                            }
                        }
                        _ => false,
                    }
                } else if name.is_static("link") {
                    link = true;
                    link_options(value)
                } else if name.is_static("monitor") {
                    monitor = true;
                    monitor_options(value)
                } else if name.is_static("reply") || name.is_static("reply_tag") {
                    // The receiver of a request accepts neither option: its
                    // `erts_parse_spawn_opts` call has no message option and
                    // no reply tag (`dist.c:2951`, `erl_process.c:12356`,
                    // `12392`).
                    false
                } else {
                    false
                }
            }
            _ => false,
        };
        if !valid {
            return SpawnOptions::Badopt;
        }
    }
    if max_heap_size != 0 && max_heap_size < min_heap_size {
        return SpawnOptions::Badopt;
    }
    SpawnOptions::Ok {
        link,
        monitor,
        flags,
    }
}

/// Return the size of a `{max_heap_size, Value}` option, or `None` when
/// ERTS rejects it (`erts_max_heap_size`, `erl_gc.c:3833-3881`): a
/// non-negative small integer, or a map with a non-negative small `size`
/// and a boolean `kill`, `error_logger`, and `include_shared_binaries`.
fn max_heap_size_option(value: &Term) -> Option<i64> {
    let Term::Map(map) = value else {
        return non_negative(value);
    };
    let size = non_negative(map.get(&Term::static_atom("size"))?)?;
    for key in ["kill", "error_logger", "include_shared_binaries"] {
        if let Some(value) = map.get(&Term::static_atom(key))
            && !matches!(value, Term::Atom(name) if name.is_static("true") || name.is_static("false"))
        {
            return None;
        }
    }
    Some(size)
}

/// Return whether `value` is an atom with a name in `names`.
fn is_one_of(value: &Term, names: &[&str]) -> bool {
    matches!(value, Term::Atom(name) if names.iter().any(|wanted| name.is_static(wanted)))
}

/// Return the value of a non-negative small integer, or `None` for each
/// other term.
fn non_negative(value: &Term) -> Option<i64> {
    value.small_integer().filter(|value| *value >= 0)
}

/// Return whether `options` is a valid `{link, Options}` option list: each
/// element is the atom `priority` (`erts_link_opts`, `bif.c:675-699`).
fn link_options(options: &Term) -> bool {
    options.proper_list().is_some_and(|options| {
        options
            .into_iter()
            .all(|option| matches!(option, Term::Atom(name) if name.is_static("priority")))
    })
}

/// Return whether `options` is a valid `{monitor, Options}` option list:
/// each element is the atom `priority` or a `{tag, Tag}` or `{alias, How}`
/// tuple (`erts_monitor_opts`, `bif.c:702-780`).
fn monitor_options(options: &Term) -> bool {
    options.proper_list().is_some_and(|options| {
        options.into_iter().all(|option| match option {
            Term::Atom(name) => name.is_static("priority"),
            Term::Tuple(fields) if fields.len() == 2 => match &fields[0] {
                Term::Atom(name) if name.is_static("tag") => true,
                Term::Atom(name) if name.is_static("alias") => is_one_of(
                    &fields[1],
                    &["explicit_unalias", "demonitor", "reply_demonitor"],
                ),
                _ => false,
            },
            _ => false,
        })
    })
}

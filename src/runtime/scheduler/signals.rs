//! Send and handle the signals of processes, as `erl_proc_sig_queue.c`
//! does on one scheduler.
//!
//! A signal goes to the end of the signal queue of its receiver when it is
//! sent (`proc_queue_signal`, `erl_proc_sig_queue.c:1076-1252`). On one
//! scheduler no in-queue buffer is installed, so the queue of each receiver
//! has the send order of all its senders. [`Scheduler::handle_signals`] is
//! the one place that applies a signal to its receiver.

use super::{
    Files, Flush, LinkId, Monitor, ProcessState, RECEIVE_FETCH_REDUCTIONS, Release,
    SIGNAL_HANDLING_REDUCTIONS, Scheduler, TargetMonitor, Ttys,
};
use crate::{
    error::BeamError,
    runtime::{
        capability::Source,
        mailbox::{NodeId, Step, Walk},
        scheduler::AliasMode,
        signal::Signal,
    },
    term::{ProcessId, Reference, Term},
};

/// A delivery of the current process to a process of another scheduler.
#[derive(Debug)]
pub(in crate::runtime) enum Held {
    /// A message of the process `from`.
    Message { from: ProcessId, term: Term },
    /// A control signal of `sender`, or of the runtime when `None`.
    Signal {
        sender: Option<ProcessId>,
        signal: Signal,
    },
}

/// The signal count of one reduction, as `ERTS_SIG_REDS_CNT_FACTOR`
/// (`erl_proc_sig_queue.c:52`).
const SIGNAL_COUNT_FACTOR: i32 = 4;

/// The signal count that a stretch of the handling limit adds:
/// `ERTS_SIG_REDS_CNT_FACTOR * 100` (`erl_proc_sig_queue.c:7246-7250`).
const SIGNAL_COUNT_STRETCH: i32 = SIGNAL_COUNT_FACTOR * 100;

/// Where a process handles its signals.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(in crate::runtime) enum HandlingPoint {
    /// At schedule-in, before the process runs Erlang code. The handling
    /// first fetches the outer part, and the process is not `RUNNING`
    /// (`erl_process.c:10318-10352`).
    ScheduleIn,
    /// While the process runs: in `loop_rec`, or in a BIF that handles the
    /// signals of its caller. The handling does not fetch (`local_only`).
    Running,
}

/// What the handling of one signal did.
enum Handled {
    /// The handling goes on. The value is the extra signal count.
    Continue(i32),
    /// The signal ended the process or was the flush signal, and the
    /// handling stops (`goto stop`). The value is the extra signal count.
    Stop(i32),
}

/// What `loop_rec` found at the end of the inner part, as
/// `erts_proc_sig_receive_helper` (`erl_proc_sig_queue.c:7730-7868`) tells.
#[derive(Debug, PartialEq, Eq)]
pub(in crate::runtime) enum Received {
    /// The inner part has a message at the save pointer.
    Message,
    /// No message and no signal is left: the process goes to its `wait`.
    Empty,
    /// The reductions are used up: `loop_rec` runs again in the next turn.
    Yield,
    /// A signal ended the process: it switches out and exits in its next
    /// turn.
    Exited,
}

/// The result of a BIF that handles the signals of its caller up to a flush
/// signal.
#[derive(Debug, PartialEq)]
pub(in crate::runtime) enum Flushed {
    /// The handling reached the flush signal.
    Done,
    /// A signal ended the caller with this reason.
    Exited(Term),
    /// The reductions are used up: the BIF runs again in the next turn.
    Yield,
}

impl Scheduler {
    /// Remove the monitor of the file resource `file` from the target
    /// monitor list of its owner `owner`, as `enif_demonitor_process` in
    /// `close_nif` (`prim_file_nif.c:633`) does, when the current process is
    /// the owner. `prim_file:close/1` calls `close_nif/1` only from the owner
    /// (`get_fd_data`, prim_file.erl:499-504). A close from another process
    /// keeps the monitor, and its `Files::monitor_down` at the exit of the
    /// owner finds the resource closed and does nothing.
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// scheduler.demonitor_file(ProcessId(1), Reference(0));
    /// ```
    pub(in crate::runtime) fn demonitor_file(&mut self, owner: ProcessId, file: Reference) {
        let current = self.current_process_mut();
        let monitor = TargetMonitor::File(file);
        let owned = current.id == owner;
        current
            .monitored_by
            .retain(|&entry| !owned || entry != monitor);
    }

    /// Send the signal `signal` to the process `target`, as
    /// `proc_queue_signal` (`erl_proc_sig_queue.c:1076-1252`) does. A pending
    /// monitor signal to the same target goes first, to keep the order of
    /// the signals of the sender. A signal to a process of another scheduler
    /// goes after each pending monitor signal
    /// ([`Self::deliver_remote`]). The signal goes to the end of the
    /// queue. A target that does not run and is not in the run queue goes to the
    /// end of the run queue to handle it (`erts_proc_notify_new_sig`,
    /// `erl_proc_sig_queue.h:2063-2075`). `sender` is the sending process,
    /// or `None` for the runtime; a signal of a process to itself marks it.
    ///
    /// Return the signal, in a box, when no process has the pid `target`:
    /// the caller then gives the ERTS result of a failed send of this kind.
    /// A pending monitor signal to such a target gives a monitor down signal
    /// `noproc` to its watcher (`erl_proc_sig_queue.c:1146-1170`).
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// scheduler.send_signal(Some(caller), child, Signal::Link { from: caller })?;
    /// ```
    pub(in crate::runtime) fn send_signal(
        &mut self,
        sender: Option<ProcessId>,
        target: ProcessId,
        signal: Signal,
    ) -> Result<(), Box<Signal>> {
        if !self.processes.contains_key(&target)
            && let Some(index) = self.remote_owner(target)
        {
            self.deliver_remote(index, target, Held::Signal { sender, signal });
            return Ok(());
        }
        self.send_signal_local(sender, target, signal)
    }

    /// Put `signal` for the remote target `target` into the inbox of its
    /// owner `index`, which `remote_owner` gave, after the pending monitor
    /// signal of the sender ([`Self::deliver_remote`]). The inbox always takes the signal: a
    /// target that left before the drain is answered there.
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// scheduler.send_signal_to_owner(index, Some(caller), target, signal);
    /// ```
    pub(in crate::runtime) fn send_signal_to_owner(
        &mut self,
        index: usize,
        sender: Option<ProcessId>,
        target: ProcessId,
        signal: Signal,
    ) {
        self.deliver_remote(index, target, Held::Signal { sender, signal });
    }

    /// Put the signal into the queue of the local process `target`, as
    /// [`Self::send_signal`] does.
    pub(in crate::runtime) fn send_signal_local(
        &mut self,
        sender: Option<ProcessId>,
        target: ProcessId,
        signal: Signal,
    ) -> Result<(), Box<Signal>> {
        if let Some(pending) = self.take_pending_monitor(target) {
            if self.processes.contains_key(&target) {
                self.queue_signal(sender, target, monitor_signal(pending));
            } else {
                self.pending_monitor_failed(sender, pending);
            }
            self.release_held();
        }
        if !self.processes.contains_key(&target) {
            return Err(Box::new(signal));
        }
        self.queue_signal(sender, target, signal);
        Ok(())
    }

    /// Put `signal` at the end of the queue of the live process `target`,
    /// and put the target in the run queue when it does not run and is not
    /// there.
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// scheduler.queue_signal(Some(caller), child, Signal::Flush);
    /// ```
    fn queue_signal(&mut self, sender: Option<ProcessId>, target: ProcessId, signal: Signal) {
        let process = self
            .processes
            .get_mut(&target)
            .expect("the target is alive");
        process.self_signals |= sender == Some(target);
        process.mailbox.push_signal(signal);
        self.enqueue(target);
    }

    /// Send the monitor signal of `monitor` from the current process to the
    /// process `target` through the pending signal slot, as
    /// `proc_queue_signal` does for `ERTS_SIG_Q_OP_MONITOR`
    /// (`erl_proc_sig_queue.c:1101-1145`). The signal waits in the slot; a
    /// signal that waited there goes now. A target that does not exist then
    /// gives a monitor down signal `noproc` to the watcher.
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// scheduler.send_monitor(child, Monitor { watcher, reference });
    /// ```
    pub(in crate::runtime) fn send_monitor(&mut self, target: ProcessId, monitor: Monitor) {
        if let Some((to, pending)) = self.pending_monitor.replace((target, monitor)) {
            self.queue_pending_monitor(Some(pending.watcher), to, pending);
            self.release_held();
        }
    }

    /// Send the pending monitor signal, when the slot has one, as
    /// `erts_proc_sig_send_pending` (`erl_proc_sig_queue.c:1254-1272`) does
    /// when its sender switches out (`erl_process.c:9665-9668`).
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// scheduler.send_pending_monitor();
    /// ```
    pub(in crate::runtime) fn send_pending_monitor(&mut self) {
        if let Some((to, pending)) = self.pending_monitor.take() {
            self.queue_pending_monitor(None, to, pending);
            self.release_held();
        }
    }

    /// Queue the monitor signal of `pending` to `to`, or give its watcher a
    /// monitor down signal `noproc` when no process has the pid `to`.
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// scheduler.queue_pending_monitor(None, child, pending);
    /// ```
    fn queue_pending_monitor(
        &mut self,
        sender: Option<ProcessId>,
        to: ProcessId,
        pending: Monitor,
    ) {
        if self.processes.contains_key(&to) {
            self.queue_signal(Some(pending.watcher), to, monitor_signal(pending));
        } else if let Some(index) = self.remote_owner(to) {
            let signal = monitor_signal(pending);
            self.node
                .send_signal(index, to, Some(pending.watcher), signal, None);
        } else {
            self.pending_monitor_failed(sender, pending);
        }
    }

    /// Give the ERTS response of the dead target `target` for a signal that
    /// a scheduler of another thread sent before the target exited.
    pub(in crate::runtime) fn dead_target_signal(&mut self, target: ProcessId, signal: Signal) {
        match signal {
            Signal::Monitor { watcher, reference } => {
                self.send_monitor_down(None, watcher, reference, Term::static_atom("noproc"));
            }
            Signal::Link { from } => {
                let exit = Signal::Exit {
                    from: LinkId::Process(target),
                    reason: Term::static_atom("noproc"),
                    linked: true,
                    normal_kills: false,
                    alias: None,
                };
                let _ = self.send_signal(None, from, exit);
            }
            // The target answered an unlink at its exit
            // (`erl_proc_sig_queue.c:7369-7386`), so the sender can remove
            // its unlinking link.
            Signal::Unlink { from, id } => {
                let ack = Signal::UnlinkAck { from: target, id };
                let _ = self.send_signal(None, from, ack);
            }
            // A request to a dead target gets the answer of the exit path
            // for a process that is gone.
            Signal::GroupLeader {
                requester,
                reference,
                ..
            } => self.send_message(
                target,
                requester,
                reply(reference, Term::static_atom("badarg")),
            ),
            Signal::IsAlive {
                requester,
                reference,
            } => self.send_message(
                target,
                requester,
                reply(reference, Term::static_atom("false")),
            ),
            Signal::ProcessInfo {
                requester,
                reference,
                ..
            } => self.send_message(
                target,
                requester,
                reply(reference, Term::static_atom("undefined")),
            ),
            _ => {}
        }
    }

    /// Send `held` to `target`, a process of the scheduler `index`. A
    /// pending monitor signal to `target` goes first, as ERTS prepends it
    /// (`erl_proc_sig_queue.c:1146-1166`, `erl_message.c:463-480`), and the
    /// held deliveries go after it. A pending monitor signal to another
    /// process stays in the slot until the switch out
    /// (`erl_process.c:9665-9668`), and the delivery waits behind it in
    /// [`Self::held`]. Thus no later signal of the receiver can pass the
    /// monitor signal: the delivery reaches the other thread after the
    /// monitor signal. This is the order of an ERTS run in which the other
    /// scheduler runs the receiver after the sender switches out.
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// scheduler.deliver_remote(index, target, Held::Signal { sender, signal });
    /// ```
    pub(in crate::runtime) fn deliver_remote(
        &mut self,
        index: usize,
        target: ProcessId,
        held: Held,
    ) {
        if let Some(pending) = self.take_pending_monitor(target) {
            self.send_held(index, target, held, Some(monitor_signal(pending)));
            self.release_held();
        } else if self.pending_monitor.is_some() {
            self.held.push((index, target, held));
        } else {
            self.send_held(index, target, held, None);
        }
    }

    /// Put `held` for `target` into the inbox of the scheduler `index`,
    /// after `pending`.
    fn send_held(&self, index: usize, target: ProcessId, held: Held, pending: Option<Signal>) {
        match held {
            Held::Message { from, term } => {
                self.node
                    .send_message(index, Some(from), term, target, pending)
            }
            Held::Signal { sender, signal } => self
                .node
                .send_signal(index, target, sender, signal, pending),
        }
    }

    /// Send the deliveries that waited behind the pending monitor signal, in
    /// send order.
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// scheduler.release_held();
    /// ```
    pub(in crate::runtime) fn release_held(&mut self) {
        for (index, target, held) in std::mem::take(&mut self.held) {
            self.send_held(index, target, held, None);
        }
    }

    /// Take the pending monitor signal when it goes to `target`.
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// assert!(scheduler.take_pending_monitor(child).is_some());
    /// ```
    pub(in crate::runtime) fn take_pending_monitor(
        &mut self,
        target: ProcessId,
    ) -> Option<Monitor> {
        if self
            .pending_monitor
            .as_ref()
            .is_some_and(|(to, _)| *to == target)
        {
            return self.pending_monitor.take().map(|(_, monitor)| monitor);
        }
        None
    }

    /// Give the watcher of a monitor signal that found no target a monitor
    /// down signal `noproc` (`erl_proc_sig_queue.c:1139-1143`).
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// scheduler.pending_monitor_failed(Some(watcher), pending);
    /// ```
    fn pending_monitor_failed(&mut self, sender: Option<ProcessId>, pending: Monitor) {
        let noproc = Term::static_atom("noproc");
        self.send_monitor_down(sender, pending.watcher, pending.reference, noproc);
    }

    /// Send a monitor down signal of the monitor `reference` with `reason`
    /// to `watcher`, as `erts_proc_sig_send_monitor_down`
    /// (`erl_proc_sig_queue.c:2814-2895`) does. A watcher that does not
    /// exist drops it.
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// scheduler.send_monitor_down(Some(child), watcher, reference, Term::static_atom("normal"));
    /// ```
    pub(in crate::runtime) fn send_monitor_down(
        &mut self,
        sender: Option<ProcessId>,
        watcher: ProcessId,
        reference: Reference,
        reason: Term,
    ) {
        let down = Signal::MonitorDown { reference, reason };
        // A watcher that does not exist drops the signal.
        let _ = self.send_signal(sender, watcher, down);
    }

    /// Handle the signals of the process `id` in the middle part, in queue
    /// order, as `erts_proc_sig_handle_incoming`
    /// (`erl_proc_sig_queue.c:6229-7176`) does. At schedule-in the handling
    /// first fetches. The process must have signals in its middle part and
    /// must not exit.
    ///
    /// Each handled signal and receive marker counts 1, and each signal
    /// kind adds its extra count (see [`Signal`]). The handling goes on
    /// while the count is at most `4 * reds`, but not more than
    /// `4 * max_reds`. When the inner part has no message after the save
    /// pointer, the limit grows by 400 up to `4 * max_reds`
    /// (`stretch_limit`, `erl_proc_sig_queue.c:7180-7256`). A signal that
    /// ends the process, and the flush signal, stop the handling. At the
    /// stop, the messages before the next signal go to the inner part.
    ///
    /// Return the reductions of the handling: the count divided by 4, plus
    /// 1 (`erl_proc_sig_queue.c:7160`).
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// let reductions = scheduler.handle_signals(id, 100, 100, HandlingPoint::ScheduleIn)?;
    /// ```
    ///
    /// # Errors
    ///
    /// A process-info request whose reduction total is above the PBeam
    /// integer range is an error (see `ProcessState::info`).
    pub(in crate::runtime) fn handle_signals(
        &mut self,
        id: ProcessId,
        reds: i32,
        max_reds: i32,
        point: HandlingPoint,
    ) -> Result<i32, BeamError> {
        self.drain_inbox();
        let process = self.processes.get_mut(&id).expect("the process is alive");
        if point == HandlingPoint::ScheduleIn {
            process.mailbox.fetch();
        }
        // ERTS returns 0 reductions for an empty middle part and for an
        // exiting process (`erl_proc_sig_queue.c:6266-6284`). Each caller
        // handles only the signals of a process that runs and has some.
        debug_assert!(process.mailbox.has_middle() && process.exiting.is_none());
        let absolute = SIGNAL_COUNT_FACTOR * max_reds;
        let mut limit = (SIGNAL_COUNT_FACTOR * reds).min(absolute);
        let mut count = 0;
        let mut walk = process.mailbox.walk();
        loop {
            let process = self.processes.get_mut(&id).expect("the process is alive");
            let Some(step) = process.mailbox.next_signal(&mut walk) else {
                break;
            };
            if let Step::Signal(node, signal) = step {
                match self.handle_signal(id, &mut walk, node, signal, point)? {
                    Handled::Continue(extra) => count += extra,
                    Handled::Stop(extra) => {
                        count += extra;
                        break;
                    }
                }
            }
            count += 1;
            if count > limit && !self.stretch_limit(id, &walk, &mut limit, absolute) {
                break;
            }
        }
        let process = self.processes.get_mut(&id).expect("the process is alive");
        process.mailbox.end_walk(walk);
        if !process.mailbox.has_incoming() {
            process.self_signals = false;
        }
        if process.hibernated && process.mailbox.has_messages() {
            self.activate(id);
        }
        Ok(count / SIGNAL_COUNT_FACTOR + 1)
    }

    /// Grow the handling limit by [`SIGNAL_COUNT_STRETCH`], up to
    /// `absolute`, when the inner part of the process `id` has no message
    /// to read and the middle part is not empty, as `stretch_limit`
    /// (`erl_proc_sig_queue.c:7180-7256`) does. Return true when the limit
    /// grew.
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// assert!(scheduler.stretch_limit(id, &walk, &mut limit, 16000));
    /// ```
    fn stretch_limit(&self, id: ProcessId, walk: &Walk, limit: &mut i32, absolute: i32) -> bool {
        if *limit == absolute || !self.processes[&id].mailbox.can_stretch(walk) {
            return false;
        }
        *limit = (*limit + SIGNAL_COUNT_STRETCH).min(absolute);
        true
    }

    /// Apply one signal that the handling took from the node `node` of the
    /// queue of the process `id`. The signal gives a message in its place,
    /// or its node leaves the queue. Return the extra signal count of the
    /// kind, and whether the handling stops.
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// let handled = scheduler.handle_signal(id, &mut walk, node, Signal::Flush, HandlingPoint::Running)?;
    /// ```
    fn handle_signal(
        &mut self,
        id: ProcessId,
        walk: &mut Walk,
        node: NodeId,
        signal: Signal,
        point: HandlingPoint,
    ) -> Result<Handled, BeamError> {
        let extra = match signal {
            Signal::Exit {
                from,
                reason,
                linked,
                normal_kills,
                alias,
            } => {
                let exit = ExitSignal {
                    from,
                    reason,
                    linked,
                    normal_kills,
                    alias,
                };
                return Ok(self.handle_exit_signal(id, walk, node, exit));
            }
            Signal::MonitorDown { reference, reason } => {
                self.handle_monitor_down(id, walk, node, reference, reason)
            }
            Signal::AliasMessage { alias, message } => {
                self.handle_alias_message(id, walk, node, alias, message)
            }
            Signal::Flush => {
                self.handled(id, node).flush = Flush::Flushed;
                return Ok(Handled::Stop(0));
            }
            Signal::Revoke { capability } => {
                self.handled(id, node).security.remove(capability);
                let signals = self.delegations.revoke(capability);
                self.send_revocations(Some(id), signals);
                0
            }
            Signal::Grant { capability } => {
                let granted = capability
                    .map(|capability| self.delegations.hold(id, [(Source::Policy, capability)]));
                let process = self.handled(id, node);
                process.policy_reply = Some(granted.is_some());
                if let Some(granted) = granted {
                    process.security.add(granted);
                }
                // The `pbeam_cap:request/2` call waits for this signal.
                self.activate(id);
                0
            }
            Signal::EnterRuntime { capabilities } => {
                let issued = capabilities
                    .into_iter()
                    .map(|capability| (Source::Launcher, capability));
                let runtime = self.delegations.hold(id, issued);
                let process = self.handled(id, node);
                // The boot context has no copies to release.
                process.security = runtime;
                process.policy_reply = Some(true);
                // The `pbeam_cap:enter_runtime/2` call waits for this signal.
                self.activate(id);
                0
            }
            Signal::Link { from } => {
                // A link that the receiver has keeps its node and its state
                // (`erl_proc_sig_queue.c:6641`).
                let links = &mut self.handled(id, node).links;
                let from = LinkId::Process(from);
                if links.get(&from).is_none() {
                    links.insert(from, 0);
                }
                0
            }
            Signal::Unlink { from, id: unlink } => {
                let links = &mut self.handled(id, node).links;
                let link = LinkId::Process(from);
                let removed = links.get(&link) == Some(&0) && links.remove(&link).is_some();
                let ack = Signal::UnlinkAck {
                    from: id,
                    id: unlink,
                };
                // A process that does not exist drops the acknowledgment.
                let _ = self.send_signal(Some(id), from, ack);
                4 * i32::from(removed)
            }
            Signal::UnlinkAck { from, id: unlink } => {
                let links = &mut self.handled(id, node).links;
                let from = LinkId::Process(from);
                let removed = links.get(&from) == Some(&unlink) && links.remove(&from).is_some();
                4 * i32::from(removed)
            }
            Signal::Monitor { watcher, reference } => {
                let monitor = TargetMonitor::Process(Monitor { watcher, reference });
                self.handled(id, node).monitored_by.push(monitor);
                2
            }
            Signal::TtyMonitor { tty } => {
                self.handled(id, node)
                    .monitored_by
                    .push(TargetMonitor::Tty(tty));
                2
            }
            Signal::FileMonitor { file } => {
                // A close before this signal removed the monitor already.
                let open = self.resources.borrow_mut::<Files>().is_open(file);
                let monitors = &mut self.handled(id, node).monitored_by;
                if open {
                    monitors.push(TargetMonitor::File(file));
                }
                2
            }
            Signal::PortLink { port } => {
                let links = &mut self.handled(id, node).links;
                if links.get(&LinkId::Port(port)).is_none() {
                    links.insert(LinkId::Port(port), 0);
                }
                0
            }
            Signal::PortUnlink { port } => {
                let links = &mut self.handled(id, node).links;
                let link = LinkId::Port(port);
                let removed = links.get(&link) == Some(&0) && links.remove(&link).is_some();
                4 * i32::from(removed)
            }
            Signal::PortMonitor { port, reference } => {
                let monitor = TargetMonitor::Port { port, reference };
                self.handled(id, node).monitored_by.push(monitor);
                2
            }
            Signal::PortDemonitor { port, reference } => {
                let monitor = TargetMonitor::Port { port, reference };
                let monitors = &mut self.handled(id, node).monitored_by;
                let before = monitors.len();
                monitors.retain(|target| *target != monitor);
                1 + i32::from(monitors.len() < before)
            }
            Signal::Demonitor { watcher, reference } => {
                let monitor = TargetMonitor::Process(Monitor { watcher, reference });
                let monitors = &mut self.handled(id, node).monitored_by;
                let before = monitors.len();
                monitors.retain(|target| *target != monitor);
                1 + i32::from(monitors.len() < before)
            }
            Signal::GroupLeader {
                leader,
                requester,
                reference,
            } => {
                self.handled(id, node).group_leader = leader;
                self.send_message(id, requester, reply(reference, Term::static_atom("true")));
                0
            }
            Signal::IsAlive {
                requester,
                reference,
            } => {
                self.handled(id, node);
                self.send_message(id, requester, reply(reference, Term::static_atom("true")));
                0
            }
            Signal::ProcessInfo {
                requester,
                reference,
                item,
            } => {
                // `erts_current_reductions` gives 0 for a process that is not
                // `RUNNING` (`beam_common.c:2393-2409`).
                let current = match point {
                    HandlingPoint::Running => self.current_reductions(),
                    HandlingPoint::ScheduleIn => 0,
                };
                let registered = self.registered_name_of(id);
                let process = self.processes.get_mut(&id).expect("the process is alive");
                let messages = process.mailbox.messages_before(node);
                // The item name comes from the atom table, so remove the
                // node here and keep the two field borrows separate.
                process.mailbox.remove(node);
                let values = process.info(
                    &self.atoms,
                    &item,
                    current,
                    messages,
                    true,
                    registered.as_ref(),
                );
                let info = values?.expect("erlang:process_info/2 sends a supported item");
                self.send_message(id, requester, reply(reference, info));
                0
            }
        };
        Ok(Handled::Continue(extra))
    }

    /// Remove the node `node` of a handled signal from the queue of the
    /// process `id`, and return the process.
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// scheduler.handled(id, node).links.insert(from, 0);
    /// ```
    fn handled(&mut self, id: ProcessId, node: NodeId) -> &mut ProcessState {
        let process = self.processes.get_mut(&id).expect("the process is alive");
        process.mailbox.remove(node);
        process
    }

    /// Handle an exit signal, as `handle_exit_signal`
    /// (`erl_proc_sig_queue.c:4632-4815`) does. The signal costs 1 more than
    /// the count of each handled signal.
    ///
    /// - An exit signal through an alias that is not active is ignored.
    /// - A link exit signal acts only through an active link to its sender,
    ///   and removes the link, also a link that is unlinking. Without an
    ///   active link it is ignored.
    /// - A receiver that traps exits gets `{'EXIT', From, Reason}` in the
    ///   place of the signal, unless the signal of `erlang:exit/2` has the
    ///   reason `kill`.
    /// - Else `normal` is ignored, unless `normal_kills` is set.
    /// - Else the receiver exits with the reason, and `kill` of
    ///   `erlang:exit/2` gives `killed`. A link exit signal keeps its
    ///   reason, also `kill`. This costs 1 more, and the handling stops.
    ///   The exit of the receiver then sends its own link exit signals
    ///   (see [`Scheduler::finish_current`]).
    ///
    /// An ignored signal costs 1 more again.
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// let handled = scheduler.handle_exit_signal(id, &mut walk, node, exit);
    /// ```
    fn handle_exit_signal(
        &mut self,
        id: ProcessId,
        walk: &mut Walk,
        node: NodeId,
        exit: ExitSignal,
    ) -> Handled {
        let ExitSignal {
            from,
            reason,
            linked,
            normal_kills,
            alias,
        } = exit;
        let process = self.processes.get_mut(&id).expect("the process is alive");
        let active = match alias {
            Some(alias) => process
                .monitors
                .get(&alias)
                .is_some_and(|origin| origin.alias.is_some()),
            None => !linked || process.links.remove(&from) == Some(0),
        };
        let kill = !linked && reason == Term::static_atom("kill");
        if active && !kill && process.trap_exit {
            let message = Term::tuple(vec![Term::static_atom("EXIT"), from.term(), reason]);
            process.mailbox.convert(walk, node, message);
            self.activate(id);
            return Handled::Continue(1);
        }
        process.mailbox.remove(node);
        if !active || (reason == Term::static_atom("normal") && !normal_kills) {
            return Handled::Continue(2);
        }
        process.exiting = Some(if kill {
            Term::static_atom("killed")
        } else {
            reason
        });
        process.waiting = false;
        self.set_exiting(id);
        Handled::Stop(2)
    }

    /// Handle a monitor down signal of the watcher `id`. With an origin
    /// monitor for `reference`, the `DOWN` message
    /// `{Tag, Reference, Type, Item, Reason}` takes the place of the
    /// signal, and the monitor leaves the monitor tree; an
    /// `explicit_unalias` alias stays (`erl_proc_sig_queue.c:6460-6500`).
    /// The message costs 8 for an immediate reason and 1 for another reason
    /// (see [`Signal::MonitorDown`]). Without such a monitor the signal is
    /// dropped for 0. Return the extra signal count.
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// assert_eq!(scheduler.handle_monitor_down(id, &mut walk, node, reference, normal), 8);
    /// ```
    fn handle_monitor_down(
        &mut self,
        id: ProcessId,
        walk: &mut Walk,
        node: NodeId,
        reference: Reference,
        reason: Term,
    ) -> i32 {
        let Some(monitor) = self.remove_origin_monitor(id, reference) else {
            let process = self.processes.get_mut(&id).expect("the process is alive");
            process.mailbox.remove(node);
            return 0;
        };
        let extra = if is_immediate(&reason) { 8 } else { 1 };
        let down = Term::tuple(vec![
            monitor.tag,
            Term::Reference(reference),
            Term::static_atom(monitor.kind),
            monitor.item,
            reason,
        ]);
        let process = self.processes.get_mut(&id).expect("the process is alive");
        process.mailbox.convert(walk, node, down);
        self.activate(id);
        extra
    }

    /// Handle a message through the alias `alias` of the process `id`, as
    /// `handle_altact_msg` (`erl_proc_sig_queue.c:6009-6221`) does. An
    /// active alias puts the message in the place of the signal for 1. A
    /// `reply_demonitor` alias ends, and its monitor ends with a demonitor
    /// signal to a process target or its removal from a port target
    /// (`erl_proc_sig_queue.c:6047-6107`). A reference
    /// that is not an active alias drops the message for 2. Return the extra
    /// signal count.
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// assert_eq!(scheduler.handle_alias_message(id, &mut walk, node, alias, message), 1);
    /// ```
    fn handle_alias_message(
        &mut self,
        id: ProcessId,
        walk: &mut Walk,
        node: NodeId,
        alias: Reference,
        message: Term,
    ) -> i32 {
        let process = self.processes.get_mut(&id).expect("the process is alive");
        let Some(mode) = process.monitors.get(&alias).and_then(|origin| origin.alias) else {
            process.mailbox.remove(node);
            return 2;
        };
        if mode == AliasMode::ReplyDemonitor
            && let Some(origin) = process.monitors.remove(&alias)
            && let Some(monitor) = origin.monitor
        {
            // The target of a process or a port monitor also loses it
            // (`erl_proc_sig_queue.c:6064-6107`).
            self.end_monitor(id, alias, &monitor);
        }
        let process = self.processes.get_mut(&id).expect("the process is alive");
        process.mailbox.convert(walk, node, message);
        self.activate(id);
        1
    }

    /// Answer the signals that the exiting process `process` did not
    /// handle, in queue order, as `erts_proc_sig_handle_exit`
    /// (`erl_proc_sig_queue.c:7259-7473`) does with `reds` reductions left.
    /// Return true when no signal is left. The process is out of the
    /// process table. A monitor signal gives a monitor down signal `noproc`
    /// to its watcher, a link signal gives a link exit signal `noproc` to
    /// its sender, a link signal of a port closes the port with `noproc`
    /// (see [`Self::port_link_exit`]), an unlink signal gives an unlink
    /// acknowledgment, and the
    /// group-leader, is-alive, and process-info requests reply `badarg`,
    /// `false`, and `undefined`. A TTY monitor signal runs its TTY monitor.
    /// A revocation signal sends the revocation on to the downstream copies:
    /// the process released its copy when it left the process table.
    /// Each other signal is dropped. Each signal
    /// and receive marker counts 1, and a monitor signal 4 more
    /// (`erl_proc_sig_queue.c:7307`, `7352`). The handling stops when the
    /// count is more than `reds` times 4 (`erl_proc_sig_queue.c:7273-7274`,
    /// `7462`), and the exit then switches out and continues here in its
    /// next turn.
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// assert!(scheduler.handle_exit_signals(&mut process, 3850)?);
    /// ```
    ///
    /// # Errors
    ///
    /// The error of [`Self::port_link_exit`].
    pub(in crate::runtime) fn handle_exit_signals(
        &mut self,
        process: &mut ProcessState,
        reds: i32,
    ) -> Result<bool, BeamError> {
        let id = process.id;
        let limit = SIGNAL_COUNT_FACTOR * reds;
        let mut count = 0;
        let mut walk = process.mailbox.walk();
        while count <= limit
            && let Some(step) = process.mailbox.next_signal(&mut walk)
        {
            count += 1;
            let Step::Signal(node, mut signal) = step else {
                continue;
            };
            process.mailbox.remove(node);
            // The handling takes the signal out of the queue here, so the
            // signal drops at the end of this arm. Its terms go to the release
            // work list first: an ignored signal can own the last root of a
            // large graph, and the drop of that graph would leave the bound of
            // the release step.
            let mut terms = Vec::new();
            signal.take_terms(&mut terms);
            for term in terms {
                self.release.push(Release::Term(term));
            }
            match signal {
                Signal::Monitor { watcher, reference } => {
                    count += 4;
                    let noproc = Term::static_atom("noproc");
                    self.send_monitor_down(Some(id), watcher, reference, noproc);
                }
                Signal::TtyMonitor { tty } => {
                    count += 4;
                    self.resources.borrow_mut::<Ttys>().monitor_down(id, tty);
                }
                Signal::FileMonitor { file } => {
                    count += 4;
                    self.resources.borrow_mut::<Files>().monitor_down(file);
                }
                Signal::PortMonitor { port, reference: _ } => {
                    count += 4;
                    self.tcp_monitor_down(port, id);
                }
                Signal::Link { from } => {
                    let exit = Signal::Exit {
                        from: LinkId::Process(id),
                        reason: Term::static_atom("noproc"),
                        linked: true,
                        normal_kills: false,
                        alias: None,
                    };
                    // A process that does not exist drops the signal.
                    let _ = self.send_signal(Some(id), from, exit);
                }
                Signal::Unlink { from, id: unlink } => {
                    let ack = Signal::UnlinkAck {
                        from: id,
                        id: unlink,
                    };
                    // A process that does not exist drops the signal.
                    let _ = self.send_signal(Some(id), from, ack);
                }
                Signal::PortLink { port } => {
                    self.port_link_exit(port, id, &Term::static_atom("noproc"));
                }
                Signal::GroupLeader {
                    requester,
                    reference,
                    ..
                } => {
                    self.send_message(id, requester, reply(reference, Term::static_atom("badarg")))
                }
                Signal::IsAlive {
                    requester,
                    reference,
                } => self.send_message(id, requester, reply(reference, Term::static_atom("false"))),
                Signal::ProcessInfo {
                    requester,
                    reference,
                    ..
                } => {
                    let undefined = Term::static_atom("undefined");
                    self.send_message(id, requester, reply(reference, undefined));
                }
                Signal::Revoke { capability } => {
                    let signals = self.delegations.revoke(capability);
                    self.send_revocations(Some(id), signals);
                }
                _ => {}
            }
        }
        process.mailbox.end_walk(walk);
        Ok(!process.mailbox.has_middle())
    }

    /// Make messages ready for `loop_rec` of the current process at the end
    /// of its inner part, as `erts_proc_sig_receive_helper`
    /// (`erl_proc_sig_queue.c:7730-7868`) does. With an empty middle part,
    /// a fetch costs [`RECEIVE_FETCH_REDUCTIONS`]; else the process handles
    /// its signals with a limit of 100 reductions and at most the reductions
    /// left. The steps repeat until a message is ready, no message and no
    /// signal is left, a signal ends the process, or the reductions are used
    /// up. The reductions come off the budget at the end.
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// assert_eq!(scheduler.receive_helper()?, Received::Empty);
    /// ```
    ///
    /// # Errors
    ///
    /// An error of [`Scheduler::handle_signals`].
    pub(in crate::runtime) fn receive_helper(&mut self) -> Result<Received, BeamError> {
        // A message of another scheduler waits in the inbox of this one. A
        // running process does not schedule before it fetches its messages,
        // so apply the inbox here, as ERTS fetches its in-queue buffers
        // (`erl_proc_sig_queue.c:7771-7799`).
        self.drain_inbox();
        let process = self.current_process_mut();
        let id = process.id;
        let mut left = process.fcalls;
        let mut consumed = 0;
        let received = loop {
            let process = self.current_process_mut();
            if process.mailbox.has_middle() {
                let max_reds = left.max(SIGNAL_HANDLING_REDUCTIONS);
                let reds = SIGNAL_HANDLING_REDUCTIONS;
                let reductions = self.handle_signals(id, reds, max_reds, HandlingPoint::Running)?;
                consumed += reductions;
                left -= reductions;
                let process = self.current_process_mut();
                if process.exiting.is_some() {
                    break Received::Exited;
                }
            } else {
                consumed += RECEIVE_FETCH_REDUCTIONS;
                left -= RECEIVE_FETCH_REDUCTIONS;
                process.mailbox.fetch();
                if process.mailbox.current().is_none() && !process.mailbox.has_middle() {
                    break Received::Empty;
                }
            }
            if self.current_process_mut().mailbox.current().is_some() {
                break Received::Message;
            }
            if left <= 0 {
                break Received::Yield;
            }
        };
        self.current_process_mut().fcalls -= consumed;
        Ok(received)
    }

    /// Handle the signals of the current process up to a flush signal, as
    /// `bif_handle_signals_return` (`bif.c:5452-5495`) and
    /// `pi_maybe_flush_signals` (`erl_bif_info.c:1198-1268`) do. The first
    /// call sends the flush signal to the process and fetches
    /// (`erts_proc_sig_init_flush_signals`, `erl_proc_sig_queue.c:3520-3557`).
    /// The handling uses the reductions of the BIF and repeats until it
    /// reaches the flush signal or the reductions are used up. A flush that
    /// a handling at schedule-in finished is done at once.
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// assert_eq!(scheduler.flush_signals()?, Flushed::Done);
    /// ```
    ///
    /// # Errors
    ///
    /// An error of [`Scheduler::handle_signals`].
    pub(in crate::runtime) fn flush_signals(&mut self) -> Result<Flushed, BeamError> {
        let process = self.current_process_mut();
        let id = process.id;
        if process.flush == Flush::None {
            process.mailbox.push_signal(Signal::Flush);
            process.mailbox.fetch();
            process.flush = Flush::Flushing;
        }
        let mut left = process.fcalls;
        loop {
            let process = self.current_process_mut();
            if process.flush == Flush::Flushed {
                process.flush = Flush::None;
                return Ok(Flushed::Done);
            }
            let reductions = self.handle_signals(id, left, left, HandlingPoint::Running)?;
            let process = self.current_process_mut();
            process.bump_reductions(reductions);
            if let Some(reason) = process.exiting.clone() {
                return Ok(Flushed::Exited(reason));
            }
            left -= reductions;
            if left <= 0 && process.flush != Flush::Flushed {
                return Ok(Flushed::Yield);
            }
        }
    }

    /// Handle the signals of the current process until a signal ends it, as
    /// `erts_internal_await_exit_trap` (`bif.c:1698-1716`) does after
    /// `exit(self(), Reason)`. Return the exit reason, or `None` when the
    /// reductions are used up first and the BIF runs again in the next turn.
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// assert_eq!(scheduler.await_exit()?, Some(Term::static_atom("killed")));
    /// ```
    ///
    /// # Errors
    ///
    /// An error of [`Scheduler::handle_signals`].
    pub(in crate::runtime) fn await_exit(&mut self) -> Result<Option<Term>, BeamError> {
        let process = self.current_process_mut();
        let (id, left) = (process.id, process.fcalls);
        let reductions = self.handle_signals(id, left, left, HandlingPoint::Running)?;
        let process = self.current_process_mut();
        process.bump_reductions(reductions);
        Ok(process.exiting.clone())
    }
}

/// The fields of [`Signal::Exit`].
struct ExitSignal {
    /// The sender.
    from: LinkId,
    /// The exit reason.
    reason: Term,
    /// True for a link exit signal.
    linked: bool,
    /// True when `normal` also ends the receiver.
    normal_kills: bool,
    /// The alias that the signal went through.
    alias: Option<Reference>,
}

/// Return the monitor signal of `monitor`.
///
/// # Examples
///
/// ```rust,ignore
/// let signal = monitor_signal(Monitor { watcher, reference });
/// ```
pub(super) fn monitor_signal(monitor: Monitor) -> Signal {
    Signal::Monitor {
        watcher: monitor.watcher,
        reference: monitor.reference,
    }
}

/// Return the reply `{Reference, Result}` of a request signal.
///
/// # Examples
///
/// ```rust,ignore
/// let message = reply(reference, Term::static_atom("true"));
/// ```
fn reply(reference: Reference, result: Term) -> Term {
    Term::tuple(vec![Term::Reference(reference), result])
}

/// Return true for a term that ERTS keeps in one word (`is_immed`): an
/// atom, a small integer, a local pid, or `[]`. A monitor down signal with
/// such a reason uses the monitor itself (`erl_proc_sig_queue.c:2826-2840`).
///
/// # Examples
///
/// ```rust,ignore
/// assert!(is_immediate(&Term::static_atom("normal")));
/// assert!(!is_immediate(&Term::tuple(vec![])));
/// ```
fn is_immediate(term: &Term) -> bool {
    match term {
        Term::Atom(_) | Term::Pid(_) => true,
        _ => term.is_nil() || term.small_integer().is_some(),
    }
}

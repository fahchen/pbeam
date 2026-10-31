//! Open, poll, and close the ports of the node, as the port operations of
//! `io.c` and the spawn driver (`sys/unix/sys_drivers.c`) do on one
//! scheduler.
//!
//! ERTS runs a port operation at once when no other thread holds the port
//! (`try_imm_drv_call`, `io.c:923-1002`), else it schedules a port task.
//! PBeam runs each operation at once under the lock of its port, on the
//! scheduler of the caller. The IO poll reads
//! the pipes of the ports only when no process can run (see
//! [`Scheduler::wait_input`]).

use super::{LinkId, LinkState, Scheduler, tcp_inet::StopNotice};
use crate::{
    error::{BeamError, error},
    runtime::{
        port::{self, Driver, Port, PortEvent, inet::Inet},
        process::MonitorLinkTree,
        signal::Signal,
    },
    term::{PortId, ProcessId, Reference, Term},
};

/// What a port does at the end of file of its program.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(in crate::runtime) enum EndOfFile {
    /// The port has `eof`: it sends `{Port, eof}` and stays open.
    Soft,
    /// The port closes with the reason `normal`.
    Close,
}

/// Whether an exit of a port keeps a port that has queued output in the
/// `CLOSING` state.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ExitMode {
    /// Flush the queued output first (`flush_port`).
    Flush,
    /// Close at once.
    Immediate,
}

impl Scheduler {
    /// Open a port with the name `name` and the driver `driver`, owned by
    /// and linked to the current process, as `erts_internal_open_port_2` does
    /// (`erl_bif_port.c:60-130`). The link goes into both the port and the
    /// link tree of the process at once, as `erts_link_tree_insert` does.
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// let spawn = Spawn { exit_status: true, soft_eof: false, packets: Packets::new(0), binary: true, lines: None, parallelism: false, program: Program::Child(child), input: Input::Open };
    /// let port = scheduler.open_port(b"/bin/sh".to_vec(), Driver::Spawn(spawn))?;
    /// assert_eq!(port, PortId(0));
    /// ```
    ///
    /// # Errors
    ///
    /// The port number overflows.
    pub(in crate::runtime) fn open_port(
        &mut self,
        name: Vec<u8>,
        driver: Driver,
    ) -> Result<PortId, BeamError> {
        let owner = self.current_process_mut().id;
        self.open_port_for(owner, name, driver)
    }

    /// Open a port with the name `name` and the driver `driver`, owned by
    /// and linked to the process `owner`, which exists (see
    /// [`Self::open_port`]).
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// let port = scheduler.open_port_for(owner, b"tcp_inet".to_vec(), driver)?;
    /// ```
    ///
    /// # Errors
    ///
    /// The port number overflows.
    pub(super) fn open_port_for(
        &mut self,
        owner: ProcessId,
        name: Vec<u8>,
        driver: Driver,
    ) -> Result<PortId, BeamError> {
        let mut links = MonitorLinkTree::default();
        links.insert(owner, ());
        // The identifiers belong to the node, so a port of one scheduler
        // never compares equal to a port of another.
        let id = self.node.next_port()?;
        let port = Port {
            id,
            owner,
            links,
            name,
            data: Term::static_atom("undefined"),
            input: 0,
            output: 0,
            driver,
            monitors: Vec::new(),
            closing: false,
            send_closed: false,
            scheduler: self.index,
            poll: std::sync::Arc::clone(&self.node.poll),
        };
        self.node.ports().insert_at(id, port);
        super::poll::NodePoll::start(&self.node);
        // An owner of another scheduler, which an accept of this scheduler
        // can give, gets the link signal, as `driver_create_port` sends it
        // (io.c:837-844).
        match self.process_mut(owner) {
            Some(process) => {
                process.links.insert(LinkId::Port(id), LinkState::ACTIVE);
            }
            None => {
                let _ = self.send_signal(None, owner, Signal::PortLink { port: id });
            }
        }
        Ok(id)
    }

    /// Handle the fd `fd` of a port that the IO poll found ready, as
    /// `ready_input` and `port_inp_failure` do (`sys_drivers.c:1205-1347`):
    ///
    /// - Output of the child goes to the owner as `{Port, {data, Bytes}}`
    ///   (`driver_output` in binary mode), and adds to the input count.
    /// - With `{packet, N}`, each complete packet goes to the owner as one
    ///   message, and its data adds to the input count.
    /// - The end of file closes the port with the reason `normal`
    ///   (`driver_failure_eof`), or sends `{Port, eof}` with `eof` (see
    ///   [`Self::port_end_of_file`]). With `exit_status`, the port first
    ///   waits for the exit of the child (`dd->alive == 1`), and then sends
    ///   `{Port, {exit_status, Status}}` to the owner before the end of
    ///   file (`driver_report_exit`).
    /// - An active `udp_inet` socket sends its datagrams to the owner (see
    ///   `Inet::receive_active_datagrams`).
    ///
    /// An fd that no port waits for changes nothing.
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// scheduler.port_ready(child.output_fd())?;
    /// ```
    ///
    /// # Errors
    ///
    /// A port with `exit_status` whose child the host reaped returns a
    /// runtime error: PBeam has no exit status for it after
    /// `os:set_signal(sigchld, ignore)`. The errors of `Scheduler::send_events`.
    pub(in crate::runtime) fn port_ready(&mut self, fd: libc::c_int) -> Result<(), BeamError> {
        let handles = self.node.ports().handles(self.index);
        if let Some(listen) = port::inet_port(&handles, fd, Inet::accept_fd) {
            return self.tcp_accept_ready(listen);
        }
        if let Some(port) = port::inet_port(&handles, fd, Inet::receive_fd) {
            return self.tcp_step(port, |udp| Some(udp.receive_active_datagrams()));
        }
        if let Some(port) = port::inet_port(&handles, fd, Inet::stream_receive_fd) {
            return self.tcp_receive_ready(port);
        }
        let Some((id, owner, event, slot)) = port::read(&handles, fd, self.node.poll.reader(fd))
        else {
            return Ok(());
        };
        // The messages go out under the port lock; the lock is released
        // before the port closes at its end of file.
        let (status, (soft, lines)) = match event {
            PortEvent::Data(parts) => {
                for data in parts {
                    self.port_send(owner, port_message(id, "data", data));
                }
                return Ok(());
            }
            PortEvent::EndOfFile(end) => (None, end),
            PortEvent::ExitStatus(Some(status), end) => (Some(status), end),
            PortEvent::ExitStatus(None, _) => {
                return Err(error(
                    "unsupported exit status of a port program after os:set_signal(sigchld, ignore): see \"Ports\" in docs/erts-differences.md",
                ));
            }
        };
        if let Some(status) = status {
            let status = Term::Integer(status.into());
            self.port_send(owner, port_message(id, "exit_status", status));
        }
        for data in lines {
            self.port_send(owner, port_message(id, "data", data));
        }
        drop(slot);
        self.port_end_of_file(
            id,
            if soft {
                EndOfFile::Soft
            } else {
                EndOfFile::Close
            },
        );
        Ok(())
    }

    /// Handle the end of file of the spawn port `id`, after its exit status
    /// and the line that is not complete, as `driver_failure_eof` does
    /// (`io.c:7191-7219`): a port with `eof` ([`EndOfFile::Soft`]) sends `{Port, eof}`
    /// to the owner and stays open, and it reads nothing more. Each other
    /// port closes with the reason `normal`.
    pub(super) fn port_end_of_file(&mut self, id: PortId, mode: EndOfFile) {
        // `{Port, eof}` goes out under the port lock, as in
        // `driver_failure_eof` (io.c:7191-7209).
        let cell = self.node.port(id);
        let slot = cell.as_ref().map(port::lock);
        // A port that another scheduler closed after the read sends nothing.
        let owner = slot
            .as_ref()
            .and_then(|slot| slot.as_ref())
            .map(|port| port.owner);
        if let Some(owner) = owner {
            if mode == EndOfFile::Soft {
                let eof = Term::tuple(vec![Term::Port(id), Term::static_atom("eof")]);
                self.port_send(owner, eof);
            } else {
                drop(slot);
                self.close_port(id, Term::static_atom("normal"));
            }
        }
    }

    /// Write the queued output of the port of the socket `fd`, which the
    /// IO poll found ready for write. An fd of no port with queued output,
    /// for example of a socket that a read of the same wait closed,
    /// changes nothing.
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// scheduler.port_writable(fd)?;
    /// ```
    ///
    /// # Errors
    ///
    /// The error of [`Self::tcp_send_ready`].
    pub(in crate::runtime) fn port_writable(&mut self, fd: libc::c_int) -> Result<(), BeamError> {
        let handles = self.node.ports().handles(self.index);
        if let Some(port) = port::inet_port(&handles, fd, Inet::send_fd) {
            return self.tcp_send_ready(port);
        }
        if let Some(port) = port::spawn_port(&handles, fd) {
            self.spawn_writable(port)?;
        }
        Ok(())
    }

    /// Write the queue of the spawn port `id` to its input pipe, as
    /// `ready_output` does (`sys_drivers.c:1455-1493`): resume the
    /// processes that waited for the busy port when the queue goes below
    /// the low limit (`set_busy_port`, `io.c:5203`), end a closing port
    /// whose queue is empty, and close the port with the `errno` of a
    /// failed write (`driver_failure_posix`).
    ///
    /// # Errors
    ///
    /// A new atom in a full atom table gives the error of `AtomTable::put`.
    fn spawn_writable(&mut self, id: PortId) -> Result<(), BeamError> {
        let cell = self.node.port(id);
        let mut slot = cell.as_ref().map(port::lock);
        // The port was found with its queue under the table lock, and only
        // this scheduler writes the queue.
        let mut flushed = Ok(Vec::new());
        if let Some(Driver::Spawn(spawn)) = slot
            .as_mut()
            .and_then(|slot| slot.as_mut())
            .map(|port| &mut port.driver)
        {
            flushed = spawn.flush();
        }
        drop(slot);
        match flushed {
            Ok(resumed) => {
                for process in resumed {
                    self.resume(process);
                }
                self.tcp_flushed(id);
            }
            Err(errno) => {
                let reason = Term::Atom(
                    self.atoms
                        .put(&crate::runtime::host::errno::errno_id(errno))?,
                );
                // `driver_failure_posix` drops the queue and ends the port
                // at once (sys_drivers.c:1482-1490).
                self.fail_port(id, reason);
            }
        }
        Ok(())
    }

    /// Close the open port `id` with the exit reason `reason`, as
    /// `erts_deliver_port_exit` and `terminate_port` do
    /// (`io.c:3698-3790`, `3841-3950`): the port leaves the table, each
    /// process linked to the port gets a link exit signal from the port
    /// with the reason, `killed` for `kill` (`link_port_exit`,
    /// `io.c:3802-3813`), each monitor of the port gets its `DOWN` signal,
    /// and then the driver stops (see [`Self::stop_port`]) and the pipes of
    /// the port close. With a reason other than `kill`, a `tcp_inet` port
    /// whose queue holds bytes goes to the `CLOSING` state instead of the
    /// stop (see [`Inet::flushes`]; io.c:3939-3948): its socket writes the
    /// queue, and the port terminates when the queue is empty (see
    /// `tcp_flushed`). With `linger {true, 0}`, the port terminates at once
    /// in the `CLOSING` state, so the stop sends no message.
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// scheduler.close_port(port, Term::static_atom("normal"));
    /// ```
    pub(in crate::runtime) fn close_port(&mut self, id: PortId, reason: Term) -> bool {
        self.exit_port(id, reason, ExitMode::Flush, None)
    }

    /// Close the open port `id` for a `{Pid, close}` message, as
    /// [`Self::close_port`] does with the reason `normal`, and send
    /// `{Port, closed}` to the owner when the port terminates
    /// (`ERTS_PORT_SFLG_SEND_CLOSED`, io.c:3713-3719, 3753-3758). A port
    /// that flushes its queue sends it after the flush. The close checks
    /// under the port lock that `sender` still owns the port (io.c:2182-2198).
    /// Return true when this close closed an open port.
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// scheduler.close_port_replying(port, owner);
    /// ```
    pub(in crate::runtime) fn close_port_replying(
        &mut self,
        id: PortId,
        sender: ProcessId,
    ) -> bool {
        self.exit_port(
            id,
            Term::static_atom("normal"),
            ExitMode::Flush,
            Some(sender),
        )
    }

    /// Close the open port `id` with the reason `reason` from its driver,
    /// as `driver_failure_term` does (io.c:7191-7225): it empties the queue
    /// of the port before `erts_deliver_port_exit`, so the port terminates
    /// at once and the close of the socket drops the queue. Else as
    /// [`Self::close_port`].
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// scheduler.fail_port(port, Term::static_atom("einval"));
    /// ```
    pub(in crate::runtime) fn fail_port(&mut self, id: PortId, reason: Term) {
        let _ = self.exit_port(id, reason, ExitMode::Immediate, None);
    }

    /// Close the open port `id` with the reason `reason` (see
    /// [`Self::close_port`]), and keep a port that flushes in the `CLOSING`
    /// state only with [`ExitMode::Flush`].
    /// Return true when this exit closed an open port, false for a port
    /// that closed before or is in the `CLOSING` state, so that one of two
    /// concurrent `erlang:port_close/1` calls gives `badarg`
    /// (io.c:2241-2290). With `closer`, the exit closes the port only when
    /// `closer` owns it, and the port sends `{Port, closed}` when it
    /// terminates.
    fn exit_port(
        &mut self,
        id: PortId,
        reason: Term,
        mode: ExitMode,
        closer: Option<ProcessId>,
    ) -> bool {
        // The port lock comes first. The table lock is taken only after the
        // port lock is released, to remove the port.
        let cell = self.node.port(id);
        let mut slot = cell.as_ref().map(port::lock);
        // A port that closed before, also on another scheduler, sends no
        // second exit.
        let Some(port) = slot.as_mut().and_then(|slot| slot.as_mut()) else {
            return false;
        };
        // A close of a port in the `CLOSING` state changes nothing
        // (io.c:3870-3874).
        if mode == ExitMode::Flush && port.closing {
            return false;
        }
        // A close message from a process that no longer owns the port sends
        // the exit signal `badsig` to the owner (io.c:2182-2184).
        if closer.is_some_and(|closer| closer != port.owner) {
            let owner = port.owner;
            drop(slot);
            let badsig = Signal::Exit {
                from: LinkId::Port(id),
                reason: Term::static_atom("badsig"),
                linked: false,
                normal_kills: false,
                alias: None,
            };
            // An owner that does not exist drops the signal.
            let _ = self.send_signal(None, owner, badsig);
            return false;
        }
        // The sends that wait behind a busy port end before the exit
        // signals: ERTS aborts their suspended `erlang:port_command/3` (see
        // `Inet::end_waiters`). A port in the `CLOSING` state has none.
        if let Driver::Inet(tcp) = &mut port.driver {
            let waiters = tcp.end_waiters();
            self.send_events(id, port, waiters)
                .expect("the reply of a waiting send makes no new atom");
        }
        let open = !port.closing;
        port.send_closed |= closer.is_some();
        // `erts_deliver_port_exit` sets `CLOSING` for each queue that holds
        // bytes before `flush_port` (io.c:3939-3948). With `linger {true,
        // 0}`, `tcp_inet_flush` drops the queue and the port terminates with
        // `CLOSING` set, so its stop sends no message (io.c:3659-3721).
        // A spawn port with a queue also waits for its queue
        // (`fd_flush`, sys_drivers.c:1468-1469).
        let queued = mode == ExitMode::Flush
            && reason != Term::static_atom("kill")
            && match &port.driver {
                Driver::Inet(tcp) => tcp.has_output(),
                Driver::Spawn(spawn) => spawn.queued_fd().is_some(),
            };
        let flushes = queued
            && !port.closing
            && match &port.driver {
                Driver::Inet(tcp) => tcp.flushes(),
                Driver::Spawn(_) => true,
            };
        let links = std::mem::take(&mut port.links);
        let monitors = std::mem::take(&mut port.monitors);
        // The exit ends the busy state and resumes each suspended sender,
        // whose command then finds no open port (`set_busy_port`,
        // io.c:3895).
        let suspended = match &mut port.driver {
            Driver::Spawn(spawn) => spawn.queue.unbusy(),
            Driver::Inet(_) => Vec::new(),
        };
        // The port loses its name under its lock, before its exit signals
        // (`erts_deliver_port_exit`, io.c:3904-3905).
        self.node.remove_port_name(id);
        // The exit of a port controller ends its connection
        // (`erts_deliver_port_exit`, io.c:3910-3916).
        let ended = self
            .node
            .dist()
            .controller_exits(super::dist_table::Controller::Port(id));
        if let Some(ended) = ended {
            self.dist_connection_down(&ended);
        }
        port.closing |= queued;
        let serving = port.scheduler;
        let port = slot
            .as_mut()
            .filter(|_| !flushes)
            .and_then(|slot| slot.take());
        drop(slot);
        if !flushes {
            self.node.ports().remove(id);
        }
        for process in suspended {
            self.resume(process);
        }
        // The scheduler that serves the port waits for its fds: wake it, so
        // that it waits again without the fds of the closed port.
        if serving != self.index {
            self.node.wake_scheduler(serving);
        }
        let reason = if reason == Term::static_atom("kill") {
            Term::static_atom("killed")
        } else {
            reason
        };
        // `erts_deliver_port_exit` walks the link tree in post-order
        // (`io.c:3922-3925`, `erl_rbtree.h:1343-1458`).
        for (linked, ()) in links.post_order() {
            let exit = Signal::Exit {
                from: LinkId::Port(id),
                reason: reason.clone(),
                linked: true,
                normal_kills: false,
                alias: None,
            };
            // A linked process that does not exist drops the signal.
            let _ = self.send_signal(None, *linked, exit);
        }
        self.port_monitors_down(&monitors, &reason);
        if let Some(port) = port {
            self.stop_port(id, port);
        }
        open
    }

    /// Close the open port `id` with the reason `reason` from its driver,
    /// as `driver_exit` does (io.c:7228-7249): the port first unlinks its
    /// owner (`unlink_proc`, io.c:2889-2909), so only the other linked
    /// processes get the link exit signal, and then fails (see
    /// [`Self::fail_port`]). The owner removes its side of
    /// the link when it handles the unlink signal [`Signal::PortUnlink`],
    /// after a link signal that `erlang:port_connect/2` queued before.
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// scheduler.driver_exit(port, Term::static_atom("normal"));
    /// ```
    pub(super) fn driver_exit(&mut self, id: PortId, reason: Term) {
        let Some((owner, linked)) = self.node.with_port(id, |port| {
            let owner = port.owner;
            (owner, port.links.remove(&owner).is_some())
        }) else {
            return;
        };
        if linked {
            // An owner that does not exist drops the signal.
            let _ = self.send_signal(None, owner, Signal::PortUnlink { port: id });
        }
        self.fail_port(id, reason);
    }

    /// Link the current process to the port `id`, as `link_opt` does for a
    /// port (`bif.c:186-232`), and return false when the port is not open
    /// and the process does not trap exits (`noproc`). A trapping process
    /// gets the link exit signal `noproc` from a port that is not open. A
    /// link that exists changes nothing.
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// assert!(scheduler.link_port(port));
    /// ```
    pub(in crate::runtime) fn link_port(&mut self, id: PortId) -> bool {
        let current = self.current_process_mut().id;
        let linked = self
            .node
            .with_port(id, |port| {
                (!port.closing).then(|| port.links.insert(current, ()))
            })
            .flatten()
            .is_some();
        if !linked {
            if !self.current_process_mut().trap_exit {
                return false;
            }
            let exit = Signal::Exit {
                from: LinkId::Port(id),
                reason: Term::static_atom("noproc"),
                linked: true,
                normal_kills: false,
                alias: None,
            };
            self.current_process_mut()
                .links
                .insert(LinkId::Port(id), LinkState::ACTIVE);
            self.send_signal(None, current, exit)
                .expect("the current process gets the exit signal");
            return true;
        }
        self.current_process_mut()
            .links
            .insert(LinkId::Port(id), LinkState::ACTIVE);
        true
    }

    /// Unlink the current process from the port `id`, as `unlink_1` does
    /// for a port (`bif.c:1252-1287`). The port runs the unlink at once, so
    /// both sides of the link go at once. A port without a link to the
    /// process changes nothing.
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// scheduler.unlink_port(port);
    /// ```
    pub(in crate::runtime) fn unlink_port(&mut self, id: PortId) {
        let current = self.current_process_mut().id;
        if self.current_process_mut().links.get(&LinkId::Port(id)) != Some(&LinkState::ACTIVE) {
            return;
        }
        self.current_process_mut().links.remove(&LinkId::Port(id));
        self.node.with_port(id, |port| port.links.remove(&current));
    }

    /// Make `pid` the owner of the open port `id`, as the BIF operation of
    /// `set_port_connected` does (io.c:2355-2400): a port without a link to
    /// `pid` adds the link and sends the link signal [`Signal::PortLink`],
    /// and the old owner keeps its link. Return false when `pid` does not
    /// exist, as the link signal to it fails (`ERTS_PORT_OP_DROPPED`). A
    /// process that exits before it handles the signal closes the port
    /// with `noproc`.
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// assert!(scheduler.connect_port(port, handler));
    /// ```
    pub(in crate::runtime) fn connect_port(&mut self, id: PortId, pid: ProcessId) -> bool {
        // The port side of the link goes in before the link signal, as
        // `erts_port_connect` does (`io.c:2367-2375`): an exit of the new
        // owner after the signal then finds the link.
        // One lock of the port covers the link, the link signal, and the
        // new owner, as in `erts_port_connect` (io.c:2359-2400): a close
        // on another scheduler comes before or after the whole connect.
        let cell = self.node.port(id);
        let mut slot = cell.as_ref().map(port::lock);
        let Some(port) = slot.as_mut().and_then(|slot| slot.as_mut()) else {
            return false;
        };
        let linked = port.links.insert(pid, ()).is_some();
        let link = Signal::PortLink { port: id };
        if !linked && self.send_signal(None, pid, link).is_err() {
            port.links.remove(&pid);
            return false;
        }
        port.owner = pid;
        true
    }

    /// Send the `DOWN` signal of each monitor of `monitors` on a port that
    /// closed with `reason`, after its link exit signal, as
    /// `erts_deliver_port_exit` does with `monitor_port_exit`
    /// (io.c:3922-3936). The watcher gets `{'DOWN', Ref, port, Port,
    /// Reason}`.
    fn port_monitors_down(&mut self, monitors: &[(ProcessId, Reference)], reason: &Term) {
        for &(watcher, reference) in monitors {
            self.send_monitor_down(None, watcher, reference, reason.clone());
        }
    }

    /// Add the monitor `reference` of `watcher` to the open port `id`, or
    /// return false when the port is not open.
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// assert!(scheduler.monitor_port(port, watcher, reference));
    /// ```
    pub(super) fn monitor_port(
        &mut self,
        id: PortId,
        watcher: ProcessId,
        reference: Reference,
    ) -> bool {
        self.node
            .with_port(id, |port| {
                (!port.closing).then(|| port.monitors.push((watcher, reference)))
            })
            .flatten()
            .is_some()
    }

    /// Remove the monitor `reference` from the port `id`, as the demonitor
    /// of a port does (`erts_port_demonitor`, io.c). A port that closed
    /// changes nothing.
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// scheduler.demonitor_port(port, reference);
    /// ```
    pub(super) fn demonitor_port(&mut self, id: PortId, reference: Reference) {
        // A port that closed has no monitor to remove.
        self.node.with_port(id, |port| {
            port.monitors.retain(|&(_, item)| item != reference);
        });
    }

    /// Stop the driver of the port `id` that left the table, after its
    /// link exit signal and its `DOWN` signals, as `terminate_port` calls
    /// the `stop` callback of the driver (`io.c:3746`): a `tcp_inet` port
    /// ends the receive and each accept that waits (see
    /// [`Self::tcp_stop`]). Dropping the port closes its pipes or its
    /// socket.
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// let port = ports.remove(id).unwrap();
    /// scheduler.stop_port(id, port);
    /// ```
    pub(super) fn stop_port(&mut self, id: PortId, port: Port) {
        let (owner, send_closed) = (port.owner, port.send_closed);
        if let Driver::Inet(tcp) = port.driver {
            let notice = if port.closing {
                StopNotice::Silent
            } else {
                StopNotice::Notify
            };
            self.tcp_stop(id, tcp, notice);
        }
        // `terminate_port` sends `{Port, closed}` after the driver stop
        // (io.c:3746-3758).
        if send_closed {
            let closed = Term::tuple(vec![Term::Port(id), Term::static_atom("closed")]);
            self.port_send(owner, closed);
        }
    }

    /// Send the link exit signal of the exiting process `from` to the port
    /// `id`, as `erts_proc_exit_handle_link` and `call_deliver_port_exit`
    /// do (`erl_process.c:14084-14099`, `io.c:2160-2190`): the link goes,
    /// and the port closes with the reason (see [`Self::close_port`]),
    /// except for the reason `normal` from a process that is not the owner
    /// (`erts_deliver_port_exit`, `io.c:3869-3872`). A port that closed
    /// changes nothing. A port without a link to `from` drops the signal
    /// (`ERTS_PORT_OP_DROPPED`, `io.c:2187-2190`): the process can have its
    /// side of the link alone, when it runs `link/1` and `unlink/1` with
    /// the port before it handles the link signal of
    /// `erlang:port_connect/2`.
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// scheduler.port_link_exit(port, owner, &Term::static_atom("normal"));
    /// ```
    pub(in crate::runtime) fn port_link_exit(
        &mut self,
        id: PortId,
        from: ProcessId,
        reason: &Term,
    ) {
        // A port that closed sent its link exit signal to `from`, and a port
        // without the link drops the signal (`io.c:2187-2190`).
        let closes = self.node.with_port(id, |port| {
            port.links.remove(&from).is_some()
                && (*reason != Term::static_atom("normal") || port.owner == from)
        }) == Some(true);
        if closes {
            self.close_port(id, reason.clone());
        }
    }
}

/// Return the message `{Port, {Tag, Value}}` of the port `id`.
///
/// # Examples
///
/// ```rust,ignore
/// let message = port_message(PortId(0), "exit_status", Term::Integer(0));
/// ```
impl Scheduler {
    /// Send the message `term` of a port to its connected process
    /// `target`, on this scheduler or on another (`erts_port_output`
    /// queues the message of the port to the process, `io.c`). A target
    /// that does not exist drops the message.
    pub(in crate::runtime) fn port_send(&mut self, target: ProcessId, term: Term) {
        match self.remote_owner(target) {
            Some(index) if !self.processes.contains_key(target) => {
                self.node.send_message(index, None, term, target, None);
            }
            _ => self.deliver(target, term),
        }
    }
}

fn port_message(id: PortId, tag: &str, value: Term) -> Term {
    Term::tuple(vec![
        Term::Port(id),
        Term::tuple(vec![Term::static_atom(tag), value]),
    ])
}

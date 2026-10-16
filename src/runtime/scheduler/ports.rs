//! Open, poll, and close the ports of the node, as the port operations of
//! `io.c` and the spawn driver (`sys/unix/sys_drivers.c`) do on one
//! scheduler.
//!
//! ERTS runs a port operation at once when no other thread holds the port
//! (`try_imm_drv_call`, `io.c:923-1002`), else it schedules a port task.
//! PBeam has one thread, so each operation runs at once. The IO poll reads
//! the pipes of the ports only when no process can run (see
//! [`Scheduler::wait_input`]).

use super::{LinkId, Scheduler};
use crate::{
    error::{BeamError, error},
    runtime::{
        port::{Driver, Port, PortEvent, Ports, inet},
        process::MonitorLinkTree,
        signal::Signal,
    },
    term::{PortId, ProcessId, Reference, Term},
};

impl Scheduler {
    /// Open a port with the name `name` and the driver `driver`, owned by
    /// and linked to the current process, as `erts_internal_open_port_2` does
    /// (`erl_bif_port.c:60-130`). The link goes into both the port and the
    /// link tree of the process at once, as `erts_link_tree_insert` does.
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// let spawn = Spawn { exit_status: true, child, end_of_file: false };
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
        let port = Port {
            owner,
            links,
            name,
            data: Term::static_atom("undefined"),
            input: 0,
            output: 0,
            driver,
            monitors: Vec::new(),
            closing: false,
        };
        let id = self.resources.borrow_mut::<Ports>().insert(port)?;
        self.process_mut(owner)
            .expect("the owner of a port exists")
            .links
            .insert(LinkId::Port(id), 0);
        Ok(id)
    }

    /// Handle the fd `fd` of a port that the IO poll found ready, as
    /// `ready_input` and `port_inp_failure` do (`sys_drivers.c:1205-1347`):
    ///
    /// - Output of the child goes to the owner as `{Port, {data, Bytes}}`
    ///   (`driver_output` in binary mode), and adds to the input count.
    /// - The end of file closes the port with the reason `normal`
    ///   (`driver_failure_eof`). With `exit_status`, the port first waits
    ///   for the exit of the child (`dd->alive == 1`), and then sends
    ///   `{Port, {exit_status, Status}}` to the owner before it closes
    ///   (`driver_report_exit`).
    /// - A datagram on an active `udp_inet` socket stops the run: PBeam does
    ///   not send `{udp, Socket, Address, Port, Data}` (inet_drv.c:14619-14621).
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
    /// `os:set_signal(sigchld, ignore)`. A ready active `udp_inet` socket
    /// returns a runtime error.
    pub(in crate::runtime) fn port_ready(&mut self, fd: libc::c_int) -> Result<(), BeamError> {
        let ports = self.resources.borrow_mut::<Ports>();
        if let Some(listen) = ports.accepting(fd) {
            return self.tcp_accept_ready(listen);
        }
        if ports.receiving(fd) {
            return Err(error(inet::DATAGRAM_RECEIVE));
        }
        if let Some(port) = ports.stream_receiving(fd) {
            return self.tcp_receive_ready(port);
        }
        let Some((id, event)) = ports.read(fd) else {
            return Ok(());
        };
        let port = ports.get_mut(id).expect("the port of a read is open");
        let owner = port.owner;
        match event {
            PortEvent::Data(bytes) => {
                port.input += bytes.len() as u64;
                self.deliver(owner, port_message(id, "data", Term::binary(bytes)));
            }
            PortEvent::EndOfFile => self.close_port(id, Term::static_atom("normal")),
            PortEvent::ExitStatus(Some(status)) => {
                let status = Term::Integer(status.into());
                self.deliver(owner, port_message(id, "exit_status", status));
                self.close_port(id, Term::static_atom("normal"));
            }
            PortEvent::ExitStatus(None) => {
                return Err(error(
                    "unsupported exit status of a port program after os:set_signal(sigchld, ignore): see \"Ports\" in docs/erts-differences.md",
                ));
            }
        }
        Ok(())
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
        match self.resources.borrow_mut::<Ports>().sending(fd) {
            Some(port) => self.tcp_send_ready(port),
            None => Ok(()),
        }
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
    pub(in crate::runtime) fn close_port(&mut self, id: PortId, reason: Term) {
        self.exit_port(id, reason, true);
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
    pub(super) fn fail_port(&mut self, id: PortId, reason: Term) {
        self.exit_port(id, reason, false);
    }

    /// Close the open port `id` with the reason `reason` (see
    /// [`Self::close_port`]), and keep a port that flushes in the `CLOSING`
    /// state only with `flush`.
    fn exit_port(&mut self, id: PortId, reason: Term, flush: bool) {
        let ports = self.resources.borrow_mut::<Ports>();
        let port = ports.get_mut(id).expect("the port to close is open");
        // `erts_deliver_port_exit` sets `CLOSING` for each queue that holds
        // bytes before `flush_port` (io.c:3939-3948). With `linger {true,
        // 0}`, `tcp_inet_flush` drops the queue and the port terminates with
        // `CLOSING` set, so its stop sends no message (io.c:3659-3721).
        let queued = flush
            && reason != Term::static_atom("kill")
            && matches!(&port.driver, Driver::Inet(tcp) if tcp.has_output());
        let flushes =
            queued && !port.closing && matches!(&port.driver, Driver::Inet(tcp) if tcp.flushes());
        let links = std::mem::take(&mut port.links);
        let monitors = std::mem::take(&mut port.monitors);
        port.closing |= queued;
        let port = (!flushes).then(|| ports.remove(id).expect("the port to close is open"));
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
        let port = self.resources.borrow_mut::<Ports>().get_mut(id);
        let port = port.expect("the port of a driver exit is open");
        let owner = port.owner;
        if port.links.remove(&owner).is_some() {
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
        let ports = self.resources.borrow_mut::<Ports>();
        let Some(port) = ports.get_mut(id).filter(|port| !port.closing) else {
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
            self.current_process_mut().links.insert(LinkId::Port(id), 0);
            self.send_signal(None, current, exit)
                .expect("the current process gets the exit signal");
            return true;
        };
        port.links.insert(current, ());
        self.current_process_mut().links.insert(LinkId::Port(id), 0);
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
        if self.current_process_mut().links.get(&LinkId::Port(id)) != Some(&0) {
            return;
        }
        self.current_process_mut().links.remove(&LinkId::Port(id));
        if let Some(port) = self.resources.borrow_mut::<Ports>().get_mut(id) {
            let _ = port.links.remove(&current);
        }
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
        let port = self.resources.borrow_mut::<Ports>().get_mut(id);
        let port = port.expect("the port to connect is open");
        if port.links.get(&pid).is_none() {
            let link = Signal::PortLink { port: id };
            if self.send_signal(None, pid, link).is_err() {
                return false;
            }
        }
        let port = self.resources.borrow_mut::<Ports>().get_mut(id);
        let port = port.expect("the port to connect is open");
        port.links.insert(pid, ());
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
        let ports = self.resources.borrow_mut::<Ports>();
        ports
            .get_mut(id)
            .filter(|port| !port.closing)
            .map(|port| port.monitors.push((watcher, reference)))
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
        let port = self.resources.borrow_mut::<Ports>().get_mut(id);
        // A port that closed has no monitor to remove.
        port.into_iter()
            .for_each(|port| port.monitors.retain(|&(_, item)| item != reference));
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
        if let Driver::Inet(tcp) = port.driver {
            self.tcp_stop(id, tcp, !port.closing);
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
        let port = self.resources.borrow_mut::<Ports>().get_mut(id);
        // A port that closed sent its link exit signal to `from`, and a port
        // without the link drops the signal (`io.c:2187-2190`).
        let closes = port.is_some_and(|port| {
            port.links.remove(&from).is_some()
                && (*reason != Term::static_atom("normal") || port.owner == from)
        });
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
fn port_message(id: PortId, tag: &str, value: Term) -> Term {
    Term::tuple(vec![
        Term::Port(id),
        Term::tuple(vec![Term::static_atom(tag), value]),
    ])
}
